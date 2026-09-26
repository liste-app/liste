//! End-to-end encryption (Section 7).
//!
//! # Threat model, in plain language
//!
//! **What the server sees.** Ciphertext for every op and snapshot, plus
//! routing metadata: account and device identifiers, space ids, sequence
//! numbers, ciphertext sizes, and timestamps. It also holds the user's
//! public keys and wrapped private material it cannot open. It never sees
//! a title, a note, a tag name, a due date, or any key. Someone who takes
//! the whole database gets exactly that and nothing more.
//!
//! **A stolen session token** proves identity to the server. With it an
//! attacker can pull the same ciphertext the device could and push
//! garbage ops. They cannot decrypt anything, because no key is derived
//! from the token or from any password; there are no passwords.
//!
//! **A stolen device with a locked keystore** (powered off, or the platform
//! keystore not yet released its secrets) yields the local database: the
//! materialized tables are plaintext at rest and rely on the operating
//! system's disk encryption, the same posture the major password managers
//! take for their local caches (Section 10). Without the root key from the
//! keystore, no new material can be decrypted and the device cannot be
//! used to approve another one.
//!
//! **A stolen device with an unlocked keystore**, or any software the
//! person runs on their own device with access to the host, sees
//! plaintext. That includes an agent connected to the local MCP server.
//! End-to-end encryption is a promise about the server, not about the
//! device (Section 7).
//!
//! **Losing every device and the recovery key** loses the data. There is
//! no reset, by design.
//!
//! # Shape
//!
//! - [`Keyring`] is the client's one handle: unlock and lock, encrypt and
//!   decrypt ops and snapshots, create spaces, wrap a space key for another
//!   member, and the two onboarding paths.
//! - [`KeyStore`] is what each platform implements over its secure storage.
//! - Every key is a distinct type in [`keys`], zeroized on drop and never
//!   printable.
//! - The primitives are XChaCha20-Poly1305, X25519, Ed25519, and
//!   HKDF-SHA-256 from the audited RustCrypto and dalek crates, pinned
//!   exactly; nothing is implemented by hand.
//!
//! Interpretations of the document: the recovery key is rendered as
//! Crockford base32 with a checksum; ops and snapshots use distinct
//! subkeys of the space key as domain separation; the sealed-box wrap
//! binds the space id and both public keys; and the approval grant is
//! signed with the user's Ed25519 key so the new device can check it
//! against the published public key.

pub mod envelope;
pub mod keyring;
pub mod keys;
pub mod keystore;
pub mod onboarding;
pub mod primitives;
pub mod recovery;
pub mod wrap;

pub use envelope::{EncryptedOp, EncryptedSnapshot, OVERHEAD, op_aad, snapshot_aad};
pub use keyring::{AccountMaterial, Keyring};
pub use keys::{RecoveryKey, RootKey, SpaceKey, UserKeyPair, UserPublicKeys};
pub use keystore::{KeyStore, KeyStoreError, MemoryKeyStore};
pub use onboarding::{ApprovalGrant, ApprovalRequest, NewDeviceApproval, ShortCode};
pub use recovery::RecoveryKeyError;
pub use wrap::{Wrapped, WrappedSpaceKey};

/// Errors from the encryption layer. None carries key material or
/// plaintext; a decryption failure says only that it failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CryptoError {
    #[error("keys are locked")]
    Locked,
    #[error("this device has no account keys yet")]
    NotInitialized,
    #[error("this device already holds account keys")]
    AlreadyInitialized,
    #[error("no key for space {0}")]
    UnknownSpace(crate::ids::Id),
    #[error("decryption failed")]
    Decrypt,
    #[error("encryption failed")]
    Encrypt,
    #[error("signature verification failed")]
    Signature,
    #[error("key was wrapped for a different user")]
    NotForThisUser,
    #[error("stored key material is corrupt")]
    Corrupt,
    #[error("the system random source is unavailable")]
    Randomness,
    #[error(transparent)]
    RecoveryKey(RecoveryKeyError),
    #[error(transparent)]
    KeyStore(#[from] KeyStoreError),
}

impl RecoveryKey {
    /// The key as fourteen groups of four characters for a person to save.
    /// Zeroized when dropped.
    pub fn render(&self) -> zeroize::Zeroizing<String> {
        recovery::render(self)
    }

    /// Parse what a person typed, rejecting any single mistyped character.
    pub fn parse(entered: &str) -> Result<RecoveryKey, RecoveryKeyError> {
        recovery::parse(entered)
    }
}
