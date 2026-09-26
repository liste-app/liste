//! Crypto fixtures (Section 15), plus the onboarding and recovery paths.

mod common;

use liste_core::crypto::{
    self, AccountMaterial, CryptoError, EncryptedOp, Keyring, MemoryKeyStore, NewDeviceApproval,
    RecoveryKey, RecoveryKeyError, SpaceKey, op_aad, snapshot_aad,
};
use liste_core::ids::Id;
use liste_core::model::{EntityType, Field, Value};
use liste_core::op::Mutation;
use liste_core::store::Store;

fn keyring() -> (Keyring, RecoveryKey, AccountMaterial) {
    let mut k = Keyring::new(Box::new(MemoryKeyStore::new()));
    let (recovery, material) = k.initialize().unwrap();
    (k, recovery, material)
}

fn sample_op(store: &mut Store, space: Id) -> liste_core::op::Op {
    store.op(
        space,
        EntityType::Task,
        Id::new(),
        Mutation::Set {
            field: Field::Title,
            value: Value::from("call mom about the dentist invoice"),
        },
    )
}

fn contains(haystack: &[u8], needle: &str) -> bool {
    haystack
        .windows(needle.len())
        .any(|w| w == needle.as_bytes())
}

#[test]
fn ciphertext_and_metadata_contain_no_title_note_or_tag_substrings() {
    let (mut k, _, _) = keyring();
    let mut store = Store::open_in_memory(Id::new()).unwrap();
    let space = Id::new();
    k.create_space(space).unwrap();
    let task = Id::new();
    let tag = Id::new();
    let ops = vec![
        store.op(
            space,
            EntityType::Task,
            task,
            Mutation::Set {
                field: Field::Title,
                value: Value::from("call mom"),
            },
        ),
        store.op(
            space,
            EntityType::Task,
            task,
            Mutation::SetNotes {
                text: "bring the cake recipe".into(),
            },
        ),
        store.op(
            space,
            EntityType::Tag,
            tag,
            Mutation::Set {
                field: Field::Name,
                value: Value::from("family"),
            },
        ),
        store.op(space, EntityType::Task, task, Mutation::AddTag { tag }),
    ];
    store.commit(&ops).unwrap();
    for op in &ops {
        let e = k.encrypt_op(op).unwrap();
        // The server-visible record: metadata fields plus ciphertext.
        let mut visible = Vec::new();
        visible.extend_from_slice(e.space_id.as_bytes());
        visible.extend_from_slice(e.op_id.as_bytes());
        visible.extend_from_slice(&e.schema_version.to_be_bytes());
        visible.extend_from_slice(&e.ciphertext);
        for word in [
            "call", "mom", "cake", "recipe", "family", "title", "notes", "kind",
        ] {
            assert!(!contains(&visible, word), "{word:?} leaked");
        }
        assert_eq!(e.ciphertext.len(), op.encode().len() + crypto::OVERHEAD);
        assert_eq!(k.decrypt_op(&e).unwrap(), *op);
    }
    // Same for a snapshot.
    let snapshot = store.snapshot_unchecked(space).unwrap();
    let e = k.encrypt_snapshot(&snapshot).unwrap();
    for word in ["call", "mom", "cake", "family"] {
        assert!(!contains(&e.ciphertext, word), "{word:?} leaked");
    }
    assert_eq!(k.decrypt_snapshot(&e).unwrap(), snapshot);
}

#[test]
fn associated_data_is_space_id_op_id_and_schema_version() {
    let space = Id::from_bytes([1; 16]);
    let op_id = Id::from_bytes([2; 16]);
    let aad = op_aad(space, op_id, 7);
    let mut expected = Vec::new();
    expected.extend_from_slice(&[1; 16]);
    expected.extend_from_slice(&[2; 16]);
    expected.extend_from_slice(&7u32.to_be_bytes());
    assert_eq!(aad.as_slice(), expected.as_slice());
    assert_eq!(aad.len(), 36, "nothing else is authenticated as metadata");
    let saad = snapshot_aad(space, 300, 1);
    assert_eq!(&saad[..16], &[1; 16]);
    assert_eq!(&saad[16..24], &300u64.to_be_bytes());
    assert_eq!(&saad[24..], &1u32.to_be_bytes());

    // Changing any of the three fields on a stored op makes it undecryptable.
    let (mut k, _, _) = keyring();
    let mut store = Store::open_in_memory(Id::new()).unwrap();
    let space = Id::new();
    k.create_space(space).unwrap();
    let other_space = Id::new();
    k.create_space(other_space).unwrap();
    let op = sample_op(&mut store, space);
    let e = k.encrypt_op(&op).unwrap();
    let mut wrong_space = e.clone();
    wrong_space.space_id = other_space;
    assert_eq!(
        k.decrypt_op(&wrong_space).unwrap_err(),
        CryptoError::Decrypt
    );
    let mut wrong_op = e.clone();
    wrong_op.op_id = Id::new();
    assert_eq!(k.decrypt_op(&wrong_op).unwrap_err(), CryptoError::Decrypt);
    let mut wrong_version = e.clone();
    wrong_version.schema_version += 1;
    assert_eq!(
        k.decrypt_op(&wrong_version).unwrap_err(),
        CryptoError::Decrypt
    );
    let mut tampered = e.clone();
    let last = tampered.ciphertext.len() - 1;
    tampered.ciphertext[last] ^= 1;
    assert_eq!(k.decrypt_op(&tampered).unwrap_err(), CryptoError::Decrypt);
    let mut truncated = e.clone();
    truncated.ciphertext.truncate(10);
    assert_eq!(k.decrypt_op(&truncated).unwrap_err(), CryptoError::Decrypt);
    assert_eq!(
        k.decrypt_op(&e).unwrap(),
        op,
        "the untouched record still decrypts"
    );
}

#[test]
fn a_session_token_cannot_decrypt() {
    // A session token is 32 random bytes from the server. Even if an
    // attacker treats it as every kind of key, nothing opens.
    let (mut k, _, material) = keyring();
    let mut store = Store::open_in_memory(Id::new()).unwrap();
    let space = Id::new();
    let wrapped_for_me = k.create_space(space).unwrap();
    let op = sample_op(&mut store, space);
    let e = k.encrypt_op(&op).unwrap();
    let mut token = [0u8; 32];
    getrandom::fill(&mut token).unwrap();
    let as_space_key = SpaceKey::from_bytes(token);
    assert_eq!(
        crypto::envelope::decrypt_op(&as_space_key, &e).unwrap_err(),
        CryptoError::Decrypt
    );
    let as_root = crypto::RootKey::from_bytes(token);
    assert_eq!(
        crypto::wrap::unwrap_user_keys(&as_root, &material.wrapped_user_keys).unwrap_err(),
        CryptoError::Decrypt
    );
    let as_recovery = crypto::RecoveryKey::from_bytes(token);
    assert_eq!(
        crypto::wrap::unwrap_root_with_recovery(
            &as_recovery,
            &material.root_wrapped_under_recovery
        )
        .unwrap_err(),
        CryptoError::Decrypt
    );
    // Another user with real keys of their own cannot open the wrap either.
    let mut other = Keyring::new(Box::new(MemoryKeyStore::new()));
    other.initialize().unwrap();
    assert_eq!(
        other.add_space_key(&wrapped_for_me).unwrap_err(),
        CryptoError::NotForThisUser
    );
    assert_eq!(
        other.decrypt_op(&e).unwrap_err(),
        CryptoError::UnknownSpace(space)
    );
}

#[test]
fn space_key_round_trips_through_wrap_and_unwrap() {
    let (mut alice, _, _) = keyring();
    let (mut bob, _, _) = keyring();
    let mut store = Store::open_in_memory(Id::new()).unwrap();
    let space = Id::new();
    alice.create_space(space).unwrap();
    let op = sample_op(&mut store, space);
    let e = alice.encrypt_op(&op).unwrap();
    // Adding Bob later is one wrap for his public key.
    let for_bob = alice
        .wrap_space_key_for(space, &bob.public_keys().unwrap())
        .unwrap();
    bob.add_space_key(&for_bob).unwrap();
    assert_eq!(bob.decrypt_op(&e).unwrap(), op);
    let from_bob = bob.encrypt_op(&op).unwrap();
    assert_ne!(from_bob.ciphertext, e.ciphertext, "fresh nonce");
    assert_eq!(alice.decrypt_op(&from_bob).unwrap(), op);
    // Bob's keys survive a lock and unlock from his keystore.
    bob.lock();
    assert_eq!(bob.decrypt_op(&e).unwrap_err(), CryptoError::Locked);
    bob.unlock().unwrap();
    assert_eq!(bob.decrypt_op(&e).unwrap(), op);
    assert_eq!(bob.spaces().unwrap(), vec![space]);
}

#[test]
fn recovery_key_round_trip_including_a_mistyped_character() {
    let (mut first, recovery, material) = keyring();
    let mut store = Store::open_in_memory(Id::new()).unwrap();
    let space = Id::new();
    let wrapped = first.create_space(space).unwrap();
    let op = sample_op(&mut store, space);
    let e = first.encrypt_op(&op).unwrap();
    let text = recovery.render();
    assert_eq!(text.len(), 69);

    // A mistyped character is caught before any key is tried.
    let mut wrong = text.as_bytes().to_vec();
    let pos = 7;
    wrong[pos] = if wrong[pos] == b'A' { b'B' } else { b'A' };
    let wrong = String::from_utf8(wrong).unwrap();
    let mut second = Keyring::new(Box::new(MemoryKeyStore::new()));
    assert_eq!(
        second.unlock_with_recovery(&wrong, &material).unwrap_err(),
        CryptoError::RecoveryKey(RecoveryKeyError::Checksum)
    );
    assert!(!second.is_initialized().unwrap());

    // The correct key, typed sloppily, restores the account on a new device.
    let sloppy = text.to_lowercase().replace('-', " ");
    second.unlock_with_recovery(&sloppy, &material).unwrap();
    assert_eq!(second.public_keys().unwrap(), first.public_keys().unwrap());
    second.add_space_key(&wrapped).unwrap();
    assert_eq!(second.decrypt_op(&e).unwrap(), op);

    // A different account's recovery key does not open this material.
    let (_, other_recovery, _) = keyring();
    let mut third = Keyring::new(Box::new(MemoryKeyStore::new()));
    assert_eq!(
        third
            .unlock_with_recovery(&other_recovery.render(), &material)
            .unwrap_err(),
        CryptoError::Decrypt
    );
}

#[test]
fn a_new_device_is_approved_by_an_existing_one() {
    let (mut existing, _, material) = keyring();
    let mut store = Store::open_in_memory(Id::new()).unwrap();
    let space = Id::new();
    let wrapped = existing.create_space(space).unwrap();
    let op = sample_op(&mut store, space);
    let e = existing.encrypt_op(&op).unwrap();

    // New device: request and a code on screen.
    let new_id = Id::new();
    let attempt = NewDeviceApproval::begin(new_id).unwrap();
    let request = attempt.request().clone();
    let shown = attempt.short_code();
    assert_eq!(shown.as_str().len(), 9);
    // Existing device: same code, person confirms, grant issued.
    let seen = existing.review_approval(&request);
    assert!(seen.matches(shown.as_str()));
    assert!(seen.matches(&shown.as_str().replace(' ', "")));
    assert!(!seen.matches("0000 0000"));
    let grant = existing.grant_approval(&request).unwrap();
    assert!(!grant.sealed_root.is_empty());

    // New device: verifies and adopts.
    let mut new_device = Keyring::new(Box::new(MemoryKeyStore::new()));
    let signer = material.public_keys.signing;
    attempt
        .complete(&grant, &signer, &mut new_device, &material)
        .unwrap();
    assert!(new_device.is_unlocked());
    new_device.add_space_key(&wrapped).unwrap();
    assert_eq!(new_device.decrypt_op(&e).unwrap(), op);

    // A grant for a different request, a forged signer, or a tampered
    // grant is rejected.
    let other_attempt = NewDeviceApproval::begin(Id::new()).unwrap();
    let mut other_device = Keyring::new(Box::new(MemoryKeyStore::new()));
    assert_eq!(
        other_attempt
            .complete(&grant, &signer, &mut other_device, &material)
            .unwrap_err(),
        CryptoError::Signature
    );
    let replay = NewDeviceApproval::begin(new_id).unwrap();
    assert_eq!(
        replay
            .complete(&grant, &signer, &mut other_device, &material)
            .unwrap_err(),
        CryptoError::Signature,
        "same device id, different ephemeral key: the signature covers the key"
    );
    let attempt2 = NewDeviceApproval::begin(Id::new()).unwrap();
    let grant2 = existing.grant_approval(attempt2.request()).unwrap();
    let mut forged = grant2.clone();
    forged.sealed_root[5] ^= 1;
    assert_eq!(
        NewDeviceApproval::begin(Id::new())
            .unwrap()
            .complete(&forged, &signer, &mut other_device, &material)
            .unwrap_err(),
        CryptoError::Signature
    );
    let (_, _, other_material) = keyring();
    assert_eq!(
        attempt2
            .complete(
                &grant2,
                &other_material.public_keys.signing,
                &mut other_device,
                &material
            )
            .unwrap_err(),
        CryptoError::Signature,
        "signed by a key that is not the account's"
    );
    // A locked approver cannot grant.
    existing.lock();
    assert_eq!(
        existing.grant_approval(&request).unwrap_err(),
        CryptoError::Locked
    );
}

#[test]
fn keyring_states_and_errors_carry_no_secrets() {
    let mut k = Keyring::new(Box::new(MemoryKeyStore::new()));
    assert_eq!(k.unlock().unwrap_err(), CryptoError::NotInitialized);
    assert_eq!(k.public_keys().unwrap_err(), CryptoError::Locked);
    let (recovery, _) = k.initialize().unwrap();
    assert_eq!(k.initialize().unwrap_err(), CryptoError::AlreadyInitialized);
    assert_eq!(format!("{k:?}"), "Keyring(unlocked)");
    assert_eq!(format!("{recovery:?}"), "RecoveryKey(..)");
    let encrypted = EncryptedOp {
        space_id: Id::new(),
        op_id: Id::new(),
        schema_version: 1,
        ciphertext: vec![0; 60],
    };
    let err = k.decrypt_op(&encrypted).unwrap_err();
    assert_eq!(err, CryptoError::UnknownSpace(encrypted.space_id));
    assert_eq!(
        err.to_string(),
        format!("no key for space {}", encrypted.space_id)
    );
    assert_eq!(CryptoError::Decrypt.to_string(), "decryption failed");
}

/// Decrypted buffers and the rendered recovery key are zeroized on drop,
/// which the types enforce.
#[test]
fn plaintext_buffers_and_the_recovery_string_are_zeroizing() {
    fn assert_zeroizing<T: zeroize::Zeroize>(_: &zeroize::Zeroizing<T>) {}
    let (mut k, recovery, _) = keyring();
    let rendered = recovery.render();
    assert_zeroizing(&rendered);
    let space = Id::new();
    k.create_space(space).unwrap();
    let key = SpaceKey::from_bytes([9u8; 32]);
    let sealed = crypto::primitives::aead_seal(&[1u8; 32], b"aad", b"plain").unwrap();
    let opened = crypto::primitives::aead_open(&[1u8; 32], b"aad", &sealed).unwrap();
    assert_zeroizing(&opened);
    assert_zeroizing(&crypto::primitives::derive_key(&[2u8; 32], b"info"));
    assert_zeroizing(&crypto::primitives::random_key().unwrap());
    let _ = key;
}
