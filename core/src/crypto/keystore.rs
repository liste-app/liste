//! Where keys rest between sessions. The core never writes a key to disk
//! itself; each platform implements [`KeyStore`] over its secure storage
//! (Keychain, Android Keystore, DPAPI, libsecret, WebCrypto), and only the
//! host process on desktop ever holds one.
//!
//! The store holds named secrets. The root key is stored raw, protected by
//! the platform. Everything else stored here is already wrapped, and would
//! be safe to store elsewhere; keeping it in the same place is simply
//! fewer moving parts.

use std::collections::BTreeMap;

use zeroize::Zeroizing;

/// Platform-secure storage for named secrets.
pub trait KeyStore {
    /// The secret stored under `name`, if any.
    fn get(&self, name: &str) -> Result<Option<Zeroizing<Vec<u8>>>, KeyStoreError>;
    /// Store or replace the secret under `name`.
    fn put(&mut self, name: &str, value: &[u8]) -> Result<(), KeyStoreError>;
    /// Remove the secret under `name`, if any.
    fn delete(&mut self, name: &str) -> Result<(), KeyStoreError>;
    /// Names starting with `prefix`, in any order.
    fn list(&self, prefix: &str) -> Result<Vec<String>, KeyStoreError>;
}

/// A platform keystore failure. Carries no secret material.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("keystore: {0}")]
pub struct KeyStoreError(pub String);

/// An in-memory keystore for tests and throwaway sessions. Values are
/// zeroized when dropped.
#[derive(Default)]
pub struct MemoryKeyStore {
    entries: BTreeMap<String, Zeroizing<Vec<u8>>>,
}

impl MemoryKeyStore {
    pub fn new() -> MemoryKeyStore {
        MemoryKeyStore::default()
    }
}

impl KeyStore for MemoryKeyStore {
    fn get(&self, name: &str) -> Result<Option<Zeroizing<Vec<u8>>>, KeyStoreError> {
        Ok(self.entries.get(name).cloned())
    }

    fn put(&mut self, name: &str, value: &[u8]) -> Result<(), KeyStoreError> {
        self.entries
            .insert(name.to_owned(), Zeroizing::new(value.to_vec()));
        Ok(())
    }

    fn delete(&mut self, name: &str) -> Result<(), KeyStoreError> {
        self.entries.remove(name);
        Ok(())
    }

    fn list(&self, prefix: &str) -> Result<Vec<String>, KeyStoreError> {
        Ok(self
            .entries
            .keys()
            .filter(|k| k.starts_with(prefix))
            .cloned()
            .collect())
    }
}
