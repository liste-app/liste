# apps/apple

iOS and macOS apps: one Xcode project, two targets, one shared Swift package. Swift 6 with strict concurrency, no third-party Swift dependencies.

- `Liste.xcodeproj`: the `Liste` (macOS) and `Liste iOS` targets. Only the macOS target is built for now.
- `ListeKit/`: the shared package. `Sources/ListeKit` holds the view models and platform-neutral code; `Sources/ListeCore` receives the UniFFI-generated Swift bindings; the XCFramework it links lives in `bindings/generated/apple/`.
- `Liste/macOS/`: the AppKit-hosted SwiftUI app. It is the host process (Section 3): it takes the store lock, opens the store at `~/Library/Application Support/Liste/`, then serves the IPC socket there. It runs with no window when launched with `--background`, drops to the menu bar when the last window closes, and only Quit stops the host.

Lists are windows (Section 4). The session asks the core for a count and then only for the rows a view is about to show, plus a margin; the list itself is an `NSTableView` with rows of one fixed height, so 40,000 rows lay out without measuring any, and each row draws its text in one pass. The board does the same per column. Saved filters are core entities and sync like lists; only view preferences (theme, collapsed subtask groups, board column order, the hotkey) live in `UserDefaults`. The board is AppKit as well: one table per column, cards that drag between columns and reorder within one, and columns reordered by dragging their heading.

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

`--measure-quick-capture` logs how long the panel took to appear. `--measure-scroll` fills a scratch store with the 50,000-task fixture, opens Anytime from Today five times and logs the median main-thread time (the thread's CPU time until the run loop goes idle, so the queries, the table update, and AppKit's layout and display of the new rows), then scrolls the whole list and logs the frame rate and the main-thread time per step. Point `LISTE_DATA_DIR` and `LISTE_SOCKET` at a scratch directory first; both flags work in every build.
