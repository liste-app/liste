//! The data model (Sections 3, 5, and 6).
//!
//! Four entity kinds live in a space: lists, tasks (a subtask is a task
//! with a parent), and tags, plus the space itself. Every field that an op
//! can touch is a [`Field`] with an explicit [`MergeClass`]; the apply
//! function in [`crate::store`] dispatches on that class and nothing else.
//!
//! Interpretations made here, where the document leaves room:
//! - A task's list is optional. A task with no list is in the space's
//!   inbox. This keeps "create task" to one op fewer than requiring a list.
//! - Due dates are a Unix-millisecond instant plus an all-day flag, as two
//!   scalar fields, so the parser and the UI can set either independently.
//! - `status` is free text (default `"open"`) so kanban columns grouped by
//!   status can be user-defined without a schema change. `completed_at`
//!   is separate from status so "done" can be a column and a fact at once.
//! - One `position` key per task orders it wherever manual order applies
//!   (its list, and a kanban column it is dragged within). A second key per
//!   grouping was rejected as larger ops for no v1 benefit.
//! - `modified_at` is not a field. It is derived at apply time as the
//!   largest wall time of any op applied to the entity, so it converges.
//! - The recurrence rule is stored as opaque text here; interpreting it is
//!   the recurrence module's job.

use serde::{Deserialize, Serialize};

use crate::ids::Id;

/// The kinds of entity an op can address.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntityType {
    Space,
    List,
    Task,
    Tag,
}

impl EntityType {
    /// Every entity kind, for iteration.
    pub const ALL: [EntityType; 4] = [
        EntityType::Space,
        EntityType::List,
        EntityType::Task,
        EntityType::Tag,
    ];

    /// The stable text name used in ops and in SQLite.
    pub fn as_str(self) -> &'static str {
        match self {
            EntityType::Space => "space",
            EntityType::List => "list",
            EntityType::Task => "task",
            EntityType::Tag => "tag",
        }
    }
}

/// How concurrent writes to a field are reconciled (Section 6).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum MergeClass {
    /// Last writer wins by `hlc`, per field.
    LwwScalar,
    /// Add-wins observed-remove set.
    AddWinsSet,
    /// Fractional-index string key, last writer wins, rebalanced periodically.
    FractionalIndex,
    /// `deleted_at` tombstone; last writer wins between delete and undelete,
    /// and the row is kept until garbage collection.
    Tombstone,
    /// Whole-field last writer wins carried by its own op type, so it can
    /// become a text CRDT later without touching any other field.
    NotesOwnOp,
}

/// The kind of value a field holds.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ValueType {
    Text,
    OptionalText,
    Int,
    OptionalInt,
    Bool,
    OptionalId,
}

/// Every field an op can address, across all entity kinds.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Field {
    // Space
    Kind,
    // List, task, tag
    Title,
    Name,
    Position,
    CreatedAt,
    DeletedAt,
    // Task
    Notes,
    ListId,
    ParentId,
    DueAt,
    DueAllDay,
    ReminderAt,
    Priority,
    Status,
    CompletedAt,
    Recurrence,
    Tags,
}

impl Field {
    /// The stable text name used in ops and as the SQLite column name.
    pub fn as_str(self) -> &'static str {
        match self {
            Field::Kind => "kind",
            Field::Title => "title",
            Field::Name => "name",
            Field::Position => "position",
            Field::CreatedAt => "created_at",
            Field::DeletedAt => "deleted_at",
            Field::Notes => "notes",
            Field::ListId => "list_id",
            Field::ParentId => "parent_id",
            Field::DueAt => "due_at",
            Field::DueAllDay => "due_all_day",
            Field::ReminderAt => "reminder_at",
            Field::Priority => "priority",
            Field::Status => "status",
            Field::CompletedAt => "completed_at",
            Field::Recurrence => "recurrence",
            Field::Tags => "tags",
        }
    }

    /// The merge rule for this field. Fixed; see Section 6.
    pub fn merge_class(self) -> MergeClass {
        match self {
            Field::Position => MergeClass::FractionalIndex,
            Field::DeletedAt => MergeClass::Tombstone,
            Field::Notes => MergeClass::NotesOwnOp,
            Field::Tags => MergeClass::AddWinsSet,
            Field::Kind
            | Field::Title
            | Field::Name
            | Field::CreatedAt
            | Field::ListId
            | Field::ParentId
            | Field::DueAt
            | Field::DueAllDay
            | Field::ReminderAt
            | Field::Priority
            | Field::Status
            | Field::CompletedAt
            | Field::Recurrence => MergeClass::LwwScalar,
        }
    }

    /// The value type this field holds.
    pub fn value_type(self) -> ValueType {
        match self {
            Field::Kind | Field::Title | Field::Name | Field::Status => ValueType::Text,
            Field::Position => ValueType::Text,
            Field::Notes => ValueType::Text,
            Field::Recurrence => ValueType::OptionalText,
            Field::CreatedAt | Field::Priority => ValueType::Int,
            Field::DeletedAt | Field::DueAt | Field::ReminderAt | Field::CompletedAt => {
                ValueType::OptionalInt
            }
            Field::DueAllDay => ValueType::Bool,
            Field::ListId | Field::ParentId => ValueType::OptionalId,
            Field::Tags => ValueType::OptionalId,
        }
    }

    /// Whether this field exists on the given entity kind.
    pub fn applies_to(self, entity: EntityType) -> bool {
        match entity {
            EntityType::Space => matches!(self, Field::Kind | Field::CreatedAt | Field::DeletedAt),
            EntityType::List => matches!(
                self,
                Field::Title | Field::Position | Field::CreatedAt | Field::DeletedAt
            ),
            EntityType::Tag => matches!(self, Field::Name | Field::CreatedAt | Field::DeletedAt),
            EntityType::Task => !matches!(self, Field::Kind | Field::Name),
        }
    }

    /// Fields that a plain `set` mutation may address on this entity kind.
    /// Tombstones, notes, and tags have their own mutation kinds.
    pub fn settable(self, entity: EntityType) -> bool {
        self.applies_to(entity)
            && matches!(
                self.merge_class(),
                MergeClass::LwwScalar | MergeClass::FractionalIndex
            )
    }

    /// The default value of a field on a freshly materialized row.
    pub fn default_value(self) -> Value {
        match self {
            Field::Kind => Value::Text("personal".into()),
            Field::Title | Field::Name | Field::Notes => Value::Text(String::new()),
            Field::Status => Value::Text("open".into()),
            Field::Position => Value::Text(crate::fractional::FIRST.into()),
            Field::CreatedAt | Field::Priority => Value::Int(0),
            Field::DueAllDay => Value::Bool(false),
            Field::Recurrence
            | Field::DeletedAt
            | Field::DueAt
            | Field::ReminderAt
            | Field::CompletedAt
            | Field::ListId
            | Field::ParentId
            | Field::Tags => Value::Null,
        }
    }

    /// All fields, for iteration.
    pub const ALL: [Field; 17] = [
        Field::Kind,
        Field::Title,
        Field::Name,
        Field::Position,
        Field::CreatedAt,
        Field::DeletedAt,
        Field::Notes,
        Field::ListId,
        Field::ParentId,
        Field::DueAt,
        Field::DueAllDay,
        Field::ReminderAt,
        Field::Priority,
        Field::Status,
        Field::CompletedAt,
        Field::Recurrence,
        Field::Tags,
    ];
}

/// A scalar field value as carried in an op.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Text(String),
    Id(Id),
}

impl Value {
    /// Whether this value is acceptable for a field of the given type.
    pub fn fits(&self, ty: ValueType) -> bool {
        matches!(
            (ty, self),
            (ValueType::Text, Value::Text(_))
                | (ValueType::OptionalText, Value::Text(_) | Value::Null)
                | (ValueType::Int, Value::Int(_))
                | (ValueType::OptionalInt, Value::Int(_) | Value::Null)
                | (ValueType::Bool, Value::Bool(_))
                | (ValueType::OptionalId, Value::Id(_) | Value::Null)
        )
    }

    pub fn as_text(&self) -> Option<&str> {
        match self {
            Value::Text(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_int(&self) -> Option<i64> {
        match self {
            Value::Int(i) => Some(*i),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_id(&self) -> Option<Id> {
        match self {
            Value::Id(id) => Some(*id),
            _ => None,
        }
    }
}

impl From<&str> for Value {
    fn from(s: &str) -> Self {
        Value::Text(s.to_owned())
    }
}
impl From<String> for Value {
    fn from(s: String) -> Self {
        Value::Text(s)
    }
}
impl From<i64> for Value {
    fn from(i: i64) -> Self {
        Value::Int(i)
    }
}
impl From<bool> for Value {
    fn from(b: bool) -> Self {
        Value::Bool(b)
    }
}
impl From<Id> for Value {
    fn from(id: Id) -> Self {
        Value::Id(id)
    }
}
impl<T: Into<Value>> From<Option<T>> for Value {
    fn from(v: Option<T>) -> Self {
        v.map(Into::into).unwrap_or(Value::Null)
    }
}

/// Task priority, stored as the integer in [`Field::Priority`].
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Priority {
    None = 0,
    Low = 1,
    Medium = 2,
    High = 3,
}

impl Priority {
    pub fn from_i64(v: i64) -> Priority {
        match v {
            1 => Priority::Low,
            2 => Priority::Medium,
            3 => Priority::High,
            _ => Priority::None,
        }
    }
}

/// A space, materialized.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Space {
    pub id: Id,
    pub kind: String,
    pub created_at: i64,
    pub modified_at: i64,
    pub deleted_at: Option<i64>,
}

/// A list or project, materialized.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct List {
    pub id: Id,
    pub space_id: Id,
    pub title: String,
    pub position: String,
    pub created_at: i64,
    pub modified_at: i64,
    pub deleted_at: Option<i64>,
}

/// A tag, materialized.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tag {
    pub id: Id,
    pub space_id: Id,
    pub name: String,
    pub created_at: i64,
    pub modified_at: i64,
    pub deleted_at: Option<i64>,
}

/// A task or subtask, materialized. `tags` is sorted by id.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Task {
    pub id: Id,
    pub space_id: Id,
    pub list_id: Option<Id>,
    pub parent_id: Option<Id>,
    pub title: String,
    pub notes: String,
    pub due_at: Option<i64>,
    pub due_all_day: bool,
    pub reminder_at: Option<i64>,
    pub priority: Priority,
    pub status: String,
    pub completed_at: Option<i64>,
    pub position: String,
    pub recurrence: Option<String>,
    pub created_at: i64,
    pub modified_at: i64,
    pub deleted_at: Option<i64>,
    pub tags: Vec<Id>,
}

impl Task {
    pub fn is_completed(&self) -> bool {
        self.completed_at.is_some()
    }

    pub fn is_deleted(&self) -> bool {
        self.deleted_at.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_field_has_a_merge_class_and_applies_somewhere() {
        for field in Field::ALL {
            let _ = field.merge_class();
            assert!(
                EntityType::ALL.iter().any(|e| field.applies_to(*e)),
                "{field:?} applies to no entity"
            );
            assert!(
                field.default_value().fits(field.value_type()),
                "{field:?} default does not fit its type"
            );
        }
    }

    #[test]
    fn special_fields_are_not_plainly_settable() {
        assert!(!Field::Notes.settable(EntityType::Task));
        assert!(!Field::Tags.settable(EntityType::Task));
        assert!(!Field::DeletedAt.settable(EntityType::Task));
        assert!(Field::Position.settable(EntityType::Task));
        assert!(Field::Title.settable(EntityType::Task));
        assert!(!Field::Title.settable(EntityType::Tag));
        assert!(Field::Name.settable(EntityType::Tag));
    }

    #[test]
    fn value_typing() {
        assert!(Value::from("x").fits(ValueType::Text));
        assert!(!Value::Null.fits(ValueType::Text));
        assert!(Value::Null.fits(ValueType::OptionalInt));
        assert!(!Value::from(1i64).fits(ValueType::Bool));
        assert!(Value::from(Some(Id::NIL)).fits(ValueType::OptionalId));
        assert_eq!(Value::from(None::<i64>), Value::Null);
    }
}
