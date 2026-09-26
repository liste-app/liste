# bindings

Foreign-language bindings for the Rust core. See Section 14 of `docs/ARCHITECTURE.md`.

| Target | Generator | Notes |
|---|---|---|
| Swift (iOS, macOS) | UniFFI | `cargo run -p liste-bindings --features cli --bin uniffi-bindgen -- generate --library <path to libliste_bindings> --language swift --out-dir generated/swift` |
| Kotlin (Android) | UniFFI | Same command with `--language kotlin`. Phase 2. |
| C# (Windows) | uniffi-bindgen-cs | Installed from the NordSecurity repository at the tag matching the workspace's UniFFI version. Phase 2. |
| Web | `wasm-bindgen` | `src/wasm.rs`, built for `wasm32-unknown-unknown`. |

Generated output goes in `generated/`, which is ignored by git. Bindings are produced at build time from the current core and are never committed or published.
