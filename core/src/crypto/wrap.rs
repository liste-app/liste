//! The key hierarchy (Section 7): how each key is wrapped under another.
//!
//! - User private keys are wrapped under the root key.
//! - The root key is wrapped under the recovery key.
//! - A space key is wrapped for each member under that member's X25519
//!   public key, sealed-box style: an ephemeral X25519 key agrees with the
//!   recipient's key, HKDF turns the shared secret into an AEAD key, and
//!   the ephemeral public key rides along. Adding a member later is one
//!   more such wrap.
//!
//! Every wrap binds a purpose label into the derived key and the
//! associated data, so a blob wrapped for one purpose cannot be presented
//! as another.

use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, Zeroizing};

use super::CryptoError;
use super::keys::{RecoveryKey, RootKey, SpaceKey, UserKeyPair, UserPublicKeys};
use super::primitives::{X25519Secret, aead_open, aead_seal, derive_key};
use crate::ids::Id;

const USER_KEYS_LABEL: &[u8] = b"liste/wrap/user-keys/v1";
const RECOVERY_LABEL: &[u8] = b"liste/wrap/root-under-recovery/v1";
const SPACE_KEY_LABEL: &[u8] = b"liste/wrap/space-key/v1";

/// Something wrapped under a symmetric key. Safe to store or publish.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Wrapped {
    pub label: String,
    pub sealed: Vec<u8>,
}

impl std::fmt::Debug for Wrapped {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Wrapped({}, {} bytes)", self.label, self.sealed.len())
    }
}

fn wrap(key: &[u8; 32], label: &[u8], plaintext: &[u8]) -> Result<Wrapped, CryptoError> {
    let k = derive_key(key, label);
    Ok(Wrapped {
        label: String::from_utf8_lossy(label).into_owned(),
        sealed: aead_seal(&k, label, plaintext)?,
    })
}

fn unwrap(
    key: &[u8; 32],
    label: &[u8],
    wrapped: &Wrapped,
) -> Result<Zeroizing<Vec<u8>>, CryptoError> {
    if wrapped.label.as_bytes() != label {
        return Err(CryptoError::Decrypt);
    }
    let k = derive_key(key, label);
    aead_open(&k, label, &wrapped.sealed)
}

/// Wrap the user's private keys under the root key.
pub fn wrap_user_keys(root: &RootKey, user: &UserKeyPair) -> Result<Wrapped, CryptoError> {
    wrap(root.as_bytes(), USER_KEYS_LABEL, user.to_bytes().as_ref())
}

pub fn unwrap_user_keys(root: &RootKey, wrapped: &Wrapped) -> Result<UserKeyPair, CryptoError> {
    let bytes = unwrap(root.as_bytes(), USER_KEYS_LABEL, wrapped)?;
    UserKeyPair::from_bytes(&bytes)
}

/// Wrap the root key under the recovery key.
pub fn wrap_root_under_recovery(
    recovery: &RecoveryKey,
    root: &RootKey,
) -> Result<Wrapped, CryptoError> {
    wrap(recovery.as_bytes(), RECOVERY_LABEL, root.as_bytes())
}

pub fn unwrap_root_with_recovery(
    recovery: &RecoveryKey,
    wrapped: &Wrapped,
) -> Result<RootKey, CryptoError> {
    let bytes = unwrap(recovery.as_bytes(), RECOVERY_LABEL, wrapped)?;
    let mut arr: [u8; 32] = bytes
        .as_slice()
        .try_into()
        .map_err(|_| CryptoError::Decrypt)?;
    let key = RootKey::from_bytes(arr);
    arr.zeroize();
    Ok(key)
}

/// A space key wrapped for one member. This is what `space_members.
/// wrapped_space_key` holds on the server.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WrappedSpaceKey {
    pub space_id: Id,
    /// The member's X25519 public key this was wrapped for.
    pub recipient: [u8; 32],
    /// The ephemeral X25519 public key used for this wrap.
    pub ephemeral: [u8; 32],
    pub sealed: Vec<u8>,
}

impl std::fmt::Debug for WrappedSpaceKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "WrappedSpaceKey({}, {} bytes)",
            self.space_id,
            self.sealed.len()
        )
    }
}

fn sealed_box_aad(space_id: Id, recipient: &[u8; 32], ephemeral: &[u8; 32]) -> Vec<u8> {
    let mut aad = Vec::with_capacity(SPACE_KEY_LABEL.len() + 16 + 64);
    aad.extend_from_slice(SPACE_KEY_LABEL);
    aad.extend_from_slice(space_id.as_bytes());
    aad.extend_from_slice(recipient);
    aad.extend_from_slice(ephemeral);
    aad
}

fn sealed_box_key(
    shared: &[u8; 32],
    recipient: &[u8; 32],
    ephemeral: &[u8; 32],
) -> Zeroizing<[u8; 32]> {
    let mut info = Vec::with_capacity(SPACE_KEY_LABEL.len() + 64);
    info.extend_from_slice(SPACE_KEY_LABEL);
    info.extend_from_slice(ephemeral);
    info.extend_from_slice(recipient);
    derive_key(shared, &info)
}

/// Wrap `space_key` for the holder of `recipient`'s X25519 public key.
pub fn wrap_space_key_for(
    space_id: Id,
    space_key: &SpaceKey,
    recipient: &UserPublicKeys,
) -> Result<WrappedSpaceKey, CryptoError> {
    let eph = X25519Secret::generate()?;
    let ephemeral = eph.public();
    let shared = eph.agree(&recipient.encryption);
    let key = sealed_box_key(&shared, &recipient.encryption, &ephemeral);
    let aad = sealed_box_aad(space_id, &recipient.encryption, &ephemeral);
    Ok(WrappedSpaceKey {
        space_id,
        recipient: recipient.encryption,
        ephemeral,
        sealed: aead_seal(&key, &aad, space_key.as_bytes())?,
    })
}

/// Unwrap a space key wrapped for `user`.
pub fn unwrap_space_key(
    user: &UserKeyPair,
    wrapped: &WrappedSpaceKey,
) -> Result<SpaceKey, CryptoError> {
    let ours = user.encryption.public();
    if !super::primitives::ct_eq(&ours, &wrapped.recipient) {
        return Err(CryptoError::NotForThisUser);
    }
    let shared = user.encryption.agree(&wrapped.ephemeral);
    let key = sealed_box_key(&shared, &wrapped.recipient, &wrapped.ephemeral);
    let aad = sealed_box_aad(wrapped.space_id, &wrapped.recipient, &wrapped.ephemeral);
    let bytes = aead_open(&key, &aad, &wrapped.sealed)?;
    let mut arr: [u8; 32] = bytes
        .as_slice()
        .try_into()
        .map_err(|_| CryptoError::Decrypt)?;
    let key = SpaceKey::from_bytes(arr);
    arr.zeroize();
    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_keys_round_trip_under_root_and_fail_under_another() {
        let root = RootKey::generate().unwrap();
        let user = UserKeyPair::generate().unwrap();
        let w = wrap_user_keys(&root, &user).unwrap();
        assert_eq!(
            unwrap_user_keys(&root, &w).unwrap().public_keys(),
            user.public_keys()
        );
        let other = RootKey::generate().unwrap();
        assert!(matches!(
            unwrap_user_keys(&other, &w),
            Err(CryptoError::Decrypt)
        ));
        // A blob wrapped for one purpose is not accepted for another.
        let rec = RecoveryKey::from_bytes(*root.as_bytes());
        assert!(unwrap_root_with_recovery(&rec, &w).is_err());
    }

    #[test]
    fn root_round_trips_under_recovery() {
        let root = RootKey::generate().unwrap();
        let recovery = RecoveryKey::generate().unwrap();
        let w = wrap_root_under_recovery(&recovery, &root).unwrap();
        assert_eq!(
            unwrap_root_with_recovery(&recovery, &w).unwrap().as_bytes(),
            root.as_bytes()
        );
        let mut tampered = w.clone();
        tampered.sealed[30] ^= 0x80;
        assert!(unwrap_root_with_recovery(&recovery, &tampered).is_err());
    }

    #[test]
    fn space_key_wraps_for_a_member_and_only_that_member() {
        let alice = UserKeyPair::generate().unwrap();
        let bob = UserKeyPair::generate().unwrap();
        let space = Id::new();
        let key = SpaceKey::generate().unwrap();
        let for_bob = wrap_space_key_for(space, &key, &bob.public_keys()).unwrap();
        assert_eq!(
            unwrap_space_key(&bob, &for_bob).unwrap().as_bytes(),
            key.as_bytes()
        );
        assert!(matches!(
            unwrap_space_key(&alice, &for_bob),
            Err(CryptoError::NotForThisUser)
        ));
        let mut wrong_space = for_bob.clone();
        wrong_space.space_id = Id::new();
        assert!(matches!(
            unwrap_space_key(&bob, &wrong_space),
            Err(CryptoError::Decrypt)
        ));
        let json = serde_json::to_string(&for_bob).unwrap();
        let back: WrappedSpaceKey = serde_json::from_str(&json).unwrap();
        assert_eq!(back, for_bob);
    }
}
