//! The sync runner (Section 6): push pending ops, pull the tail after the
//! space's cursor, and upload snapshots on the Section 6 cadence.
//!
//! The merge rules themselves live in [`crate::store`]: applying an op is
//! the same code whether it came from this device, another device, or a
//! snapshot's tail. This module will own the network side, which is not
//! implemented yet. Its inputs and outputs already exist on the store:
//! [`Store::pending_ops`], [`Store::mark_pushed`], [`Store::apply_remote`],
//! [`Store::cursor`], and [`Store::snapshot`].
//!
//! [`Store::pending_ops`]: crate::store::Store::pending_ops
//! [`Store::mark_pushed`]: crate::store::Store::mark_pushed
//! [`Store::apply_remote`]: crate::store::Store::apply_remote
//! [`Store::cursor`]: crate::store::Store::cursor
//! [`Store::snapshot`]: crate::store::Store::snapshot
