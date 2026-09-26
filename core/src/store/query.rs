//! Reads over the materialized tables and the op log.

use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};

use super::Result;
use crate::ids::Id;
use crate::model::{List, Priority, Space, Tag, Task};
use crate::op::Op;

/// Which tasks to list. Defaults to every live, incomplete task in the space.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TaskFilter {
    /// `Some(None)` means the inbox (no list); `Some(Some(id))` one list.
    pub list: Option<Option<Id>>,
    /// Only direct subtasks of this task.
    pub parent: Option<Id>,
    /// Only tasks due in `[from, to)` (Unix milliseconds).
    pub due_between: Option<(i64, i64)>,
    pub include_completed: bool,
    pub include_deleted: bool,
    /// Zero means no limit.
    pub limit: usize,
}

/// Everything materialized for one space, sorted by id.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpaceState {
    pub space: Option<Space>,
    pub lists: Vec<List>,
    pub tags: Vec<Tag>,
    pub tasks: Vec<Task>,
}

const TASK_COLUMNS: &str = "id, space_id, list_id, parent_id, title, notes, due_at, due_all_day,
    reminder_at, priority, status, completed_at, position, recurrence, created_at, modified_at,
    deleted_at";

fn task_from_row(r: &Row<'_>) -> rusqlite::Result<Task> {
    Ok(Task {
        id: r.get(0)?,
        space_id: r.get(1)?,
        list_id: r.get(2)?,
        parent_id: r.get(3)?,
        title: r.get(4)?,
        notes: r.get(5)?,
        due_at: r.get(6)?,
        due_all_day: r.get::<_, i64>(7)? != 0,
        reminder_at: r.get(8)?,
        priority: Priority::from_i64(r.get(9)?),
        status: r.get(10)?,
        completed_at: r.get(11)?,
        position: r.get(12)?,
        recurrence: r.get(13)?,
        created_at: r.get(14)?,
        modified_at: r.get(15)?,
        deleted_at: r.get(16)?,
        tags: Vec::new(),
    })
}

fn fill_tags(conn: &Connection, tasks: &mut [Task]) -> Result<()> {
    let mut stmt = conn.prepare_cached(
        "SELECT DISTINCT tag_id FROM task_tags WHERE task_id = ?1 ORDER BY tag_id",
    )?;
    for task in tasks {
        task.tags = stmt
            .query_map(params![task.id], |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<Id>>>()?;
    }
    Ok(())
}

pub(super) fn tag_adds(conn: &Connection, task: Id, tag: Id) -> Result<Vec<Id>> {
    Ok(conn
        .prepare_cached(
            "SELECT add_id FROM task_tags WHERE task_id = ?1 AND tag_id = ?2 ORDER BY add_id",
        )?
        .query_map(params![task, tag], |r| r.get(0))?
        .collect::<rusqlite::Result<Vec<Id>>>()?)
}

pub(super) fn space(conn: &Connection, id: Id) -> Result<Option<Space>> {
    Ok(conn
        .prepare_cached(
            "SELECT id, kind, created_at, modified_at, deleted_at FROM spaces WHERE id = ?1",
        )?
        .query_row(params![id], |r| {
            Ok(Space {
                id: r.get(0)?,
                kind: r.get(1)?,
                created_at: r.get(2)?,
                modified_at: r.get(3)?,
                deleted_at: r.get(4)?,
            })
        })
        .optional()?)
}

fn list_from_row(r: &Row<'_>) -> rusqlite::Result<List> {
    Ok(List {
        id: r.get(0)?,
        space_id: r.get(1)?,
        title: r.get(2)?,
        position: r.get(3)?,
        created_at: r.get(4)?,
        modified_at: r.get(5)?,
        deleted_at: r.get(6)?,
    })
}

const LIST_COLUMNS: &str = "id, space_id, title, position, created_at, modified_at, deleted_at";

pub(super) fn list(conn: &Connection, id: Id) -> Result<Option<List>> {
    Ok(conn
        .prepare_cached(&format!("SELECT {LIST_COLUMNS} FROM lists WHERE id = ?1"))?
        .query_row(params![id], list_from_row)
        .optional()?)
}

pub(super) fn lists(conn: &Connection, space_id: Id) -> Result<Vec<List>> {
    Ok(conn
        .prepare_cached(&format!(
            "SELECT {LIST_COLUMNS} FROM lists WHERE space_id = ?1 AND deleted_at IS NULL
             ORDER BY position, id"
        ))?
        .query_map(params![space_id], list_from_row)?
        .collect::<rusqlite::Result<_>>()?)
}

fn tag_from_row(r: &Row<'_>) -> rusqlite::Result<Tag> {
    Ok(Tag {
        id: r.get(0)?,
        space_id: r.get(1)?,
        name: r.get(2)?,
        created_at: r.get(3)?,
        modified_at: r.get(4)?,
        deleted_at: r.get(5)?,
    })
}

const TAG_COLUMNS: &str = "id, space_id, name, created_at, modified_at, deleted_at";

pub(super) fn tag(conn: &Connection, id: Id) -> Result<Option<Tag>> {
    Ok(conn
        .prepare_cached(&format!("SELECT {TAG_COLUMNS} FROM tags WHERE id = ?1"))?
        .query_row(params![id], tag_from_row)
        .optional()?)
}

pub(super) fn tags(conn: &Connection, space_id: Id) -> Result<Vec<Tag>> {
    Ok(conn
        .prepare_cached(&format!(
            "SELECT {TAG_COLUMNS} FROM tags WHERE space_id = ?1 AND deleted_at IS NULL
             ORDER BY name, id"
        ))?
        .query_map(params![space_id], tag_from_row)?
        .collect::<rusqlite::Result<_>>()?)
}

pub(super) fn task(conn: &Connection, id: Id) -> Result<Option<Task>> {
    let task = conn
        .prepare_cached(&format!("SELECT {TASK_COLUMNS} FROM tasks WHERE id = ?1"))?
        .query_row(params![id], task_from_row)
        .optional()?;
    let mut tasks: Vec<Task> = task.into_iter().collect();
    fill_tags(conn, &mut tasks)?;
    Ok(tasks.pop())
}

pub(super) fn tasks(conn: &Connection, space_id: Id, filter: &TaskFilter) -> Result<Vec<Task>> {
    let mut sql = format!("SELECT {TASK_COLUMNS} FROM tasks WHERE space_id = ?1");
    let mut args: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(space_id)];
    match filter.list {
        Some(None) => sql.push_str(" AND list_id IS NULL"),
        Some(Some(list)) => {
            args.push(Box::new(list));
            sql.push_str(&format!(" AND list_id = ?{}", args.len()));
        }
        None => {}
    }
    if let Some(parent) = filter.parent {
        args.push(Box::new(parent));
        sql.push_str(&format!(" AND parent_id = ?{}", args.len()));
    }
    if let Some((from, to)) = filter.due_between {
        args.push(Box::new(from));
        sql.push_str(&format!(" AND due_at >= ?{}", args.len()));
        args.push(Box::new(to));
        sql.push_str(&format!(" AND due_at < ?{}", args.len()));
    }
    if !filter.include_completed {
        sql.push_str(" AND completed_at IS NULL");
    }
    if !filter.include_deleted {
        sql.push_str(" AND deleted_at IS NULL");
    }
    sql.push_str(" ORDER BY position, id");
    if filter.limit > 0 {
        sql.push_str(&format!(" LIMIT {}", filter.limit));
    }
    let mut stmt = conn.prepare_cached(&sql)?;
    let mut tasks: Vec<Task> = stmt
        .query_map(rusqlite::params_from_iter(args.iter()), task_from_row)?
        .collect::<rusqlite::Result<_>>()?;
    fill_tags(conn, &mut tasks)?;
    Ok(tasks)
}

/// Turn free text into an FTS5 query: each term is quoted and matched as a
/// prefix, terms are ANDed.
fn fts_query(text: &str) -> String {
    text.split_whitespace()
        .map(|term| format!("\"{}\"*", term.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" ")
}

pub(super) fn search(
    conn: &Connection,
    space_id: Id,
    text: &str,
    limit: usize,
) -> Result<Vec<Task>> {
    let query = fts_query(text);
    if query.is_empty() {
        return Ok(Vec::new());
    }
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT {TASK_COLUMNS} FROM tasks t
         WHERE t.rid IN (SELECT rowid FROM tasks_fts WHERE tasks_fts MATCH ?1 ORDER BY rank)
           AND t.space_id = ?2 AND t.deleted_at IS NULL
         ORDER BY t.completed_at IS NOT NULL, t.modified_at DESC
         LIMIT ?3"
    ))?;
    let mut tasks: Vec<Task> = stmt
        .query_map(params![query, space_id, limit as i64], task_from_row)?
        .collect::<rusqlite::Result<_>>()?;
    fill_tags(conn, &mut tasks)?;
    Ok(tasks)
}

pub(super) fn space_state(conn: &Connection, space_id: Id) -> Result<SpaceState> {
    let lists = conn
        .prepare_cached(&format!(
            "SELECT {LIST_COLUMNS} FROM lists WHERE space_id = ?1 ORDER BY id"
        ))?
        .query_map(params![space_id], list_from_row)?
        .collect::<rusqlite::Result<_>>()?;
    let tags = conn
        .prepare_cached(&format!(
            "SELECT {TAG_COLUMNS} FROM tags WHERE space_id = ?1 ORDER BY id"
        ))?
        .query_map(params![space_id], tag_from_row)?
        .collect::<rusqlite::Result<_>>()?;
    let mut tasks: Vec<Task> = conn
        .prepare_cached(&format!(
            "SELECT {TASK_COLUMNS} FROM tasks WHERE space_id = ?1 ORDER BY id"
        ))?
        .query_map(params![space_id], task_from_row)?
        .collect::<rusqlite::Result<_>>()?;
    fill_tags(conn, &mut tasks)?;
    Ok(SpaceState {
        space: space(conn, space_id)?,
        lists,
        tags,
        tasks,
    })
}

pub(super) fn pending_ops(conn: &Connection, space_id: Id) -> Result<Vec<Op>> {
    let payloads: Vec<Vec<u8>> = conn
        .prepare_cached(
            "SELECT payload FROM ops WHERE space_id = ?1 AND seq IS NULL
             ORDER BY hlc_wall, hlc_counter, device_id",
        )?
        .query_map(params![space_id], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    payloads
        .iter()
        .map(|p| Op::decode(p).map_err(Into::into))
        .collect()
}

pub(super) fn logged_op(conn: &Connection, space_id: Id, op_id: Id) -> Result<Option<Op>> {
    let payload: Option<Vec<u8>> = conn
        .prepare_cached("SELECT payload FROM ops WHERE space_id = ?1 AND op_id = ?2")?
        .query_row(params![space_id, op_id], |r| r.get(0))
        .optional()?;
    payload
        .map(|p| Op::decode(&p).map_err(Into::into))
        .transpose()
}

pub(super) fn cursor(conn: &Connection, space_id: Id) -> Result<u64> {
    Ok(conn
        .prepare_cached("SELECT last_seq FROM sync_cursor WHERE space_id = ?1")?
        .query_row(params![space_id], |r| r.get::<_, i64>(0))
        .optional()?
        .unwrap_or(0) as u64)
}

pub(super) fn op_count(conn: &Connection, space_id: Id) -> Result<u64> {
    Ok(conn
        .prepare_cached("SELECT count(*) FROM ops WHERE space_id = ?1")?
        .query_row(params![space_id], |r| r.get::<_, i64>(0))? as u64)
}
