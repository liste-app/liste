//! The local MCP server (Section 13).
//!
//! Transport is stdio in v1: a local agent starts this process and talks to
//! it over stdin and stdout. Every tool handler is a thin wrapper over a
//! host call made over IPC. There is no second task model, no second date
//! parser, no direct SQL, and no network listener. If the store is locked,
//! tools return a `locked` error. On first connection from a new agent
//! client the server returns a one-time notice that the agent sees
//! plaintext (Section 7).

fn main() {
    eprintln!("liste-mcp: not implemented yet");
    std::process::exit(1);
}
