//! Foreign bindings for the Liste core (Section 14).
//!
//! Swift and Kotlin bindings are generated with Mozilla UniFFI, C# bindings
//! with uniffi-bindgen-cs, and the web build exposes a `wasm-bindgen`
//! wrapper. Bindings are generated at build time from the current core;
//! nothing generated is committed or published.

uniffi::setup_scaffolding!();

#[cfg(all(target_family = "wasm", target_os = "unknown"))]
pub mod wasm;
