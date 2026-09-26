# apps/apple

iOS and macOS apps: one Xcode project, two targets, one shared Swift package. Swift 6 with strict concurrency, no third-party Swift dependencies.

- `Liste.xcodeproj`: the `Liste` (macOS) and `Liste iOS` targets. Only the macOS target is built for now.
- `ListeKit/`: the shared package. `Sources/ListeKit` holds the view models and platform-neutral code; `Sources/ListeCore` receives the UniFFI-generated Swift bindings; the XCFramework it links lives in `bindings/generated/apple/`.
- `Liste/macOS/`: the AppKit-hosted SwiftUI app. It is the host process (Section 3): it takes the store lock, opens the store at `~/Library/Application Support/Liste/`, then serves the IPC socket there. It runs with no window when launched with `--background`, drops to the menu bar when the last window closes, and only Quit stops the host.

Build order:

```
just apple-bindings   # core for arm64 and x86_64, Swift bindings, XCFramework
just apple-test       # ListeKit tests, including the acceptance driver
just apple-build      # the macOS app, into apps/apple/build
```

`xcodebuild` needs Xcode selected; the recipes set `DEVELOPER_DIR` to `/Applications/Xcode.app` when it is not already set.

The bundle identifier `com.example.liste` is a placeholder. It, and every bundle or application identifier across the apps, must be replaced with the reverse form of the project domain before the first Xcode or Android project is created, and must never change afterwards.
