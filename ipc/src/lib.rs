//! Local IPC between the host process and its thin clients (Section 3).
//!
//! This crate holds the message types, the framing, the transport glue,
//! and the client. It holds no core logic: a client sends what the person
//! typed and receives what the host computed. The transport is a Unix
//! domain socket on macOS and Linux (mode 0600, in the app's data directory
//! or `$XDG_RUNTIME_DIR`) and a named pipe on Windows. It is a private
//! protocol, not a public API; the operating system's file permissions are
//! the authentication boundary.

pub mod client;
pub mod launch;
pub mod paths;
pub mod protocol;
pub mod transport;

pub use client::{Client, ClientError, READINESS_TIMEOUT};
pub use paths::Endpoint;
pub use protocol::{
    DebugRequest, IpcError, ListView, Message, OpView, PROTOCOL_VERSION, PreviewView, Request,
    Response, SpanView, StatusView, TagView, TaskPatch, TaskQuery, TaskView,
};
