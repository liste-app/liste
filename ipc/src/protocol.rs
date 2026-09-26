//! The message types and framing.
//!
//! Every frame is a 4-byte big-endian length followed by a JSON
//! [`Message`]. A client's first message must be [`Request::Hello`]; the
//! host answers [`Response::Hello`] or [`IpcError::VersionMismatch`] and
//! closes. After that, each request carries an id that the response echoes.

use std::fmt;
use std::io::{self, Read, Write};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// The IPC protocol version. Client and host must agree exactly.
pub const PROTOCOL_VERSION: u32 = 1;

/// Frames larger than this are refused.
pub const MAX_FRAME: usize = 16 * 1024 * 1024;

/// One frame in either direction.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub id: u64,
    #[serde(flatten)]
    pub body: Body,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "body", rename_all = "snake_case")]
pub enum Body {
    Request(Request),
    Response(Response),
}

/// What a client can ask.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", content = "args", rename_all = "snake_case")]
pub enum Request {
    Hello {
        protocol_version: u32,
        client: String,
    },
    Status,
    /// One natural-language line. `tz` overrides the host's zone (IANA name).
    Capture {
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tz: Option<String>,
    },
    /// Parse a line without creating anything, for live highlighting.
    Preview {
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tz: Option<String>,
    },
    Search {
        query: String,
        limit: usize,
    },
    Today,
    Upcoming {
        days: u32,
    },
    /// Open tasks in one list, or in the inbox when `list_id` is absent.
    ListTasks {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        list_id: Option<Uuid>,
    },
    /// Any combination of filters (smart lists, tags, saved filters),
    /// windowed by `offset` and `limit`.
    Query(TaskQuery),
    /// How many tasks a query matches, ignoring its window. A list asks
    /// this once, then fetches only the rows on screen.
    Count(TaskQuery),
    /// Every status name in use, for board columns.
    Statuses,
    /// Move a task in manual order to sit between two neighbours; either
    /// may be absent for the ends.
    Reorder {
        id: Uuid,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        after: Option<Uuid>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        before: Option<Uuid>,
    },
    /// Tombstone a task. Undo restores it.
    Delete {
        id: Uuid,
    },
    Tags,
    CreateList {
        title: String,
    },
    UpdateList {
        id: Uuid,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        after: Option<Uuid>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        before: Option<Uuid>,
    },
    DeleteList {
        id: Uuid,
    },
    /// Saved filters (Section 5), in manual order.
    Filters,
    CreateFilter {
        name: String,
        #[serde(default)]
        definition: FilterDefinition,
    },
    /// Rename, redefine, or move a saved filter; absent parts are kept.
    UpdateFilter {
        id: Uuid,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        definition: Option<FilterDefinition>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        after: Option<Uuid>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        before: Option<Uuid>,
    },
    DeleteFilter {
        id: Uuid,
    },
    GetTask {
        id: Uuid,
    },
    UpdateTask {
        id: Uuid,
        patch: TaskPatch,
    },
    Complete {
        id: Uuid,
    },
    Uncomplete {
        id: Uuid,
    },
    Lists,
    Undo,
    Redo,
    Debug(DebugRequest),
}

/// What a saved filter selects. Absent fields do not filter.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct FilterDefinition {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub list_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag_id: Option<Uuid>,
    /// `high`, `medium`, `low`, or `none`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// Due in `[today + due_from_day, today + due_to_day)`, whole days.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due_from_day: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due_to_day: Option<i64>,
    #[serde(default)]
    pub include_completed: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FilterView {
    pub id: Uuid,
    pub name: String,
    #[serde(flatten)]
    pub definition: FilterDefinition,
}

/// A filter over tasks. Absent fields do not filter.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TaskQuery {
    /// A saved filter's criteria, evaluated by the host; the fields below
    /// narrow it further, and the window applies as usual.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub list_id: Option<Uuid>,
    /// Only tasks with no list.
    #[serde(default)]
    pub inbox: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// Only direct subtasks of this task.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<Uuid>,
    /// Due in `[due_from, due_to)`, Unix milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due_from: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due_to: Option<i64>,
    /// Due in `[today + due_from_day, today + due_to_day)` in whole days
    /// of the host's zone, so a client never computes a day boundary:
    /// today is `due_to_day: 1`, overdue is `due_to_day: 0`, the next
    /// week is `due_from_day: 1, due_to_day: 8`. Combined with the
    /// absolute bounds above by intersection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due_from_day: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due_to_day: Option<i64>,
    #[serde(default)]
    pub has_reminder: bool,
    #[serde(default)]
    pub include_completed: bool,
    #[serde(default)]
    pub completed_only: bool,
    /// `manual` (default), `due`, or `completed`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order: Option<String>,
    /// Rows to skip.
    #[serde(default)]
    pub offset: usize,
    /// Zero means no limit.
    #[serde(default)]
    pub limit: usize,
}

/// Fields to change on a task. Absent fields are untouched.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TaskPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    /// Natural-language date text; an empty string clears the due date.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due: Option<String>,
    /// `high`, `medium`, `low`, or `none`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<String>,
    /// A list name; an empty string moves the task to the inbox.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub list: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub add_tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub remove_tags: Vec<String>,
    /// A status name, for kanban columns grouped by status.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// A parent task id; an empty string makes the task top-level.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// Reminder as natural-language date text; an empty string clears it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reminder: Option<String>,
}

/// The developer harness (Section 13, `liste debug`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum DebugRequest {
    /// The most recent ops in the log.
    OpLog { limit: usize },
    /// One materialized row: `entity` is `task`, `list`, `tag`, or `filter`.
    Row { entity: String, id: Uuid },
    /// The sync cursor and pending count.
    Cursor,
    /// Fill the store with the fixture.
    Fixture { tasks: usize, seed: u64 },
    /// Run a named two-device scenario in memory.
    Scenario { name: String },
    /// Run the sync runner once.
    SyncNow,
}

/// What the host answers.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "result", content = "data", rename_all = "snake_case")]
pub enum Response {
    Hello {
        protocol_version: u32,
        host_version: String,
    },
    Status(StatusView),
    Task(TaskView),
    Tasks(Vec<TaskView>),
    Count {
        total: usize,
    },
    Statuses(Vec<String>),
    Captured {
        task: TaskView,
        spans: Vec<SpanView>,
    },
    Preview(PreviewView),
    Lists(Vec<ListView>),
    List(ListView),
    Tags(Vec<TagView>),
    Filters(Vec<FilterView>),
    Filter(FilterView),
    Done {
        changed: bool,
    },
    OpLog(Vec<OpView>),
    Row(serde_json::Value),
    Cursor {
        last_seq: u64,
        pending: u64,
        op_count: u64,
    },
    Fixture {
        tasks: usize,
        ops: usize,
    },
    Scenario {
        name: String,
        passed: bool,
        report: String,
    },
    Synced {
        pushed: usize,
        pulled: usize,
    },
    Error(IpcError),
}

/// Why a request failed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum IpcError {
    #[error("locked: keys have not been unlocked on this device")]
    Locked,
    #[error("restart Liste to finish updating (host protocol {host}, client protocol {client})")]
    VersionMismatch { host: u32, client: u32 },
    #[error("not found: {id}")]
    NotFound { id: Uuid },
    #[error("invalid request: {message}")]
    Invalid { message: String },
    #[error("{message}")]
    Internal { message: String },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StatusView {
    pub locked: bool,
    pub device_id: Uuid,
    pub space_id: Uuid,
    pub data_dir: String,
    pub pending_ops: u64,
    pub host_version: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ListView {
    pub id: Uuid,
    pub title: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TagView {
    pub id: Uuid,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TaskView {
    pub id: Uuid,
    pub title: String,
    pub notes: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub list: Option<ListView>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due_at: Option<i64>,
    /// `due_at` rendered in the host's zone, for display.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due: Option<String>,
    pub due_all_day: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reminder_at: Option<i64>,
    pub priority: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<Uuid>,
    /// How far this row is indented under rows above it in the same
    /// listing; zero outside manual order.
    #[serde(default)]
    pub depth: u32,
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recurrence: Option<String>,
    pub created_at: i64,
    pub modified_at: i64,
}

/// What a line would become, before it is captured.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PreviewView {
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due: Option<String>,
    pub due_all_day: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub list: Option<String>,
    pub tags: Vec<String>,
    pub priority: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recurrence: Option<String>,
    pub spans: Vec<SpanView>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SpanView {
    pub start: usize,
    pub end: usize,
    pub kind: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OpView {
    pub op_id: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seq: Option<u64>,
    pub applied: bool,
    pub hlc: String,
    pub entity_type: String,
    pub entity_id: Uuid,
    pub mutation: serde_json::Value,
}

impl fmt::Display for Message {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}",
            serde_json::to_string(self).map_err(|_| fmt::Error)?
        )
    }
}

/// Write one frame.
pub fn write_frame<W: Write>(w: &mut W, message: &Message) -> io::Result<()> {
    let bytes = serde_json::to_vec(message).map_err(io::Error::other)?;
    if bytes.len() > MAX_FRAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "frame too large",
        ));
    }
    w.write_all(&(bytes.len() as u32).to_be_bytes())?;
    w.write_all(&bytes)?;
    w.flush()
}

/// Read one frame. `Ok(None)` at a clean end of stream.
pub fn read_frame<R: Read>(r: &mut R) -> io::Result<Option<Message>> {
    let mut len = [0u8; 4];
    match r.read_exact(&mut len) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let len = u32::from_be_bytes(len) as usize;
    if len > MAX_FRAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "frame too large",
        ));
    }
    let mut bytes = vec![0u8; len];
    r.read_exact(&mut bytes)?;
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_round_trip() {
        let msg = Message {
            id: 7,
            body: Body::Request(Request::Capture {
                text: "call mom".into(),
                tz: None,
            }),
        };
        let mut buf = Vec::new();
        write_frame(&mut buf, &msg).unwrap();
        assert_eq!(&buf[..4], &(buf.len() as u32 - 4).to_be_bytes());
        let mut cursor = io::Cursor::new(buf);
        assert_eq!(read_frame(&mut cursor).unwrap(), Some(msg));
        assert_eq!(read_frame(&mut cursor).unwrap(), None);
    }

    #[test]
    fn encoding_is_readable() {
        let msg = Message {
            id: 1,
            body: Body::Response(Response::Error(IpcError::VersionMismatch {
                host: 2,
                client: 1,
            })),
        };
        assert_eq!(
            msg.to_string(),
            r#"{"id":1,"kind":"response","body":{"result":"error","data":{"code":"version_mismatch","host":2,"client":1}}}"#
        );
        // Every response shape encodes, including sequences and scalars.
        for r in [
            Response::Tasks(vec![]),
            Response::Count { total: 3 },
            Response::Statuses(vec!["open".into()]),
            Response::Filters(vec![FilterView {
                id: Uuid::nil(),
                name: "Soon".into(),
                definition: FilterDefinition {
                    due_to_day: Some(7),
                    ..Default::default()
                },
            }]),
            Response::Lists(vec![]),
            Response::OpLog(vec![]),
            Response::Row(serde_json::Value::Null),
            Response::Done { changed: true },
            Response::Error(IpcError::Locked),
        ] {
            let text = serde_json::to_string(&r).unwrap();
            assert_eq!(
                serde_json::from_str::<Response>(&text).unwrap(),
                r,
                "{text}"
            );
        }
        let req = Request::Debug(DebugRequest::OpLog { limit: 5 });
        let text = serde_json::to_string(&req).unwrap();
        assert_eq!(
            serde_json::from_str::<Request>(&text).unwrap(),
            req,
            "{text}"
        );
        assert_eq!(
            IpcError::VersionMismatch { host: 2, client: 1 }.to_string(),
            "restart Liste to finish updating (host protocol 2, client protocol 1)"
        );
    }

    #[test]
    fn oversized_frames_are_refused() {
        let mut cursor = io::Cursor::new((MAX_FRAME as u32 + 1).to_be_bytes().to_vec());
        assert!(read_frame(&mut cursor).is_err());
    }
}
