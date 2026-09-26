//! Starting a host when none is running (Section 3, background launch).
//!
//! On macOS the installed desktop app is preferred, launched hidden and in
//! the background; otherwise the headless daemon next to the calling
//! binary. Windows is a stub until the Windows app exists.

use std::io;
use std::path::Path;
use std::process::{Command, Stdio};

/// The installed macOS app, if any.
pub fn macos_app() -> Option<&'static Path> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    let app = Path::new("/Applications/Liste.app");
    app.exists().then_some(app)
}

/// Launch a host in the background: the desktop app if installed, else
/// `daemon_exe daemon`. The caller then polls for the socket.
pub fn launch_host(daemon_exe: &Path) -> io::Result<()> {
    if macos_app().is_some() {
        return Command::new("open")
            .args(["-g", "-j", "-a", "Liste", "--args", "--background"])
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
