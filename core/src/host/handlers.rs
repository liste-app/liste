//! Request dispatch: every request maps onto store methods.

use jiff::Timestamp;
use jiff::tz::TimeZone;
use liste_ipc::protocol::{
    DebugRequest, IpcError, ListView, OpView, Request, Response, SpanView, StatusView, TaskPatch,
    TaskView,
};
use uuid::Uuid;

use super::{HOST_VERSION, Inner, sync_once};
use crate::ids::Id;
use crate::model::{EntityType, Field, List, Priority, Task, Value};
use crate::op::Mutation;
use crate::parse::{Context, parse};
use crate::store::{Store, StoreError, TaskFilter, fixture};

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

fn task_view(store: &Store, task: &Task, tz: &TimeZone) -> Result<TaskView, StoreError> {
    let list = match task.list_id {
        Some(l) => store.list(l)?.map(|l| ListView {
            id: uuid(l.id),
            title: l.title,
        }),
        None => None,
    };
    let mut tags = Vec::new();
    for t in &task.tags {
        if let Some(tag) = store.tag(*t)? {
            tags.push(tag.name);
        }
    }
    Ok(TaskView {
        id: uuid(task.id),
        title: task.title.clone(),
        notes: task.notes.clone(),
        list,
        due_at: task.due_at,
        due: render_due(task, tz),
        due_all_day: task.due_all_day,
        priority: priority_name(task.priority).to_owned(),
        status: task.status.clone(),
        completed_at: task.completed_at,
        tags,
        recurrence: task.recurrence.clone(),
        created_at: task.created_at,
        modified_at: task.modified_at,
    })
}

fn task_views(store: &Store, tasks: &[Task], tz: &TimeZone) -> Result<Vec<TaskView>, StoreError> {
    tasks.iter().map(|t| task_view(store, t, tz)).collect()
}

fn now(inner: &Inner) -> jiff::Zoned {
    Timestamp::now().to_zoned(inner.tz.clone())
}

fn day_bounds(now: &jiff::Zoned, days_ahead: i64) -> (i64, i64) {
    let start = now.date().at(0, 0, 0, 0).to_zoned(now.time_zone().clone());
    let start = start.map(|z| z.timestamp().as_millisecond()).unwrap_or(0);
    let end = now
        .date()
        .checked_add(jiff::Span::new().days(days_ahead))
        .and_then(|d| d.at(0, 0, 0, 0).to_zoned(now.time_zone().clone()))
        .map(|z| z.timestamp().as_millisecond())
        .unwrap_or(i64::MAX);
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
        Request::Search { query, limit } => {
            let tasks = store
                .search(space, &query, limit.clamp(1, 500))
                .map_err(error)?;
            Response::Tasks(task_views(store, &tasks, tz).map_err(error)?)
        }
        Request::Today => {
            let now = now(inner);
            let (_, end) = day_bounds(&now, 1);
            // Due today or overdue.
            let tasks = store
                .tasks(
                    space,
                    &TaskFilter {
                        due_between: Some((i64::MIN, end)),
                        ..Default::default()
                    },
                )
                .map_err(error)?;
            Response::Tasks(task_views(store, &tasks, tz).map_err(error)?)
        }
        Request::Upcoming { days } => {
            let now = now(inner);
            let (_, start) = day_bounds(&now, 1);
            let (_, end) = day_bounds(&now, 1 + i64::from(days.clamp(1, 3650)));
            let tasks = store
                .tasks(
                    space,
                    &TaskFilter {
                        due_between: Some((start, end)),
                        ..Default::default()
                    },
                )
                .map_err(error)?;
            Response::Tasks(task_views(store, &tasks, tz).map_err(error)?)
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
                other => {
                    return Err(IpcError::Invalid {
                        message: format!("unknown entity {other:?}; use task, list, or tag"),
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
