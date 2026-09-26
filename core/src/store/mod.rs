//! The local store (Section 10): one SQLite database holding the op log,
//! the per-space sync cursor, and the materialized tables, behind a small
//! API that every client path goes through.
//!
//! - [`Store::apply`] takes any op from any device in any order and
//!   converges; it is the only write path to materialized state.
//! - [`Store::commit`] applies a group of local ops in one transaction,
//!   queues them for push, and records an undo entry.
//! - [`Store::op`] stamps a mutation with this device's id and clock.
//! - Query methods read materialized rows; [`Store::search`] uses FTS5.
//! - [`Store::snapshot`] and [`Store::restore`] bootstrap a device.
//!
//! Networking, encryption, and IPC attach on top: the op log's `payload`
//! column holds the encoded op that the sync runner will encrypt and push,
//! and `seq` is the server's per-space sequence once assigned.

mod apply;
mod capture;
pub mod fixture;
mod query;
pub mod schema;
mod snapshot;

use std::path::Path;

use rusqlite::{Connection, OpenFlags};

use crate::hlc::{Hlc, HlcClock, WallClock};
use crate::ids::Id;
use crate::model::EntityType;
use crate::op::{DecodeError, Mutation, Op, SCHEMA_VERSION};
use crate::undo::{Inverse, UndoStack};

pub use capture::{Captured, Completed};
pub use query::{LoggedOp, SpaceState, TaskFilter, TaskOrder};
pub use snapshot::Snapshot;

/// Errors from the store.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    #[error(transparent)]
    Decode(#[from] DecodeError),
    #[error("database schema version {found} is newer than this build supports ({supported})")]
    NewerSchema { found: u32, supported: u32 },
    #[error("snapshot is for space {expected}, not {found}")]
    WrongSpace { expected: Id, found: Id },
    #[error("snapshot format version {0} is not supported")]
    SnapshotVersion(u32),
    #[error(
        "snapshot not ready: {unpushed} op(s) not yet pushed and {ahead_of_cursor} pushed op(s) not yet pulled back; sync first"
    )]
    SnapshotNotReady {
        unpushed: usize,
        ahead_of_cursor: usize,
    },
    #[error("locked: keys have not been unlocked on this device")]
    Locked,
    #[error("no task {0}")]
    NoSuchTask(Id),
}

/// Result alias for store operations.
pub type Result<T> = std::result::Result<T, StoreError>;

/// What happened to an op handed to [`Store::apply`].
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Applied {
    /// Logged and materialized.
    Applied,
    /// Already in the log; nothing changed.
    Duplicate,
    /// Logged but not materialized: a newer schema version or an unknown
    /// mutation kind. [`Store::reapply_skipped`] retries after an update.
    Skipped,
}

impl WallClock for Box<dyn WallClock + Send> {
    fn now_ms(&self) -> u64 {
        (**self).now_ms()
    }
}

/// The device's store. One per device; on desktop only the host holds it.
/// Called after any change to materialized state, from whichever path made
/// it (a local commit, a pulled op, undo, a restore). UIs refresh on it.
pub type ChangeListener = std::sync::Arc<dyn Fn() + Send + Sync>;

pub struct Store {
    conn: Connection,
    clock: HlcClock<Box<dyn WallClock + Send>>,
    undo: UndoStack,
    supported_schema: u32,
    locked: bool,
    on_change: Option<ChangeListener>,
}

impl Store {
    /// Open or create the database at `path` for `device`.
    #[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
    pub fn open(path: impl AsRef<Path>, device: Id) -> Result<Store> {
        Store::open_with_clock(
            Some(path.as_ref()),
            device,
            Box::new(crate::hlc::SystemClock),
        )
    }

    /// An in-memory store, for tests and throwaway work.
    #[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
    pub fn open_in_memory(device: Id) -> Result<Store> {
        Store::open_with_clock(None, device, Box::new(crate::hlc::SystemClock))
    }

    /// Open with an explicit wall clock. `None` opens an in-memory database.
    pub fn open_with_clock(
        path: Option<&Path>,
        device: Id,
        wall: Box<dyn WallClock + Send>,
    ) -> Result<Store> {
        let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX;
        let mut conn = match path {
            Some(path) => Connection::open_with_flags(path, flags)?,
            None => Connection::open_in_memory_with_flags(flags)?,
        };
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "temp_store", "MEMORY")?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        schema::migrate(&mut conn)?;
        Ok(Store {
            conn,
            clock: HlcClock::with_wall(device, wall),
            undo: UndoStack::default(),
            supported_schema: SCHEMA_VERSION,
            locked: false,
            on_change: None,
        })
    }

    /// Register the listener notified after every change; replaces any
    /// earlier one. It runs on the thread that made the change.
    pub fn set_change_listener(&mut self, listener: Option<ChangeListener>) {
        self.on_change = listener;
    }

    fn changed(&self) {
        if let Some(l) = &self.on_change {
            l();
        }
    }

    /// Refuse every read and write of task data until [`unlock`](Self::unlock).
    /// The host locks the store while the device's keys are not unlocked,
    /// so the CLI and the MCP server get [`StoreError::Locked`] instead of
    /// plaintext (Section 3). Metadata such as the cursor stays readable.
    pub fn lock(&mut self) {
        self.locked = true;
    }

    /// Allow data access again.
    pub fn unlock(&mut self) {
        self.locked = false;
    }

    pub fn is_locked(&self) -> bool {
        self.locked
    }

    fn check_unlocked(&self) -> Result<()> {
        if self.locked {
            Err(StoreError::Locked)
        } else {
            Ok(())
        }
    }

    /// The device this store belongs to.
    pub fn device(&self) -> Id {
        self.clock.device()
    }

    /// The highest op `schema_version` this store will materialize.
    /// Defaults to the build's version; tests lower it to simulate an old
    /// client and raise it to simulate the update.
    pub fn supported_schema_version(&self) -> u32 {
        self.supported_schema
    }

    /// See [`supported_schema_version`](Self::supported_schema_version).
    pub fn set_supported_schema_version(&mut self, version: u32) {
        self.supported_schema = version;
    }

    /// Issue a fresh timestamp from this device's clock.
    pub fn now(&mut self) -> Hlc {
        self.clock.tick()
    }

    /// Stamp a mutation as a new op from this device.
    pub fn op(
        &mut self,
        space_id: Id,
        entity_type: EntityType,
        entity_id: Id,
        mutation: Mutation,
    ) -> Op {
        Op {
            schema_version: self.supported_schema.min(SCHEMA_VERSION),
            op_id: Id::new(),
            space_id,
            device_id: self.device(),
            hlc: self.clock.tick(),
            entity_type,
            entity_id,
            mutation,
        }
    }

    /// The op that removes `tag` from `task`, citing every add currently
    /// observed for it. Returns `None` if the task does not carry the tag.
    pub fn remove_tag_op(&mut self, space_id: Id, task: Id, tag: Id) -> Result<Option<Op>> {
        self.check_unlocked()?;
        let observed = query::tag_adds(&self.conn, task, tag)?;
        if observed.is_empty() {
            return Ok(None);
        }
        Ok(Some(self.op(
            space_id,
            EntityType::Task,
            task,
            Mutation::RemoveTag { tag, observed },
        )))
    }

    /// Apply one op of any origin. Idempotent on `(space_id, op_id)`.
    pub fn apply(&mut self, op: &Op) -> Result<Applied> {
        self.check_unlocked()?;
        self.clock.observe(&op.hlc);
        let now = self.clock.current().wall_ms;
        let tx = self.conn.transaction()?;
        let outcome = apply::apply_op(&tx, op, None, self.supported_schema, now)?;
        tx.commit()?;
        if outcome.status == Applied::Applied {
            self.changed();
        }
        Ok(outcome.status)
    }

    /// Apply an op pulled from the server with its assigned `seq`, and
    /// advance the space's cursor.
    pub fn apply_remote(&mut self, op: &Op, seq: u64) -> Result<Applied> {
        self.check_unlocked()?;
        self.clock.observe(&op.hlc);
        let now = self.clock.current().wall_ms;
        let tx = self.conn.transaction()?;
        let outcome = apply::apply_op(&tx, op, Some(seq), self.supported_schema, now)?;
        apply::advance_cursor(&tx, op.space_id, seq, now)?;
        tx.commit()?;
        if outcome.status == Applied::Applied {
            self.changed();
        }
        Ok(outcome.status)
    }

    /// Apply many ops in one transaction with no undo entry: bulk import,
    /// fixtures, or a batch pulled from the server (pass each op's `seq`).
    pub fn apply_batch(&mut self, ops: &[(Op, Option<u64>)]) -> Result<usize> {
        self.check_unlocked()?;
        for (op, _) in ops {
            self.clock.observe(&op.hlc);
        }
        let now = self.clock.current().wall_ms;
        let tx = self.conn.transaction()?;
        let mut applied = 0;
        for (op, seq) in ops {
            let outcome = apply::apply_op(&tx, op, *seq, self.supported_schema, now)?;
            if outcome.status == Applied::Applied {
                applied += 1;
            }
            if let Some(seq) = seq {
                apply::advance_cursor(&tx, op.space_id, *seq, now)?;
            }
        }
        tx.commit()?;
        if applied > 0 {
            self.changed();
        }
        Ok(applied)
    }

    /// Apply a group of local ops as one transaction and one undo step.
    /// The ops are queued for push (no `seq` yet).
    pub fn commit(&mut self, ops: &[Op]) -> Result<()> {
        self.check_unlocked()?;
        let now = self.clock.current().wall_ms;
        let inverses = {
            let tx = self.conn.transaction()?;
            let mut inverses = Vec::new();
            for op in ops {
                let outcome = apply::apply_op(&tx, op, None, self.supported_schema, now)?;
                if let Some(inverse) = outcome.inverse {
                    inverses.push(inverse);
                }
            }
            tx.commit()?;
            inverses.reverse();
            inverses
        };
        if !inverses.is_empty() {
            self.undo.record(inverses);
        }
        self.changed();
        Ok(())
    }

    /// Undo the most recent local step. Returns whether there was one.
    pub fn undo(&mut self) -> Result<bool> {
        self.check_unlocked()?;
        let Some(entry) = self.undo.take_undo() else {
            return Ok(false);
        };
        let redo = self.apply_inverses(&entry)?;
        self.undo.record_redo(redo);
        self.changed();
        Ok(true)
    }

    /// Redo the most recently undone step. Returns whether there was one.
    pub fn redo(&mut self) -> Result<bool> {
        self.check_unlocked()?;
        let Some(entry) = self.undo.take_redo() else {
            return Ok(false);
        };
        let undo = self.apply_inverses(&entry)?;
        self.undo.record_undo(undo);
        self.changed();
        Ok(true)
    }

    /// Whether [`undo`](Self::undo) and [`redo`](Self::redo) have work.
    pub fn can_undo(&self) -> bool {
        self.undo.can_undo()
    }

    pub fn can_redo(&self) -> bool {
        self.undo.can_redo()
    }

    fn apply_inverses(&mut self, entry: &[Inverse]) -> Result<Vec<Inverse>> {
        let ops: Vec<Op> = entry
            .iter()
            .map(|inv| {
                self.op(
                    inv.space_id,
                    inv.entity_type,
                    inv.entity_id,
                    inv.mutation.clone(),
                )
            })
            .collect();
        let now = self.clock.current().wall_ms;
        let tx = self.conn.transaction()?;
        let mut inverses = Vec::new();
        for op in &ops {
            let outcome = apply::apply_op(&tx, op, None, self.supported_schema, now)?;
            if let Some(inverse) = outcome.inverse {
                inverses.push(inverse);
            }
        }
        tx.commit()?;
        inverses.reverse();
        Ok(inverses)
    }

    /// Retry ops that were logged but skipped, after the supported schema
    /// version was raised. Returns how many were materialized.
    pub fn reapply_skipped(&mut self, space_id: Id) -> Result<usize> {
        self.check_unlocked()?;
        let now = self.clock.current().wall_ms;
        let tx = self.conn.transaction()?;
        let count = apply::reapply_skipped(&tx, space_id, self.supported_schema, now)?;
        tx.commit()?;
        if count > 0 {
            self.changed();
        }
        Ok(count)
    }

    /// Local ops not yet acknowledged by the server, oldest first.
    pub fn pending_ops(&self, space_id: Id) -> Result<Vec<Op>> {
        self.check_unlocked()?;
        query::pending_ops(&self.conn, space_id)
    }

    /// Record the `seq` the server assigned to pushed ops. The cursor does
    /// not move: it tracks what has been pulled, and other devices' ops may
    /// sit between the cursor and a freshly assigned `seq`. The next pull
    /// returns this device's own ops too, which apply as duplicates.
    pub fn mark_pushed(&mut self, space_id: Id, assigned: &[(Id, u64)]) -> Result<()> {
        let tx = self.conn.transaction()?;
        for (op_id, seq) in assigned {
            tx.execute(
                "UPDATE ops SET seq = ?3 WHERE space_id = ?1 AND op_id = ?2",
                rusqlite::params![space_id, op_id, *seq as i64],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// The highest server `seq` this device has pulled for the space.
    pub fn cursor(&self, space_id: Id) -> Result<u64> {
        query::cursor(&self.conn, space_id)
    }

    /// How many ops the log holds for the space.
    pub fn op_count(&self, space_id: Id) -> Result<u64> {
        query::op_count(&self.conn, space_id)
    }

    /// The most recent ops in the log, newest first, with their sync state.
    pub fn recent_ops(&self, space_id: Id, limit: usize) -> Result<Vec<LoggedOp>> {
        self.check_unlocked()?;
        query::recent_ops(&self.conn, space_id, limit)
    }

    /// How many ops are not yet acknowledged by the server.
    pub fn pending_count(&self, space_id: Id) -> Result<u64> {
        query::pending_count(&self.conn, space_id)
    }

    /// Fold the write-ahead log into the database. The host runs this on a
    /// background thread so writes never pay for it.
    pub fn checkpoint(&self) -> Result<()> {
        self.conn.execute_batch("PRAGMA wal_checkpoint(PASSIVE)")?;
        Ok(())
    }

    /// Read one op back from the log.
    pub fn logged_op(&self, space_id: Id, op_id: Id) -> Result<Option<Op>> {
        self.check_unlocked()?;
        query::logged_op(&self.conn, space_id, op_id)
    }

    // ---- Queries ---------------------------------------------------------

    pub fn space(&self, id: Id) -> Result<Option<crate::model::Space>> {
        self.check_unlocked()?;
        query::space(&self.conn, id)
    }

    pub fn list(&self, id: Id) -> Result<Option<crate::model::List>> {
        self.check_unlocked()?;
        query::list(&self.conn, id)
    }

    /// Live lists in a space in manual order.
    pub fn lists(&self, space_id: Id) -> Result<Vec<crate::model::List>> {
        self.check_unlocked()?;
        query::lists(&self.conn, space_id)
    }

    pub fn tag(&self, id: Id) -> Result<Option<crate::model::Tag>> {
        self.check_unlocked()?;
        query::tag(&self.conn, id)
    }

    /// Live tags in a space by name.
    pub fn tags(&self, space_id: Id) -> Result<Vec<crate::model::Tag>> {
        self.check_unlocked()?;
        query::tags(&self.conn, space_id)
    }

    pub fn task(&self, id: Id) -> Result<Option<crate::model::Task>> {
        self.check_unlocked()?;
        query::task(&self.conn, id)
    }

    /// Live tasks matching the filter, in manual order.
    pub fn tasks(&self, space_id: Id, filter: &TaskFilter) -> Result<Vec<crate::model::Task>> {
        self.check_unlocked()?;
        query::tasks(&self.conn, space_id, filter)
    }

    /// Full-text search over live task titles and notes; every whitespace
    /// separated term is a prefix, so results update on each keystroke.
    pub fn search(
        &self,
        space_id: Id,
        text: &str,
        limit: usize,
    ) -> Result<Vec<crate::model::Task>> {
        self.check_unlocked()?;
        query::search(&self.conn, space_id, text, limit)
    }

    /// Everything materialized for a space, including tombstoned rows,
    /// sorted by id. Two stores that converged have equal states.
    pub fn space_state(&self, space_id: Id) -> Result<SpaceState> {
        self.check_unlocked()?;
        query::space_state(&self.conn, space_id)
    }

    // ---- Snapshots -------------------------------------------------------

    /// Export the space's materialized state at the current cursor.
    ///
    /// Only allowed when every local op has been pushed and pulled back, so
    /// the state is exactly a replay of the server's log through the
    /// cursor. Otherwise returns [`StoreError::SnapshotNotReady`]; the sync
    /// runner pushes, pulls, and retries.
    pub fn snapshot(&self, space_id: Id) -> Result<Snapshot> {
        self.check_unlocked()?;
        let (unpushed, ahead_of_cursor) = query::ops_not_covered_by_cursor(&self.conn, space_id)?;
        if unpushed > 0 || ahead_of_cursor > 0 {
            return Err(StoreError::SnapshotNotReady {
                unpushed,
                ahead_of_cursor,
            });
        }
        snapshot::export(&self.conn, space_id)
    }

    /// Export without the readiness check. For tests and the debug harness;
    /// never uploaded.
    #[doc(hidden)]
    pub fn snapshot_unchecked(&self, space_id: Id) -> Result<Snapshot> {
        self.check_unlocked()?;
        snapshot::export(&self.conn, space_id)
    }

    /// Replace the space's materialized state with a snapshot and set the
    /// cursor to the snapshot's `seq`. Ops after that `seq` are then applied
    /// with [`apply_remote`](Self::apply_remote).
    pub fn restore(&mut self, snapshot: &Snapshot) -> Result<()> {
        self.check_unlocked()?;
        let now = self.clock.current().wall_ms;
        let tx = self.conn.transaction()?;
        snapshot::import(&tx, snapshot, now)?;
        tx.commit()?;
        self.changed();
        Ok(())
    }

    /// Direct access for tests and the debug harness.
    #[doc(hidden)]
    pub fn connection(&self) -> &Connection {
        &self.conn
    }

    /// The query plan SQLite chooses for [`search`](Self::search), one line
    /// per step. Tests pin the plan so a SQLite upgrade cannot quietly turn
    /// the FTS scan into one probe per task.
    #[doc(hidden)]
    pub fn search_plan(&self) -> Result<Vec<String>> {
        query::search_plan(&self.conn)
    }
}
