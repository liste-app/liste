//! Op and snapshot encryption (Section 7).
//!
//! An op's pinned encoding is sealed under a subkey of the space key with
//! exactly `space_id || op_id || schema_version` as associated data, so
//! the server-visible routing fields cannot be swapped without the
//! decryption failing. Snapshots use a different subkey and
//! `space_id || seq || format_version`. The sealed bytes are precisely
//! what the server stores in `ops.ciphertext` and `snapshots.ciphertext`.
//!
//! Overhead per message: a 24-byte nonce and a 16-byte tag.

use super::CryptoError;
use super::keys::SpaceKey;
use super::primitives::{NONCE_LEN, TAG_LEN, aead_open, aead_seal, derive_key};
use crate::ids::Id;
use crate::op::Op;
use crate::store::Snapshot;

const OP_SUBKEY: &[u8] = b"liste/space-key/ops/v1";
const SNAPSHOT_SUBKEY: &[u8] = b"liste/space-key/snapshots/v1";

/// Bytes added to every encrypted op or snapshot.
pub const OVERHEAD: usize = NONCE_LEN + TAG_LEN;

/// The associated data for an op: 16 bytes of `space_id`, 16 of `op_id`,
/// and `schema_version` as four big-endian bytes. Nothing else.
pub fn op_aad(space_id: Id, op_id: Id, schema_version: u32) -> [u8; 36] {
    let mut aad = [0u8; 36];
    aad[..16].copy_from_slice(space_id.as_bytes());
    aad[16..32].copy_from_slice(op_id.as_bytes());
    aad[32..].copy_from_slice(&schema_version.to_be_bytes());
    aad
}

/// The associated data for a snapshot: `space_id`, `seq` (8 bytes big
/// endian), and the snapshot `format_version` (4 bytes).
pub fn snapshot_aad(space_id: Id, seq: u64, format_version: u32) -> [u8; 28] {
    let mut aad = [0u8; 28];
    aad[..16].copy_from_slice(space_id.as_bytes());
    aad[16..24].copy_from_slice(&seq.to_be_bytes());
    aad[24..].copy_from_slice(&format_version.to_be_bytes());
    aad
}

/// An op as the server stores it: routing metadata in the clear, the op
/// itself sealed.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct EncryptedOp {
    pub space_id: Id,
    pub op_id: Id,
    pub schema_version: u32,
    pub ciphertext: Vec<u8>,
}

/// A snapshot as the server stores it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct EncryptedSnapshot {
    pub space_id: Id,
    pub seq: u64,
    pub format_version: u32,
    pub ciphertext: Vec<u8>,
}

pub fn encrypt_op(space_key: &SpaceKey, op: &Op) -> Result<EncryptedOp, CryptoError> {
    let key = derive_key(space_key.as_bytes(), OP_SUBKEY);
    let aad = op_aad(op.space_id, op.op_id, op.schema_version);
    Ok(EncryptedOp {
        space_id: op.space_id,
        op_id: op.op_id,
        schema_version: op.schema_version,
        ciphertext: aead_seal(&key, &aad, &op.encode())?,
    })
}

/// Decrypt and decode an op. Fails if the ciphertext, the key, or any of
/// the three metadata fields differ from what was sealed, or if the
/// decoded op's own fields disagree with the metadata.
pub fn decrypt_op(space_key: &SpaceKey, encrypted: &EncryptedOp) -> Result<Op, CryptoError> {
    let key = derive_key(space_key.as_bytes(), OP_SUBKEY);
    let aad = op_aad(
        encrypted.space_id,
        encrypted.op_id,
        encrypted.schema_version,
    );
    let plain = aead_open(&key, &aad, &encrypted.ciphertext)?;
    let op = Op::decode(&plain).map_err(|_| CryptoError::Decrypt)?;
    if op.space_id != encrypted.space_id
        || op.op_id != encrypted.op_id
        || op.schema_version != encrypted.schema_version
    {
        return Err(CryptoError::Decrypt);
    }
    Ok(op)
}

pub fn encrypt_snapshot(
    space_key: &SpaceKey,
    snapshot: &Snapshot,
) -> Result<EncryptedSnapshot, CryptoError> {
    let key = derive_key(space_key.as_bytes(), SNAPSHOT_SUBKEY);
    let aad = snapshot_aad(snapshot.space_id, snapshot.seq, snapshot.format_version);
    Ok(EncryptedSnapshot {
        space_id: snapshot.space_id,
        seq: snapshot.seq,
        format_version: snapshot.format_version,
        ciphertext: aead_seal(&key, &aad, &snapshot.encode())?,
    })
}

pub fn decrypt_snapshot(
    space_key: &SpaceKey,
    encrypted: &EncryptedSnapshot,
) -> Result<Snapshot, CryptoError> {
    let key = derive_key(space_key.as_bytes(), SNAPSHOT_SUBKEY);
    let aad = snapshot_aad(encrypted.space_id, encrypted.seq, encrypted.format_version);
    let plain = aead_open(&key, &aad, &encrypted.ciphertext)?;
    let snapshot = Snapshot::decode(&plain).map_err(|_| CryptoError::Decrypt)?;
    if snapshot.space_id != encrypted.space_id
        || snapshot.seq != encrypted.seq
        || snapshot.format_version != encrypted.format_version
    {
        return Err(CryptoError::Decrypt);
    }
    Ok(snapshot)
}
