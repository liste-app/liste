//! Typed keys. Each kind is its own type so a root key cannot be passed
//! where a space key belongs, every secret is zeroized when dropped, and
//! none of them can be printed.

use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

use super::CryptoError;
use super::primitives::{Ed25519Secret, KEY_LEN, X25519Secret, random_key};

macro_rules! symmetric_key {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Zeroize, ZeroizeOnDrop)]
        pub struct $name([u8; KEY_LEN]);

        impl $name {
            /// A fresh random key.
            pub fn generate() -> Result<$name, CryptoError> {
                Ok($name(*random_key()?))
            }

            /// Rebuild from raw bytes read back from a keystore.
            pub fn from_bytes(bytes: [u8; KEY_LEN]) -> $name {
                $name(bytes)
            }

            pub(crate) fn as_bytes(&self) -> &[u8; KEY_LEN] {
                &self.0
            }
        }

        impl std::fmt::Debug for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(concat!(stringify!($name), "(..)"))
            }
        }
    };
}

symmetric_key! {
    /// The account root key: random, generated on the first device, wraps
    /// the user's private keys. Held only by the platform keystore.
    RootKey
}

symmetric_key! {
    /// A recovery key: random, shown once at signup, wraps the root key.
    RecoveryKey
}

symmetric_key! {
    /// A space key: random, one per space, wrapped for each member.
    SpaceKey
}

/// The user's private keys: X25519 for receiving wrapped keys, Ed25519 for
/// signing.
pub struct UserKeyPair {
    pub(crate) encryption: X25519Secret,
    pub(crate) signing: Ed25519Secret,
}

impl std::fmt::Debug for UserKeyPair {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("UserKeyPair(..)")
    }
}

/// The user's public keys, published to the server.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct UserPublicKeys {
    pub encryption: [u8; 32],
    pub signing: [u8; 32],
}

impl UserKeyPair {
    pub fn generate() -> Result<UserKeyPair, CryptoError> {
        Ok(UserKeyPair {
            encryption: X25519Secret::generate()?,
            signing: Ed25519Secret::generate()?,
        })
    }

    pub fn public_keys(&self) -> UserPublicKeys {
        UserPublicKeys {
            encryption: self.encryption.public(),
            signing: self.signing.public(),
        }
    }

    /// 64 bytes: the X25519 secret then the Ed25519 seed. Only ever
    /// wrapped under the root key before leaving memory.
    pub(crate) fn to_bytes(&self) -> Zeroizing<[u8; 64]> {
        let mut out = Zeroizing::new([0u8; 64]);
        out[..32].copy_from_slice(self.encryption.to_bytes().as_ref());
        out[32..].copy_from_slice(self.signing.to_bytes().as_ref());
        out
    }

    pub(crate) fn from_bytes(bytes: &[u8]) -> Result<UserKeyPair, CryptoError> {
        if bytes.len() != 64 {
            return Err(CryptoError::Decrypt);
        }
        let mut enc = [0u8; 32];
        let mut sig = [0u8; 32];
        enc.copy_from_slice(&bytes[..32]);
        sig.copy_from_slice(&bytes[32..]);
        let pair = UserKeyPair {
            encryption: X25519Secret::from_bytes(enc),
            signing: Ed25519Secret::from_bytes(sig),
        };
        enc.zeroize();
        sig.zeroize();
        Ok(pair)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_do_not_print_their_bytes() {
        let k = RootKey::generate().unwrap();
        assert_eq!(format!("{k:?}"), "RootKey(..)");
        let p = UserKeyPair::generate().unwrap();
        assert_eq!(format!("{p:?}"), "UserKeyPair(..)");
    }

    #[test]
    fn user_key_pair_round_trips_through_bytes() {
        let p = UserKeyPair::generate().unwrap();
        let back = UserKeyPair::from_bytes(p.to_bytes().as_ref()).unwrap();
        assert_eq!(p.public_keys(), back.public_keys());
        assert!(UserKeyPair::from_bytes(&[0u8; 63]).is_err());
    }
}
