//! Reads over the materialized tables and the op log.

use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};

use super::Result;
use crate::ids::Id;
use crate::model::{Filter, List, Priority, Space, Tag, Task};

use crate::op::Op;

/// How a task list is ordered.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TaskOrder {
    /// The person's manual order, then id.
    #[default]
    Manual,
    /// Soonest due first, undated last, then manual order.
    DueThenManual,
    /// Most recently completed first.
    CompletedDesc,
}

/// Which tasks to list. Defaults to every live, incomplete task in the space.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TaskFilter {
    /// `Some(None)` means the inbox (no list); `Some(Some(id))` one list.
    pub list: Option<Option<Id>>,
    /// Only direct subtasks of this task.
    pub parent: Option<Id>,
    /// Only tasks carrying this tag.
    pub tag: Option<Id>,
    pub priority: Option<Priority>,
    pub status: Option<String>,
    /// Only tasks due in `[from, to)` (Unix milliseconds).
    pub due_between: Option<(i64, i64)>,
    /// Only tasks with a reminder set.
    pub has_reminder: bool,
    pub include_completed: bool,
    /// Only completed tasks.
    pub completed_only: bool,
    pub include_deleted: bool,
    pub order: TaskOrder,
    /// Rows to skip, for a window into a long list.
    pub offset: usize,
    /// Zero means no limit.
    pub limit: usize,
}

/// A task in a listing, with its place in the outline: `depth` is how many
/// of its ancestors the listing shows above it, so a subtask whose parent
/// is not shown (completed, filtered out) moves up to its nearest shown
/// ancestor. Zero outside manual order, where subtasks are plain rows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskRow {
    pub task: Task,
    pub depth: u32,
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
    /// Absent in snapshots written before saved filters existed.
    #[serde(default)]
    pub filters: Vec<Filter>,
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
    let mut refs: Vec<&mut Task> = tasks.iter_mut().collect();
    fill_tags_each(conn, &mut refs)
}

/// One tag query per task; right for a window of rows.
fn fill_tags_each(conn: &Connection, tasks: &mut [&mut Task]) -> Result<()> {
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
    tasks: &mut [&mut Task],
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

const FILTER_COLUMNS: &str = "id, space_id, name, position, list_id, tag_id, filter_priority,
    filter_status, due_from_day, due_to_day, include_completed, created_at, modified_at, deleted_at";

fn filter_from_row(r: &Row<'_>) -> rusqlite::Result<Filter> {
    Ok(Filter {
        id: r.get(0)?,
        space_id: r.get(1)?,
        name: r.get(2)?,
        position: r.get(3)?,
        list_id: r.get(4)?,
        tag_id: r.get(5)?,
        priority: r.get::<_, Option<i64>>(6)?.map(Priority::from_i64),
        status: r.get(7)?,
        due_from_day: r.get(8)?,
        due_to_day: r.get(9)?,
        include_completed: r.get::<_, i64>(10)? != 0,
        created_at: r.get(11)?,
        modified_at: r.get(12)?,
        deleted_at: r.get(13)?,
    })
}

pub(super) fn filter(conn: &Connection, id: Id) -> Result<Option<Filter>> {
    Ok(conn
        .prepare_cached(&format!(
            "SELECT {FILTER_COLUMNS} FROM filters WHERE id = ?1"
        ))?
        .query_row(params![id], filter_from_row)
        .optional()?)
}

pub(super) fn filters(conn: &Connection, space_id: Id) -> Result<Vec<Filter>> {
    Ok(conn
        .prepare_cached(&format!(
            "SELECT {FILTER_COLUMNS} FROM filters WHERE space_id = ?1 AND deleted_at IS NULL
             ORDER BY position, id"
        ))?
        .query_map(params![space_id], filter_from_row)?
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

/// The `WHERE` predicate (over `tasks`) and its positional arguments.
fn where_clause(space_id: Id, filter: &TaskFilter) -> (String, Vec<Box<dyn rusqlite::ToSql>>) {
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
    if let Some(tag) = filter.tag {
        args.push(Box::new(tag));
        where_sql.push_str(&format!(
            " AND EXISTS (SELECT 1 FROM task_tags tt WHERE tt.task_id = tasks.id AND tt.tag_id = ?{})",
            args.len()
        ));
    }
    if let Some(priority) = filter.priority {
        args.push(Box::new(priority as i64));
        where_sql.push_str(&format!(" AND tasks.priority = ?{}", args.len()));
    }
    if let Some(status) = &filter.status {
        args.push(Box::new(status.clone()));
        where_sql.push_str(&format!(" AND tasks.status = ?{}", args.len()));
    }
    if filter.has_reminder {
        where_sql.push_str(" AND tasks.reminder_at IS NOT NULL");
    }
    if filter.completed_only {
        where_sql.push_str(" AND tasks.completed_at IS NOT NULL");
    } else if !filter.include_completed {
        where_sql.push_str(" AND tasks.completed_at IS NULL");
    }
    if !filter.include_deleted {
        where_sql.push_str(" AND tasks.deleted_at IS NULL");
    }
    (where_sql, args)
}

/// Status names in use on live tasks: `open` first, then the rest by name.
pub(super) fn statuses(conn: &Connection, space_id: Id) -> Result<Vec<String>> {
    let mut names: Vec<String> = conn
        .prepare_cached(
            "SELECT DISTINCT status FROM tasks WHERE space_id = ?1 AND deleted_at IS NULL
             ORDER BY status",
        )?
        .query_map(params![space_id], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    if let Some(i) = names.iter().position(|s| s == "open") {
        names.remove(i);
    }
    names.insert(0, "open".into());
    Ok(names)
}

/// How many tasks match, ignoring `offset` and `limit`.
pub(super) fn count(conn: &Connection, space_id: Id, filter: &TaskFilter) -> Result<usize> {
    let (where_sql, args) = where_clause(space_id, filter);
    let sql = format!("SELECT count(*) FROM tasks WHERE {where_sql}");
    let mut stmt = conn.prepare_cached(&sql)?;
    let n: i64 = stmt.query_row(rusqlite::params_from_iter(args.iter()), |r| r.get(0))?;
    Ok(n as usize)
}

pub(super) fn tasks(conn: &Connection, space_id: Id, filter: &TaskFilter) -> Result<Vec<Task>> {
    Ok(task_rows(conn, space_id, filter)?
        .into_iter()
        .map(|r| r.task)
        .collect())
}

/// The matching tasks in the filter's order, `offset` rows in and at most
/// `limit` rows long, each with its outline depth.
pub(super) fn task_rows(
    conn: &Connection,
    space_id: Id,
    filter: &TaskFilter,
) -> Result<Vec<TaskRow>> {
    let (where_sql, args) = where_clause(space_id, filter);
    let order = match filter.order {
        TaskOrder::Manual => "sort_key, id",
        TaskOrder::DueThenManual => "due_at IS NULL, due_at, sort_key, id",
        TaskOrder::CompletedDesc => "completed_at DESC, id",
    };
    let mut sql =
        format!("SELECT {TASK_COLUMNS}, sort_key FROM tasks WHERE {where_sql} ORDER BY {order}");
    if filter.limit > 0 || filter.offset > 0 {
        let limit = if filter.limit > 0 {
            filter.limit as i64
        } else {
            -1
        };
        sql.push_str(&format!(" LIMIT {limit} OFFSET {}", filter.offset));
    }
    let mut stmt = conn.prepare_cached(&sql)?;
    let mut rows: Vec<(Task, String)> = stmt
        .query_map(rusqlite::params_from_iter(args.iter()), |r| {
            Ok((task_from_row(r)?, r.get::<_, String>(17)?))
        })?
        .collect::<rusqlite::Result<_>>()?;
    let windowed = filter.limit > 0 || filter.offset > 0;
    {
        let mut tasks: Vec<&mut Task> = rows.iter_mut().map(|(t, _)| t).collect();
        if windowed && tasks.len() < 1_000 {
            fill_tags_each(conn, &mut tasks)?;
        } else {
            fill_tags_where(conn, &mut tasks, &where_sql, &args)?;
        }
    }
    let depths: Vec<u32> = if filter.order == TaskOrder::Manual {
        visible_depths(&rows)
    } else {
        vec![0; rows.len()]
    };
    Ok(rows
        .into_iter()
        .zip(depths)
        .map(|((task, _), depth)| TaskRow { task, depth })
        .collect())
}

/// Depths relative to what the listing shows. Rows come in outline order
/// and each carries its materialized path, so a row's shown ancestors are
/// exactly the rows above it whose path is a prefix of its own; a parent
/// that is not in the listing (completed, filtered out) closes the gap.
/// Ancestors of the first row may sit above the window and count as shown.
fn visible_depths(rows: &[(Task, String)]) -> Vec<u32> {
    let mut stack: Vec<String> = Vec::new();
    if let Some((_, first)) = rows.first() {
        let mut prefix = String::new();
        for part in first.split('/') {
            if !prefix.is_empty() {
                stack.push(prefix.clone());
                prefix.push('/');
            }
            prefix.push_str(part);
        }
    }
    let mut out = Vec::with_capacity(rows.len());
    for (_, key) in rows {
        while let Some(top) = stack.last() {
            if key.len() > top.len() && key.starts_with(top) && key.as_bytes()[top.len()] == b'/' {
                break;
            }
            stack.pop();
        }
        out.push(stack.len() as u32);
        stack.push(key.clone());
    }
    out
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
    let mut refs: Vec<&mut Task> = tasks.iter_mut().collect();
    fill_tags_where(conn, &mut refs, "tasks.space_id = ?1", &args)?;
    let filters = conn
        .prepare_cached(&format!(
            "SELECT {FILTER_COLUMNS} FROM filters WHERE space_id = ?1 ORDER BY id"
        ))?
        .query_map(params![space_id], filter_from_row)?
        .collect::<rusqlite::Result<_>>()?;
    Ok(SpaceState {
        space: space(conn, space_id)?,
        lists,
        tags,
        tasks,
        filters,
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
