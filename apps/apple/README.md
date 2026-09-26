# apps/apple

iOS and macOS apps: one Xcode project, shared Swift code, platform-specific views where they matter. Swift + SwiftUI, with UIKit or AppKit where needed. Phase 1.

The Rust core is reached through UniFFI-generated Swift bindings (see `bindings/`). On macOS the app is the host process: it links the core in-process, includes the core's `host` module, and exposes the local IPC socket. It must run windowless when launched with `--background`, must not quit when the last window closes, and must expose its socket only after the store is open (Section 3 of `docs/ARCHITECTURE.md`).

The Xcode project has not been created yet.
