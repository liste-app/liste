//! Where the host listens (Section 3).
//!
//! macOS direct build: `~/Library/Application Support/Liste/host.sock`.
//! Linux: `$XDG_RUNTIME_DIR/liste/host.sock`. Windows:
//! `\\.\pipe\liste-<user-sid>`. Clients check the known endpoints in order.
//! `LISTE_SOCKET` overrides the list with one endpoint, for tests and for
//! running several hosts on one machine.

use std::path::{Path, PathBuf};

/// A place the host listens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Endpoint {
    /// A Unix domain socket at this path.
    Path(PathBuf),
    /// A named pipe in the local namespace (`\\.\pipe\<name>` on Windows).
    Name(String),
}

impl Endpoint {
    /// The endpoints a client tries, in order.
    pub fn known() -> Vec<Endpoint> {
        if let Some(path) = std::env::var_os("LISTE_SOCKET") {
            let path = PathBuf::from(path);
            return vec![Endpoint::from_path(&path)];
        }
        let mut out = Vec::new();
        if cfg!(target_os = "macos") {
            if let Some(home) = home() {
                out.push(Endpoint::Path(
                    home.join("Library/Application Support/Liste/host.sock"),
                ));
            }
        } else if cfg!(windows) {
            out.push(Endpoint::Name(format!("liste-{}", user_sid())));
        } else {
            if let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR") {
                out.push(Endpoint::Path(
                    PathBuf::from(runtime).join("liste/host.sock"),
                ));
            }
            if let Some(home) = home() {
                out.push(Endpoint::Path(home.join(".local/share/liste/host.sock")));
            }
        }
        out
    }

    /// The endpoint a host creates by default: the first known one.
    pub fn default_for_host() -> Option<Endpoint> {
        Endpoint::known().into_iter().next()
    }

    /// An endpoint from a filesystem path; on Windows a path is used as a
    /// pipe name.
    pub fn from_path(path: &Path) -> Endpoint {
        if cfg!(windows) {
            Endpoint::Name(
                path.file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "liste".into()),
            )
        } else {
            Endpoint::Path(path.to_path_buf())
        }
    }

    /// The directory that must exist for a path endpoint.
    pub fn parent_dir(&self) -> Option<&Path> {
        match self {
            Endpoint::Path(p) => p.parent(),
            Endpoint::Name(_) => None,
        }
    }
}

impl std::fmt::Display for Endpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Endpoint::Path(p) => write!(f, "{}", p.display()),
            Endpoint::Name(n) => write!(f, "\\\\.\\pipe\\{n}"),
        }
    }
}

/// The default data directory (Section 3): the same folder the socket
/// lives in on macOS, `$XDG_DATA_HOME/liste` on Linux, `%LOCALAPPDATA%\Liste`
/// on Windows. `LISTE_DATA_DIR` overrides it.
pub fn default_data_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("LISTE_DATA_DIR") {
        return Some(PathBuf::from(dir));
    }
    if cfg!(target_os = "macos") {
        home().map(|h| h.join("Library/Application Support/Liste"))
    } else if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(|d| PathBuf::from(d).join("Liste"))
    } else if let Some(data) = std::env::var_os("XDG_DATA_HOME") {
        Some(PathBuf::from(data).join("liste"))
    } else {
        home().map(|h| h.join(".local/share/liste"))
    }
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

/// The current user's SID on Windows. A placeholder until the Windows app
/// exists (Phase 2): the user name keeps pipes per user in the meantime.
fn user_sid() -> String {
    std::env::var("USERNAME").unwrap_or_else(|_| "user".into())
}
