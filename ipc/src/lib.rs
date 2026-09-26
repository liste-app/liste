//! Local IPC between the host process and its thin clients (Section 3).
//!
//! Message types are defined here and shared by the core's `host` module,
//! the `liste` CLI, and the MCP server. The transport is a Unix domain socket
//! on macOS and Linux and a named pipe on Windows. Messages are framed and
//! versioned. This is a private protocol, not a public API; the operating
//! system's file permissions are the authentication boundary.

/// Protocol version exchanged on connect. A mismatch between client and host
/// (typically an update that has not restarted the host yet) must produce a
/// clear "restart Liste to finish updating" message.
pub const PROTOCOL_VERSION: u32 = 1;

/// How long a client waits for the host's socket to appear after triggering
/// a background launch, before failing with "Liste did not start".
pub const READINESS_TIMEOUT_MS: u64 = 3_000;
