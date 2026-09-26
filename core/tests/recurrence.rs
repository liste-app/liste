//! Recurrence rules (Section 15).

mod common;
use common::pending;

const SUITE: &str = "recurrence";

#[test]
fn every_second_tuesday() {
    pending(SUITE, "every_second_tuesday");
}

#[test]
fn three_days_after_completion() {
    pending(SUITE, "three_days_after_completion");
}

#[test]
fn dst_boundaries_keep_the_local_wall_time() {
    pending(SUITE, "dst_boundaries_keep_the_local_wall_time");
}
