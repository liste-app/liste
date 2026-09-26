//! The `liste` command-line tool (Section 13).
//!
//! Every user command is a thin IPC client of the host process: it contains
//! no core logic, opens no database, and holds no keys. If no host is
//! running it starts one in the background and connects once the socket is
//! ready. `liste daemon` is the one exception: it runs the core's host
//! module headless for machines without a GUI.
//!
//! Exit codes: 0 success, 1 other failure, 2 the store is locked, 3 no host
//! is running or one could not be started, 4 protocol version mismatch.

use std::io::{self, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use liste_ipc::{Client, ClientError, DebugRequest, Endpoint, Response, TaskPatch, TaskView};
use uuid::Uuid;

#[derive(Parser)]
#[command(name = "liste", version, about = "Liste command-line tool")]
struct Cli {
    /// Print JSON instead of text.
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Capture a task from a natural-language string.
    Capture {
        text: Vec<String>,
        /// IANA time zone to interpret dates in (default: the host's).
        #[arg(long)]
        tz: Option<String>,
    },
    /// Search tasks (local full-text search).
    Search {
        query: Vec<String>,
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    /// Tasks due today or overdue.
    Today,
    /// Tasks due in the coming days.
    Upcoming {
        #[arg(long, default_value_t = 7)]
        days: u32,
    },
    /// Show a task.
    Show { id: Uuid },
    /// Mark a task complete.
    Complete { id: Uuid },
    /// Mark a task not complete.
    Uncomplete { id: Uuid },
    /// Change fields of a task.
    Update {
        id: Uuid,
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        notes: Option<String>,
        /// Natural-language date; an empty string clears it.
        #[arg(long)]
        due: Option<String>,
        /// high, medium, low, or none.
        #[arg(long)]
        priority: Option<String>,
        /// A list name; an empty string moves the task to the inbox.
        #[arg(long)]
        list: Option<String>,
        #[arg(long = "tag")]
        add_tags: Vec<String>,
        #[arg(long = "untag")]
        remove_tags: Vec<String>,
    },
    /// The lists in the space.
    Lists,
    /// Undo the last change made on this device.
    Undo,
    /// Redo the last undone change.
    Redo,
    /// Host status.
    Status,
    /// Run the headless host for machines without the desktop app.
    Daemon {
        /// Data directory (default: the platform's, or LISTE_DATA_DIR).
        #[arg(long)]
        data_dir: Option<PathBuf>,
        /// Socket path (default: the platform's, or LISTE_SOCKET).
        #[arg(long)]
        socket: Option<PathBuf>,
        /// Start with the store locked.
        #[arg(long)]
        locked: bool,
    },
    /// Developer harness commands served by the host.
    Debug {
        #[command(subcommand)]
        command: DebugCmd,
    },
}

#[derive(Subcommand)]
enum DebugCmd {
    /// The most recent ops in the local log.
    Oplog {
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    /// A materialized row: task, list, or tag.
    Row { entity: String, id: Uuid },
    /// The sync cursor and pending count.
    Cursor,
    /// Apply the fixture.
    Fixture {
        #[arg(long, default_value_t = 1000)]
        tasks: usize,
        #[arg(long, default_value_t = 42)]
        seed: u64,
    },
    /// Run a named two-device scenario.
    Scenario { name: String },
    /// Run the sync runner once.
    Sync,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let json = cli.json;
    match run(cli) {
        Ok(()) => ExitCode::from(0),
        Err(e) => {
            let code = match &e {
                Error::Client(ClientError::Locked(_)) => 2,
                Error::Client(ClientError::NotRunning | ClientError::DidNotStart) => 3,
                Error::Client(ClientError::VersionMismatch(_)) => 4,
                _ => 1,
            };
            if json {
                let _ = writeln!(
                    io::stderr(),
                    "{}",
                    serde_json::json!({ "error": e.to_string(), "exit_code": code })
                );
            } else {
                let _ = writeln!(io::stderr(), "liste: {e}");
            }
            ExitCode::from(code)
        }
    }
}

#[derive(Debug)]
enum Error {
    Client(ClientError),
    Host(String),
    Other(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Client(e) => write!(f, "{e}"),
            Error::Host(m) | Error::Other(m) => f.write_str(m),
        }
    }
}

impl From<ClientError> for Error {
    fn from(e: ClientError) -> Self {
        Error::Client(e)
    }
}

impl std::error::Error for Error {}

fn run(cli: Cli) -> Result<(), Error> {
    match cli.command {
        Cmd::Daemon {
            data_dir,
            socket,
            locked,
        } => daemon(data_dir, socket, locked),
        command => {
            let mut client = Client::connect_or_start("liste-cli", &launch_host)?;
            let out = execute(&mut client, command)?;
            print(out, cli.json);
            Ok(())
        }
    }
}

/// What a command produced, rendered later as text or JSON.
enum Output {
    Task(TaskView),
    Captured(TaskView, Vec<liste_ipc::SpanView>),
    Tasks(Vec<TaskView>),
    Lists(Vec<liste_ipc::ListView>),
    Done(bool),
    Status(liste_ipc::StatusView),
    Response(Response),
}

fn execute(client: &mut Client, command: Cmd) -> Result<Output, Error> {
    Ok(match command {
        Cmd::Capture { text, tz } => {
            let (task, spans) = client.capture(&text.join(" "), tz.as_deref())?;
            Output::Captured(task, spans)
        }
        Cmd::Search { query, limit } => Output::Tasks(client.search(&query.join(" "), limit)?),
        Cmd::Today => Output::Tasks(client.today()?),
        Cmd::Upcoming { days } => Output::Tasks(client.upcoming(days)?),
        Cmd::Show { id } => Output::Task(client.task(id)?),
        Cmd::Complete { id } => Output::Task(client.complete(id)?),
        Cmd::Uncomplete { id } => Output::Task(client.uncomplete(id)?),
        Cmd::Update {
            id,
            title,
            notes,
            due,
            priority,
            list,
            add_tags,
            remove_tags,
        } => Output::Task(client.update(
            id,
            TaskPatch {
                title,
                notes,
                due,
                priority,
                list,
                add_tags,
                remove_tags,
            },
        )?),
        Cmd::Lists => Output::Lists(client.lists()?),
        Cmd::Undo => Output::Done(client.undo()?),
        Cmd::Redo => Output::Done(client.redo()?),
        Cmd::Status => Output::Status(client.status()?),
        Cmd::Debug { command } => Output::Response(client.debug(match command {
            DebugCmd::Oplog { limit } => DebugRequest::OpLog { limit },
            DebugCmd::Row { entity, id } => DebugRequest::Row { entity, id },
            DebugCmd::Cursor => DebugRequest::Cursor,
            DebugCmd::Fixture { tasks, seed } => DebugRequest::Fixture { tasks, seed },
            DebugCmd::Scenario { name } => DebugRequest::Scenario { name },
            DebugCmd::Sync => DebugRequest::SyncNow,
        })?),
        Cmd::Daemon { .. } => unreachable!("handled before connecting"),
    })
}

fn print(out: Output, json: bool) {
    let mut stdout = io::stdout().lock();
    if json {
        let value = match &out {
            Output::Task(t) => serde_json::to_value(t),
            Output::Captured(t, spans) => {
                serde_json::to_value(serde_json::json!({ "task": t, "spans": spans }))
            }
            Output::Tasks(t) => serde_json::to_value(t),
            Output::Lists(l) => serde_json::to_value(l),
            Output::Done(changed) => {
                serde_json::to_value(serde_json::json!({ "changed": changed }))
            }
            Output::Status(s) => serde_json::to_value(s),
            Output::Response(r) => serde_json::to_value(r),
        };
        let _ = writeln!(stdout, "{}", value.unwrap_or_default());
        return;
    }
    match out {
        Output::Task(t) => {
            let _ = writeln!(stdout, "{}", task_line(&t));
            if !t.notes.is_empty() {
                let _ = writeln!(stdout, "  {}", t.notes.replace('\n', "\n  "));
            }
        }
        Output::Captured(t, spans) => {
            let _ = writeln!(stdout, "{}", task_line(&t));
            let _ = writeln!(stdout, "  title:    {}", t.title);
            if let Some(due) = &t.due {
                let _ = writeln!(
                    stdout,
                    "  due:      {due}{}",
                    if t.due_all_day { " (all day)" } else { "" }
                );
            }
            if let Some(list) = &t.list {
                let _ = writeln!(stdout, "  list:     {}", list.title);
            }
            if !t.tags.is_empty() {
                let _ = writeln!(stdout, "  tags:     {}", t.tags.join(", "));
            }
            if t.priority != "none" {
                let _ = writeln!(stdout, "  priority: {}", t.priority);
            }
            if let Some(rule) = &t.recurrence {
                let _ = writeln!(stdout, "  repeats:  {rule}");
            }
            if !spans.is_empty() {
                let parts: Vec<String> = spans
                    .iter()
                    .map(|s| format!("{}@{}..{}", s.kind, s.start, s.end))
                    .collect();
                let _ = writeln!(stdout, "  spans:    {}", parts.join(" "));
            }
        }
        Output::Tasks(tasks) => {
            if tasks.is_empty() {
                let _ = writeln!(stdout, "nothing here");
            }
            for t in tasks {
                let _ = writeln!(stdout, "{}", task_line(&t));
            }
        }
        Output::Lists(lists) => {
            if lists.is_empty() {
                let _ = writeln!(stdout, "no lists");
            }
            for l in lists {
                let _ = writeln!(stdout, "{}  {}", l.id, l.title);
            }
        }
        Output::Done(changed) => {
            let _ = writeln!(stdout, "{}", if changed { "done" } else { "nothing to do" });
        }
        Output::Status(s) => {
            let _ = writeln!(
                stdout,
                "host {} · {} · device {} · space {} · {} pending op(s)\n{}",
                s.host_version,
                if s.locked { "locked" } else { "unlocked" },
                s.device_id,
                s.space_id,
                s.pending_ops,
                s.data_dir
            );
        }
        Output::Response(r) => match r {
            Response::OpLog(ops) => {
                for op in ops {
                    let _ = writeln!(
                        stdout,
                        "{} seq={} applied={} {} {} {} {}",
                        op.op_id,
                        op.seq.map(|s| s.to_string()).unwrap_or_else(|| "-".into()),
                        op.applied,
                        op.hlc,
                        op.entity_type,
                        op.entity_id,
                        op.mutation
                    );
                }
            }
            Response::Row(v) => {
                let _ = writeln!(
                    stdout,
                    "{}",
                    serde_json::to_string_pretty(&v).unwrap_or_default()
                );
            }
            Response::Cursor {
                last_seq,
                pending,
                op_count,
            } => {
                let _ = writeln!(
                    stdout,
                    "cursor {last_seq} · {pending} pending · {op_count} ops in log"
                );
            }
            Response::Fixture { tasks, ops } => {
                let _ = writeln!(stdout, "fixture: {tasks} tasks, {ops} ops");
            }
            Response::Scenario {
                name,
                passed,
                report,
            } => {
                let _ = writeln!(
                    stdout,
                    "{name}: {}\n{report}",
                    if passed { "passed" } else { "FAILED" }
                );
            }
            Response::Synced { pushed, pulled } => {
                let _ = writeln!(stdout, "synced: pushed {pushed}, pulled {pulled}");
            }
            other => {
                let _ = writeln!(
                    stdout,
                    "{}",
                    serde_json::to_string(&other).unwrap_or_default()
                );
            }
        },
    }
}

fn task_line(t: &TaskView) -> String {
    let mut line = format!(
        "{} {}  {}",
        if t.completed_at.is_some() {
            "[x]"
        } else {
            "[ ]"
        },
        t.id,
        t.title
    );
    if let Some(due) = &t.due {
        line.push_str(&format!("  · {due}"));
    }
    if let Some(list) = &t.list {
        line.push_str(&format!("  · /{}", list.title));
    }
    for tag in &t.tags {
        line.push_str(&format!("  #{tag}"));
    }
    if t.priority != "none" {
        line.push_str(&format!("  !{}", t.priority));
    }
    if t.recurrence.is_some() {
        line.push_str("  ↻");
    }
    line
}

/// Start a host in the background with this binary as the daemon.
fn launch_host() -> io::Result<()> {
    let exe = std::env::current_exe()?;
    liste_ipc::launch::launch_host(&exe)
}

fn daemon(data_dir: Option<PathBuf>, socket: Option<PathBuf>, locked: bool) -> Result<(), Error> {
    use liste_core::host::{Host, HostConfig};
    let data_dir = data_dir
        .or_else(liste_ipc::paths::default_data_dir)
        .ok_or_else(|| {
            Error::Other("cannot determine the data directory; set LISTE_DATA_DIR".into())
        })?;
    let endpoint = match socket {
        Some(p) => Endpoint::from_path(&p),
        None => Endpoint::default_for_host().ok_or_else(|| {
            Error::Other("cannot determine the socket path; set LISTE_SOCKET".into())
        })?,
    };
    let mut config = HostConfig::new(&data_dir, endpoint.clone());
    config.locked = locked;
    let host = Host::start(config).map_err(|e| Error::Host(e.to_string()))?;
    eprintln!(
        "liste daemon: store {} · listening on {}",
        data_dir.display(),
        endpoint
    );
    let (tx, rx) = std::sync::mpsc::channel();
    ctrlc::set_handler(move || {
        let _ = tx.send(());
    })
    .map_err(|e| Error::Other(e.to_string()))?;
    let _ = rx.recv();
    eprintln!("liste daemon: shutting down");
    host.shutdown();
    Ok(())
}
