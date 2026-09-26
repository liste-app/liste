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

/// An op as the log holds it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoggedOp {
    pub op: Op,
    pub seq: Option<u64>,
    pub applied: bool,
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

/// Fill tags for many tasks with one query: every tag row whose task
/// satisfies `where_sql` (a predicate over the `tasks` table with the given
/// positional `args`), merged into `tasks` by id.
fn fill_tags_where(
    conn: &Connection,
    tasks: &mut [Task],
    where_sql: &str,
    args: &[Box<dyn rusqlite::ToSql>],
) -> Result<()> {
    let sql = format!(
        "SELECT DISTINCT tt.task_id, tt.tag_id FROM task_tags tt
         JOIN tasks ON tasks.id = tt.task_id WHERE {where_sql}
         ORDER BY tt.task_id, tt.tag_id"
    );
    let mut stmt = conn.prepare_cached(&sql)?;
    let mut by_task: std::collections::HashMap<Id, Vec<Id>> = std::collections::HashMap::new();
    let rows = stmt.query_map(rusqlite::params_from_iter(args.iter()), |r| {
        Ok((r.get::<_, Id>(0)?, r.get::<_, Id>(1)?))
    })?;
    for row in rows {
        let (task, tag) = row?;
        by_task.entry(task).or_default().push(tag);
    }
    for task in tasks {
        if let Some(tags) = by_task.remove(&task.id) {
            task.tags = tags;
        }
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
    let mut where_sql = String::from("tasks.space_id = ?1");
    let mut args: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(space_id)];
    match filter.list {
        Some(None) => where_sql.push_str(" AND tasks.list_id IS NULL"),
        Some(Some(list)) => {
            args.push(Box::new(list));
            where_sql.push_str(&format!(" AND tasks.list_id = ?{}", args.len()));
        }
        None => {}
    }
    if let Some(parent) = filter.parent {
        args.push(Box::new(parent));
        where_sql.push_str(&format!(" AND tasks.parent_id = ?{}", args.len()));
    }
    if let Some((from, to)) = filter.due_between {
        args.push(Box::new(from));
        where_sql.push_str(&format!(" AND tasks.due_at >= ?{}", args.len()));
        args.push(Box::new(to));
        where_sql.push_str(&format!(" AND tasks.due_at < ?{}", args.len()));
    }
    if !filter.include_completed {
        where_sql.push_str(" AND tasks.completed_at IS NULL");
    }
    if !filter.include_deleted {
        where_sql.push_str(" AND tasks.deleted_at IS NULL");
    }
    let mut sql =
        format!("SELECT {TASK_COLUMNS} FROM tasks WHERE {where_sql} ORDER BY position, id");
    if filter.limit > 0 {
        sql.push_str(&format!(" LIMIT {}", filter.limit));
    }
    let mut stmt = conn.prepare_cached(&sql)?;
    let mut tasks: Vec<Task> = stmt
        .query_map(rusqlite::params_from_iter(args.iter()), task_from_row)?
        .collect::<rusqlite::Result<_>>()?;
    if filter.limit > 0 && tasks.len() >= filter.limit {
        fill_tags(conn, &mut tasks)?;
    } else {
        fill_tags_where(conn, &mut tasks, &where_sql, &args)?;
    }
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

/// Results are newest first (by row id), not by relevance rank: ranking
/// every match before applying the limit is what makes a one-letter query
/// slow against a large database, while a rowid-ordered scan stops after
/// `limit` rows. A relevance rank for longer queries is a later refinement.
///
/// `CROSS JOIN` pins the join order: the FTS scan drives and each hit is a
/// primary-key lookup. Left to itself, the bundled SQLite starts from the
/// `tasks` index and probes FTS once per task, which is quadratic.
fn search_sql() -> String {
    format!(
        "SELECT {} FROM tasks_fts f CROSS JOIN tasks t ON t.rid = f.rowid
         WHERE f.tasks_fts MATCH ?1 AND t.space_id = ?2 AND t.deleted_at IS NULL
         ORDER BY f.rowid DESC
         LIMIT ?3",
        TASK_COLUMNS
            .split(',')
            .map(|c| format!("t.{}", c.trim()))
            .collect::<Vec<_>>()
            .join(", ")
    )
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
    let mut stmt = conn.prepare_cached(&search_sql())?;
    let mut tasks: Vec<Task> = stmt
        .query_map(params![query, space_id, limit as i64], task_from_row)?
        .collect::<rusqlite::Result<_>>()?;
    fill_tags(conn, &mut tasks)?;
    Ok(tasks)
}

pub(super) fn search_plan(conn: &Connection) -> Result<Vec<String>> {
    let sql = format!("EXPLAIN QUERY PLAN {}", search_sql());
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params!["\"c\"*", Id::NIL, 50i64], |r| r.get::<_, String>(3))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Counts of ops the cursor does not cover: not yet pushed (no `seq`), and
/// pushed with a `seq` beyond what has been pulled.
pub(super) fn ops_not_covered_by_cursor(conn: &Connection, space_id: Id) -> Result<(usize, usize)> {
    let cursor = cursor(conn, space_id)? as i64;
    let unpushed: i64 = conn
        .prepare_cached("SELECT count(*) FROM ops WHERE space_id = ?1 AND seq IS NULL")?
        .query_row(params![space_id], |r| r.get(0))?;
    let ahead: i64 = conn
        .prepare_cached("SELECT count(*) FROM ops WHERE space_id = ?1 AND seq > ?2")?
        .query_row(params![space_id, cursor], |r| r.get(0))?;
    Ok((unpushed as usize, ahead as usize))
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
    let args: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(space_id)];
    fill_tags_where(conn, &mut tasks, "tasks.space_id = ?1", &args)?;
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

pub(super) fn recent_ops(conn: &Connection, space_id: Id, limit: usize) -> Result<Vec<LoggedOp>> {
    let rows: Vec<(Vec<u8>, Option<i64>, i64)> = conn
        .prepare_cached(
            "SELECT payload, seq, applied FROM ops WHERE space_id = ?1
             ORDER BY hlc_wall DESC, hlc_counter DESC, device_id DESC LIMIT ?2",
        )?
        .query_map(params![space_id, limit as i64], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })?
        .collect::<rusqlite::Result<_>>()?;
    rows.into_iter()
        .map(|(payload, seq, applied)| {
            Ok(LoggedOp {
                op: Op::decode(&payload)?,
                seq: seq.map(|s| s as u64),
                applied: applied != 0,
            })
        })
        .collect()
}

pub(super) fn pending_count(conn: &Connection, space_id: Id) -> Result<u64> {
    Ok(conn
        .prepare_cached("SELECT count(*) FROM ops WHERE space_id = ?1 AND seq IS NULL")?
        .query_row(params![space_id], |r| r.get::<_, i64>(0))? as u64)
}
