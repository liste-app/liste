//! Encrypt and decrypt of a typical op, so the sync path's overhead is
//! known. Reported, not enforced.

mod common;

use liste_core::crypto::{Keyring, MemoryKeyStore};
use liste_core::ids::Id;
use liste_core::model::{EntityType, Field, Value};
use liste_core::op::Mutation;
use liste_core::store::Store;

fn main() {
    let mut keyring = Keyring::new(Box::new(MemoryKeyStore::new()));
    keyring.initialize().unwrap();
    let mut store = Store::open_in_memory(Id::new()).unwrap();
    let space = Id::new();
    keyring.create_space(space).unwrap();
    let op = store.op(
        space,
        EntityType::Task,
        Id::new(),
        Mutation::Set {
            field: Field::Title,
            value: Value::from("call mom about the dentist invoice"),
        },
    );
    let plain_len = op.encode().len();

    let mut encrypt = common::Stats::new();
    let mut decrypt = common::Stats::new();
    let mut sealed_len = 0;
    for _ in 0..2_000 {
        let e = encrypt.time(|| keyring.encrypt_op(&op).unwrap());
        sealed_len = e.ciphertext.len();
        let back = decrypt.time(|| keyring.decrypt_op(&e).unwrap());
        assert_eq!(back, op);
    }
    encrypt.report("encrypt_op (typical set-title op)");
    decrypt.report("decrypt_op (typical set-title op)");
    println!(
        "op size: {plain_len} bytes plain, {sealed_len} bytes sealed, {} bytes overhead",
        sealed_len - plain_len
    );
    common::finish(Vec::new());
}
