//! The local MCP server (Section 13).
//!
//! Transport is stdio only: an agent host starts this process and talks to
//! it over stdin and stdout; there is no network listener. Every tool is a
//! thin call over the local IPC client to the host process, with the same
//! auto-start the CLI uses. This binary holds no core logic, opens no
//! database, and holds no keys. If the store is locked, tools return a
//! locked error; they never prompt for credentials and never call any
//! server. The only bytes that leave the machine are the host's own
//! encrypted sync traffic.
//!
//! On the first connection from an agent client this install has not seen,
//! the initialize response's instructions carry the one-time plaintext
//! notice from Section 7. Seen client names are recorded in the host's
//! data directory, not in the store.

mod server;

use rmcp::{ServiceExt, transport::stdio};

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let service = match server::Liste::new().serve(stdio()).await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("liste-mcp: {e}");
            std::process::exit(1);
        }
    };
    if let Err(e) = service.waiting().await {
        eprintln!("liste-mcp: {e}");
        std::process::exit(1);
    }
}
