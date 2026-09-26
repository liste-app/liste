//! Request dispatch: every request maps onto store methods.

use jiff::Timestamp;
use jiff::tz::TimeZone;
use liste_ipc::protocol::{
    DebugRequest, FilterDefinition, FilterView, IpcError, ListView, OpView, PreviewView, Request,
    Response, SpanView, StatusView, TagView, TaskPatch, TaskQuery, TaskView,
};
use uuid::Uuid;

use super::{HOST_VERSION, Inner, sync_once};
use crate::ids::Id;
use crate::model::{EntityType, Field, Filter, List, Priority, Task, Value};
use crate::op::Mutation;
use crate::parse::{Context, parse};
use crate::store::{Store, StoreError, TaskFilter, TaskOrder, TaskRow, fixture};

fn id(u: Uuid) -> Id {
    Id::from_bytes(*u.as_bytes())
}

fn uuid(i: Id) -> Uuid {
    Uuid::from_bytes(*i.as_bytes())
}

fn error(e: StoreError) -> IpcError {
    match e {
        StoreError::Locked => IpcError::Locked,
        StoreError::NoSuchTask(t) => IpcError::NotFound { id: uuid(t) },
        other => IpcError::Internal {
            message: other.to_string(),
        },
    }
}

fn priority_name(p: Priority) -> &'static str {
    match p {
        Priority::None => "none",
        Priority::Low => "low",
        Priority::Medium => "medium",
        Priority::High => "high",
    }
}

fn parse_priority(s: &str) -> Option<Priority> {
    Some(match s.to_ascii_lowercase().as_str() {
        "none" | "" => Priority::None,
        "low" => Priority::Low,
        "medium" | "med" => Priority::Medium,
        "high" => Priority::High,
        _ => return None,
    })
}

fn render_due(task: &Task, tz: &TimeZone) -> Option<String> {
    let at = task.due_at?;
    let z = Timestamp::from_millisecond(at).ok()?.to_zoned(tz.clone());
    Some(if task.due_all_day {
        z.date().to_string()
    } else {
        format!("{} {:02}:{:02}", z.date(), z.hour(), z.minute())
    })
}

/// Names looked up once per response, so a 50,000-row list is one query
/// for lists and one for tags rather than one per row.
struct Names {
    lists: std::collections::HashMap<Id, String>,
    tags: std::collections::HashMap<Id, String>,
}

impl Names {
    fn load(store: &Store, space: Id) -> Result<Names, StoreError> {
        Ok(Names {
            lists: store
                .lists(space)?
                .into_iter()
                .map(|l| (l.id, l.title))
                .collect(),
            tags: store
                .tags(space)?
                .into_iter()
                .map(|t| (t.id, t.name))
                .collect(),
        })
    }
}

fn task_view(store: &Store, task: &Task, tz: &TimeZone) -> Result<TaskView, StoreError> {
    let names = Names::load(store, task.space_id)?;
    task_view_with(&names, task, tz)
}

fn task_view_with(names: &Names, task: &Task, tz: &TimeZone) -> Result<TaskView, StoreError> {
    task_view_at(names, task, 0, false, tz)
}

fn task_view_at(
    names: &Names,
    task: &Task,
    depth: u32,
    has_subtasks: bool,
    tz: &TimeZone,
) -> Result<TaskView, StoreError> {
    let list = task.list_id.and_then(|l| {
        names.lists.get(&l).map(|title| ListView {
            id: uuid(l),
            title: title.clone(),
        })
    });
    let tags: Vec<String> = task
        .tags
        .iter()
        .filter_map(|t| names.tags.get(t).cloned())
        .collect();
    Ok(TaskView {
        id: uuid(task.id),
        title: task.title.clone(),
        notes: task.notes.clone(),
        list,
        due_at: task.due_at,
        due: render_due(task, tz),
        due_all_day: task.due_all_day,
        reminder_at: task.reminder_at,
        priority: priority_name(task.priority).to_owned(),
        status: task.status.clone(),
        completed_at: task.completed_at,
        parent_id: task.parent_id.map(uuid),
        depth,
        has_subtasks,
        tags,
        recurrence: task.recurrence.clone(),
        created_at: task.created_at,
        modified_at: task.modified_at,
    })
}

fn task_views(store: &Store, tasks: &[Task], tz: &TimeZone) -> Result<Vec<TaskView>, StoreError> {
    let Some(first) = tasks.first() else {
        return Ok(Vec::new());
    };
    let names = Names::load(store, first.space_id)?;
    tasks
        .iter()
        .map(|t| task_view_with(&names, t, tz))
        .collect()
}

fn row_views(store: &Store, rows: &[TaskRow], tz: &TimeZone) -> Result<Vec<TaskView>, StoreError> {
    let Some(first) = rows.first() else {
        return Ok(Vec::new());
    };
    let names = Names::load(store, first.task.space_id)?;
    rows.iter()
        .map(|r| task_view_at(&names, &r.task, r.depth, r.has_subtasks, tz))
        .collect()
}

/// The windowed listing every list request ends in.
fn query_window(inner: &Inner, store: &Store, q: &TaskQuery) -> Result<Response, IpcError> {
    let filter = query_filter(inner, store, q)?;
    let rows = store.task_rows(inner.space_id, &filter).map_err(error)?;
    Ok(Response::Tasks(
        row_views(store, &rows, &inner.tz).map_err(error)?,
    ))
}

fn now(inner: &Inner) -> jiff::Zoned {
    Timestamp::now().to_zoned(inner.tz.clone())
}

/// Midnight `days_ahead` days from today in `now`'s zone (negative for
/// days back), and the same for the day after it.
fn day_bounds(now: &jiff::Zoned, days_ahead: i64) -> (i64, i64) {
    let at = |days: i64| -> Option<i64> {
        now.date()
            .checked_add(jiff::Span::new().days(days))
            .ok()
            .and_then(|d| d.at(0, 0, 0, 0).to_zoned(now.time_zone().clone()).ok())
            .map(|z| z.timestamp().as_millisecond())
    };
    let start = at(days_ahead).unwrap_or(if days_ahead < 0 { i64::MIN } else { i64::MAX });
    let end = at(days_ahead + 1).unwrap_or(i64::MAX);
    (start, end)
}

pub(super) fn handle(inner: &Inner, request: Request) -> Response {
    match dispatch(inner, request) {
        Ok(r) => r,
        Err(e) => Response::Error(e),
    }
}

fn dispatch(inner: &Inner, request: Request) -> Result<Response, IpcError> {
    // The sync runner takes the store itself, so it runs before the guard
    // below is taken.
    if let Request::Debug(DebugRequest::SyncNow) = &request {
        let report = sync_once(inner).map_err(|e| IpcError::Internal {
            message: e.to_string(),
        })?;
        return Ok(Response::Synced {
            pushed: report.pushed,
            pulled: report.pulled,
        });
    }
    let mut store = inner.store.lock().unwrap_or_else(|e| e.into_inner());
    let store = &mut *store;
    let space = inner.space_id;
    let tz = &inner.tz;
    let r = match request {
        Request::Hello { .. } => Response::Hello {
            protocol_version: liste_ipc::PROTOCOL_VERSION,
            host_version: HOST_VERSION.to_owned(),
        },
        Request::Status => Response::Status(StatusView {
            locked: store.is_locked(),
            device_id: uuid(inner.device_id),
            space_id: uuid(space),
            data_dir: inner.data_dir.display().to_string(),
            pending_ops: store.pending_count(space).map_err(error)?,
            host_version: HOST_VERSION.to_owned(),
        }),
        Request::Capture { text, tz: zone } => {
            let tz = match zone {
                Some(name) => TimeZone::get(&name).map_err(|_| IpcError::Invalid {
                    message: format!("unknown time zone {name:?}"),
                })?,
                None => tz.clone(),
            };
            let now = Timestamp::now().to_zoned(tz.clone());
            let captured = store
                .capture(space, &text, &now, inner.locale)
                .map_err(error)?;
            let task = store
                .task(captured.task_id)
                .map_err(error)?
                .ok_or(IpcError::NotFound {
                    id: uuid(captured.task_id),
                })?;
            Response::Captured {
                task: task_view(store, &task, &tz).map_err(error)?,
                spans: captured
                    .capture
                    .spans
                    .iter()
                    .map(|s| SpanView {
                        start: s.start,
                        end: s.end,
                        kind: format!("{:?}", s.kind).to_lowercase(),
                    })
                    .collect(),
            }
        }
        Request::Preview { text, tz: zone } => {
            let tz = match zone {
                Some(name) => TimeZone::get(&name).map_err(|_| IpcError::Invalid {
                    message: format!("unknown time zone {name:?}"),
                })?,
                None => tz.clone(),
            };
            let now = Timestamp::now().to_zoned(tz.clone());
            let names: Vec<String> = store
                .lists(space)
                .map_err(error)?
                .into_iter()
                .map(|l| l.title)
                .collect();
            let cap = parse(&text, &Context::new(now, inner.locale, &names));
            let due = cap.due.map(|d| {
                let z = crate::recurrence::to_zoned(&d, &tz);
                if d.time.is_none() {
                    z.date().to_string()
                } else {
                    format!("{} {:02}:{:02}", z.date(), z.hour(), z.minute())
                }
            });
            Response::Preview(PreviewView {
                title: cap.title.clone(),
                due,
                due_all_day: cap.is_all_day(),
                list: cap.list.as_ref().map(|l| l.name().to_owned()),
                tags: cap.tags.clone(),
                priority: priority_name(cap.priority).to_owned(),
                recurrence: cap.recurrence.as_ref().map(|r| r.to_text()),
                spans: cap
                    .spans
                    .iter()
                    .map(|s| SpanView {
                        start: s.start,
                        end: s.end,
                        kind: format!("{:?}", s.kind).to_lowercase(),
                    })
                    .collect(),
            })
        }
        Request::Search { query, limit } => {
            let tasks = store
                .search(space, &query, limit.clamp(1, 500))
                .map_err(error)?;
            Response::Tasks(task_views(store, &tasks, tz).map_err(error)?)
        }
        Request::Today => query_window(
            inner,
            store,
            &TaskQuery {
                due_to_day: Some(1),
                order: Some("due".into()),
                ..Default::default()
            },
        )?,
        Request::Upcoming { days } => query_window(
            inner,
            store,
            &TaskQuery {
                due_from_day: Some(1),
                due_to_day: Some(1 + i64::from(days.clamp(1, 3650))),
                order: Some("due".into()),
                ..Default::default()
            },
        )?,
        Request::ListTasks { list_id } => query_window(
            inner,
            store,
            &TaskQuery {
                list_id,
                inbox: list_id.is_none(),
                ..Default::default()
            },
        )?,
        Request::Query(q) => query_window(inner, store, &q)?,
        Request::Count(q) => {
            let filter = query_filter(inner, store, &q)?;
            Response::Count {
                total: store.count(space, &filter).map_err(error)?,
            }
        }
        Request::Statuses => Response::Statuses(store.statuses(space).map_err(error)?),
        Request::Reorder {
            id: task_id,
            after,
            before,
        } => {
            reorder_task(inner, store, id(task_id), after.map(id), before.map(id))?;
            let task = store
                .task(id(task_id))
                .map_err(error)?
                .ok_or(IpcError::NotFound { id: task_id })?;
            Response::Task(task_view(store, &task, tz).map_err(error)?)
        }
        Request::Delete { id: task_id } => {
            store
                .task(id(task_id))
                .map_err(error)?
                .ok_or(IpcError::NotFound { id: task_id })?;
            let op = store.op(space, EntityType::Task, id(task_id), Mutation::Delete);
            store.commit(std::slice::from_ref(&op)).map_err(error)?;
            Response::Done { changed: true }
        }
        Request::Tags => Response::Tags(
            store
                .tags(space)
                .map_err(error)?
                .into_iter()
                .map(|t| TagView {
                    id: uuid(t.id),
                    name: t.name,
                })
                .collect(),
        ),
        Request::CreateList { title } => {
            let title = title.trim().to_owned();
            if title.is_empty() {
                return Err(IpcError::Invalid {
                    message: "a list needs a title".into(),
                });
            }
            let lists = store.lists(space).map_err(error)?;
            let last = lists.last().map(|l| l.position.clone());
            let position = crate::fractional::between(last.as_deref(), None)
                .unwrap_or_else(|| crate::fractional::FIRST.to_owned());
            let list_id = Id::new();
            let now_ms = Timestamp::now().as_millisecond();
            let ops = vec![
                store.op(
                    space,
                    EntityType::List,
                    list_id,
                    Mutation::Set {
                        field: Field::Title,
                        value: Value::from(title.as_str()),
                    },
                ),
                store.op(
                    space,
                    EntityType::List,
                    list_id,
                    Mutation::Set {
                        field: Field::Position,
                        value: Value::from(position),
                    },
                ),
                store.op(
                    space,
                    EntityType::List,
                    list_id,
                    Mutation::Set {
                        field: Field::CreatedAt,
                        value: Value::Int(now_ms),
                    },
                ),
            ];
            store.commit(&ops).map_err(error)?;
            Response::List(ListView {
                id: uuid(list_id),
                title,
            })
        }
        Request::UpdateList {
            id: list_id,
            title,
            after,
            before,
        } => {
            let list = store
                .list(id(list_id))
                .map_err(error)?
                .ok_or(IpcError::NotFound { id: list_id })?;
            let mut ops = Vec::new();
            if let Some(title) = title {
                let title = title.trim();
                if title.is_empty() {
                    return Err(IpcError::Invalid {
                        message: "a list needs a title".into(),
                    });
                }
                ops.push(store.op(
                    space,
                    EntityType::List,
                    list.id,
                    Mutation::Set {
                        field: Field::Title,
                        value: Value::from(title),
                    },
                ));
            }
            if after.is_some() || before.is_some() {
                let lists = store.lists(space).map_err(error)?;
                let pos = |target: Option<Uuid>| -> Result<Option<String>, IpcError> {
                    match target {
                        None => Ok(None),
                        Some(t) => lists
                            .iter()
                            .find(|l| l.id == id(t))
                            .map(|l| Some(l.position.clone()))
                            .ok_or(IpcError::NotFound { id: t }),
                    }
                };
                let lo = pos(after)?;
                let hi = pos(before)?;
                let key =
                    crate::fractional::between(lo.as_deref(), hi.as_deref()).ok_or_else(|| {
                        IpcError::Invalid {
                            message: "no position between those lists".into(),
                        }
                    })?;
                ops.push(store.op(
                    space,
                    EntityType::List,
                    list.id,
                    Mutation::Set {
                        field: Field::Position,
                        value: Value::from(key),
                    },
                ));
            }
            if !ops.is_empty() {
                store.commit(&ops).map_err(error)?;
            }
            let list = store
                .list(id(list_id))
                .map_err(error)?
                .ok_or(IpcError::NotFound { id: list_id })?;
            Response::List(ListView {
                id: uuid(list.id),
                title: list.title,
            })
        }
        Request::DeleteList { id: list_id } => {
            store
                .list(id(list_id))
                .map_err(error)?
                .ok_or(IpcError::NotFound { id: list_id })?;
            // Tasks in the list move to the inbox in the same step, so the
            // list's disappearance never hides them.
            let tasks = store
                .tasks(
                    space,
                    &TaskFilter {
                        list: Some(Some(id(list_id))),
                        include_completed: true,
                        ..Default::default()
                    },
                )
                .map_err(error)?;
            let mut ops: Vec<crate::op::Op> = tasks
                .iter()
                .map(|t| {
                    store.op(
                        space,
                        EntityType::Task,
                        t.id,
                        Mutation::Set {
                            field: Field::ListId,
                            value: Value::Null,
                        },
                    )
                })
                .collect();
            ops.push(store.op(space, EntityType::List, id(list_id), Mutation::Delete));
            store.commit(&ops).map_err(error)?;
            Response::Done { changed: true }
        }
        Request::Filters => Response::Filters(
            store
                .filters(space)
                .map_err(error)?
                .iter()
                .map(filter_view)
                .collect(),
        ),
        Request::CreateFilter { name, definition } => {
            let name = name.trim().to_owned();
            if name.is_empty() {
                return Err(IpcError::Invalid {
                    message: "a filter needs a name".into(),
                });
            }
            let filters = store.filters(space).map_err(error)?;
            let last = filters.last().map(|f| f.position.clone());
            let position = crate::fractional::between(last.as_deref(), None)
                .unwrap_or_else(|| crate::fractional::FIRST.to_owned());
            let filter_id = Id::new();
            let now_ms = Timestamp::now().as_millisecond();
            let mut ops = vec![
                filter_set(
                    store,
                    space,
                    filter_id,
                    Field::Name,
                    Value::from(name.as_str()),
                ),
                filter_set(
                    store,
                    space,
                    filter_id,
                    Field::Position,
                    Value::from(position),
                ),
                filter_set(
                    store,
                    space,
                    filter_id,
                    Field::CreatedAt,
                    Value::Int(now_ms),
                ),
            ];
            ops.extend(definition_ops(store, space, filter_id, &definition)?);
            store.commit(&ops).map_err(error)?;
            let filter = store
                .filter(filter_id)
                .map_err(error)?
                .ok_or(IpcError::NotFound {
                    id: uuid(filter_id),
                })?;
            Response::Filter(filter_view(&filter))
        }
        Request::UpdateFilter {
            id: filter_id,
            name,
            definition,
            after,
            before,
        } => {
            let filter = store
                .filter(id(filter_id))
                .map_err(error)?
                .ok_or(IpcError::NotFound { id: filter_id })?;
            let mut ops = Vec::new();
            if let Some(name) = name {
                let name = name.trim();
                if name.is_empty() {
                    return Err(IpcError::Invalid {
                        message: "a filter needs a name".into(),
                    });
                }
                ops.push(filter_set(
                    store,
                    space,
                    filter.id,
                    Field::Name,
                    Value::from(name),
                ));
            }
            if let Some(definition) = &definition {
                ops.extend(definition_ops(store, space, filter.id, definition)?);
            }
            if after.is_some() || before.is_some() {
                let filters = store.filters(space).map_err(error)?;
                let pos = |target: Option<Uuid>| -> Result<Option<String>, IpcError> {
                    match target {
                        None => Ok(None),
                        Some(t) => filters
                            .iter()
                            .find(|f| f.id == id(t))
                            .map(|f| Some(f.position.clone()))
                            .ok_or(IpcError::NotFound { id: t }),
                    }
                };
                let lo = pos(after)?;
                let hi = pos(before)?;
                let key =
                    crate::fractional::between(lo.as_deref(), hi.as_deref()).ok_or_else(|| {
                        IpcError::Invalid {
                            message: "no position between those filters".into(),
                        }
                    })?;
                ops.push(filter_set(
                    store,
                    space,
                    filter.id,
                    Field::Position,
                    Value::from(key),
                ));
            }
            if !ops.is_empty() {
                store.commit(&ops).map_err(error)?;
            }
            let filter = store
                .filter(id(filter_id))
                .map_err(error)?
                .ok_or(IpcError::NotFound { id: filter_id })?;
            Response::Filter(filter_view(&filter))
        }
        Request::DeleteFilter { id: filter_id } => {
            store
                .filter(id(filter_id))
                .map_err(error)?
                .ok_or(IpcError::NotFound { id: filter_id })?;
            let op = store.op(space, EntityType::Filter, id(filter_id), Mutation::Delete);
            store.commit(std::slice::from_ref(&op)).map_err(error)?;
            Response::Done { changed: true }
        }
        Request::GetTask { id: task_id } => {
            let task = store
                .task(id(task_id))
                .map_err(error)?
                .ok_or(IpcError::NotFound { id: task_id })?;
            Response::Task(task_view(store, &task, tz).map_err(error)?)
        }
        Request::UpdateTask { id: task_id, patch } => {
            update_task(inner, store, id(task_id), patch)?;
            let task = store
                .task(id(task_id))
                .map_err(error)?
                .ok_or(IpcError::NotFound { id: task_id })?;
            Response::Task(task_view(store, &task, tz).map_err(error)?)
        }
        Request::Complete { id: task_id } => {
            let now = now(inner);
            store.complete(space, id(task_id), &now).map_err(error)?;
            let task = store
                .task(id(task_id))
                .map_err(error)?
                .ok_or(IpcError::NotFound { id: task_id })?;
            Response::Task(task_view(store, &task, tz).map_err(error)?)
        }
        Request::Uncomplete { id: task_id } => {
            store
                .task(id(task_id))
                .map_err(error)?
                .ok_or(IpcError::NotFound { id: task_id })?;
            let op = store.op(
                space,
                EntityType::Task,
                id(task_id),
                Mutation::Set {
                    field: Field::CompletedAt,
                    value: Value::Null,
                },
            );
            store.commit(std::slice::from_ref(&op)).map_err(error)?;
            let task = store
                .task(id(task_id))
                .map_err(error)?
                .ok_or(IpcError::NotFound { id: task_id })?;
            Response::Task(task_view(store, &task, tz).map_err(error)?)
        }
        Request::Lists => Response::Lists(
            store
                .lists(space)
                .map_err(error)?
                .into_iter()
                .map(|l: List| ListView {
                    id: uuid(l.id),
                    title: l.title,
                })
                .collect(),
        ),
        Request::Undo => Response::Done {
            changed: store.undo().map_err(error)?,
        },
        Request::Redo => Response::Done {
            changed: store.redo().map_err(error)?,
        },
        Request::Debug(d) => debug(inner, store, d)?,
    };
    Ok(r)
}

fn update_task(
    inner: &Inner,
    store: &mut Store,
    task_id: Id,
    patch: TaskPatch,
) -> Result<(), IpcError> {
    let space = inner.space_id;
    let task = store
        .task(task_id)
        .map_err(error)?
        .ok_or(IpcError::NotFound { id: uuid(task_id) })?;
    let now_ms = Timestamp::now().as_millisecond();
    let mut ops = Vec::new();
    let set = |store: &mut Store, field: Field, value: Value| -> crate::op::Op {
        store.op(
            space,
            EntityType::Task,
            task_id,
            Mutation::Set { field, value },
        )
    };
    if let Some(title) = &patch.title {
        ops.push(set(store, Field::Title, Value::from(title.trim())));
    }
    if let Some(due) = &patch.due {
        if due.trim().is_empty() {
            ops.push(set(store, Field::DueAt, Value::Null));
            ops.push(set(store, Field::DueAllDay, Value::Bool(false)));
        } else {
            let now = now(inner);
            let cap = parse(due, &Context::new(now.clone(), inner.locale, &[]));
            let Some(occ) = cap.due else {
                return Err(IpcError::Invalid {
                    message: format!("could not read a date from {due:?}"),
                });
            };
            set(
                store,
                Field::DueAt,
                Value::Int(crate::recurrence::to_millis(&occ, now.time_zone())),
            );
            ops.push(set(
                store,
                Field::DueAllDay,
                Value::Bool(occ.time.is_none()),
            ));
        }
    }
    if let Some(p) = &patch.priority {
        let priority = parse_priority(p).ok_or_else(|| IpcError::Invalid {
            message: format!("unknown priority {p:?}"),
        })?;
        ops.push(set(store, Field::Priority, Value::Int(priority as i64)));
    }
    if let Some(name) = &patch.list {
        if name.trim().is_empty() {
            ops.push(set(store, Field::ListId, Value::Null));
        } else {
            let lists = store.lists(space).map_err(error)?;
            let list_id = match lists
                .iter()
                .find(|l| l.title.eq_ignore_ascii_case(name.trim()))
            {
                Some(l) => l.id,
                None => {
                    let id = Id::new();
                    ops.push(store.op(
                        space,
                        EntityType::List,
                        id,
                        Mutation::Set {
                            field: Field::Title,
                            value: Value::from(name.trim()),
                        },
                    ));
                    ops.push(store.op(
                        space,
                        EntityType::List,
                        id,
                        Mutation::Set {
                            field: Field::CreatedAt,
                            value: Value::Int(now_ms),
                        },
                    ));
                    id
                }
            };
            ops.push(set(store, Field::ListId, Value::Id(list_id)));
        }
    }
    if let Some(status) = &patch.status {
        let status = status.trim();
        ops.push(set(
            store,
            Field::Status,
            Value::from(if status.is_empty() { "open" } else { status }),
        ));
    }
    if let Some(parent) = &patch.parent {
        if parent.trim().is_empty() {
            ops.push(set(store, Field::ParentId, Value::Null));
        } else {
            let parent_id: Uuid = parent.trim().parse().map_err(|_| IpcError::Invalid {
                message: format!("{parent:?} is not a task id"),
            })?;
            if id(parent_id) == task_id {
                return Err(IpcError::Invalid {
                    message: "a task cannot be its own parent".into(),
                });
            }
            store
                .task(id(parent_id))
                .map_err(error)?
                .ok_or(IpcError::NotFound { id: parent_id })?;
            ops.push(set(store, Field::ParentId, Value::Id(id(parent_id))));
        }
    }
    if let Some(reminder) = &patch.reminder {
        if reminder.trim().is_empty() {
            ops.push(set(store, Field::ReminderAt, Value::Null));
        } else {
            let now = now(inner);
            let cap = parse(reminder, &Context::new(now.clone(), inner.locale, &[]));
            let Some(occ) = cap.due else {
                return Err(IpcError::Invalid {
                    message: format!("could not read a time from {reminder:?}"),
                });
            };
            ops.push(set(
                store,
                Field::ReminderAt,
                Value::Int(crate::recurrence::to_millis(&occ, now.time_zone())),
            ));
        }
    }
    if let Some(notes) = &patch.notes {
        ops.push(store.op(
            space,
            EntityType::Task,
            task_id,
            Mutation::SetNotes {
                text: notes.clone(),
            },
        ));
    }
    let tags = store.tags(space).map_err(error)?;
    for name in &patch.add_tags {
        let name = name.trim_start_matches('#').trim();
        if name.is_empty() {
            continue;
        }
        let tag_id = match tags.iter().find(|t| t.name.eq_ignore_ascii_case(name)) {
            Some(t) => t.id,
            None => {
                let id = Id::new();
                ops.push(store.op(
                    space,
                    EntityType::Tag,
                    id,
                    Mutation::Set {
                        field: Field::Name,
                        value: Value::from(name),
                    },
                ));
                id
            }
        };
        if !task.tags.contains(&tag_id) {
            ops.push(store.op(
                space,
                EntityType::Task,
                task_id,
                Mutation::AddTag { tag: tag_id },
            ));
        }
    }
    for name in &patch.remove_tags {
        let name = name.trim_start_matches('#').trim();
        if let Some(t) = tags.iter().find(|t| t.name.eq_ignore_ascii_case(name))
            && let Some(op) = store.remove_tag_op(space, task_id, t.id).map_err(error)?
        {
            ops.push(op);
        }
    }
    if ops.is_empty() {
        return Ok(());
    }
    store.commit(&ops).map_err(error)
}

fn debug(inner: &Inner, store: &mut Store, request: DebugRequest) -> Result<Response, IpcError> {
    let space = inner.space_id;
    Ok(match request {
        DebugRequest::OpLog { limit } => Response::OpLog(
            store
                .recent_ops(space, limit.clamp(1, 1000))
                .map_err(error)?
                .into_iter()
                .map(|l| OpView {
                    op_id: uuid(l.op.op_id),
                    seq: l.seq,
                    applied: l.applied,
                    hlc: format!(
                        "{}:{}:{}",
                        l.op.hlc.wall_ms, l.op.hlc.counter, l.op.hlc.device
                    ),
                    entity_type: l.op.entity_type.as_str().to_owned(),
                    entity_id: uuid(l.op.entity_id),
                    mutation: serde_json::to_value(&l.op.mutation).unwrap_or_default(),
                })
                .collect(),
        ),
        DebugRequest::Row { entity, id: row_id } => {
            let value = match entity.as_str() {
                "task" => serde_json::to_value(store.task(id(row_id)).map_err(error)?),
                "list" => serde_json::to_value(store.list(id(row_id)).map_err(error)?),
                "tag" => serde_json::to_value(store.tag(id(row_id)).map_err(error)?),
                "filter" => serde_json::to_value(store.filter(id(row_id)).map_err(error)?),
                other => {
                    return Err(IpcError::Invalid {
                        message: format!(
                            "unknown entity {other:?}; use task, list, tag, or filter"
                        ),
                    });
                }
            }
            .unwrap_or_default();
            if value.is_null() {
                return Err(IpcError::NotFound { id: row_id });
            }
            Response::Row(value)
        }
        DebugRequest::Cursor => Response::Cursor {
            last_seq: store.cursor(space).map_err(error)?,
            pending: store.pending_count(space).map_err(error)?,
            op_count: store.op_count(space).map_err(error)?,
        },
        DebugRequest::Fixture { tasks, seed } => {
            let f = fixture::populate(store, space, tasks.min(1_000_000), seed).map_err(error)?;
            Response::Fixture {
                tasks: f.tasks.len(),
                ops: f.ops,
            }
        }
        DebugRequest::Scenario { name } => {
            let r = super::scenario::run(&name);
            Response::Scenario {
                name: r.name,
                passed: r.passed,
                report: r.report,
            }
        }
        DebugRequest::SyncNow => {
            return Err(IpcError::Invalid {
                message: "sync_now is handled before the store is locked".into(),
            });
        }
    })
}

fn filter_view(f: &Filter) -> FilterView {
    FilterView {
        id: uuid(f.id),
        name: f.name.clone(),
        definition: FilterDefinition {
            list_id: f.list_id.map(uuid),
            tag_id: f.tag_id.map(uuid),
            priority: f.priority.map(|p| priority_name(p).to_owned()),
            status: f.status.clone(),
            due_from_day: f.due_from_day,
            due_to_day: f.due_to_day,
            include_completed: f.include_completed,
        },
    }
}

fn filter_set(
    store: &mut Store,
    space: Id,
    filter: Id,
    field: Field,
    value: Value,
) -> crate::op::Op {
    store.op(
        space,
        EntityType::Filter,
        filter,
        Mutation::Set { field, value },
    )
}

/// One op per criterion, so a definition is replaced whole.
fn definition_ops(
    store: &mut Store,
    space: Id,
    filter: Id,
    d: &FilterDefinition,
) -> Result<Vec<crate::op::Op>, IpcError> {
    let priority = match &d.priority {
        Some(p) => Some(parse_priority(p).ok_or_else(|| IpcError::Invalid {
            message: format!("unknown priority {p:?}"),
        })?),
        None => None,
    };
    let status = d
        .status
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    Ok(vec![
        filter_set(
            store,
            space,
            filter,
            Field::ListId,
            Value::from(d.list_id.map(id)),
        ),
        filter_set(
            store,
            space,
            filter,
            Field::TagId,
            Value::from(d.tag_id.map(id)),
        ),
        filter_set(
            store,
            space,
            filter,
            Field::FilterPriority,
            Value::from(priority.map(|p| p as i64)),
        ),
        filter_set(
            store,
            space,
            filter,
            Field::FilterStatus,
            Value::from(status),
        ),
        filter_set(
            store,
            space,
            filter,
            Field::DueFromDay,
            Value::from(d.due_from_day),
        ),
        filter_set(
            store,
            space,
            filter,
            Field::DueToDay,
            Value::from(d.due_to_day),
        ),
        filter_set(
            store,
            space,
            filter,
            Field::IncludeCompleted,
            Value::Bool(d.include_completed),
        ),
    ])
}

fn query_filter(inner: &Inner, store: &Store, q: &TaskQuery) -> Result<TaskFilter, IpcError> {
    let q = match q.filter_id {
        None => q.clone(),
        Some(filter_id) => {
            // The saved criteria come first; the request's own fields
            // narrow them. A filter with a due window lists by due date.
            let f = store
                .filter(id(filter_id))
                .map_err(error)?
                .filter(|f| f.deleted_at.is_none())
                .ok_or(IpcError::NotFound { id: filter_id })?;
            let has_window = f.due_from_day.is_some() || f.due_to_day.is_some();
            TaskQuery {
                filter_id: None,
                list_id: q.list_id.or(f.list_id.map(uuid)),
                tag_id: q.tag_id.or(f.tag_id.map(uuid)),
                priority: q
                    .priority
                    .clone()
                    .or(f.priority.map(|p| priority_name(p).to_owned())),
                status: q.status.clone().or(f.status.clone()),
                due_from_day: q.due_from_day.or(f.due_from_day),
                due_to_day: q.due_to_day.or(f.due_to_day),
                include_completed: q.include_completed || f.include_completed,
                order: q
                    .order
                    .clone()
                    .or_else(|| has_window.then(|| "due".to_owned())),
                ..q.clone()
            }
        }
    };
    let q = &q;
    let priority = match &q.priority {
        Some(p) => Some(parse_priority(p).ok_or_else(|| IpcError::Invalid {
            message: format!("unknown priority {p:?}"),
        })?),
        None => None,
    };
    let order = match q.order.as_deref() {
        None | Some("manual") => TaskOrder::Manual,
        Some("due") => TaskOrder::DueThenManual,
        Some("completed") => TaskOrder::CompletedDesc,
        Some(other) => {
            return Err(IpcError::Invalid {
                message: format!("unknown order {other:?}"),
            });
        }
    };
    // Day-relative bounds are resolved here, in the host's zone, and
    // intersected with any absolute bounds.
    let mut from = q.due_from;
    let mut to = q.due_to;
    if q.due_from_day.is_some() || q.due_to_day.is_some() {
        let now = now(inner);
        if let Some(d) = q.due_from_day {
            let (start, _) = day_bounds(&now, d);
            from = Some(from.map_or(start, |f| f.max(start)));
        }
        if let Some(d) = q.due_to_day {
            let (end, _) = day_bounds(&now, d);
            to = Some(to.map_or(end, |t| t.min(end)));
        }
    }
    Ok(TaskFilter {
        list: if q.inbox {
            Some(None)
        } else {
            q.list_id.map(|l| Some(id(l)))
        },
        parent: q.parent_id.map(id),
        tag: q.tag_id.map(id),
        priority,
        status: q.status.clone(),
        due_between: match (from, to) {
            (None, None) => None,
            (from, to) => Some((from.unwrap_or(i64::MIN), to.unwrap_or(i64::MAX))),
        },
        has_reminder: q.has_reminder,
        include_completed: q.include_completed,
        completed_only: q.completed_only,
        include_deleted: false,
        collapsed: q.collapsed.iter().copied().map(id).collect(),
        order,
        offset: q.offset,
        limit: q.limit.min(100_000),
    })
}

/// Give `task_id` a position key between its two new neighbours. Only the
/// moved task changes; a list too crowded for a key is rebalanced first.
fn reorder_task(
    inner: &Inner,
    store: &mut Store,
    task_id: Id,
    after: Option<Id>,
    before: Option<Id>,
) -> Result<(), IpcError> {
    let space = inner.space_id;
    let task = store
        .task(task_id)
        .map_err(error)?
        .ok_or(IpcError::NotFound { id: uuid(task_id) })?;
    let position_of = |store: &Store, t: Option<Id>| -> Result<Option<String>, IpcError> {
        match t {
            None => Ok(None),
            Some(t) => store
                .task(t)
                .map_err(error)?
                .map(|t| Some(t.position))
                .ok_or(IpcError::NotFound { id: uuid(t) }),
        }
    };
    let lo = position_of(store, after)?;
    let hi = position_of(store, before)?;
    let key = match crate::fractional::between(lo.as_deref(), hi.as_deref()) {
        Some(k) if !crate::fractional::needs_rebalance([k.as_str()]) => k,
        _ => {
            // Rebalance every sibling in the same list, then place the task.
            let siblings = store
                .tasks(
                    space,
                    &TaskFilter {
                        list: Some(task.list_id),
                        include_completed: true,
                        ..Default::default()
                    },
                )
                .map_err(error)?;
            let mut order: Vec<Id> = siblings
                .iter()
                .map(|t| t.id)
                .filter(|t| *t != task_id)
                .collect();
            let index = match (after, before) {
                (Some(a), _) => order
                    .iter()
                    .position(|t| *t == a)
                    .map(|i| i + 1)
                    .unwrap_or(order.len()),
                (None, Some(b)) => order.iter().position(|t| *t == b).unwrap_or(0),
                (None, None) => order.len(),
            };
            order.insert(index.min(order.len()), task_id);
            let keys = crate::fractional::rebalanced(order.len());
            let ops: Vec<crate::op::Op> = order
                .iter()
                .zip(keys)
                .map(|(t, k)| {
                    store.op(
                        space,
                        EntityType::Task,
                        *t,
                        Mutation::Set {
                            field: Field::Position,
                            value: Value::from(k),
                        },
                    )
                })
                .collect();
            store.commit(&ops).map_err(error)?;
            return Ok(());
        }
    };
    let op = store.op(
        space,
        EntityType::Task,
        task_id,
        Mutation::Set {
            field: Field::Position,
            value: Value::from(key),
        },
    );
    store.commit(std::slice::from_ref(&op)).map_err(error)
}
