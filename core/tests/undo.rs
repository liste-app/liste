//! Undo (Section 15).

mod common;
use common::pending;

const SUITE: &str = "undo";

#[test]
fn inverse_op_restores_previous_materialized_state_including_order() {
    pending(
        SUITE,
        "inverse_op_restores_previous_materialized_state_including_order",
    );
}
