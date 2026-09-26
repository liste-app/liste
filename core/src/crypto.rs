//! End-to-end encryption (Section 7).
//!
//! Account root key, X25519/Ed25519 user keypair, per-space symmetric keys
//! wrapped per member, and XChaCha20-Poly1305 for ops and snapshots with
//! `space_id`, op id, and `schema_version` as associated data. Built only on
//! audited RustCrypto crates; no primitive is ever implemented by hand.
