//! Undo and redo (Section 5) via inverse ops.
//!
//! When a local op is applied, the store computes the op that would restore
//! the field's previous value: a `set` back to the old value, an `undelete`
//! for a `delete`, a `remove_tag` citing the add, or a `delete` when the op
//! created the row. Undo applies those inverses as ordinary new ops, so an
//! undo syncs like any other change and its own inverse becomes the redo.

use crate::ids::Id;
use crate::model::EntityType;
use crate::op::Mutation;

/// A mutation that restores an entity's previous state, stamped when applied.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Inverse {
    pub space_id: Id,
    pub entity_type: EntityType,
    pub entity_id: Id,
    pub mutation: Mutation,
}

/// Per-device undo and redo history. Entries are groups of inverses in the
/// order they must be applied.
#[derive(Debug, Default)]
pub struct UndoStack {
    undo: Vec<Vec<Inverse>>,
    redo: Vec<Vec<Inverse>>,
}

/// Steps kept before the oldest is dropped.
pub const UNDO_LIMIT: usize = 200;

impl UndoStack {
    /// Record a new local step. Clears the redo history.
    pub fn record(&mut self, inverses: Vec<Inverse>) {
        self.redo.clear();
        self.record_undo(inverses);
    }

    pub(crate) fn record_undo(&mut self, inverses: Vec<Inverse>) {
        self.undo.push(inverses);
        if self.undo.len() > UNDO_LIMIT {
            self.undo.remove(0);
        }
    }

    pub(crate) fn record_redo(&mut self, inverses: Vec<Inverse>) {
        self.redo.push(inverses);
    }

    pub(crate) fn take_undo(&mut self) -> Option<Vec<Inverse>> {
        self.undo.pop()
    }

    pub(crate) fn take_redo(&mut self) -> Option<Vec<Inverse>> {
        self.redo.pop()
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
}
