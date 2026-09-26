//! The sync runner: the one place a device pushes and pulls (Section 6,
//! one sync runner per device). The host owns exactly one; clients own
//! none. The network client is not written yet, so the trait ships with a
//! no-op implementation and the host calls it on a schedule whenever the
//! space has pending ops.

use crate::crypto::{CryptoError, Keyring};
use crate::store::{Store, StoreError};

/// What one sync pass did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SyncReport {
    pub pushed: usize,
    pub pulled: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Crypto(#[from] CryptoError),
    #[error("sync: {0}")]
    Transport(String),
}

/// Pushes pending ops and pulls the tail for every space the device holds.
pub trait SyncRunner: Send {
    /// A short name for status output.
    fn name(&self) -> &str;

    /// Run one pass. Called from the host's maintenance thread with the
    /// store and keyring locked for its duration.
    fn sync(&mut self, store: &mut Store, keyring: &Keyring) -> Result<SyncReport, SyncError>;
}

/// Does nothing. The default until the network client exists.
#[derive(Debug, Default)]
pub struct NoopSync;

impl SyncRunner for NoopSync {
    fn name(&self) -> &str {
        "none"
    }

    fn sync(&mut self, _store: &mut Store, _keyring: &Keyring) -> Result<SyncReport, SyncError> {
        Ok(SyncReport::default())
    }
}
