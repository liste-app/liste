//! Bringing keys to a new device (Section 7, "New device onboarding").
//!
//! **Approval from an existing device.** The new device makes an ephemeral
//! X25519 key and sends an [`ApprovalRequest`] through the server. Both
//! devices derive the same eight-digit [`ShortCode`] from that request and
//! show it; the person confirms the codes match on the existing device,
//! then the existing device seals the root key for the ephemeral key,
//! signs the grant with the user's signing key, and sends the
//! [`ApprovalGrant`] back. The new device checks the signature against the
//! signer it expects (the account's published signing key), unseals the
//! root key, and adopts it.
//!
//! **What the short code proves, and to whom.** The code is not a secret
//! and is not random: it is a truncated SHA-256 of the account id, the new
//! device id, and the new device's ephemeral public key. It therefore
//! proves one thing, to the existing device: that the request it is
//! looking at carries the same ephemeral key as the device in the person's
//! hand. A server (or anyone between the two devices) that substitutes its
//! own ephemeral key to intercept the root key changes the code on the
//! existing device's screen, the person sees a mismatch, and declines.
//! Binding the account id means a request relayed into another account
//! shows a different code there too. The code proves nothing to the new
//! device; that direction rests on the signed grant, and on the fact that
//! a root key not belonging to the account cannot unwrap the account's
//! published user keys. A forged grant cannot be signed, and a swapped
//! signing key still cannot produce a root key that unwraps those keys.
//!
//! **Recovery key.** The person types the recovery key; the root key is
//! unwrapped from the account material. See [`Keyring::unlock_with_recovery`].
//!
//! Transport is out of scope. Every message here is plain data for the
//! sync client to carry; none of it is secret on the wire.

use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, Zeroizing};

use super::CryptoError;
use super::keyring::{AccountMaterial, Keyring};
use super::keys::RootKey;
use super::primitives::{
    Signature, X25519Secret, aead_open, aead_seal, ct_eq, derive_key, sha256, verify,
};
use crate::ids::Id;

const CODE_LABEL: &[u8] = b"liste/approval/short-code/v1";
const GRANT_LABEL: &[u8] = b"liste/approval/grant/v1";

/// Sent by the device asking to be approved.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct ApprovalRequest {
    /// The account the new device is joining.
    pub account_id: Id,
    pub device_id: Id,
    /// The new device's ephemeral X25519 public key.
    pub ephemeral: [u8; 32],
}

/// Eight digits both devices show, as `1234 5678`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ShortCode(String);

impl ShortCode {
    fn derive(request: &ApprovalRequest) -> ShortCode {
        let mut input = Vec::with_capacity(CODE_LABEL.len() + 32 + 32);
        input.extend_from_slice(CODE_LABEL);
        input.extend_from_slice(request.account_id.as_bytes());
        input.extend_from_slice(request.device_id.as_bytes());
        input.extend_from_slice(&request.ephemeral);
        let h = sha256(&input);
        let n = u64::from_be_bytes(h[..8].try_into().expect("8 bytes")) % 100_000_000;
        ShortCode(format!("{:04} {:04}", n / 10_000, n % 10_000))
    }

    /// The code to display.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Compare against what the person read from the other screen, in
    /// constant time and ignoring spacing.
    pub fn matches(&self, entered: &str) -> bool {
        let a: Vec<u8> = self.0.bytes().filter(u8::is_ascii_digit).collect();
        let b: Vec<u8> = entered.bytes().filter(u8::is_ascii_digit).collect();
        ct_eq(&a, &b)
    }
}

/// Sent by the approving device: the root key sealed for the new device.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct ApprovalGrant {
    pub device_id: Id,
    /// The approver's ephemeral X25519 public key for this grant.
    pub ephemeral: [u8; 32],
    pub sealed_root: Vec<u8>,
    /// The signing public key that produced `signature`.
    pub signer: [u8; 32],
    pub signature: Signature,
}

fn grant_key(
    shared: &[u8; 32],
    request: &ApprovalRequest,
    approver_ephemeral: &[u8; 32],
) -> Zeroizing<[u8; 32]> {
    let mut info = Vec::with_capacity(GRANT_LABEL.len() + 32 + 64);
    info.extend_from_slice(GRANT_LABEL);
    info.extend_from_slice(request.account_id.as_bytes());
    info.extend_from_slice(request.device_id.as_bytes());
    info.extend_from_slice(&request.ephemeral);
    info.extend_from_slice(approver_ephemeral);
    derive_key(shared, &info)
}

fn grant_aad(request: &ApprovalRequest, approver_ephemeral: &[u8; 32]) -> Vec<u8> {
    let mut aad = Vec::with_capacity(GRANT_LABEL.len() + 32 + 64);
    aad.extend_from_slice(GRANT_LABEL);
    aad.extend_from_slice(request.account_id.as_bytes());
    aad.extend_from_slice(request.device_id.as_bytes());
    aad.extend_from_slice(&request.ephemeral);
    aad.extend_from_slice(approver_ephemeral);
    aad
}

fn signed_bytes(
    grant_device: Id,
    request_ephemeral: &[u8; 32],
    approver_ephemeral: &[u8; 32],
    sealed: &[u8],
) -> Vec<u8> {
    let mut m = Vec::with_capacity(GRANT_LABEL.len() + 16 + 64 + sealed.len());
    m.extend_from_slice(GRANT_LABEL);
    m.extend_from_slice(grant_device.as_bytes());
    m.extend_from_slice(request_ephemeral);
    m.extend_from_slice(approver_ephemeral);
    m.extend_from_slice(sealed);
    m
}

/// The new device's side of approval. Holds the ephemeral secret until the
/// grant arrives or the attempt is dropped.
pub struct NewDeviceApproval {
    request: ApprovalRequest,
    secret: X25519Secret,
}

impl std::fmt::Debug for NewDeviceApproval {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "NewDeviceApproval({})", self.request.device_id)
    }
}

impl NewDeviceApproval {
    /// Start an approval attempt for this device joining `account_id`.
    pub fn begin(account_id: Id, device_id: Id) -> Result<NewDeviceApproval, CryptoError> {
        let secret = X25519Secret::generate()?;
        Ok(NewDeviceApproval {
            request: ApprovalRequest {
                account_id,
                device_id,
                ephemeral: secret.public(),
            },
            secret,
        })
    }

    /// The request to send to the existing devices.
    pub fn request(&self) -> &ApprovalRequest {
        &self.request
    }

    /// The code to show on this device.
    pub fn short_code(&self) -> ShortCode {
        ShortCode::derive(&self.request)
    }

    /// Finish with the grant an existing device sent. `expected_signer` is
    /// the account's published signing key; the grant must be signed by
    /// it. On success the root key is adopted into `keyring`.
    pub fn complete(
        self,
        grant: &ApprovalGrant,
        expected_signer: &[u8; 32],
        keyring: &mut Keyring,
        material: &AccountMaterial,
    ) -> Result<(), CryptoError> {
        if grant.device_id != self.request.device_id {
            return Err(CryptoError::Signature);
        }
        if !ct_eq(&grant.signer, expected_signer) {
            return Err(CryptoError::Signature);
        }
        let message = signed_bytes(
            grant.device_id,
            &self.request.ephemeral,
            &grant.ephemeral,
            &grant.sealed_root,
        );
        verify(&grant.signer, &message, &grant.signature)?;
        let shared = self.secret.agree(&grant.ephemeral);
        let key = grant_key(&shared, &self.request, &grant.ephemeral);
        let aad = grant_aad(&self.request, &grant.ephemeral);
        let bytes = aead_open(&key, &aad, &grant.sealed_root)?;
        let mut arr: [u8; 32] = bytes
            .as_slice()
            .try_into()
            .map_err(|_| CryptoError::Decrypt)?;
        let root = RootKey::from_bytes(arr);
        arr.zeroize();
        keyring.adopt_root(root, material)
    }
}

impl Keyring {
    /// The code to show on this (existing) device for a request. The person
    /// confirms it matches the new device's screen before
    /// [`grant_approval`](Self::grant_approval) is called.
    pub fn review_approval(&self, request: &ApprovalRequest) -> ShortCode {
        ShortCode::derive(request)
    }

    /// Seal the root key for the requesting device and sign the grant.
    /// Only call after the person confirmed the short code.
    pub fn grant_approval(&self, request: &ApprovalRequest) -> Result<ApprovalGrant, CryptoError> {
        let root = self.root()?;
        let user = self.user()?;
        let eph = X25519Secret::generate()?;
        let ephemeral = eph.public();
        let shared = eph.agree(&request.ephemeral);
        let key = grant_key(&shared, request, &ephemeral);
        let aad = grant_aad(request, &ephemeral);
        let sealed_root = aead_seal(&key, &aad, root.as_bytes())?;
        let message = signed_bytes(
            request.device_id,
            &request.ephemeral,
            &ephemeral,
            &sealed_root,
        );
        let signature = user.signing.sign(&message);
        Ok(ApprovalGrant {
            device_id: request.device_id,
            ephemeral,
            sealed_root,
            signer: user.signing.public(),
            signature,
        })
    }
}
