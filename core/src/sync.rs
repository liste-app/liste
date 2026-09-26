//! The sync engine (Section 6).
//!
//! A server-ordered operation log with fixed per-field merge rules: last
//! writer wins by hybrid logical clock for scalars, add-wins observed-remove
//! sets, fractional indexing for manual order, and tombstones for deletes.
//! Ops are idempotent and commutative under these rules.
