//! The client: connect, handshake, typed calls, and auto-start.

use std::io;
use std::time::{Duration, Instant};

use uuid::Uuid;

use crate::paths::Endpoint;
use crate::protocol::{
    Body, DebugRequest, FilterDefinition, FilterView, IpcError, ListView, Message,
    PROTOCOL_VERSION, PreviewView, Request, Response, SpanView, StatusView, TagView, TaskPatch,
    TaskQuery, TaskView,
};
use crate::transport::{self, Stream};

/// How long a client waits for the socket after launching a host
/// (Section 3, readiness handshake).
pub const READINESS_TIMEOUT: Duration = Duration::from_secs(3);

/// Why a call failed.
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("Liste is not running")]
    NotRunning,
    #[error("Liste did not start")]
    DidNotStart,
    #[error("{0}")]
    VersionMismatch(IpcError),
    #[error("{0}")]
    Locked(IpcError),
    #[error("{0}")]
    Remote(IpcError),
    #[error("protocol error: {0}")]
    Protocol(String),
    #[error(transparent)]
    Io(io::Error),
}

impl From<IpcError> for ClientError {
    fn from(e: IpcError) -> Self {
        match e {
            IpcError::Locked => ClientError::Locked(e),
            IpcError::VersionMismatch { .. } => ClientError::VersionMismatch(e),
            other => ClientError::Remote(other),
        }
    }
}

/// A connection to the host, after a successful handshake.
pub struct Client {
    stream: Stream,
    next_id: u64,
    host_version: String,
}

impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Client(host {})", self.host_version)
    }
}

impl Client {
    /// Connect to one endpoint and perform the handshake.
    pub fn connect(endpoint: &Endpoint) -> Result<Client, ClientError> {
        Client::connect_as(endpoint, "liste-ipc")
    }

    /// Connect, identifying the client program in the handshake.
    pub fn connect_as(endpoint: &Endpoint, client_name: &str) -> Result<Client, ClientError> {
        // Whatever the reason nothing answers at the endpoint (no socket
        // file, a corpse socket, a missing directory), the host is not
        // running as far as the client is concerned.
        let stream = transport::connect(endpoint).map_err(|_| ClientError::NotRunning)?;
        let mut client = Client {
            stream,
            next_id: 1,
            host_version: String::new(),
        };
        match client.call(Request::Hello {
            protocol_version: PROTOCOL_VERSION,
            client: client_name.to_owned(),
        })? {
            Response::Hello { host_version, .. } => {
                client.host_version = host_version;
                Ok(client)
            }
            other => Err(ClientError::Protocol(format!(
                "unexpected handshake reply {other:?}"
            ))),
        }
    }

    /// Try the known endpoints in order.
    pub fn connect_known(client_name: &str) -> Result<Client, ClientError> {
        let mut last = ClientError::NotRunning;
        for endpoint in Endpoint::known() {
            match Client::connect_as(&endpoint, client_name) {
                Ok(c) => return Ok(c),
                Err(ClientError::NotRunning) => {}
                Err(e) => last = e,
            }
        }
        if matches!(last, ClientError::NotRunning) {
            Err(ClientError::NotRunning)
        } else {
            Err(last)
        }
    }

    /// Connect, or launch a host with `launch` and poll for the socket for
    /// up to [`READINESS_TIMEOUT`].
    pub fn connect_or_start(
        client_name: &str,
        launch: &dyn Fn() -> io::Result<()>,
    ) -> Result<Client, ClientError> {
        match Client::connect_known(client_name) {
            Err(ClientError::NotRunning) => {}
            other => return other,
        }
        launch().map_err(|_| ClientError::DidNotStart)?;
        let deadline = Instant::now() + READINESS_TIMEOUT;
        loop {
            match Client::connect_known(client_name) {
                Err(ClientError::NotRunning) => {}
                other => return other,
            }
            if Instant::now() >= deadline {
                return Err(ClientError::DidNotStart);
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// The host's reported version.
    pub fn host_version(&self) -> &str {
        &self.host_version
    }

    /// Send a request and wait for its response.
    pub fn call(&mut self, request: Request) -> Result<Response, ClientError> {
        let id = self.next_id;
        self.next_id += 1;
        transport::send(
            &mut self.stream,
            &Message {
                id,
                body: Body::Request(request),
            },
        )
        .map_err(ClientError::Io)?;
        loop {
            let message = transport::receive(&mut self.stream)
                .map_err(ClientError::Io)?
                .ok_or(ClientError::NotRunning)?;
            if message.id != id {
                continue;
            }
            return match message.body {
                Body::Response(Response::Error(e)) => Err(e.into()),
                Body::Response(r) => Ok(r),
                Body::Request(_) => Err(ClientError::Protocol("host sent a request".into())),
            };
        }
    }

    fn expect_task(r: Response) -> Result<TaskView, ClientError> {
        match r {
            Response::Task(t) => Ok(t),
            other => Err(ClientError::Protocol(format!(
                "expected task, got {other:?}"
            ))),
        }
    }

    fn expect_tasks(r: Response) -> Result<Vec<TaskView>, ClientError> {
        match r {
            Response::Tasks(t) => Ok(t),
            other => Err(ClientError::Protocol(format!(
                "expected tasks, got {other:?}"
            ))),
        }
    }

    fn expect_done(r: Response) -> Result<bool, ClientError> {
        match r {
            Response::Done { changed } => Ok(changed),
            other => Err(ClientError::Protocol(format!(
                "expected done, got {other:?}"
            ))),
        }
    }

    pub fn status(&mut self) -> Result<StatusView, ClientError> {
        match self.call(Request::Status)? {
            Response::Status(s) => Ok(s),
            other => Err(ClientError::Protocol(format!(
                "expected status, got {other:?}"
            ))),
        }
    }

    pub fn capture(
        &mut self,
        text: &str,
        tz: Option<&str>,
    ) -> Result<(TaskView, Vec<SpanView>), ClientError> {
        match self.call(Request::Capture {
            text: text.to_owned(),
            tz: tz.map(str::to_owned),
        })? {
            Response::Captured { task, spans } => Ok((task, spans)),
            other => Err(ClientError::Protocol(format!(
                "expected capture, got {other:?}"
            ))),
        }
    }

    pub fn preview(&mut self, text: &str, tz: Option<&str>) -> Result<PreviewView, ClientError> {
        match self.call(Request::Preview {
            text: text.to_owned(),
            tz: tz.map(str::to_owned),
        })? {
            Response::Preview(p) => Ok(p),
            other => Err(ClientError::Protocol(format!(
                "expected preview, got {other:?}"
            ))),
        }
    }

    pub fn search(&mut self, query: &str, limit: usize) -> Result<Vec<TaskView>, ClientError> {
        Client::expect_tasks(self.call(Request::Search {
            query: query.to_owned(),
            limit,
        })?)
    }

    pub fn today(&mut self) -> Result<Vec<TaskView>, ClientError> {
        Client::expect_tasks(self.call(Request::Today)?)
    }

    pub fn upcoming(&mut self, days: u32) -> Result<Vec<TaskView>, ClientError> {
        Client::expect_tasks(self.call(Request::Upcoming { days })?)
    }

    pub fn list_tasks(&mut self, list_id: Option<Uuid>) -> Result<Vec<TaskView>, ClientError> {
        Client::expect_tasks(self.call(Request::ListTasks { list_id })?)
    }

    pub fn query(&mut self, query: TaskQuery) -> Result<Vec<TaskView>, ClientError> {
        Client::expect_tasks(self.call(Request::Query(query))?)
    }

    /// How many tasks `query` matches, ignoring its window.
    pub fn count(&mut self, query: TaskQuery) -> Result<usize, ClientError> {
        match self.call(Request::Count(query))? {
            Response::Count { total } => Ok(total),
            other => Err(ClientError::Protocol(format!(
                "expected count, got {other:?}"
            ))),
        }
    }

    /// Every status name in use.
    pub fn statuses(&mut self) -> Result<Vec<String>, ClientError> {
        match self.call(Request::Statuses)? {
            Response::Statuses(s) => Ok(s),
            other => Err(ClientError::Protocol(format!(
                "expected statuses, got {other:?}"
            ))),
        }
    }

    pub fn reorder(
        &mut self,
        id: Uuid,
        after: Option<Uuid>,
        before: Option<Uuid>,
    ) -> Result<TaskView, ClientError> {
        Client::expect_task(self.call(Request::Reorder { id, after, before })?)
    }

    pub fn delete(&mut self, id: Uuid) -> Result<bool, ClientError> {
        Client::expect_done(self.call(Request::Delete { id })?)
    }

    pub fn tags(&mut self) -> Result<Vec<TagView>, ClientError> {
        match self.call(Request::Tags)? {
            Response::Tags(t) => Ok(t),
            other => Err(ClientError::Protocol(format!(
                "expected tags, got {other:?}"
            ))),
        }
    }

    pub fn create_list(&mut self, title: &str) -> Result<ListView, ClientError> {
        match self.call(Request::CreateList {
            title: title.to_owned(),
        })? {
            Response::List(l) => Ok(l),
            other => Err(ClientError::Protocol(format!(
                "expected list, got {other:?}"
            ))),
        }
    }

    pub fn filters(&mut self) -> Result<Vec<FilterView>, ClientError> {
        match self.call(Request::Filters)? {
            Response::Filters(f) => Ok(f),
            other => Err(ClientError::Protocol(format!(
                "expected filters, got {other:?}"
            ))),
        }
    }

    pub fn create_filter(
        &mut self,
        name: &str,
        definition: FilterDefinition,
    ) -> Result<FilterView, ClientError> {
        Client::expect_filter(self.call(Request::CreateFilter {
            name: name.to_owned(),
            definition,
        })?)
    }

    pub fn update_filter(
        &mut self,
        id: Uuid,
        name: Option<String>,
        definition: Option<FilterDefinition>,
        after: Option<Uuid>,
        before: Option<Uuid>,
    ) -> Result<FilterView, ClientError> {
        Client::expect_filter(self.call(Request::UpdateFilter {
            id,
            name,
            definition,
            after,
            before,
        })?)
    }

    pub fn delete_filter(&mut self, id: Uuid) -> Result<bool, ClientError> {
        Client::expect_done(self.call(Request::DeleteFilter { id })?)
    }

    fn expect_filter(r: Response) -> Result<FilterView, ClientError> {
        match r {
            Response::Filter(f) => Ok(f),
            other => Err(ClientError::Protocol(format!(
                "expected filter, got {other:?}"
            ))),
        }
    }

    pub fn task(&mut self, id: Uuid) -> Result<TaskView, ClientError> {
        Client::expect_task(self.call(Request::GetTask { id })?)
    }

    pub fn update(&mut self, id: Uuid, patch: TaskPatch) -> Result<TaskView, ClientError> {
        Client::expect_task(self.call(Request::UpdateTask { id, patch })?)
    }

    pub fn complete(&mut self, id: Uuid) -> Result<TaskView, ClientError> {
        Client::expect_task(self.call(Request::Complete { id })?)
    }

    pub fn uncomplete(&mut self, id: Uuid) -> Result<TaskView, ClientError> {
        Client::expect_task(self.call(Request::Uncomplete { id })?)
    }

    pub fn lists(&mut self) -> Result<Vec<ListView>, ClientError> {
        match self.call(Request::Lists)? {
            Response::Lists(l) => Ok(l),
            other => Err(ClientError::Protocol(format!(
                "expected lists, got {other:?}"
            ))),
        }
    }

    pub fn undo(&mut self) -> Result<bool, ClientError> {
        Client::expect_done(self.call(Request::Undo)?)
    }

    pub fn redo(&mut self) -> Result<bool, ClientError> {
        Client::expect_done(self.call(Request::Redo)?)
    }

    pub fn debug(&mut self, request: DebugRequest) -> Result<Response, ClientError> {
        self.call(Request::Debug(request))
    }
}
