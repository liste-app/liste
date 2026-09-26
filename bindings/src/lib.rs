//! Foreign bindings for the Liste core (Section 14).
//!
//! Swift and Kotlin bindings are generated with Mozilla UniFFI from this
//! crate, C# with uniffi-bindgen-cs, and the web build exposes a
//! `wasm-bindgen` wrapper. Bindings are generated at build time from the
//! current core; nothing generated is committed or published.
//!
//! The surface is deliberately small and mirrors what the CLI and the MCP
//! server can do, because it is the same path: [`ListeHost`] wraps the
//! core's host and every call is served by the host's request handler,
//! exactly as an IPC request would be. The app therefore cannot do
//! anything a CLI client cannot, and there is no second write path.

uniffi::setup_scaffolding!();

#[cfg(all(target_family = "wasm", target_os = "unknown"))]
pub mod wasm;

#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
mod native {
    use std::sync::{Arc, Mutex};

    use liste_core::host::{Host, HostConfig, HostError};
    use liste_core::parse::Locale;
    use liste_ipc::protocol::{DebugRequest, TagView, TaskQuery};
    use liste_ipc::protocol::{IpcError, Request, Response};
    use liste_ipc::{Endpoint, ListView, PreviewView, SpanView, TaskView};

    /// Errors surfaced to the app.
    #[derive(Debug, thiserror::Error, uniffi::Error)]
    #[uniffi(flat_error)]
    pub enum ListeError {
        #[error("the store at {0} is held by another Liste process")]
        StoreHeld(String),
        #[error("locked: keys have not been unlocked on this device")]
        Locked,
        #[error("not found: {0}")]
        NotFound(String),
        #[error("invalid: {0}")]
        Invalid(String),
        #[error("{0}")]
        Internal(String),
    }

    impl From<IpcError> for ListeError {
        fn from(e: IpcError) -> Self {
            match e {
                IpcError::Locked => ListeError::Locked,
                IpcError::NotFound { id } => ListeError::NotFound(id.to_string()),
                IpcError::Invalid { message } => ListeError::Invalid(message),
                other => ListeError::Internal(other.to_string()),
            }
        }
    }

    impl From<HostError> for ListeError {
        fn from(e: HostError) -> Self {
            match e {
                HostError::StoreHeld(p) => ListeError::StoreHeld(p.display().to_string()),
                other => ListeError::Internal(other.to_string()),
            }
        }
    }

    #[derive(Debug, Clone, uniffi::Record)]
    pub struct TaskItem {
        pub id: String,
        pub title: String,
        pub notes: String,
        pub list_id: Option<String>,
        pub list_title: Option<String>,
        pub due_at: Option<i64>,
        /// The due date rendered in the host's zone, for display.
        pub due: Option<String>,
        pub due_all_day: bool,
        pub reminder_at: Option<i64>,
        pub priority: String,
        pub status: String,
        pub completed_at: Option<i64>,
        pub parent_id: Option<String>,
        /// Indentation under the rows above it in the same listing.
        pub depth: u32,
        pub tags: Vec<String>,
        pub recurrence: Option<String>,
        pub created_at: i64,
        pub modified_at: i64,
    }

    impl From<TaskView> for TaskItem {
        fn from(t: TaskView) -> Self {
            TaskItem {
                id: t.id.to_string(),
                title: t.title,
                notes: t.notes,
                list_id: t.list.as_ref().map(|l| l.id.to_string()),
                list_title: t.list.map(|l| l.title),
                due_at: t.due_at,
                due: t.due,
                due_all_day: t.due_all_day,
                reminder_at: t.reminder_at,
                priority: t.priority,
                status: t.status,
                completed_at: t.completed_at,
                parent_id: t.parent_id.map(|p| p.to_string()),
                depth: t.depth,
                tags: t.tags,
                recurrence: t.recurrence,
                created_at: t.created_at,
                modified_at: t.modified_at,
            }
        }
    }

    #[derive(Debug, Clone, uniffi::Record)]
    pub struct TaskList {
        pub id: String,
        pub title: String,
    }

    impl From<ListView> for TaskList {
        fn from(l: ListView) -> Self {
            TaskList {
                id: l.id.to_string(),
                title: l.title,
            }
        }
    }

    #[derive(Debug, Clone, uniffi::Record)]
    pub struct Tag {
        pub id: String,
        pub name: String,
    }

    impl From<TagView> for Tag {
        fn from(t: TagView) -> Self {
            Tag {
                id: t.id.to_string(),
                name: t.name,
            }
        }
    }

    /// A filter over tasks; absent fields do not filter. `order` is
    /// `manual`, `due`, or `completed`. `due_from_day` and `due_to_day`
    /// are whole days from today in the host's zone (today is
    /// `due_to_day: 1`, overdue `due_to_day: 0`), so the app never
    /// computes a day boundary. `offset` and `limit` pick a window.
    #[derive(Debug, Clone, Default, uniffi::Record)]
    pub struct Query {
        #[uniffi(default = None)]
        pub list_id: Option<String>,
        #[uniffi(default = false)]
        pub inbox: bool,
        #[uniffi(default = None)]
        pub tag_id: Option<String>,
        #[uniffi(default = None)]
        pub parent_id: Option<String>,
        #[uniffi(default = None)]
        pub priority: Option<String>,
        #[uniffi(default = None)]
        pub status: Option<String>,
        #[uniffi(default = None)]
        pub due_from: Option<i64>,
        #[uniffi(default = None)]
        pub due_to: Option<i64>,
        #[uniffi(default = None)]
        pub due_from_day: Option<i64>,
        #[uniffi(default = None)]
        pub due_to_day: Option<i64>,
        #[uniffi(default = false)]
        pub has_reminder: bool,
        #[uniffi(default = false)]
        pub include_completed: bool,
        #[uniffi(default = false)]
        pub completed_only: bool,
        #[uniffi(default = None)]
        pub order: Option<String>,
        #[uniffi(default = 0)]
        pub offset: u32,
        #[uniffi(default = 0)]
        pub limit: u32,
    }

    impl Query {
        fn into_ipc(self) -> Result<TaskQuery, ListeError> {
            Ok(TaskQuery {
                list_id: self.list_id.as_deref().map(uuid).transpose()?,
                inbox: self.inbox,
                tag_id: self.tag_id.as_deref().map(uuid).transpose()?,
                parent_id: self.parent_id.as_deref().map(uuid).transpose()?,
                priority: self.priority,
                status: self.status,
                due_from: self.due_from,
                due_to: self.due_to,
                due_from_day: self.due_from_day,
                due_to_day: self.due_to_day,
                has_reminder: self.has_reminder,
                include_completed: self.include_completed,
                completed_only: self.completed_only,
                order: self.order,
                offset: self.offset as usize,
                limit: self.limit as usize,
            })
        }
    }

    /// A byte range of the captured text that was interpreted. Offsets are
    /// UTF-8 byte offsets into the text the caller passed.
    #[derive(Debug, Clone, uniffi::Record)]
    pub struct Span {
        pub start: u64,
        pub end: u64,
        /// `date`, `time`, `list`, `tag`, `priority`, or `recurrence`.
        pub kind: String,
    }

    impl From<SpanView> for Span {
        fn from(s: SpanView) -> Self {
            Span {
                start: s.start as u64,
                end: s.end as u64,
                kind: s.kind,
            }
        }
    }

    #[derive(Debug, Clone, uniffi::Record)]
    pub struct Captured {
        pub task: TaskItem,
        pub spans: Vec<Span>,
    }

    /// What a line would become, for live highlighting while typing.
    /// Named to stay clear of SwiftUI's `Preview`; `TaskItem` and `TaskList`
    /// likewise avoid Swift's `Task` and SwiftUI's `List`.
    #[derive(Debug, Clone, uniffi::Record)]
    pub struct CapturePreview {
        pub title: String,
        pub due: Option<String>,
        pub due_all_day: bool,
        pub list: Option<String>,
        pub tags: Vec<String>,
        pub priority: String,
        pub recurrence: Option<String>,
        pub spans: Vec<Span>,
    }

    impl From<PreviewView> for CapturePreview {
        fn from(p: PreviewView) -> Self {
            CapturePreview {
                title: p.title,
                due: p.due,
                due_all_day: p.due_all_day,
                list: p.list,
                tags: p.tags,
                priority: p.priority,
                recurrence: p.recurrence,
                spans: p.spans.into_iter().map(Into::into).collect(),
            }
        }
    }

    /// Fields to change on a task; absent fields are untouched. `due` is
    /// natural-language text and an empty string clears it; `list` is a
    /// name and an empty string moves the task to the inbox.
    #[derive(Debug, Clone, Default, uniffi::Record)]
    pub struct TaskPatch {
        #[uniffi(default = None)]
        pub title: Option<String>,
        #[uniffi(default = None)]
        pub notes: Option<String>,
        #[uniffi(default = None)]
        pub due: Option<String>,
        #[uniffi(default = None)]
        pub priority: Option<String>,
        #[uniffi(default = None)]
        pub list: Option<String>,
        #[uniffi(default = [])]
        pub add_tags: Vec<String>,
        #[uniffi(default = [])]
        pub remove_tags: Vec<String>,
        /// A status name for kanban columns.
        #[uniffi(default = None)]
        pub status: Option<String>,
        /// A parent task id; an empty string makes the task top-level.
        #[uniffi(default = None)]
        pub parent: Option<String>,
        /// A reminder as natural-language time; an empty string clears it.
        #[uniffi(default = None)]
        pub reminder: Option<String>,
    }

    #[derive(Debug, Clone, uniffi::Record)]
    pub struct HostStatus {
        pub locked: bool,
        pub device_id: String,
        pub space_id: String,
        pub data_dir: String,
        pub socket_path: String,
        pub pending_ops: u64,
    }

    /// Implemented by the app; called after every change to the store from
    /// any client, on the thread that made the change.
    #[uniffi::export(with_foreign)]
    pub trait ChangeListener: Send + Sync {
        fn on_change(&self);
    }

    /// The in-process host: the app's one handle on the core.
    #[derive(uniffi::Object)]
    pub struct ListeHost {
        host: Mutex<Option<Host>>,
    }

    fn uuid(id: &str) -> Result<uuid::Uuid, ListeError> {
        id.trim()
            .parse()
            .map_err(|_| ListeError::Invalid(format!("{id:?} is not a task id")))
    }

    fn tasks(r: Response) -> Result<Vec<TaskItem>, ListeError> {
        match r {
            Response::Tasks(t) => Ok(t.into_iter().map(Into::into).collect()),
            Response::Error(e) => Err(e.into()),
            other => Err(ListeError::Internal(format!("unexpected reply {other:?}"))),
        }
    }

    fn task(r: Response) -> Result<TaskItem, ListeError> {
        match r {
            Response::Task(t) => Ok(t.into()),
            Response::Error(e) => Err(e.into()),
            other => Err(ListeError::Internal(format!("unexpected reply {other:?}"))),
        }
    }

    impl ListeHost {
        fn with<T>(&self, f: impl FnOnce(&Host) -> Result<T, ListeError>) -> Result<T, ListeError> {
            let guard = self.host.lock().unwrap_or_else(|e| e.into_inner());
            match guard.as_ref() {
                Some(h) => f(h),
                None => Err(ListeError::Internal("the host has been shut down".into())),
            }
        }

        fn handle(&self, request: Request) -> Result<Response, ListeError> {
            self.with(|h| match h.handle(request) {
                Response::Error(e) => Err(e.into()),
                other => Ok(other),
            })
        }
    }

    #[uniffi::export]
    impl ListeHost {
        /// Take the store lock, open the store, and start serving IPC.
        /// `data_dir` and `socket_path` default to the platform's paths
        /// (Section 3), or to `LISTE_DATA_DIR` and `LISTE_SOCKET` if set.
        #[uniffi::constructor]
        pub fn start(
            data_dir: Option<String>,
            socket_path: Option<String>,
        ) -> Result<Arc<Self>, ListeError> {
            let data_dir = match data_dir {
                Some(d) => std::path::PathBuf::from(d),
                None => liste_ipc::paths::default_data_dir()
                    .ok_or_else(|| ListeError::Internal("no data directory".into()))?,
            };
            let endpoint = match socket_path {
                Some(p) => Endpoint::from_path(std::path::Path::new(&p)),
                None => Endpoint::default_for_host()
                    .ok_or_else(|| ListeError::Internal("no socket path".into()))?,
            };
            let mut config = HostConfig::new(data_dir, endpoint);
            config.locale = Locale::US;
            let host = Host::start(config)?;
            Ok(Arc::new(ListeHost {
                host: Mutex::new(Some(host)),
            }))
        }

        pub fn status(&self) -> Result<HostStatus, ListeError> {
            self.with(|h| match h.handle(Request::Status) {
                Response::Status(s) => Ok(HostStatus {
                    locked: s.locked,
                    device_id: s.device_id.to_string(),
                    space_id: s.space_id.to_string(),
                    data_dir: s.data_dir,
                    socket_path: h.endpoint().to_string(),
                    pending_ops: s.pending_ops,
                }),
                Response::Error(e) => Err(e.into()),
                other => Err(ListeError::Internal(format!("unexpected reply {other:?}"))),
            })
        }

        pub fn is_locked(&self) -> bool {
            self.with(|h| Ok(h.is_locked())).unwrap_or(true)
        }

        pub fn lock(&self) {
            let _ = self.with(|h| {
                h.lock();
                Ok(())
            });
        }

        pub fn unlock(&self) {
            let _ = self.with(|h| {
                h.unlock();
                Ok(())
            });
        }

        pub fn set_change_listener(&self, listener: Option<Arc<dyn ChangeListener>>) {
            let _ = self.with(|h| {
                h.set_change_listener(listener.map(|l| {
                    let l: liste_core::store::ChangeListener = Arc::new(move || l.on_change());
                    l
                }));
                Ok(())
            });
        }

        pub fn capture(
            &self,
            text: String,
            time_zone: Option<String>,
        ) -> Result<Captured, ListeError> {
            match self.handle(Request::Capture {
                text,
                tz: time_zone,
            })? {
                Response::Captured { task, spans } => Ok(Captured {
                    task: task.into(),
                    spans: spans.into_iter().map(Into::into).collect(),
                }),
                other => Err(ListeError::Internal(format!("unexpected reply {other:?}"))),
            }
        }

        pub fn preview(
            &self,
            text: String,
            time_zone: Option<String>,
        ) -> Result<CapturePreview, ListeError> {
            match self.handle(Request::Preview {
                text,
                tz: time_zone,
            })? {
                Response::Preview(p) => Ok(p.into()),
                other => Err(ListeError::Internal(format!("unexpected reply {other:?}"))),
            }
        }

        pub fn today(&self) -> Result<Vec<TaskItem>, ListeError> {
            tasks(self.handle(Request::Today)?)
        }

        pub fn upcoming(&self, days: u32) -> Result<Vec<TaskItem>, ListeError> {
            tasks(self.handle(Request::Upcoming { days })?)
        }

        /// Open tasks in a list, or in the inbox when `list_id` is `None`.
        pub fn tasks_in_list(&self, list_id: Option<String>) -> Result<Vec<TaskItem>, ListeError> {
            let list_id = list_id.as_deref().map(uuid).transpose()?;
            tasks(self.handle(Request::ListTasks { list_id })?)
        }

        pub fn search(&self, query: String, limit: u32) -> Result<Vec<TaskItem>, ListeError> {
            tasks(self.handle(Request::Search {
                query,
                limit: limit as usize,
            })?)
        }

        pub fn lists(&self) -> Result<Vec<TaskList>, ListeError> {
            match self.handle(Request::Lists)? {
                Response::Lists(l) => Ok(l.into_iter().map(Into::into).collect()),
                other => Err(ListeError::Internal(format!("unexpected reply {other:?}"))),
            }
        }

        pub fn task(&self, id: String) -> Result<TaskItem, ListeError> {
            task(self.handle(Request::GetTask { id: uuid(&id)? })?)
        }

        pub fn update(&self, id: String, patch: TaskPatch) -> Result<TaskItem, ListeError> {
            task(self.handle(Request::UpdateTask {
                id: uuid(&id)?,
                patch: liste_ipc::TaskPatch {
                    title: patch.title,
                    notes: patch.notes,
                    due: patch.due,
                    priority: patch.priority,
                    list: patch.list,
                    add_tags: patch.add_tags,
                    remove_tags: patch.remove_tags,
                    status: patch.status,
                    parent: patch.parent,
                    reminder: patch.reminder,
                },
            })?)
        }

        /// A window of the tasks matching the query, in its order.
        pub fn query(&self, query: Query) -> Result<Vec<TaskItem>, ListeError> {
            tasks(self.handle(Request::Query(query.into_ipc()?))?)
        }

        /// How many tasks match the query, ignoring its window.
        pub fn count(&self, query: Query) -> Result<u32, ListeError> {
            match self.handle(Request::Count(query.into_ipc()?))? {
                Response::Count { total } => Ok(total.min(u32::MAX as usize) as u32),
                other => Err(ListeError::Internal(format!("unexpected reply {other:?}"))),
            }
        }

        /// Every status name in use, `open` first.
        pub fn statuses(&self) -> Result<Vec<String>, ListeError> {
            match self.handle(Request::Statuses)? {
                Response::Statuses(s) => Ok(s),
                other => Err(ListeError::Internal(format!("unexpected reply {other:?}"))),
            }
        }

        /// Move a task in manual order to sit after `after` and before
        /// `before`; either may be absent for an end of the list.
        pub fn reorder(
            &self,
            id: String,
            after: Option<String>,
            before: Option<String>,
        ) -> Result<TaskItem, ListeError> {
            task(self.handle(Request::Reorder {
                id: uuid(&id)?,
                after: after.as_deref().map(uuid).transpose()?,
                before: before.as_deref().map(uuid).transpose()?,
            })?)
        }

        /// Tombstone a task. Undo restores it.
        pub fn delete(&self, id: String) -> Result<(), ListeError> {
            self.handle(Request::Delete { id: uuid(&id)? })?;
            Ok(())
        }

        pub fn tags(&self) -> Result<Vec<Tag>, ListeError> {
            match self.handle(Request::Tags)? {
                Response::Tags(t) => Ok(t.into_iter().map(Into::into).collect()),
                other => Err(ListeError::Internal(format!("unexpected reply {other:?}"))),
            }
        }

        pub fn create_list(&self, title: String) -> Result<TaskList, ListeError> {
            match self.handle(Request::CreateList { title })? {
                Response::List(l) => Ok(l.into()),
                other => Err(ListeError::Internal(format!("unexpected reply {other:?}"))),
            }
        }

        /// Rename a list and/or move it between two neighbours.
        pub fn update_list(
            &self,
            id: String,
            title: Option<String>,
            after: Option<String>,
            before: Option<String>,
        ) -> Result<TaskList, ListeError> {
            match self.handle(Request::UpdateList {
                id: uuid(&id)?,
                title,
                after: after.as_deref().map(uuid).transpose()?,
                before: before.as_deref().map(uuid).transpose()?,
            })? {
                Response::List(l) => Ok(l.into()),
                other => Err(ListeError::Internal(format!("unexpected reply {other:?}"))),
            }
        }

        /// Delete a list; its tasks move to the inbox.
        pub fn delete_list(&self, id: String) -> Result<(), ListeError> {
            self.handle(Request::DeleteList { id: uuid(&id)? })?;
            Ok(())
        }

        /// Fill the store with the fixture, for benches and the acceptance
        /// driver. Returns the number of tasks created.
        pub fn populate_fixture(&self, tasks: u32, seed: u64) -> Result<u32, ListeError> {
            match self.handle(Request::Debug(DebugRequest::Fixture {
                tasks: tasks as usize,
                seed,
            }))? {
                Response::Fixture { tasks, .. } => Ok(tasks as u32),
                other => Err(ListeError::Internal(format!("unexpected reply {other:?}"))),
            }
        }

        pub fn complete(&self, id: String) -> Result<TaskItem, ListeError> {
            task(self.handle(Request::Complete { id: uuid(&id)? })?)
        }

        pub fn uncomplete(&self, id: String) -> Result<TaskItem, ListeError> {
            task(self.handle(Request::Uncomplete { id: uuid(&id)? })?)
        }

        pub fn undo(&self) -> Result<bool, ListeError> {
            match self.handle(Request::Undo)? {
                Response::Done { changed } => Ok(changed),
                other => Err(ListeError::Internal(format!("unexpected reply {other:?}"))),
            }
        }

        pub fn redo(&self) -> Result<bool, ListeError> {
            match self.handle(Request::Redo)? {
                Response::Done { changed } => Ok(changed),
                other => Err(ListeError::Internal(format!("unexpected reply {other:?}"))),
            }
        }

        /// Stop serving, release the store lock, remove the socket. The
        /// object is unusable afterwards.
        pub fn shutdown(&self) {
            let mut guard = self.host.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(h) = guard.take() {
                h.shutdown();
            }
        }
    }

    /// The core's version, for the About box.
    #[uniffi::export]
    pub fn core_version() -> String {
        liste_core::host::HOST_VERSION.to_owned()
    }
}

#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
pub use native::*;
