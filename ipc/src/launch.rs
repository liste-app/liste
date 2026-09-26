//! Starting a host when none is running (Section 3, background launch).
//!
//! On macOS the installed desktop app is preferred, launched hidden and in
//! the background; otherwise the headless daemon next to the calling
//! binary. Windows is a stub until the Windows app exists.

use std::io;
use std::path::Path;
use std::process::{Command, Stdio};

/// The installed macOS app, if any: `LISTE_APP` (a path, for development
/// builds and tests), then `/Applications/Liste.app`, then
/// `~/Applications/Liste.app`.
pub fn macos_app() -> Option<std::path::PathBuf> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    let mut candidates = Vec::new();
    if let Some(p) = std::env::var_os("LISTE_APP") {
        candidates.push(std::path::PathBuf::from(p));
    }
    candidates.push(std::path::PathBuf::from("/Applications/Liste.app"));
    if let Some(home) = std::env::var_os("HOME") {
        candidates.push(std::path::PathBuf::from(home).join("Applications/Liste.app"));
    }
    candidates
        .into_iter()
        .find(|p| p.join("Contents/MacOS").is_dir())
}

/// Launch a host in the background: the desktop app if installed, else
/// `daemon_exe daemon`. The caller then polls for the socket.
pub fn launch_host(daemon_exe: &Path) -> io::Result<()> {
    if let Some(app) = macos_app() {
        // `open -g -j`: do not bring the app forward, launch hidden. Apps
        // launched this way do not inherit the environment, so the two
        // variables that relocate the store are forwarded explicitly.
        let mut cmd = Command::new("open");
        cmd.args(["-g", "-j"]);
        for var in ["LISTE_DATA_DIR", "LISTE_SOCKET"] {
            if let Ok(value) = std::env::var(var) {
                cmd.arg("--env").arg(format!("{var}={value}"));
            }
        }
        return cmd
            .arg("-a")
            .arg(&app)
            .args(["--args", "--background"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map(|_| ());
    }
    if cfg!(windows) {
        return Err(io::Error::other(
            "auto-start is not implemented on Windows yet",
        ));
    }
    Command::new(daemon_exe)
        .arg("daemon")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
}

/// The `liste` binary that ships beside the calling binary, or on `PATH`.
pub fn sibling_liste() -> std::path::PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|exe| {
            exe.parent()
                .map(|d| d.join(if cfg!(windows) { "liste.exe" } else { "liste" }))
        })
        .filter(|p| p.exists())
        .unwrap_or_else(|| std::path::PathBuf::from("liste"))
}
