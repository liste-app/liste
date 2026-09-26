//! Shared helpers for the core suite (Section 15).

/// Marks a suite case that is specified but not yet implemented. Every case
/// fails until the behavior it names exists, which keeps the suite honest
/// about what is done.
pub fn pending(suite: &str, case: &str) -> ! {
    panic!(
        "{suite}: `{case}` is specified in docs/ARCHITECTURE.md Section 15 but not implemented yet"
    )
}
