//! Applying one op (Section 6): append to the log, then materialize under
//! the field's merge rule, all inside the caller's transaction.
//!
//! Every op first ensures its entity's row exists, so ops for one entity
//! can arrive in any order; there is no separate "create" op. Then:
//! - `set` and `notes`: last writer wins by comparing the op's `hlc` with
//!   the field's recorded clock; the row is updated only if the op wins.
//! - `delete` and `undelete`: the same rule on `deleted_at`.
//! - `add_tag`: inserts `(task, tag, op_id)` unless a remove already
//!   observed that add. `remove_tag`: deletes the cited adds and remembers
//!   them. Adds a remove never saw survive, which is add-wins.
//! - `modified_at` becomes the largest wall time of any applied op.
//!
//! An op that is well-formed but nonsensical for its entity (a tag op on a
//! list, a field that does not exist there) is logged, marked applied, and
//! changes nothing, so one bad peer cannot stall sync.

use rusqlite::types::{ToSql, ToSqlOutput, Value as SqlValue, ValueRef};
use rusqlite::{OptionalExtension, Transaction, params};

use super::{Applied, Result};
use crate::hlc::Hlc;
use crate::ids::Id;
use crate::model::{EntityType, Field, Value, ValueType};
use crate::op::{Mutation, Op};
use crate::undo::Inverse;

pub(super) struct Outcome {
    pub status: Applied,
    pub inverse: Option<Inverse>,
}

impl ToSql for Value {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(match self {
            Value::Null => ToSqlOutput::Owned(SqlValue::Null),
            Value::Bool(b) => ToSqlOutput::Owned(SqlValue::Integer(i64::from(*b))),
            Value::Int(i) => ToSqlOutput::Owned(SqlValue::Integer(*i)),
            Value::Text(s) => ToSqlOutput::Borrowed(ValueRef::Text(s.as_bytes())),
            Value::Id(id) => ToSqlOutput::Borrowed(ValueRef::Blob(id.as_bytes())),
        })
    }
}

fn value_from_sql(field: Field, v: SqlValue) -> Value {
    match (field.value_type(), v) {
        (_, SqlValue::Null) => Value::Null,
        (ValueType::Bool, SqlValue::Integer(i)) => Value::Bool(i != 0),
        (_, SqlValue::Integer(i)) => Value::Int(i),
        (_, SqlValue::Text(s)) => Value::Text(s),
        (_, SqlValue::Blob(b)) => Id::from_slice(&b).map(Value::Id).unwrap_or(Value::Null),
        (_, SqlValue::Real(f)) => Value::Int(f as i64),
    }
}

fn table(entity: EntityType) -> &'static str {
    match entity {
        EntityType::Space => "spaces",
        EntityType::List => "lists",
        EntityType::Task => "tasks",
        EntityType::Tag => "tags",
    }
}

/// Log the op and materialize it. `seq` is the server's number if known.
pub(super) fn apply_op(
    tx: &Transaction,
    op: &Op,
    seq: Option<u64>,
    supported_schema: u32,
    now_ms: u64,
) -> Result<Outcome> {
    let inserted = tx
        .prepare_cached(
            "INSERT OR IGNORE INTO ops (space_id, op_id, device_id, hlc_wall, hlc_counter,
                entity_type, entity_id, schema_version, seq, applied, payload, received_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 0, ?10, ?11)",
        )?
        .execute(params![
            op.space_id,
            op.op_id,
            op.device_id,
            op.hlc.wall_ms as i64,
            op.hlc.counter,
            op.entity_type.as_str(),
            op.entity_id,
            op.schema_version,
            seq.map(|s| s as i64),
            op.encode(),
            now_ms as i64,
        ])?;
    if inserted == 0 {
        if let Some(seq) = seq {
            tx.prepare_cached("UPDATE ops SET seq = ?3 WHERE space_id = ?1 AND op_id = ?2")?
                .execute(params![op.space_id, op.op_id, seq as i64])?;
        }
        return Ok(Outcome {
            status: Applied::Duplicate,
            inverse: None,
        });
    }
    if op.schema_version > supported_schema || matches!(op.mutation, Mutation::Unknown { .. }) {
        return Ok(Outcome {
            status: Applied::Skipped,
            inverse: None,
        });
    }
    let inverse = materialize(tx, op)?;
    tx.prepare_cached("UPDATE ops SET applied = 1 WHERE space_id = ?1 AND op_id = ?2")?
        .execute(params![op.space_id, op.op_id])?;
    Ok(Outcome {
        status: Applied::Applied,
        inverse,
    })
}

/// Materialize ops in the space that were logged but skipped and are now
/// within the supported schema version.
pub(super) fn reapply_skipped(
    tx: &Transaction,
    space_id: Id,
    supported_schema: u32,
    _now_ms: u64,
) -> Result<usize> {
    let payloads: Vec<Vec<u8>> = tx
        .prepare_cached(
            "SELECT payload FROM ops WHERE space_id = ?1 AND applied = 0
             ORDER BY hlc_wall, hlc_counter, device_id",
        )?
        .query_map(params![space_id], |r| r.get(0))?
        .collect::<std::result::Result<_, _>>()?;
    let mut count = 0;
    for payload in payloads {
        let op = Op::decode(&payload)?;
        if op.schema_version > supported_schema || matches!(op.mutation, Mutation::Unknown { .. }) {
            continue;
        }
        materialize(tx, &op)?;
        tx.prepare_cached("UPDATE ops SET applied = 1 WHERE space_id = ?1 AND op_id = ?2")?
            .execute(params![op.space_id, op.op_id])?;
        count += 1;
    }
    Ok(count)
}

/// Move the space's cursor forward to `seq` if it is ahead.
pub(super) fn advance_cursor(tx: &Transaction, space_id: Id, seq: u64, now_ms: u64) -> Result<()> {
    tx.prepare_cached(
        "INSERT INTO sync_cursor (space_id, last_seq, updated_at) VALUES (?1, ?2, ?3)
         ON CONFLICT (space_id) DO UPDATE SET
            last_seq = max(last_seq, excluded.last_seq),
            updated_at = excluded.updated_at",
    )?
    .execute(params![space_id, seq as i64, now_ms as i64])?;
    Ok(())
}

fn ensure_row(tx: &Transaction, op: &Op) -> Result<bool> {
    let changed = match op.entity_type {
        EntityType::Space => tx
            .prepare_cached("INSERT OR IGNORE INTO spaces (id) VALUES (?1)")?
            .execute(params![op.entity_id])?,
        EntityType::List => tx
            .prepare_cached("INSERT OR IGNORE INTO lists (id, space_id) VALUES (?1, ?2)")?
            .execute(params![op.entity_id, op.space_id])?,
        EntityType::Task => tx
            .prepare_cached("INSERT OR IGNORE INTO tasks (id, space_id) VALUES (?1, ?2)")?
            .execute(params![op.entity_id, op.space_id])?,
        EntityType::Tag => tx
            .prepare_cached("INSERT OR IGNORE INTO tags (id, space_id) VALUES (?1, ?2)")?
            .execute(params![op.entity_id, op.space_id])?,
    };
    Ok(changed > 0)
}

fn bump_modified(tx: &Transaction, op: &Op) -> Result<()> {
    let sql = format!(
        "UPDATE {} SET modified_at = max(modified_at, ?2) WHERE id = ?1",
        table(op.entity_type)
    );
    tx.prepare_cached(&sql)?
        .execute(params![op.entity_id, op.hlc.wall_ms as i64])?;
    Ok(())
}

fn field_clock(tx: &Transaction, entity_id: Id, field: Field) -> Result<Option<Hlc>> {
    Ok(tx
        .prepare_cached(
            "SELECT hlc_wall, hlc_counter, device_id FROM field_clocks
             WHERE entity_id = ?1 AND field = ?2",
        )?
        .query_row(params![entity_id, field.as_str()], |r| {
            Ok(Hlc {
                wall_ms: r.get::<_, i64>(0)? as u64,
                counter: r.get(1)?,
                device: r.get(2)?,
            })
        })
        .optional()?)
}

fn set_field_clock(tx: &Transaction, op: &Op, field: Field) -> Result<()> {
    tx.prepare_cached(
        "INSERT INTO field_clocks (entity_id, field, space_id, hlc_wall, hlc_counter, device_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT (entity_id, field) DO UPDATE SET
            hlc_wall = excluded.hlc_wall, hlc_counter = excluded.hlc_counter,
            device_id = excluded.device_id",
    )?
    .execute(params![
        op.entity_id,
        field.as_str(),
        op.space_id,
        op.hlc.wall_ms as i64,
        op.hlc.counter,
        op.hlc.device,
    ])?;
    Ok(())
}

/// Last-writer-wins write of one column. Returns the previous value if the
/// op won, `None` if an equal or later write already holds the field.
fn lww_set(tx: &Transaction, op: &Op, field: Field, value: &Value) -> Result<Option<Value>> {
    if let Some(existing) = field_clock(tx, op.entity_id, field)?
        && existing >= op.hlc
    {
        return Ok(None);
    }
    let t = table(op.entity_type);
    let col = field.as_str();
    let previous: SqlValue = tx
        .prepare_cached(&format!("SELECT {col} FROM {t} WHERE id = ?1"))?
        .query_row(params![op.entity_id], |r| r.get(0))?;
    tx.prepare_cached(&format!("UPDATE {t} SET {col} = ?2 WHERE id = ?1"))?
        .execute(params![op.entity_id, value])?;
    set_field_clock(tx, op, field)?;
    Ok(Some(value_from_sql(field, previous)))
}

fn inverse(op: &Op, mutation: Mutation) -> Option<Inverse> {
    Some(Inverse {
        space_id: op.space_id,
        entity_type: op.entity_type,
        entity_id: op.entity_id,
        mutation,
    })
}

/// Apply the op's mutation to the materialized tables. Returns the inverse
/// mutation when the op changed state.
fn materialize(tx: &Transaction, op: &Op) -> Result<Option<Inverse>> {
    let created = ensure_row(tx, op)?;
    bump_modified(tx, op)?;
    let is_task = op.entity_type == EntityType::Task;
    let result = match &op.mutation {
        Mutation::Set { field, value } => {
            if !field.settable(op.entity_type) || !value.fits(field.value_type()) {
                None
            } else {
                lww_set(tx, op, *field, value)?.map(|previous| Mutation::Set {
                    field: *field,
                    value: previous,
                })
            }
        }
        Mutation::SetNotes { text } => {
            if !is_task {
                None
            } else {
                lww_set(tx, op, Field::Notes, &Value::Text(text.clone()))?.map(|previous| {
                    Mutation::SetNotes {
                        text: previous.as_text().unwrap_or_default().to_owned(),
                    }
                })
            }
        }
        Mutation::Delete => {
            let at = Value::Int(op.hlc.wall_ms as i64);
            lww_set(tx, op, Field::DeletedAt, &at)?.map(|previous| match previous {
                Value::Null => Mutation::Undelete,
                _ => Mutation::Delete,
            })
        }
        Mutation::Undelete => {
            lww_set(tx, op, Field::DeletedAt, &Value::Null)?.map(|previous| match previous {
                Value::Null => Mutation::Undelete,
                _ => Mutation::Delete,
            })
        }
        Mutation::AddTag { tag } => {
            if !is_task {
                None
            } else {
                let removed: bool = tx
                    .prepare_cached("SELECT 1 FROM removed_tag_adds WHERE add_id = ?1")?
                    .query_row(params![op.op_id], |_| Ok(()))
                    .optional()?
                    .is_some();
                if removed {
                    None
                } else {
                    let inserted = tx
                        .prepare_cached(
                            "INSERT OR IGNORE INTO task_tags (task_id, tag_id, add_id) VALUES (?1, ?2, ?3)",
                        )?
                        .execute(params![op.entity_id, tag, op.op_id])?;
                    (inserted > 0).then(|| Mutation::RemoveTag {
                        tag: *tag,
                        observed: vec![op.op_id],
                    })
                }
            }
        }
        Mutation::RemoveTag { tag, observed } => {
            if !is_task {
                None
            } else {
                let was_present = tag_present(tx, op.entity_id, *tag)?;
                for add_id in observed {
                    tx.prepare_cached(
                        "DELETE FROM task_tags WHERE task_id = ?1 AND tag_id = ?2 AND add_id = ?3",
                    )?
                    .execute(params![op.entity_id, tag, add_id])?;
                    tx.prepare_cached(
                        "INSERT OR IGNORE INTO removed_tag_adds (add_id, space_id) VALUES (?1, ?2)",
                    )?
                    .execute(params![add_id, op.space_id])?;
                }
                let now_present = tag_present(tx, op.entity_id, *tag)?;
                (was_present && !now_present).then_some(Mutation::AddTag { tag: *tag })
            }
        }
        // Never reached: unknown mutations are skipped before materializing.
        Mutation::Unknown { .. } => None,
    };
    if created {
        // Undoing the op that brought the entity into existence removes it
        // again; restoring one field of a row that should not exist would
        // leave an empty entity behind.
        return Ok(inverse(op, Mutation::Delete));
    }
    Ok(result.and_then(|m| inverse(op, m)))
}

fn tag_present(tx: &Transaction, task: Id, tag: Id) -> Result<bool> {
    Ok(tx
        .prepare_cached("SELECT 1 FROM task_tags WHERE task_id = ?1 AND tag_id = ?2 LIMIT 1")?
        .query_row(params![task, tag], |_| Ok(()))
        .optional()?
        .is_some())
}
