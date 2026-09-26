//! Shared helpers for the core suite (Section 15).

/// Marks a suite case that is specified but not yet implemented. Such cases
/// are `#[ignore]`d so `cargo test` stays green; `just test-pending` runs
/// them and shows what is still missing. Remove the attribute as each
/// behavior lands.
pub fn pending(suite: &str, case: &str) -> ! {
    panic!(
        "{suite}: `{case}` is specified in docs/ARCHITECTURE.md Section 15 but not implemented yet"
    )
}
