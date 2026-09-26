//! The `liste` command-line tool (Section 13).
//!
//! Every user command is a thin IPC client of the host process: it contains
//! no core logic, opens no database, and holds no keys. If no host is
//! running it starts the desktop app in the background and connects once
//! the socket is ready. `liste daemon` is the one exception: it runs the
//! core's `host` module headless for machines without a GUI.

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "liste", version, about = "Liste command-line tool")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Capture a task from a natural-language string.
    Capture { text: Vec<String> },
    /// Search tasks (local full-text search).
    Search { query: Vec<String> },
    /// List today's tasks.
    Today,
    /// List upcoming tasks.
    Upcoming,
    /// Mark a task complete.
    Complete { id: String },
    /// Mark a task not complete.
    Uncomplete { id: String },
    /// Show a task.
    Show { id: String },
    /// Run the headless host for machines without the desktop app.
    Daemon,
    /// Developer harness commands served by the host.
    Debug {
        #[command(subcommand)]
        command: DebugCommand,
    },
}

#[derive(Subcommand)]
enum DebugCommand {
    /// Inspect the local op log.
    Oplog,
    /// Inspect a materialized row.
    Row { entity: String, id: String },
    /// Show the sync cursor.
    Cursor,
    /// Apply a named fixture.
    Fixture { name: String },
    /// Run a named two-device scenario.
    Scenario { name: String },
}

fn main() {
    let _cli = Cli::parse();
    eprintln!("liste: not implemented yet");
    std::process::exit(1);
}
