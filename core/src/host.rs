//! The desktop host (Section 3).
//!
//! Exactly one process per device owns the local store. This module is that
//! process's core: it opens SQLite, holds keys, runs sync, and serves the
//! local IPC endpoint that the CLI and the MCP server talk to. The socket is
//! created only after the store has been opened (locked or not).
