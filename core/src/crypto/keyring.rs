//! The keyring: the one object a client holds for everything encryption.
//! It owns a [`KeyStore`], is locked until [`Keyring::unlock`] succeeds,
//! and while unlocked holds the root key, the user's private keys, and
//! every space key in memory. [`Keyring::lock`] drops them all.

use std::collections::BTreeMap;

use zeroize::Zeroize;

use super::envelope::{self, EncryptedOp, EncryptedSnapshot};
use super::keys::{RecoveryKey, RootKey, SpaceKey, UserKeyPair, UserPublicKeys};
use super::keystore::KeyStore;
use super::wrap::{self, Wrapped, WrappedSpaceKey};
use super::{CryptoError, recovery};
use crate::ids::Id;
use crate::op::Op;
use crate::store::Snapshot;

const ROOT_NAME: &str = "root";
const USER_KEYS_NAME: &str = "user_keys";
const SPACE_PREFIX: &str = "space_key:";

/// What the server holds for an account so any approved device can
/// finish unlocking. None of it is secret.
#[derive(Clone, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
pub struct AccountMaterial {
    pub public_keys: UserPublicKeys,
    /// The user's private keys, wrapped under the root key.
    pub wrapped_user_keys: Wrapped,
    /// The root key, wrapped under the recovery key.
    pub root_wrapped_under_recovery: Wrapped,
}

struct Unlocked {
    root: RootKey,
    user: UserKeyPair,
    spaces: BTreeMap<Id, SpaceKey>,
}

/// See the module documentation.
pub struct Keyring {
    store: Box<dyn KeyStore>,
    unlocked: Option<Unlocked>,
}

impl std::fmt::Debug for Keyring {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Keyring({})",
            if self.unlocked.is_some() {
                "unlocked"
            } else {
                "locked"
            }
        )
    }
}

impl Keyring {
    /// A locked keyring over the platform's keystore.
    pub fn new(store: Box<dyn KeyStore>) -> Keyring {
        Keyring {
            store,
            unlocked: None,
        }
    }

    /// Whether keys are loaded.
    pub fn is_unlocked(&self) -> bool {
        self.unlocked.is_some()
    }

    /// Whether this device already holds an account's root key.
    pub fn is_initialized(&self) -> Result<bool, CryptoError> {
        Ok(self.store.get(ROOT_NAME)?.is_some())
    }

    fn unlocked(&self) -> Result<&Unlocked, CryptoError> {
        self.unlocked.as_ref().ok_or(CryptoError::Locked)
    }

    fn unlocked_mut(&mut self) -> Result<&mut Unlocked, CryptoError> {
        self.unlocked.as_mut().ok_or(CryptoError::Locked)
    }

    /// First device of a new account: generate the root key, the user's
    /// keypair, and the recovery key; store what this device keeps; return
    /// the recovery key to show once and the material to publish.
    pub fn initialize(&mut self) -> Result<(RecoveryKey, AccountMaterial), CryptoError> {
        if self.is_initialized()? {
            return Err(CryptoError::AlreadyInitialized);
        }
        let root = RootKey::generate()?;
        let user = UserKeyPair::generate()?;
        let recovery = RecoveryKey::generate()?;
        let material = AccountMaterial {
            public_keys: user.public_keys(),
            wrapped_user_keys: wrap::wrap_user_keys(&root, &user)?,
            root_wrapped_under_recovery: wrap::wrap_root_under_recovery(&recovery, &root)?,
        };
        self.store.put(ROOT_NAME, root.as_bytes())?;
        self.store
            .put(USER_KEYS_NAME, &encode(&material.wrapped_user_keys))?;
        self.unlocked = Some(Unlocked {
            root,
            user,
            spaces: BTreeMap::new(),
        });
        Ok((recovery, material))
    }

    /// Load keys from the keystore. Fails with [`CryptoError::NotInitialized`]
    /// if this device has never received the root key.
    pub fn unlock(&mut self) -> Result<(), CryptoError> {
        if self.unlocked.is_some() {
            return Ok(());
        }
        let root_bytes = self
            .store
            .get(ROOT_NAME)?
            .ok_or(CryptoError::NotInitialized)?;
        let mut arr: [u8; 32] = root_bytes
            .as_slice()
            .try_into()
            .map_err(|_| CryptoError::Corrupt)?;
        let root = RootKey::from_bytes(arr);
        arr.zeroize();
        let wrapped: Wrapped = self
            .store
            .get(USER_KEYS_NAME)?
            .map(|b| decode(&b))
            .transpose()?
            .ok_or(CryptoError::NotInitialized)?;
        let user = wrap::unwrap_user_keys(&root, &wrapped)?;
        let mut spaces = BTreeMap::new();
        for name in self.store.list(SPACE_PREFIX)? {
            if let Some(bytes) = self.store.get(&name)? {
                let w: WrappedSpaceKey = decode(&bytes)?;
                let key = wrap::unwrap_space_key(&user, &w)?;
                spaces.insert(w.space_id, key);
            }
        }
        self.unlocked = Some(Unlocked { root, user, spaces });
        Ok(())
    }

    /// Drop every key from memory. The keystore is untouched, so
    /// [`unlock`](Self::unlock) restores them.
    pub fn lock(&mut self) {
        self.unlocked = None;
    }

    /// Adopt a root key that arrived through device approval or recovery:
    /// check that it unwraps the account's user keys and that those match
    /// the published public keys, then keep it and unlock.
    pub fn adopt_root(
        &mut self,
        root: RootKey,
        material: &AccountMaterial,
    ) -> Result<(), CryptoError> {
        if self.is_initialized()? {
            return Err(CryptoError::AlreadyInitialized);
        }
        let user = wrap::unwrap_user_keys(&root, &material.wrapped_user_keys)?;
        if user.public_keys() != material.public_keys {
            return Err(CryptoError::Decrypt);
        }
        self.store.put(ROOT_NAME, root.as_bytes())?;
        self.store
            .put(USER_KEYS_NAME, &encode(&material.wrapped_user_keys))?;
        self.unlocked = Some(Unlocked {
            root,
            user,
            spaces: BTreeMap::new(),
        });
        Ok(())
    }

    /// Recovery-key entry on a device with no keys: parse what the person
    /// typed, unwrap the root key, and adopt it.
    pub fn unlock_with_recovery(
        &mut self,
        entered: &str,
        material: &AccountMaterial,
    ) -> Result<(), CryptoError> {
        let recovery = recovery::parse(entered)?;
        let root =
            wrap::unwrap_root_with_recovery(&recovery, &material.root_wrapped_under_recovery)?;
        self.adopt_root(root, material)
    }

    /// The user's public keys.
    pub fn public_keys(&self) -> Result<UserPublicKeys, CryptoError> {
        Ok(self.unlocked()?.user.public_keys())
    }

    /// Create the key for a new space, wrapped for this user, and keep it.
    /// The returned wrap is what the server stores for this member.
    pub fn create_space(&mut self, space_id: Id) -> Result<WrappedSpaceKey, CryptoError> {
        let key = SpaceKey::generate()?;
        let wrapped = {
            let u = self.unlocked()?;
            wrap::wrap_space_key_for(space_id, &key, &u.user.public_keys())?
        };
        self.store
            .put(&format!("{SPACE_PREFIX}{space_id}"), &encode(&wrapped))?;
        self.unlocked_mut()?.spaces.insert(space_id, key);
        Ok(wrapped)
    }

    /// Wrap a space key this user holds for another member. Adding a
    /// collaborator later is exactly this call.
    pub fn wrap_space_key_for(
        &self,
        space_id: Id,
        member: &UserPublicKeys,
    ) -> Result<WrappedSpaceKey, CryptoError> {
        let key = self.space_key(space_id)?;
        wrap::wrap_space_key_for(space_id, key, member)
    }

    /// Take in a space key the server delivered for this user.
    pub fn add_space_key(&mut self, wrapped: &WrappedSpaceKey) -> Result<(), CryptoError> {
        let key = wrap::unwrap_space_key(&self.unlocked()?.user, wrapped)?;
        self.store.put(
            &format!("{SPACE_PREFIX}{}", wrapped.space_id),
            &encode(wrapped),
        )?;
        self.unlocked_mut()?.spaces.insert(wrapped.space_id, key);
        Ok(())
    }

    /// Spaces this keyring can decrypt.
    pub fn spaces(&self) -> Result<Vec<Id>, CryptoError> {
        Ok(self.unlocked()?.spaces.keys().copied().collect())
    }

    fn space_key(&self, space_id: Id) -> Result<&SpaceKey, CryptoError> {
        self.unlocked()?
            .spaces
            .get(&space_id)
            .ok_or(CryptoError::UnknownSpace(space_id))
    }

    pub fn encrypt_op(&self, op: &Op) -> Result<EncryptedOp, CryptoError> {
        envelope::encrypt_op(self.space_key(op.space_id)?, op)
    }

    pub fn decrypt_op(&self, encrypted: &EncryptedOp) -> Result<Op, CryptoError> {
        envelope::decrypt_op(self.space_key(encrypted.space_id)?, encrypted)
    }

    pub fn encrypt_snapshot(&self, snapshot: &Snapshot) -> Result<EncryptedSnapshot, CryptoError> {
        envelope::encrypt_snapshot(self.space_key(snapshot.space_id)?, snapshot)
    }

    pub fn decrypt_snapshot(&self, encrypted: &EncryptedSnapshot) -> Result<Snapshot, CryptoError> {
        envelope::decrypt_snapshot(self.space_key(encrypted.space_id)?, encrypted)
    }

    pub(super) fn root(&self) -> Result<&RootKey, CryptoError> {
        Ok(&self.unlocked()?.root)
    }

    pub(super) fn user(&self) -> Result<&UserKeyPair, CryptoError> {
        Ok(&self.unlocked()?.user)
    }
}

fn encode<T: serde::Serialize>(value: &T) -> Vec<u8> {
    serde_json::to_vec(value).expect("wrapped key serialization cannot fail")
}

fn decode<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T, CryptoError> {
    serde_json::from_slice(bytes).map_err(|_| CryptoError::Corrupt)
}
