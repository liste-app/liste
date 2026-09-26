//! The primitives, each a thin call into an audited crate. Nothing in this
//! module implements cryptography; it fixes the parameters (XChaCha20-
//! Poly1305, X25519, Ed25519, HKDF-SHA-256), the nonce policy (24 random
//! bytes per message), and the wire shapes so the rest of the layer cannot
//! misuse them.

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};
use ed25519_dalek::Signer;
use hkdf::Hkdf;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use super::CryptoError;

/// Bytes of a symmetric key.
pub const KEY_LEN: usize = 32;
/// Bytes of an XChaCha20-Poly1305 nonce.
pub const NONCE_LEN: usize = 24;
/// Bytes of a Poly1305 tag.
pub const TAG_LEN: usize = 16;

/// Fill `buf` from the operating system's random source.
pub fn random_bytes(buf: &mut [u8]) -> Result<(), CryptoError> {
    getrandom::fill(buf).map_err(|_| CryptoError::Randomness)
}

/// A fresh 32-byte secret.
pub fn random_key() -> Result<Zeroizing<[u8; KEY_LEN]>, CryptoError> {
    let mut k = Zeroizing::new([0u8; KEY_LEN]);
    random_bytes(k.as_mut())?;
    Ok(k)
}

/// SHA-256 of `data`.
pub fn sha256(data: &[u8]) -> [u8; 32] {
    Sha256::digest(data).into()
}

/// HKDF-SHA-256: a 32-byte subkey of `ikm` bound to `info`. No salt; every
/// `ikm` here is already uniformly random.
pub fn derive_key(ikm: &[u8], info: &[u8]) -> Zeroizing<[u8; KEY_LEN]> {
    let hk = Hkdf::<Sha256>::new(None, ikm);
    let mut okm = Zeroizing::new([0u8; KEY_LEN]);
    hk.expand(info, okm.as_mut())
        .expect("32 bytes is within HKDF-SHA-256 output length");
    okm
}

/// Encrypt with XChaCha20-Poly1305 under a random nonce. Output is
/// `nonce || ciphertext || tag`.
pub fn aead_seal(
    key: &[u8; KEY_LEN],
    aad: &[u8],
    plaintext: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    let cipher = XChaCha20Poly1305::new(Key::from(*key).as_ref());
    let mut nonce = [0u8; NONCE_LEN];
    random_bytes(&mut nonce)?;
    let ct = cipher
        .encrypt(
            XNonce::from(nonce).as_ref(),
            Payload {
                msg: plaintext,
                aad,
            },
        )
        .map_err(|_| CryptoError::Encrypt)?;
    let mut out = Vec::with_capacity(NONCE_LEN + ct.len());
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ct);
    Ok(out)
}

/// Decrypt the output of [`aead_seal`]. Any change to the bytes, the key,
/// or the associated data fails with [`CryptoError::Decrypt`].
pub fn aead_open(
    key: &[u8; KEY_LEN],
    aad: &[u8],
    sealed: &[u8],
) -> Result<Zeroizing<Vec<u8>>, CryptoError> {
    if sealed.len() < NONCE_LEN + TAG_LEN {
        return Err(CryptoError::Decrypt);
    }
    let (nonce, ct) = sealed.split_at(NONCE_LEN);
    let nonce: [u8; NONCE_LEN] = nonce.try_into().expect("length checked");
    let cipher = XChaCha20Poly1305::new(Key::from(*key).as_ref());
    cipher
        .decrypt(XNonce::from(nonce).as_ref(), Payload { msg: ct, aad })
        .map(Zeroizing::new)
        .map_err(|_| CryptoError::Decrypt)
}

/// An X25519 secret scalar.
pub struct X25519Secret(x25519_dalek::StaticSecret);

impl X25519Secret {
    pub fn generate() -> Result<X25519Secret, CryptoError> {
        let bytes = random_key()?;
        Ok(X25519Secret(x25519_dalek::StaticSecret::from(*bytes)))
    }

    pub fn from_bytes(bytes: [u8; 32]) -> X25519Secret {
        X25519Secret(x25519_dalek::StaticSecret::from(bytes))
    }

    pub fn to_bytes(&self) -> Zeroizing<[u8; 32]> {
        Zeroizing::new(self.0.to_bytes())
    }

    pub fn public(&self) -> [u8; 32] {
        x25519_dalek::PublicKey::from(&self.0).to_bytes()
    }

    /// The raw shared secret with `their_public`. Never used directly as a
    /// key; callers pass it through [`derive_key`].
    pub fn agree(&self, their_public: &[u8; 32]) -> Zeroizing<[u8; 32]> {
        let shared = self
            .0
            .diffie_hellman(&x25519_dalek::PublicKey::from(*their_public));
        Zeroizing::new(*shared.as_bytes())
    }
}

/// An Ed25519 signing key.
pub struct Ed25519Secret(ed25519_dalek::SigningKey);

/// An Ed25519 signature.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Signature(#[serde(with = "serde_bytes64")] pub [u8; 64]);

impl std::fmt::Debug for Signature {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Signature(..)")
    }
}

impl Ed25519Secret {
    pub fn generate() -> Result<Ed25519Secret, CryptoError> {
        let bytes = random_key()?;
        Ok(Ed25519Secret(ed25519_dalek::SigningKey::from_bytes(&bytes)))
    }

    pub fn from_bytes(bytes: [u8; 32]) -> Ed25519Secret {
        Ed25519Secret(ed25519_dalek::SigningKey::from_bytes(&bytes))
    }

    pub fn to_bytes(&self) -> Zeroizing<[u8; 32]> {
        Zeroizing::new(self.0.to_bytes())
    }

    pub fn public(&self) -> [u8; 32] {
        self.0.verifying_key().to_bytes()
    }

    pub fn sign(&self, message: &[u8]) -> Signature {
        Signature(self.0.sign(message).to_bytes())
    }
}

/// Verify an Ed25519 signature with strict validation.
pub fn verify(public: &[u8; 32], message: &[u8], signature: &Signature) -> Result<(), CryptoError> {
    let key =
        ed25519_dalek::VerifyingKey::from_bytes(public).map_err(|_| CryptoError::Signature)?;
    let sig = ed25519_dalek::Signature::from_bytes(&signature.0);
    key.verify_strict(message, &sig)
        .map_err(|_| CryptoError::Signature)
}

/// Constant-time equality of two byte strings of any length.
pub fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    use subtle::ConstantTimeEq;
    a.len() == b.len() && bool::from(a.ct_eq(b))
}

/// serde for 64-byte arrays, which serde does not derive for.
mod serde_bytes64 {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(v: &[u8; 64], s: S) -> Result<S::Ok, S::Error> {
        s.collect_seq(v.iter())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<[u8; 64], D::Error> {
        let v: Vec<u8> = Vec::deserialize(d)?;
        v.try_into()
            .map_err(|_| serde::de::Error::custom("expected 64 bytes"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seal_open_round_trip_and_tamper() {
        let key = random_key().unwrap();
        let sealed = aead_seal(&key, b"aad", b"hello").unwrap();
        assert_eq!(sealed.len(), NONCE_LEN + 5 + TAG_LEN);
        assert_eq!(
            aead_open(&key, b"aad", &sealed).unwrap().as_slice(),
            b"hello"
        );
        assert!(aead_open(&key, b"aad2", &sealed).is_err());
        let other = random_key().unwrap();
        assert!(aead_open(&other, b"aad", &sealed).is_err());
        let mut t = sealed.clone();
        t[NONCE_LEN + 1] ^= 1;
        assert!(aead_open(&key, b"aad", &t).is_err());
        assert!(aead_open(&key, b"aad", &sealed[..NONCE_LEN + TAG_LEN - 1]).is_err());
        let again = aead_seal(&key, b"aad", b"hello").unwrap();
        assert_ne!(again, sealed, "fresh nonce each time");
    }

    #[test]
    fn x25519_agreement_matches_both_ways() {
        let a = X25519Secret::generate().unwrap();
        let b = X25519Secret::generate().unwrap();
        assert_eq!(*a.agree(&b.public()), *b.agree(&a.public()));
        assert_ne!(*a.agree(&b.public()), *a.agree(&a.public()));
    }

    #[test]
    fn ed25519_sign_verify() {
        let k = Ed25519Secret::generate().unwrap();
        let sig = k.sign(b"msg");
        assert!(verify(&k.public(), b"msg", &sig).is_ok());
        assert!(verify(&k.public(), b"msh", &sig).is_err());
        let other = Ed25519Secret::generate().unwrap();
        assert!(verify(&other.public(), b"msg", &sig).is_err());
        let json = serde_json::to_string(&sig).unwrap();
        assert_eq!(serde_json::from_str::<Signature>(&json).unwrap(), sig);
    }

    #[test]
    fn derive_key_is_deterministic_and_info_bound() {
        let ikm = [7u8; 32];
        assert_eq!(*derive_key(&ikm, b"a"), *derive_key(&ikm, b"a"));
        assert_ne!(*derive_key(&ikm, b"a"), *derive_key(&ikm, b"b"));
    }

    #[test]
    fn ct_eq_checks_length_and_content() {
        assert!(ct_eq(b"abc", b"abc"));
        assert!(!ct_eq(b"abc", b"abd"));
        assert!(!ct_eq(b"abc", b"ab"));
    }
}
