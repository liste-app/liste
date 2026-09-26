//! Shared helpers for the core suite (Section 15).

#![allow(dead_code)]

use std::cell::Cell;
use std::rc::Rc;

use liste_core::hlc::WallClock;
use liste_core::ids::Id;
use liste_core::model::{EntityType, Field, Value};
use liste_core::op::{Mutation, Op};
use liste_core::store::{Applied, Store};

/// Marks a suite case that is specified but not yet implemented. Such cases
/// are `#[ignore]`d so `cargo test` stays green; `just test-pending` runs
/// them and shows what is still missing. Remove the attribute as each
/// behavior lands.
pub fn pending(suite: &str, case: &str) -> ! {
    panic!(
        "{suite}: `{case}` is specified in docs/ARCHITECTURE.md Section 15 but not implemented yet"
    )
}

/// A fresh directory under the system temp dir for file-backed stores.
pub fn temp_dir(label: &str) -> std::path::PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("liste-{label}-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A wall clock the test advances by hand.
#[derive(Clone)]
pub struct FakeWall(pub Rc<Cell<u64>>);

impl WallClock for FakeWall {
    fn now_ms(&self) -> u64 {
        self.0.get()
    }
}

/// A simulated device: an in-memory store with a controllable wall clock.
pub struct Device {
    pub store: Store,
    pub wall: Rc<Cell<u64>>,
}

impl Device {
    /// Device `n` with its wall clock at `start_ms`.
    pub fn new(n: u8, start_ms: u64) -> Device {
        let wall = Rc::new(Cell::new(start_ms));
        let store = Store::open_with_clock(
            None,
            Id::from_bytes([n; 16]),
            Box::new(FakeWall(wall.clone())),
        )
        .unwrap();
        Device { store, wall }
    }

    pub fn advance(&self, ms: u64) {
        self.wall.set(self.wall.get() + ms);
    }

    pub fn set(
        &mut self,
        space: Id,
        ty: EntityType,
        id: Id,
        field: Field,
        value: impl Into<Value>,
    ) -> Op {
        self.store.op(
            space,
            ty,
            id,
            Mutation::Set {
                field,
                value: value.into(),
            },
        )
    }

    /// Commit one local op and return it.
    pub fn commit_one(&mut self, op: Op) -> Op {
        self.store.commit(std::slice::from_ref(&op)).unwrap();
        op
    }
}

/// The server's view of one space: ops in arrival order, each with its seq.
#[derive(Default)]
pub struct Server {
    pub log: Vec<(Op, u64)>,
}

impl Server {
    /// Accept ops, assigning increasing seqs; duplicates are ignored.
    pub fn push(&mut self, ops: &[Op]) -> Vec<(Id, u64)> {
        let mut assigned = Vec::new();
        for op in ops {
            if let Some((_, seq)) = self.log.iter().find(|(o, _)| o.op_id == op.op_id) {
                assigned.push((op.op_id, *seq));
                continue;
            }
            let seq = self.log.len() as u64 + 1;
            self.log.push((op.clone(), seq));
            assigned.push((op.op_id, seq));
        }
        assigned
    }

    /// Everything after `seq`.
    pub fn pull(&self, after: u64) -> Vec<(Op, u64)> {
        self.log
            .iter()
            .filter(|(_, s)| *s > after)
            .cloned()
            .collect()
    }
}

/// Push a device's pending ops, then pull and apply everything it lacks.
pub fn sync(device: &mut Device, server: &mut Server, space: Id) {
    let pending = device.store.pending_ops(space).unwrap();
    let assigned = server.push(&pending);
    device.store.mark_pushed(space, &assigned).unwrap();
    let cursor = device.store.cursor(space).unwrap();
    for (op, seq) in server.pull(cursor) {
        let status = device.store.apply_remote(&op, seq).unwrap();
        assert_ne!(
            status,
            Applied::Skipped,
            "sync tests use only understood ops"
        );
    }
}

/// Apply ops directly, in the given order, as if received from anywhere.
pub fn apply_all(store: &mut Store, ops: &[Op]) {
    for op in ops {
        store.apply(op).unwrap();
    }
}
