//! The tools, the connection to the host, and the first-connection notice.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use liste_ipc::{Client, ClientError, IpcError, ListView, TaskPatch, TaskView};
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{
    CallToolResult, ContentBlock, Implementation, InitializeRequestParams, InitializeResult,
    ServerCapabilities, ServerConfig,
};
use rmcp::service::RequestContext;
use rmcp::{
    ErrorData as McpError, RoleServer, ServerHandler, schemars, tool, tool_handler, tool_router,
};
use serde::Deserialize;
use serde_json::{Value, json};

const CLIENT_NAME: &str = "liste-mcp";

/// The one-time notice (Section 7), verbatim in the instructions field the
/// first time a client identity connects.
pub const PLAINTEXT_NOTICE: &str = "Notice for the person using this agent: Liste's tasks are end-to-end encrypted between devices and the Liste servers never see them, but this MCP connection is on the device side of that encryption. The agent reading these tools sees task titles, notes, tags, dates, and lists in plaintext, and if the agent is a cloud model, that content goes to the model's provider. That is your choice to make; disconnect the agent if it is not. Nothing here reaches Liste's servers unencrypted.";

const DESCRIPTION: &str = "Liste is a to-do app. These tools read and change the tasks on this device through the running Liste app. Use `capture` with one natural-language line, the same syntax a person types into Liste (dates, times, #tags, !priority, /list, recurrence). Task ids are UUID strings returned by every tool; pass them to get_task, update_task, complete, and uncomplete. `list_lists` gives the list names a task can be filed under.";

#[derive(Clone)]
pub struct Liste {
    host: Arc<Mutex<Option<Client>>>,
    tool_router: ToolRouter<Liste>,
}

fn launch() -> std::io::Result<()> {
    liste_ipc::launch::launch_host(&liste_ipc::launch::sibling_liste())
}

/// Run `f` against a connected client, opening the connection on first use
/// and reopening it once if the host went away since the last call.
fn with_host<T>(
    host: &Mutex<Option<Client>>,
    f: impl Fn(&mut Client) -> Result<T, ClientError>,
) -> Result<T, ClientError> {
    let mut guard = host.lock().unwrap_or_else(|e| e.into_inner());
    if guard.is_none() {
        *guard = Some(Client::connect_or_start(CLIENT_NAME, &launch)?);
    }
    let first = f(guard.as_mut().expect("just connected"));
    match first {
        Err(ClientError::Io(_)) | Err(ClientError::NotRunning) => {
            *guard = None;
            let mut fresh = Client::connect_or_start(CLIENT_NAME, &launch)?;
            let out = f(&mut fresh);
            *guard = Some(fresh);
            out
        }
        other => other,
    }
}

/// Tool error with the same categories as the CLI exit codes.
fn tool_error(e: ClientError) -> CallToolResult {
    let (code, message) = match &e {
        ClientError::Locked(_) => (
            "locked",
            "The Liste store on this device is locked. Ask the person to unlock the Liste app; no credentials can be entered here, and nothing is sent to any server.".to_owned(),
        ),
        ClientError::NotRunning | ClientError::DidNotStart => (
            "not_running",
            "Liste is not running on this device and could not be started.".to_owned(),
        ),
        ClientError::VersionMismatch(m) => ("version_mismatch", m.to_string()),
        ClientError::Remote(IpcError::NotFound { id }) => ("not_found", format!("no task {id}")),
        ClientError::Remote(IpcError::Invalid { message }) => ("invalid", message.clone()),
        other => ("internal", other.to_string()),
    };
    let mut r = CallToolResult::error(vec![ContentBlock::text(format!("{code}: {message}"))]);
    r.structured_content = Some(json!({ "error": { "code": code, "message": message } }));
    r
}

fn ok(text: String, value: Value) -> CallToolResult {
    let mut r = CallToolResult::success(vec![ContentBlock::text(text)]);
    r.structured_content = Some(value);
    r
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
        line.push_str(&format!("  due {due}"));
    }
    if let Some(list) = &t.list {
        line.push_str(&format!("  /{}", list.title));
    }
    for tag in &t.tags {
        line.push_str(&format!("  #{tag}"));
    }
    if t.priority != "none" {
        line.push_str(&format!("  !{}", t.priority));
    }
    if let Some(r) = &t.recurrence {
        line.push_str(&format!("  repeats {r}"));
    }
    line
}

fn task_result(t: TaskView) -> CallToolResult {
    ok(task_line(&t), serde_json::to_value(&t).unwrap_or_default())
}

fn tasks_result(tasks: Vec<TaskView>) -> CallToolResult {
    let text = if tasks.is_empty() {
        "no tasks".to_owned()
    } else {
        tasks.iter().map(task_line).collect::<Vec<_>>().join("\n")
    };
    ok(text, json!({ "tasks": tasks }))
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CaptureArgs {
    /// One line as a person would type it into Liste. Everything that is
    /// not a recognized date, time, list, tag, priority, or recurrence
    /// phrase becomes the title.
    pub text: String,
    /// IANA time zone to interpret dates and times in, e.g. "Europe/Istanbul".
    /// Defaults to the device's zone.
    #[serde(default)]
    pub time_zone: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SearchArgs {
    /// Words to look for in titles and notes. Each word matches as a
    /// prefix, so "pass" finds "passport".
    pub query: String,
    /// Maximum number of tasks to return, 1 to 500. Default 20.
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct UpcomingArgs {
    /// How many days ahead to look, starting tomorrow. Default 7.
    #[serde(default)]
    pub days: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct TaskIdArgs {
    /// The task id, a UUID string as returned by the other tools.
    pub id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct UpdateTaskArgs {
    /// The task id, a UUID string as returned by the other tools.
    pub id: String,
    /// New title.
    #[serde(default)]
    pub title: Option<String>,
    /// New notes text; replaces the existing notes.
    #[serde(default)]
    pub notes: Option<String>,
    /// New due date as natural language, e.g. "tomorrow 5pm", "next friday",
    /// "jan 5". An empty string clears the due date.
    #[serde(default)]
    pub due: Option<String>,
    /// One of "high", "medium", "low", "none".
    #[serde(default)]
    pub priority: Option<String>,
    /// A list name from list_lists. An empty string moves the task to the
    /// inbox; a name that does not exist creates that list.
    #[serde(default)]
    pub list: Option<String>,
    /// Tag names to add (without the leading #). Unknown tags are created.
    #[serde(default)]
    pub add_tags: Vec<String>,
    /// Tag names to remove.
    #[serde(default)]
    pub remove_tags: Vec<String>,
}

fn parse_id(id: &str) -> Result<uuid::Uuid, CallToolResult> {
    id.trim().parse().map_err(|_| {
        tool_error(ClientError::Remote(IpcError::Invalid {
            message: format!(
                "{id:?} is not a task id; ids are UUID strings returned by the other tools"
            ),
        }))
    })
}

#[tool_router]
impl Liste {
    pub fn new() -> Self {
        Liste {
            host: Arc::new(Mutex::new(None)),
            tool_router: Self::tool_router(),
        }
    }

    fn call<T>(&self, f: impl Fn(&mut Client) -> Result<T, ClientError>) -> Result<T, ClientError> {
        with_host(&self.host, f)
    }

    #[tool(
        name = "capture",
        description = "Create a task from one line of natural language, exactly as a person types it into Liste, e.g. \"call mom tomorrow 5pm #family !high /Errands\". Recognized: dates (today, tomorrow, next tuesday, jan 5, 2026-01-05, in 3 days), times (5pm, 17:00, noon), a list (/Name, or \"in Name\" for an existing list), tags (#tag), priority (!high, !medium, !low, !!, p1), and recurrence (every tuesday, every 2 weeks, every 2nd tuesday, monthly on the 15th, 3 days after completion). Everything else becomes the title; nothing is guessed. Returns the created task with its id, and the byte spans of the input that were interpreted."
    )]
    fn capture(&self, Parameters(args): Parameters<CaptureArgs>) -> CallToolResult {
        match self.call(|c| c.capture(&args.text, args.time_zone.as_deref())) {
            Ok((task, spans)) => {
                let mut text = task_line(&task);
                if !spans.is_empty() {
                    let parts: Vec<String> = spans
                        .iter()
                        .map(|s| format!("{}={:?}", s.kind, &args.text[s.start..s.end]))
                        .collect();
                    text.push_str(&format!("\ninterpreted: {}", parts.join(", ")));
                }
                ok(text, json!({ "task": task, "spans": spans }))
            }
            Err(e) => tool_error(e),
        }
    }

    #[tool(
        name = "search",
        description = "Full-text search over the titles and notes of live tasks on this device. Each word in the query is a prefix. Returns matching tasks, newest first, up to `limit` (default 20)."
    )]
    fn search(&self, Parameters(args): Parameters<SearchArgs>) -> CallToolResult {
        match self.call(|c| c.search(&args.query, args.limit.unwrap_or(20).clamp(1, 500))) {
            Ok(tasks) => tasks_result(tasks),
            Err(e) => tool_error(e),
        }
    }

    #[tool(
        name = "list_today",
        description = "Tasks that are due today or overdue and not completed, in the person's manual order. Takes no arguments."
    )]
    fn list_today(&self) -> CallToolResult {
        match self.call(|c| c.today()) {
            Ok(tasks) => tasks_result(tasks),
            Err(e) => tool_error(e),
        }
    }

    #[tool(
        name = "list_upcoming",
        description = "Tasks due in the coming days, starting tomorrow, not completed. `days` is how far ahead to look (default 7)."
    )]
    fn list_upcoming(&self, Parameters(args): Parameters<UpcomingArgs>) -> CallToolResult {
        match self.call(|c| c.upcoming(args.days.unwrap_or(7).clamp(1, 3650))) {
            Ok(tasks) => tasks_result(tasks),
            Err(e) => tool_error(e),
        }
    }

    #[tool(
        name = "complete",
        description = "Mark a task complete by id. If the task recurs, Liste creates the next instance automatically. Returns the completed task."
    )]
    fn complete(&self, Parameters(args): Parameters<TaskIdArgs>) -> CallToolResult {
        let id = match parse_id(&args.id) {
            Ok(id) => id,
            Err(e) => return e,
        };
        match self.call(|c| c.complete(id)) {
            Ok(task) => task_result(task),
            Err(e) => tool_error(e),
        }
    }

    #[tool(
        name = "uncomplete",
        description = "Clear a task's completion by id, making it open again. Returns the task."
    )]
    fn uncomplete(&self, Parameters(args): Parameters<TaskIdArgs>) -> CallToolResult {
        let id = match parse_id(&args.id) {
            Ok(id) => id,
            Err(e) => return e,
        };
        match self.call(|c| c.uncomplete(id)) {
            Ok(task) => task_result(task),
            Err(e) => tool_error(e),
        }
    }

    #[tool(
        name = "get_task",
        description = "Fetch one task by id: title, notes, list, due date (and whether it is all-day), priority, status, completion, tags, and recurrence rule."
    )]
    fn get_task(&self, Parameters(args): Parameters<TaskIdArgs>) -> CallToolResult {
        let id = match parse_id(&args.id) {
            Ok(id) => id,
            Err(e) => return e,
        };
        match self.call(|c| c.task(id)) {
            Ok(task) => task_result(task),
            Err(e) => tool_error(e),
        }
    }

    #[tool(
        name = "update_task",
        description = "Change fields of a task by id. Only the fields given change: title, notes, due (natural-language date text, empty string clears), priority (high, medium, low, none), list (a name from list_lists; empty string moves to the inbox; an unknown name creates the list), add_tags, remove_tags. Use this to fix a capture that came out wrong. Returns the updated task."
    )]
    fn update_task(&self, Parameters(args): Parameters<UpdateTaskArgs>) -> CallToolResult {
        let id = match parse_id(&args.id) {
            Ok(id) => id,
            Err(e) => return e,
        };
        let patch = TaskPatch {
            title: args.title,
            notes: args.notes,
            due: args.due,
            priority: args.priority,
            list: args.list,
            add_tags: args.add_tags,
            remove_tags: args.remove_tags,
        };
        match self.call(|c| c.update(id, patch.clone())) {
            Ok(task) => task_result(task),
            Err(e) => tool_error(e),
        }
    }

    #[tool(
        name = "list_lists",
        description = "The lists (projects) in this space, with ids and titles, so a task can be filed with capture's /Name or update_task's list. Takes no arguments."
    )]
    fn list_lists(&self) -> CallToolResult {
        match self.call(|c| c.lists()) {
            Ok(lists) => {
                let text = if lists.is_empty() {
                    "no lists".to_owned()
                } else {
                    lists
                        .iter()
                        .map(|l: &ListView| format!("{}  {}", l.id, l.title))
                        .collect::<Vec<_>>()
                        .join("\n")
                };
                ok(text, json!({ "lists": lists }))
            }
            Err(e) => tool_error(e),
        }
    }
}

impl Default for Liste {
    fn default() -> Self {
        Liste::new()
    }
}

/// Where seen client identities are recorded: beside the host's database,
/// never in the store. The host tells us its data directory; if it cannot
/// be reached, the platform default is used.
fn seen_file(host: &Mutex<Option<Client>>) -> Option<PathBuf> {
    let dir = with_host(host, |c| c.status().map(|s| PathBuf::from(s.data_dir)))
        .ok()
        .or_else(liste_ipc::paths::default_data_dir)?;
    Some(dir.join("mcp-clients.json"))
}

/// Whether this client identity has connected before; records it if not.
fn first_connection(path: Option<PathBuf>, identity: &str) -> bool {
    let Some(path) = path else {
        return true;
    };
    let mut seen: Vec<String> = std::fs::read(&path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    if seen.iter().any(|s| s == identity) {
        return false;
    }
    seen.push(identity.to_owned());
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(&path, serde_json::to_vec_pretty(&seen).unwrap_or_default());
    true
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for Liste {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::from_build_env())
            .with_instructions(DESCRIPTION.to_string())
    }

    async fn initialize(
        &self,
        request: InitializeRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<InitializeResult, McpError> {
        context.peer.set_peer_info(request.clone());
        let mut info = self.negotiate_initialize(&request)?;
        let identity = request.client_info.name.to_string();
        if first_connection(seen_file(&self.host), &identity) {
            info.instructions = Some(format!("{PLAINTEXT_NOTICE}\n\n{DESCRIPTION}"));
        }
        Ok(info)
    }
}
