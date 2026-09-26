//! Space snapshots (Section 6): the materialized state of a space at a
//! server `seq`, so a new device bootstraps from snapshot plus tail instead
//! of replaying the whole log.
//!
//! A snapshot carries everything apply needs to keep converging after it:
//! the rows, the per-field clocks, the tag-add tokens, and the add tokens
//! that removes have observed. It does not carry the op log; the tail is
//! pulled from the server. Ops at or below the snapshot's `seq` that arrive
//! anyway are harmless, because apply is idempotent.
//!
//! The position is the space's cursor at export time. Local ops not yet
//! acknowledged by the server are included in the state; they reach the
//! server on the next push and re-apply as no-ops on the new device.

use rusqlite::{Connection, Transaction, params};
use serde::{Deserialize, Serialize};

use super::query::{self, SpaceState};
use super::{Result, StoreError};
use crate::hlc::Hlc;
use crate::ids::Id;
use crate::model::Task;

/// Snapshot encoding version, independent of the op `schema_version`.
pub const SNAPSHOT_VERSION: u32 = 1;

/// One tag on one task, with the add token a remove would cite.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TagAdd {
    pub task_id: Id,
    pub tag_id: Id,
    pub add_id: Id,
}

/// The winning clock for one field of one entity.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldClock {
    pub entity_id: Id,
    pub field: String,
    pub hlc: Hlc,
}

/// A space's materialized state at `seq`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    pub format_version: u32,
    pub space_id: Id,
    pub seq: u64,
    pub state: SpaceState,
    pub tag_adds: Vec<TagAdd>,
    pub removed_adds: Vec<Id>,
    pub clocks: Vec<FieldClock>,
}

impl Snapshot {
    /// Bytes for upload; encrypted by the sync layer before it leaves.
    pub fn encode(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("snapshot serialization cannot fail")
    }

    pub fn decode(bytes: &[u8]) -> Result<Snapshot> {
        let snapshot: Snapshot = serde_json::from_slice(bytes)
            .map_err(|e| StoreError::Decode(crate::op::DecodeError::from(e)))?;
        if snapshot.format_version > SNAPSHOT_VERSION {
            return Err(StoreError::SnapshotVersion(snapshot.format_version));
        }
        Ok(snapshot)
    }
}

pub(super) fn export(conn: &Connection, space_id: Id) -> Result<Snapshot> {
    let state = query::space_state(conn, space_id)?;
    let tag_adds = conn
        .prepare_cached(
            "SELECT tt.task_id, tt.tag_id, tt.add_id FROM task_tags tt
             JOIN tasks t ON t.id = tt.task_id WHERE t.space_id = ?1
             ORDER BY tt.task_id, tt.tag_id, tt.add_id",
        )?
        .query_map(params![space_id], |r| {
            Ok(TagAdd {
                task_id: r.get(0)?,
                tag_id: r.get(1)?,
                add_id: r.get(2)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    let removed_adds = conn
        .prepare_cached("SELECT add_id FROM removed_tag_adds WHERE space_id = ?1 ORDER BY add_id")?
        .query_map(params![space_id], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let clocks = conn
        .prepare_cached(
            "SELECT entity_id, field, hlc_wall, hlc_counter, device_id FROM field_clocks
             WHERE space_id = ?1 ORDER BY entity_id, field",
        )?
        .query_map(params![space_id], |r| {
            Ok(FieldClock {
                entity_id: r.get(0)?,
                field: r.get(1)?,
                hlc: Hlc {
                    wall_ms: r.get::<_, i64>(2)? as u64,
                    counter: r.get(3)?,
                    device: r.get(4)?,
                },
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    Ok(Snapshot {
        format_version: SNAPSHOT_VERSION,
        space_id,
        seq: query::cursor(conn, space_id)?,
        state,
        tag_adds,
        removed_adds,
        clocks,
    })
}

pub(super) fn import(tx: &Transaction, snapshot: &Snapshot, now_ms: u64) -> Result<()> {
    if snapshot.format_version > SNAPSHOT_VERSION {
        return Err(StoreError::SnapshotVersion(snapshot.format_version));
    }
    let space_id = snapshot.space_id;
    for task in &snapshot.state.tasks {
        if task.space_id != space_id {
            return Err(StoreError::WrongSpace {
                expected: space_id,
                found: task.space_id,
            });
        }
    }
    tx.execute(
        "DELETE FROM task_tags WHERE task_id IN (SELECT id FROM tasks WHERE space_id = ?1)",
        params![space_id],
    )?;
    tx.execute("DELETE FROM tasks WHERE space_id = ?1", params![space_id])?;
    tx.execute("DELETE FROM lists WHERE space_id = ?1", params![space_id])?;
    tx.execute("DELETE FROM tags WHERE space_id = ?1", params![space_id])?;
    tx.execute("DELETE FROM filters WHERE space_id = ?1", params![space_id])?;
    tx.execute(
        "DELETE FROM removed_tag_adds WHERE space_id = ?1",
        params![space_id],
    )?;
    tx.execute(
        "DELETE FROM field_clocks WHERE space_id = ?1",
        params![space_id],
    )?;
    tx.execute("DELETE FROM spaces WHERE id = ?1", params![space_id])?;

    if let Some(space) = &snapshot.state.space {
        tx.execute(
            "INSERT INTO spaces (id, kind, created_at, modified_at, deleted_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![space.id, space.kind, space.created_at, space.modified_at, space.deleted_at],
        )?;
    }
    for list in &snapshot.state.lists {
        tx.prepare_cached(
            "INSERT INTO lists (id, space_id, title, position, created_at, modified_at, deleted_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )?
        .execute(params![
            list.id,
            space_id,
            list.title,
            list.position,
            list.created_at,
            list.modified_at,
            list.deleted_at
        ])?;
    }
    for tag in &snapshot.state.tags {
        tx.prepare_cached(
            "INSERT INTO tags (id, space_id, name, created_at, modified_at, deleted_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )?
        .execute(params![
            tag.id,
            space_id,
            tag.name,
            tag.created_at,
            tag.modified_at,
            tag.deleted_at
        ])?;
    }
    for f in &snapshot.state.filters {
        tx.prepare_cached(
            "INSERT INTO filters (id, space_id, name, position, list_id, tag_id, filter_priority,
                filter_status, due_from_day, due_to_day, include_completed, created_at,
                modified_at, deleted_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
        )?
        .execute(params![
            f.id,
            space_id,
            f.name,
            f.position,
            f.list_id,
            f.tag_id,
            f.priority.map(|p| p as i64),
            f.status,
            f.due_from_day,
            f.due_to_day,
            i64::from(f.include_completed),
            f.created_at,
            f.modified_at,
            f.deleted_at
        ])?;
    }
    for task in &snapshot.state.tasks {
        insert_task(tx, task)?;
    }
    super::apply::rebuild_outline(tx, space_id)?;
    for add in &snapshot.tag_adds {
        tx.prepare_cached(
            "INSERT OR IGNORE INTO task_tags (task_id, tag_id, add_id) VALUES (?1, ?2, ?3)",
        )?
        .execute(params![add.task_id, add.tag_id, add.add_id])?;
    }
    for add_id in &snapshot.removed_adds {
        tx.prepare_cached(
            "INSERT OR IGNORE INTO removed_tag_adds (add_id, space_id) VALUES (?1, ?2)",
        )?
        .execute(params![add_id, space_id])?;
    }
    for clock in &snapshot.clocks {
        tx.prepare_cached(
            "INSERT OR REPLACE INTO field_clocks (entity_id, field, space_id, hlc_wall, hlc_counter, device_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )?
        .execute(params![
            clock.entity_id,
            clock.field,
            space_id,
            clock.hlc.wall_ms as i64,
            clock.hlc.counter,
            clock.hlc.device
        ])?;
    }
    tx.execute(
        "INSERT INTO sync_cursor (space_id, last_seq, snapshot_seq, updated_at) VALUES (?1, ?2, ?2, ?3)
         ON CONFLICT (space_id) DO UPDATE SET
            last_seq = max(last_seq, excluded.last_seq),
            snapshot_seq = excluded.snapshot_seq,
            updated_at = excluded.updated_at",
        params![space_id, snapshot.seq as i64, now_ms as i64],
    )?;
    Ok(())
}

fn insert_task(tx: &Transaction, task: &Task) -> Result<()> {
    tx.prepare_cached(
        "INSERT INTO tasks (id, space_id, list_id, parent_id, title, notes, due_at, due_all_day,
            reminder_at, priority, status, completed_at, position, recurrence, created_at,
            modified_at, deleted_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
    )?
    .execute(params![
        task.id,
        task.space_id,
        task.list_id,
        task.parent_id,
        task.title,
        task.notes,
        task.due_at,
        i64::from(task.due_all_day),
        task.reminder_at,
        task.priority as i64,
        task.status,
        task.completed_at,
        task.position,
        task.recurrence,
        task.created_at,
        task.modified_at,
        task.deleted_at
    ])?;
    Ok(())
}
