//! Host and IPC (Section 15), the in-process half: store ownership, the
//! locked state over IPC, the version handshake, clean shutdown, and the
//! single sync runner. The process-level half (a second `liste daemon`,
//! auto-start, a crashed host) lives in `cli/tests/host_ipc.rs`, because
//! it drives the `liste` binary.

mod common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use liste_core::crypto::Keyring;
use liste_core::host::{Host, HostConfig, HostError, StoreLock, SyncError, SyncReport, SyncRunner};
use liste_core::store::Store;
use liste_ipc::protocol::{Body, Message, Request, Response};
use liste_ipc::{Client, ClientError, DebugRequest, Endpoint, IpcError, transport};

fn config(label: &str) -> HostConfig {
    let dir = common::temp_dir(label);
    let endpoint = Endpoint::from_path(&dir.join("host.sock"));
    let mut c = HostConfig::new(&dir, endpoint);
    c.maintenance_interval = Duration::from_millis(100);
    c
}

#[test]
fn a_second_process_cannot_open_the_store_while_a_host_holds_it() {
    let c = config("host-lock");
    let dir = c.data_dir.clone();
    let host = Host::start(c).unwrap();
    // A second host on the same directory fails at once, before touching SQLite.
    let second = config("host-lock-2");
    let mut second_config = second;
    second_config.data_dir = dir.clone();
    match Host::start(second_config) {
        Err(HostError::StoreHeld(d)) => assert_eq!(d, dir),
        other => panic!("expected StoreHeld, got {other:?}"),
    }
    // The raw lock says the same.
    assert!(matches!(
        StoreLock::acquire(&dir),
        Err(HostError::StoreHeld(_))
    ));
    // Clean shutdown releases it and removes the socket.
    let endpoint = host.endpoint().clone();
    host.shutdown();
    let lock = StoreLock::acquire(&dir).unwrap();
    drop(lock);
    if let Endpoint::Path(p) = &endpoint {
        assert!(!p.exists(), "socket file removed on shutdown");
    }
    assert!(matches!(
        Client::connect(&endpoint),
        Err(ClientError::NotRunning)
    ));
    // And a new host can take the directory over.
    let mut again = config("host-lock-3");
    again.data_dir = dir;
    let host = Host::start(again).unwrap();
    host.shutdown();
}

#[test]
fn cli_capture_works_with_the_gui_running() {
    // The "GUI" here is a host started in-process, as the app does; the
    // client is a separate connection, as the CLI is.
    let host = Host::start(config("host-capture")).unwrap();
    let mut client = Client::connect(host.endpoint()).unwrap();
    let (task, spans) = client
        .capture(
            "call mom tomorrow 5pm #family !high",
            Some("Europe/Istanbul"),
        )
        .unwrap();
    assert_eq!(task.title, "call mom");
    assert_eq!(task.tags, vec!["family"]);
    assert_eq!(task.priority, "high");
    assert!(task.due.as_deref().is_some_and(|d| d.ends_with("17:00")));
    assert_eq!(spans.len(), 4);
    // The GUI sees the same row in-process, through the same store.
    let title = host.with_store(|s| {
        s.task(liste_core::ids::Id::from_bytes(*task.id.as_bytes()))
            .unwrap()
            .unwrap()
            .title
    });
    assert_eq!(title, "call mom");
    // Search, lists, update, complete, undo over IPC.
    assert_eq!(client.search("call", 10).unwrap().len(), 1);
    let updated = client
        .update(
            task.id,
            liste_ipc::TaskPatch {
                title: Some("call dad".into()),
                list: Some("Family".into()),
                add_tags: vec!["weekend".into()],
                remove_tags: vec!["family".into()],
                priority: Some("low".into()),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(updated.title, "call dad");
    assert_eq!(
        updated.list.as_ref().map(|l| l.title.as_str()),
        Some("Family")
    );
    assert_eq!(updated.tags, vec!["weekend"]);
    assert_eq!(updated.priority, "low");
    assert_eq!(client.lists().unwrap().len(), 1);
    let done = client.complete(task.id).unwrap();
    assert!(done.completed_at.is_some());
    assert!(client.undo().unwrap());
    assert!(client.task(task.id).unwrap().completed_at.is_none());
    assert!(client.redo().unwrap());
    assert!(client.task(task.id).unwrap().completed_at.is_some());
    // Many clients at once.
    let endpoint = host.endpoint().clone();
    let handles: Vec<_> = (0..8)
        .map(|i| {
            let endpoint = endpoint.clone();
            std::thread::spawn(move || {
                let mut c = Client::connect(&endpoint).unwrap();
                for k in 0..10 {
                    c.capture(&format!("task {i}-{k}"), None).unwrap();
                }
                c.status().unwrap().pending_ops
            })
        })
        .collect();
    for h in handles {
        assert!(h.join().unwrap() > 0);
    }
    assert_eq!(client.search("task", 200).unwrap().len(), 80);
    host.shutdown();
}

#[test]
fn a_locked_store_returns_locked_over_ipc() {
    let mut c = config("host-locked");
    c.locked = true;
    let host = Host::start(c).unwrap();
    assert!(host.is_locked());
    let mut client = Client::connect(host.endpoint()).unwrap();
    // Status answers normally; every data operation is `locked`.
    let status = client.status().unwrap();
    assert!(status.locked);
    for r in [
        client.capture("x", None).map(|_| ()),
        client.search("x", 5).map(|_| ()),
        client.today().map(|_| ()),
        client.lists().map(|_| ()),
        client.undo().map(|_| ()),
        client.debug(DebugRequest::OpLog { limit: 5 }).map(|_| ()),
    ] {
        assert!(
            matches!(r, Err(ClientError::Locked(IpcError::Locked))),
            "{r:?}"
        );
    }
    // The GUI unlocks in-process; the client then works without reconnecting.
    host.unlock();
    assert!(!client.status().unwrap().locked);
    client.capture("now it works", None).unwrap();
    host.lock();
    assert!(matches!(
        client.search("now", 5),
        Err(ClientError::Locked(_))
    ));
    host.shutdown();
}

#[test]
fn a_protocol_version_mismatch_returns_the_restart_message() {
    let host = Host::start(config("host-version")).unwrap();
    let mut stream = transport::connect(host.endpoint()).unwrap();
    transport::send(
        &mut stream,
        &Message {
            id: 1,
            body: Body::Request(Request::Hello {
                protocol_version: liste_ipc::PROTOCOL_VERSION + 1,
                client: "future-cli".into(),
            }),
        },
    )
    .unwrap();
    let reply = transport::receive(&mut stream).unwrap().unwrap();
    match reply.body {
        Body::Response(Response::Error(e @ IpcError::VersionMismatch { host: h, client: c })) => {
            assert_eq!(h, liste_ipc::PROTOCOL_VERSION);
            assert_eq!(c, liste_ipc::PROTOCOL_VERSION + 1);
            assert!(
                e.to_string()
                    .starts_with("restart Liste to finish updating")
            );
        }
        other => panic!("expected version mismatch, got {other:?}"),
    }
    // The host closed the connection after the mismatch.
    assert!(transport::receive(&mut stream).unwrap().is_none());
    // A current client still connects.
    Client::connect(host.endpoint()).unwrap();
    host.shutdown();
}

/// Counts pushes and remembers which thread and process did them.
#[derive(Default)]
struct CountingSync {
    pushes: Arc<Mutex<Vec<(u32, usize)>>>,
}

impl SyncRunner for CountingSync {
    fn name(&self) -> &str {
        "counting"
    }

    fn sync(&mut self, store: &mut Store, _keyring: &Keyring) -> Result<SyncReport, SyncError> {
        // A real runner would push these; this one records the intent and
        // marks them pushed so the count is exact.
        let mut pushed = 0;
        let mut assigned = Vec::new();
        let mut seq = 0u64;
        for space in spaces(store) {
            for op in store.pending_ops(space)? {
                seq += 1;
                assigned.push((op.op_id, seq));
                pushed += 1;
            }
            store.mark_pushed(space, &assigned)?;
            assigned.clear();
        }
        if pushed > 0 {
            self.pushes
                .lock()
                .unwrap()
                .push((std::process::id(), pushed));
        }
        Ok(SyncReport { pushed, pulled: 0 })
    }
}

fn spaces(store: &Store) -> Vec<liste_core::ids::Id> {
    store
        .connection()
        .prepare("SELECT DISTINCT space_id FROM ops")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

#[test]
fn exactly_one_process_pushes_to_the_server_during_a_scenario() {
    let pushes = Arc::new(Mutex::new(Vec::new()));
    let mut c = config("host-sync");
    c.sync = Box::new(CountingSync {
        pushes: pushes.clone(),
    });
    let host = Host::start(c).unwrap();
    let endpoint = host.endpoint().clone();
    // Several clients write; none of them has a sync runner, structurally:
    // the client type owns a socket and nothing else.
    let handles: Vec<_> = (0..4)
        .map(|i| {
            let endpoint = endpoint.clone();
            std::thread::spawn(move || {
                let mut client = Client::connect(&endpoint).unwrap();
                for k in 0..5 {
                    client.capture(&format!("write {i}-{k}"), None).unwrap();
                }
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }
    // The host's maintenance thread pushes on its own schedule; force one
    // pass so the test does not wait on it.
    let mut client = Client::connect(&endpoint).unwrap();
    let synced = client.debug(DebugRequest::SyncNow).unwrap();
    assert!(matches!(synced, Response::Synced { .. }));
    std::thread::sleep(Duration::from_millis(350));
    let recorded = pushes.lock().unwrap().clone();
    let processes: std::collections::BTreeSet<u32> = recorded.iter().map(|(p, _)| *p).collect();
    assert_eq!(processes.len(), 1, "one process pushed: {recorded:?}");
    assert_eq!(processes.into_iter().next(), Some(std::process::id()));
    let total: usize = recorded.iter().map(|(_, n)| n).sum();
    assert!(
        total >= 20,
        "every op was pushed exactly once: {recorded:?}"
    );
    assert_eq!(client.status().unwrap().pending_ops, 0);
    host.shutdown();
}
