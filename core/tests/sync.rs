//! Two-device sync simulations (Section 15).

mod common;
use common::pending;

const SUITE: &str = "sync";

#[test]
fn same_field_edited_offline_on_both_devices_converges_by_hlc() {
    pending(
        SUITE,
        "same_field_edited_offline_on_both_devices_converges_by_hlc",
    );
}

#[test]
fn different_fields_edited_on_both_devices_both_survive() {
    pending(
        SUITE,
        "different_fields_edited_on_both_devices_both_survive",
    );
}

#[test]
fn delete_versus_edit_follows_the_tombstone_rule() {
    pending(SUITE, "delete_versus_edit_follows_the_tombstone_rule");
}

#[test]
fn duplicate_space_id_and_op_id_is_a_noop() {
    pending(SUITE, "duplicate_space_id_and_op_id_is_a_noop");
}

#[test]
fn snapshot_plus_tail_equals_full_replay() {
    pending(SUITE, "snapshot_plus_tail_equals_full_replay");
}

#[test]
fn unknown_future_op_type_is_stored_skipped_then_applied_after_update() {
    pending(
        SUITE,
        "unknown_future_op_type_is_stored_skipped_then_applied_after_update",
    );
}
