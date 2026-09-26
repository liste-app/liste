//! MCP over stdio against the real `liste-mcp` binary and a real
//! `liste daemon`: list the tools, call each once, agree with the CLI on the
//! same store, the locked and version-mismatch errors, and the one-time
//! notice.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

fn short_temp_dir(label: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos()
        % 1_000_000_000;
    let base = if cfg!(unix) {
        PathBuf::from("/tmp")
    } else {
        std::env::temp_dir()
    };
    let dir = base.join(format!("liste-m-{label}-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The `liste` binary: built beside `liste-mcp`, or built now.
fn liste_bin() -> PathBuf {
    let mcp = PathBuf::from(env!("CARGO_BIN_EXE_liste-mcp"));
    let candidate = mcp
        .parent()
        .unwrap()
        .join(if cfg!(windows) { "liste.exe" } else { "liste" });
    if !candidate.exists() {
        let status = Command::new(env!("CARGO"))
            .args(["build", "-p", "liste-cli"])
            .status()
            .expect("build liste");
        assert!(status.success());
    }
    assert!(candidate.exists(), "{} missing", candidate.display());
    candidate
}

fn env<'a>(cmd: &'a mut Command, dir: &Path) -> &'a mut Command {
    cmd.env("LISTE_DATA_DIR", dir)
        .env("LISTE_SOCKET", dir.join("host.sock"))
}

fn daemon(dir: &Path, extra: &[&str]) -> Child {
    let child = env(&mut Command::new(liste_bin()), dir)
        .arg("daemon")
        .args(extra)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    while !dir.join("host.sock").exists() {
        assert!(Instant::now() < deadline, "daemon did not start");
        std::thread::sleep(Duration::from_millis(20));
    }
    child
}

fn cli(dir: &Path, args: &[&str]) -> (i32, String) {
    let out = env(&mut Command::new(liste_bin()), dir)
        .args(args)
        .output()
        .unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
    )
}

/// A JSON-RPC session with one `liste-mcp` process over stdio.
struct Mcp {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
}

impl Mcp {
    fn start(dir: &Path) -> Mcp {
        let mut child = env(&mut Command::new(env!("CARGO_BIN_EXE_liste-mcp")), dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Mcp {
            child,
            stdin,
            stdout,
            next_id: 1,
        }
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        let msg = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        writeln!(self.stdin, "{msg}").unwrap();
        self.stdin.flush().unwrap();
        loop {
            let mut line = String::new();
            let n = self.stdout.read_line(&mut line).unwrap();
            assert!(n > 0, "mcp closed its stdout");
            let v: Value = match serde_json::from_str(line.trim()) {
                Ok(v) => v,
                Err(_) => continue,
            };
            if v.get("id") == Some(&json!(id)) {
                return v;
            }
        }
    }

    fn notify(&mut self, method: &str) {
        let msg = json!({ "jsonrpc": "2.0", "method": method });
        writeln!(self.stdin, "{msg}").unwrap();
        self.stdin.flush().unwrap();
    }

    /// Initialize as `client_name`; returns the result object.
    fn initialize(&mut self, client_name: &str) -> Value {
        let r = self.request(
            "initialize",
            json!({
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": { "name": client_name, "version": "0.0.1" }
            }),
        );
        assert!(r.get("error").is_none(), "{r}");
        self.notify("notifications/initialized");
        r["result"].clone()
    }

    fn call(&mut self, tool: &str, args: Value) -> Value {
        let r = self.request("tools/call", json!({ "name": tool, "arguments": args }));
        assert!(r.get("error").is_none(), "{tool}: {r}");
        r["result"].clone()
    }

    fn tools(&mut self) -> Vec<Value> {
        let r = self.request("tools/list", json!({}));
        r["result"]["tools"].as_array().cloned().unwrap()
    }
}

impl Drop for Mcp {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn text_of(result: &Value) -> String {
    result["content"]
        .as_array()
        .map(|c| {
            c.iter()
                .filter_map(|b| b["text"].as_str())
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

#[test]
fn every_tool_once_and_the_cli_agrees() {
    let dir = short_temp_dir("tools");
    let mut d = daemon(&dir, &[]);
    let mut mcp = Mcp::start(&dir);
    let init = mcp.initialize("test-agent");
    assert!(init["capabilities"]["tools"].is_object());

    let tools = mcp.tools();
    let mut names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    names.sort_unstable();
    assert_eq!(
        names,
        [
            "capture",
            "complete",
            "get_task",
            "list_lists",
            "list_today",
            "list_upcoming",
            "search",
            "uncomplete",
            "update_task",
        ]
    );
    for t in &tools {
        let desc = t["description"].as_str().unwrap();
        assert!(desc.len() > 40, "{}: description too short", t["name"]);
        assert!(t["inputSchema"].is_object(), "{}: schema", t["name"]);
    }
    let capture_desc = tools.iter().find(|t| t["name"] == "capture").unwrap()["description"]
        .as_str()
        .unwrap();
    assert!(capture_desc.contains("natural language") && capture_desc.contains("#tag"));

    // capture
    let r = mcp.call(
        "capture",
        json!({ "text": "call mom today 11pm #family !high /Errands", "time_zone": "Europe/Istanbul" }),
    );
    assert_eq!(r["isError"], json!(false));
    let task = &r["structuredContent"]["task"];
    assert_eq!(task["title"], "call mom");
    assert_eq!(task["priority"], "high");
    assert_eq!(task["tags"], json!(["family"]));
    assert_eq!(task["list"]["title"], "Errands");
    assert!(r["structuredContent"]["spans"].as_array().unwrap().len() == 5);
    assert!(text_of(&r).contains("call mom"));
    let id = task["id"].as_str().unwrap().to_owned();

    // list_today agrees with the CLI on the same store.
    let r = mcp.call("list_today", json!({}));
    let mcp_titles: Vec<String> = r["structuredContent"]["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["title"].as_str().unwrap().to_owned())
        .collect();
    let (code, out) = cli(&dir, &["--json", "today"]);
    assert_eq!(code, 0);
    let cli_tasks: Vec<Value> = serde_json::from_str(&out).unwrap();
    let cli_titles: Vec<String> = cli_tasks
        .iter()
        .map(|t| t["title"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(mcp_titles, cli_titles);
    assert_eq!(mcp_titles, vec!["call mom"]);
    // And a CLI capture shows up through MCP.
    let (code, _) = cli(&dir, &["capture", "renew passport in 3 days"]);
    assert_eq!(code, 0);
    let r = mcp.call("list_upcoming", json!({ "days": 5 }));
    assert_eq!(
        r["structuredContent"]["tasks"][0]["title"],
        "renew passport"
    );

    // search, get_task, update_task, complete, uncomplete, list_lists
    let r = mcp.call("search", json!({ "query": "mom", "limit": 5 }));
    assert_eq!(r["structuredContent"]["tasks"].as_array().unwrap().len(), 1);
    let r = mcp.call("get_task", json!({ "id": id }));
    assert_eq!(r["structuredContent"]["id"], json!(id));
    let r = mcp.call(
        "update_task",
        json!({ "id": id, "title": "call dad", "due": "", "priority": "low", "list": "Family", "add_tags": ["weekend"], "remove_tags": ["family"] }),
    );
    let t = &r["structuredContent"];
    assert_eq!(t["title"], "call dad");
    assert!(t["due"].is_null());
    assert_eq!(t["priority"], "low");
    assert_eq!(t["list"]["title"], "Family");
    assert_eq!(t["tags"], json!(["weekend"]));
    let r = mcp.call("list_lists", json!({}));
    let lists: Vec<&str> = r["structuredContent"]["lists"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l["title"].as_str().unwrap())
        .collect();
    assert_eq!(lists, ["Errands", "Family"]);
    let r = mcp.call("complete", json!({ "id": id }));
    assert!(r["structuredContent"]["completed_at"].is_number());
    let r = mcp.call("uncomplete", json!({ "id": id }));
    assert!(r["structuredContent"]["completed_at"].is_null());
    // Errors for a bad id and an unknown id are tool errors, not protocol errors.
    let r = mcp.call("get_task", json!({ "id": "nope" }));
    assert_eq!(r["isError"], json!(true));
    assert_eq!(r["structuredContent"]["error"]["code"], "invalid");
    let r = mcp.call(
        "get_task",
        json!({ "id": "00000000-0000-7000-8000-000000000000" }),
    );
    assert_eq!(r["isError"], json!(true));
    assert_eq!(r["structuredContent"]["error"]["code"], "not_found");

    drop(mcp);
    let _ = d.kill();
    let _ = d.wait();
}

#[test]
fn a_locked_store_is_a_tool_error_that_names_the_app() {
    let dir = short_temp_dir("locked");
    let mut d = daemon(&dir, &["--locked"]);
    let mut mcp = Mcp::start(&dir);
    mcp.initialize("test-agent");
    for (tool, args) in [
        ("capture", json!({ "text": "x" })),
        ("list_today", json!({})),
        ("search", json!({ "query": "x" })),
    ] {
        let r = mcp.call(tool, args);
        assert_eq!(r["isError"], json!(true), "{tool}: {r}");
        assert_eq!(r["structuredContent"]["error"]["code"], "locked", "{tool}");
        let text = text_of(&r);
        assert!(
            text.contains("locked") && text.contains("unlock the Liste app"),
            "{text}"
        );
        assert!(!text.to_lowercase().contains("password"), "{text}");
    }
    drop(mcp);
    let _ = d.kill();
    let _ = d.wait();
}

#[test]
fn a_version_mismatch_is_the_restart_message() {
    use liste_ipc::protocol::{Body, IpcError, Message, Request, Response};
    use liste_ipc::transport;
    let dir = short_temp_dir("version");
    // A fake host that speaks a newer protocol: answers every hello with
    // the mismatch error, as a real newer host would.
    let endpoint = liste_ipc::Endpoint::from_path(&dir.join("host.sock"));
    let listener = transport::bind(&endpoint).unwrap();
    std::thread::spawn(move || {
        use liste_ipc::transport::ListenerExt;
        for conn in listener.incoming() {
            let Ok(mut stream) = conn else { continue };
            if let Ok(Some(msg)) = transport::receive(&mut stream) {
                let client = match msg.body {
                    Body::Request(Request::Hello {
                        protocol_version, ..
                    }) => protocol_version,
                    _ => 0,
                };
                let _ = transport::send(
                    &mut stream,
                    &Message {
                        id: msg.id,
                        body: Body::Response(Response::Error(IpcError::VersionMismatch {
                            host: liste_ipc::PROTOCOL_VERSION + 1,
                            client,
                        })),
                    },
                );
            }
        }
    });
    let mut mcp = Mcp::start(&dir);
    mcp.initialize("test-agent");
    let r = mcp.call("list_today", json!({}));
    assert_eq!(r["isError"], json!(true));
    assert_eq!(r["structuredContent"]["error"]["code"], "version_mismatch");
    assert!(
        text_of(&r).contains("restart Liste to finish updating"),
        "{r}"
    );
}

#[test]
fn the_plaintext_notice_appears_once_per_client_identity() {
    let dir = short_temp_dir("notice");
    let mut d = daemon(&dir, &[]);
    let notice = |init: &Value| {
        init["instructions"]
            .as_str()
            .unwrap_or_default()
            .contains("sees task titles, notes, tags, dates, and lists in plaintext")
    };
    let first = Mcp::start(&dir).initialize("agent-a");
    assert!(notice(&first), "{first}");
    assert!(
        first["instructions"]
            .as_str()
            .unwrap()
            .contains("cloud model")
    );
    let second = Mcp::start(&dir).initialize("agent-a");
    assert!(!notice(&second), "{second}");
    assert!(
        second["instructions"]
            .as_str()
            .unwrap()
            .contains("Liste is a to-do app")
    );
    let other = Mcp::start(&dir).initialize("agent-b");
    assert!(notice(&other));
    let seen: Vec<String> =
        serde_json::from_slice(&std::fs::read(dir.join("mcp-clients.json")).unwrap()).unwrap();
    assert_eq!(seen, ["agent-a", "agent-b"]);
    let _ = d.kill();
    let _ = d.wait();
}
