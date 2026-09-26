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

## Keyboard

Every action has a shortcut, shown in the menus. Keys marked global work with no window open.

| Keys | Action |
|---|---|
| ⌥Space | Quick capture (global; change it in Settings) |
| ⌘N | New task in the current list, tag, or today |
| ⌘⇧N | New list |
| ⌘F | Search; Escape returns to the previous view |
| ⌘K | Command palette: actions, places, lists, tags, tasks |
| ⌘I | Show or hide the inspector |
| ⌘Z, ⌘⇧Z | Undo, redo, everywhere including reorder and complete |
| Space | Complete or reopen the selected task |
| Return | Edit the selected task's title |
| ⌫ | Delete the selected task (undo restores it) |
| ⌥↑, ⌥↓ | Move the selected task up or down (list view) |
| ⌥←, ⌥→ | Move the selected card to the previous or next column (board view) |
| ⌘1 … ⌘5 | Today, Upcoming, Anytime, Completed, Inbox |
| ⌘⇧1, ⌘⇧2 | List view, board view |
| ⌘, | Settings |
| ⌘0 | Show the main window |
| j, k, x | Down, up, complete, when vim keys are on in Settings |

Debug builds accept `--measure-quick-capture` and `--measure-scroll`, which log the panel's appearance time and the list's frame rate over the 50,000-task fixture; point `LISTE_DATA_DIR` at a scratch directory first.
