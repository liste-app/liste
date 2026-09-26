//! Host and IPC (Section 15), the process-level half: `liste daemon` as a
//! real second process, auto-start with the readiness handshake, a second
//! daemon refused, and a crashed daemon leaving a lock the next one can take.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

fn liste() -> Command {
    Command::new(env!("CARGO_BIN_EXE_liste"))
}

/// A short directory: Unix socket paths are limited to about 100 bytes,
/// and macOS's default temp dir alone is longer than half of that.
fn temp_dir(label: &str) -> PathBuf {
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
    let dir = base.join(format!("liste-t-{label}-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The environment that points every `liste` at one directory and socket.
fn env<'a>(cmd: &'a mut Command, dir: &Path) -> &'a mut Command {
    cmd.env("LISTE_DATA_DIR", dir)
        .env("LISTE_SOCKET", dir.join("host.sock"))
}

fn run(dir: &Path, args: &[&str]) -> (i32, String, String) {
    let out = env(&mut liste(), dir)
        .args(args)
        .output()
        .expect("run liste");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn spawn_daemon(dir: &Path, extra: &[&str]) -> Child {
    env(&mut liste(), dir)
        .arg("daemon")
        .args(extra)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn daemon")
}

fn wait_for_socket(dir: &Path) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while !dir.join("host.sock").exists() {
        assert!(
            Instant::now() < deadline,
            "daemon did not create its socket"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn a_second_daemon_on_the_same_store_is_refused() {
    let dir = temp_dir("second");
    let mut first = spawn_daemon(&dir, &[]);
    wait_for_socket(&dir);
    let second = spawn_daemon(&dir, &[]).wait_with_output().unwrap();
    assert_ne!(second.status.code(), Some(0));
    let err = String::from_utf8_lossy(&second.stderr);
    assert!(err.contains("held by another Liste process"), "{err}");
    // The first is untouched and still serves.
    let (code, out, _) = run(&dir, &["status"]);
    assert_eq!(code, 0, "{out}");
    let _ = first.kill();
    let _ = first.wait();
}

#[test]
fn capture_auto_starts_a_daemon_within_the_readiness_timeout() {
    let dir = temp_dir("autostart");
    assert!(!dir.join("host.sock").exists());
    let start = Instant::now();
    let (code, out, err) = run(&dir, &["capture", "call mom tomorrow 5pm #family !high"]);
    assert_eq!(code, 0, "stdout: {out}\nstderr: {err}");
    assert!(
        start.elapsed() < Duration::from_secs(3),
        "readiness handshake"
    );
    assert!(out.contains("call mom"), "{out}");
    assert!(out.contains("family"), "{out}");
    assert!(dir.join("host.sock").exists());
    // The daemon it started keeps serving the next command.
    let (code, out, _) = run(&dir, &["upcoming", "--days", "3", "--json"]);
    assert_eq!(code, 0);
    assert!(out.contains("\"title\":\"call mom\""), "{out}");
    let (code, out, _) = run(&dir, &["capture", "pay rent today"]);
    assert_eq!(code, 0, "{out}");
    let (code, out, _) = run(&dir, &["today"]);
    assert_eq!(code, 0);
    assert!(
        out.contains("pay rent") && !out.contains("call mom"),
        "{out}"
    );
    let (code, out, _) = run(&dir, &["--json", "status"]);
    assert_eq!(code, 0);
    let status: serde_json::Value = serde_json::from_str(out.trim()).unwrap();
    assert_eq!(status["locked"], false);
    // Stop it: the daemon is our child of a child; find it through the lock.
    kill_daemon(&dir);
}

/// Stop whichever daemon holds this directory, by asking it over the lock's
/// owner: we do not know its pid, so we take the blunt route through pkill
/// on the exact socket argument is unavailable; instead spawn nothing and
/// let the test's temp dir be abandoned. Daemons started by auto-start exit
/// when the socket file is removed and their next accept fails? No: they
/// keep running. So we use the process list.
fn kill_daemon(dir: &Path) {
    #[cfg(unix)]
    {
        let sock = dir.join("host.sock");
        let _ = Command::new("pkill")
            .args(["-f", &format!("LISTE_SOCKET={}", sock.display())])
            .status();
        // Auto-started daemons inherit the environment, not the argument;
        // match on the data dir passed through the environment via `ps`.
        let out = Command::new("sh")
            .arg("-c")
            .arg(format!(
                "for p in $(pgrep -f 'liste daemon'); do if ps eww -p $p 2>/dev/null | grep -q 'LISTE_DATA_DIR={}'; then kill $p; fi; done",
                dir.display()
            ))
            .output();
        let _ = out;
    }
    let deadline = Instant::now() + Duration::from_secs(3);
    while dir.join("host.sock").exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn exit_codes_locked_not_running_and_did_not_start() {
    let dir = temp_dir("codes");
    // No host and auto-start pointed at a directory that cannot be created.
    let bad = dir.join("nope").join("deeper");
    std::fs::write(dir.join("nope"), b"a file, not a directory").unwrap();
    let out = env(&mut liste(), &bad).args(["today"]).output().unwrap();
    assert_eq!(
        out.status.code(),
        Some(3),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stderr).contains("did not start"));
    // A locked daemon answers status and exits 2 on data commands.
    let mut daemon = spawn_daemon(&dir, &["--locked"]);
    wait_for_socket(&dir);
    let (code, out, _) = run(&dir, &["status"]);
    assert_eq!(code, 0);
    assert!(out.contains("locked"), "{out}");
    let (code, _, err) = run(&dir, &["capture", "x"]);
    assert_eq!(code, 2, "{err}");
    assert!(err.contains("locked"), "{err}");
    let (code, _, _) = run(&dir, &["--json", "search", "x"]);
    assert_eq!(code, 2);
    let _ = daemon.kill();
    let _ = daemon.wait();
}

#[cfg(unix)]
#[test]
fn a_crashed_daemon_leaves_a_lock_the_next_one_can_take() {
    let dir = temp_dir("crash");
    let mut daemon = spawn_daemon(&dir, &[]);
    wait_for_socket(&dir);
    let (code, _, _) = run(&dir, &["capture", "before the crash"]);
    assert_eq!(code, 0);
    // SIGKILL: no cleanup runs; the socket file and lock file remain.
    daemon.kill().unwrap();
    daemon.wait().unwrap();
    assert!(dir.join("host.sock").exists(), "corpse socket left behind");
    assert!(dir.join("host.lock").exists());
    // The next daemon takes the lock and replaces the corpse socket.
    let mut next = spawn_daemon(&dir, &[]);
    let deadline = Instant::now() + Duration::from_secs(3);
    let ok = loop {
        let (code, out, _) = run(&dir, &["search", "crash"]);
        if code == 0 && out.contains("before the crash") {
            break true;
        }
        if Instant::now() > deadline {
            break false;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    assert!(ok, "the next daemon serves the same store");
    let _ = next.kill();
    let _ = next.wait();
}

#[cfg(unix)]
#[test]
fn a_terminated_daemon_shuts_down_cleanly() {
    let dir = temp_dir("term");
    let daemon = spawn_daemon(&dir, &[]);
    wait_for_socket(&dir);
    let (code, _, _) = run(&dir, &["capture", "before shutdown"]);
    assert_eq!(code, 0);
    // SIGTERM: the handler stops the host, which removes its socket and
    // releases the lock.
    let status = Command::new("kill")
        .args(["-TERM", &daemon.id().to_string()])
        .status()
        .unwrap();
    assert!(status.success());
    let out = daemon.wait_with_output().unwrap();
    assert!(out.status.success(), "clean exit: {:?}", out.status);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("shutting down"), "{err}");
    assert!(!dir.join("host.sock").exists(), "socket removed");
    // The next daemon starts at once and still has the data.
    let mut next = spawn_daemon(&dir, &[]);
    wait_for_socket(&dir);
    let (code, out, _) = run(&dir, &["search", "shutdown"]);
    assert_eq!(code, 0);
    assert!(out.contains("before shutdown"), "{out}");
    let _ = next.kill();
    let _ = next.wait();
}
