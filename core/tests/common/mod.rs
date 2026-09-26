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

/// A fresh directory under the system temp dir for file-backed stores.
#[allow(dead_code)]
pub fn temp_dir(label: &str) -> std::path::PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("liste-{label}-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}
