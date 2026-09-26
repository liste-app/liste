//! Exclusive ownership of a store directory (Section 3: exactly one
//! process per device owns the local store).
//!
//! The lock is an OS file lock (`flock` on Unix, `LockFileEx` on Windows)
//! on `host.lock` inside the directory, taken before SQLite is opened. The
//! kernel releases it when the holding process exits for any reason, so a
//! crashed host never leaves a stale lock behind, and a live host cannot be
//! displaced.

use std::fs::{File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};

use super::HostError;

/// Held for as long as this process owns the store.
pub struct StoreLock {
    _file: File,
    path: PathBuf,
}

impl std::fmt::Debug for StoreLock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "StoreLock({})", self.path.display())
    }
}

impl StoreLock {
    /// Take the lock, or fail at once if another process holds it.
    pub fn acquire(dir: &Path) -> Result<StoreLock, HostError> {
        std::fs::create_dir_all(dir).map_err(|e| HostError::Io(dir.to_path_buf(), e))?;
        let path = dir.join("host.lock");
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(|e| HostError::Io(path.clone(), e))?;
        match file.try_lock() {
            Ok(()) => Ok(StoreLock { _file: file, path }),
            Err(std::fs::TryLockError::WouldBlock) => Err(HostError::StoreHeld(dir.to_path_buf())),
            Err(std::fs::TryLockError::Error(e)) => Err(HostError::Io(path, e)),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for StoreLock {
    fn drop(&mut self) {
        // Closing the file releases the lock; an explicit unlock makes the
        // intent visible and covers platforms that need it.
        let _ = self._file.unlock();
    }
}

#[allow(dead_code)]
fn _io_error_type_check(_: io::Error) {}
