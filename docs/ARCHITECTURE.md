# Liste: Architecture and Engineering Decisions

This document records the product and engineering decisions for Liste. Decisions marked settled are final unless explicitly reopened by the maintainers. Section 20 lists the decisions that have a scheduled revisit point; until that point they are settled too. There are no open questions.

Read `CONTRIBUTING.md` at the repo root before writing code. It restates the non-negotiables in this document. If the two ever disagree, this document wins and `CONTRIBUTING.md` is updated in the same change.

Last updated: September 26, 2026.

---

## 1. Vision

**Liste** is a to-do app designed around one goal: **speed.** Capture, interaction, search, and sync should all feel instant. Every product and engineering decision is judged against that goal.

This is a design goal, not a marketing claim. Liste does not describe itself as "the fastest" anywhere: in the product, on the website, in store listings, or in the README. Speed is demonstrated with measured budgets (Section 4) and left for users to judge. Superlatives invite comparison tests that can only be lost.

Capture includes every path a task can enter the system: global hotkey, widget, share sheet, natural language, the local CLI, and the user's own agent via the local MCP server. All of those paths hit the same core. None of them wait on the network.

It must be available on **every major platform** as a **fully native app** (explicitly *not* cross-platform UI frameworks like React Native, Flutter, or Electron), plus a first-class web app. Going native is about full control, deep platform integration, and maximum optimization.

The UI layers are deliberately thin. There is exactly one implementation of sync, crypto, recurrence, and parsing, and it lives in the Rust core.

Liste is **open source from the first commit** (AGPL-3.0) and **self-hostable**. The business is the hosted service, Liste Cloud, not the code. See Section 18.

## 2. Platforms (6 clients + local agent interface)

| Platform | UI technology | Bindings to the Rust core | Distribution |
|---|---|---|---|
| iOS | Swift + SwiftUI (UIKit where needed) | UniFFI (Swift) | App Store |
| macOS | Swift + SwiftUI (AppKit where needed) | UniFFI (Swift) | **Direct download (primary; notarized, Sparkle updates, not sandboxed)** + App Store (secondary; App Group container) |
| Android | Kotlin + Jetpack Compose | UniFFI (Kotlin) | Google Play |
| Windows | C# + WinUI 3 | uniffi-bindgen-cs (NordSecurity) | **MSIX via App Installer from downloads.<project-domain> (primary; built-in auto-update)** + Microsoft Store (secondary, same MSIX) |
| Linux | Rust + GTK4 + libadwaita | Direct (same language, no FFI) | Flatpak on Flathub (primary), AppImage |
| Web | SvelteKit | Rust core compiled to WASM, in a Web Worker | PWA, installable |

The project domain is not chosen yet; `<project-domain>` is a placeholder for it throughout this document.

iOS and macOS share one Xcode project with shared Swift code and platform-specific views where they matter.

**Local agent interface (not a seventh GUI):** a Rust CLI and a tiny local MCP server. Both are thin IPC clients of the device's **host process** (Section 3). They are Phase 1, not a Linux follow-up. See Section 13.

**Why direct download is primary on desktop:** (see also "Background operation" in Section 3) the CLI and MCP server must reach the running app's store and the app must control its own update cadence. Store sandboxes complicate both. The store builds exist for discovery and use an App Group (macOS) to share the same container with a bundled CLI.

**Launch order (settled):**
- **Phase 1:** macOS, iOS, web, **local CLI, local MCP server.** (Apple users pay for productivity tools; the two Apple apps share one project; web covers everyone else while sync and encryption prove themselves in production. CLI and MCP are how the product is driven without a GUI, and how the core is exercised before the other native shells exist.)
- **Phase 2:** Android, Windows.
- **Phase 3:** Linux GUI. The CLI already exists; the GTK app is the new work.

Do not start Android, Windows, or the Linux GUI early, however cheap another shell looks. The hard work in Phase 1 is sync, encryption, device onboarding, and capture latency. Extra shells do not prove those.

## 3. Architecture

**A shared Rust core with fully native UIs on every platform. CLI and MCP are core consumers, not UI layers with their own logic.**

```
                   ┌──────────────────────────────┐
                   │        Rust core             │
                   │  data model · SQLite · sync  │
                   │  crypto · search · NL parse  │
                   │  recurrence · undo · host    │
                   └──────────────┬───────────────┘
        ┌──────────┬──────────┬───┴─────┬──────────┬──────────┐
     UniFFI     UniFFI    uniffi-    direct      WASM
     (Swift)   (Kotlin)  bindgen-cs  (Rust)   (Web Worker)
        │          │          │         │          │
   iOS/macOS   Android    Windows    Linux       Web
        │                     │         │
        └──── desktop host process (owns store, keys, sync) ────┐
                              ▲ local IPC                        │
                        CLI · MCP server  (thin clients)         │
                                                                 │
                   ┌──────────────────────────────┐              │
                   │   Rust sync server (Axum)    │◄─────────────┘
                   │  Postgres via sqlx · shares  │   encrypted ops only
                   │  types/protocol with core    │
                   └──────────────────────────────┘
```

### Why
- Six separate implementations of sync, crypto, recurrence, and natural-language parsing would drift apart and cause cross-platform bugs. The hard logic is written **once**, in Rust. Five more copies, however produced, is how cross-platform bugs get invented.
- The UI layer stays **idiomatic to each platform**. The Rust core owns logic; native code owns presentation and platform integration.
- The sync server is also Rust, so the sync protocol and data types are shared between server and clients.
- A local CLI and MCP server mean the user's own agent can capture, search, and complete without a bolted-on chatbot and without the server ever seeing plaintext.

### Process model on desktop (settled)
**Exactly one process per device owns the local store: the host.** The host is the only process that opens SQLite, touches the platform keystore, and runs sync. Everything else on that machine talks to it over local IPC.

- **The GUI app is the host.** On macOS, Windows, and Linux the native app links the core in-process (so GUI capture and search never cross a process boundary) and includes the core's `host` module, which exposes a local IPC endpoint.
- **CLI and MCP server are thin IPC clients.** They contain no core logic, open no database, and hold no keys. If no host is running, they start the GUI app in the background (macOS: launched as a background/menu-bar process; Windows: hidden window; Linux: `liste daemon`) and connect once it is up.
- **Headless mode:** `liste daemon` runs the same `host` module with no window, for machines with no GUI installed. It is the only non-GUI host. The CLI and MCP never embed the core.
- **IPC transport:** Unix domain socket on macOS/Linux (`0600`, in the app's data directory or `$XDG_RUNTIME_DIR`), named pipe on Windows. Framed, versioned messages defined in the core. This is a private protocol, not a public API; the OS's file permissions are the auth boundary.
- **Locked store:** if the host's store is locked (keys not yet unlocked on this device), CLI and MCP tools return a `locked` error. They do not prompt for credentials and do not call the server.
- **Mobile and web:** no host process; the app links the core in-process and is the only thing on that device that touches the store.

This resolves, in one design, sandbox access, multi-process SQLite, single-sync-runner, and key exposure: only the host ever holds keys.

### Background operation (Phase 1 deliverable, not an afterthought)
The desktop app must be comfortable running with no window, because the CLI and MCP server will start it that way. This is designed in from the first macOS build:

- **Two activation states, one process.** macOS: the app switches between `.regular` (windows and Dock icon) and `.accessory` (menu bar only) with `NSApp.setActivationPolicy`. Closing the last window does not quit; it drops to the menu bar. "Quit Liste" stops the host; the next CLI/MCP call starts it again. Windows: hide to tray on close, same semantics. Linux: the GTK app supports `--gapplication-service`, and `liste daemon` covers headless machines.
- **Background launch.** The app accepts a `--background` argument (macOS: launched via `open -g -j -a Liste --args --background`; Windows: hidden window; Linux: service mode) and starts without showing a window or stealing focus.
- **Launch at login, on by default.** macOS: `SMAppService` login item. Windows: startup entry. Linux: XDG autostart. Users can turn it off; the CLI/MCP auto-start still covers them.
- **Menu bar / tray item** with quick capture and a "show window" action. Hidden by a setting only if the user wants no persistent presence, in which case auto-start still works but the host quits when idle after the last client disconnects and the last window closes.
- **Readiness handshake.** The host creates its IPC socket only after the store is opened (locked or not). Clients poll for the socket for up to 3 s after triggering a launch, then fail with a clear "Liste did not start" error. A locked store still accepts connections and answers `locked`.
- **Socket path.** macOS direct build: `~/Library/Application Support/Liste/host.sock`; App Store build: the App Group container. Windows: `\\.\pipe\liste-<user-sid>`. Linux: `$XDG_RUNTIME_DIR/liste/host.sock`. Clients check the known paths in order.
- **CLI and MCP binaries ship inside the desktop app bundle** (macOS: `Liste.app/Contents/Helpers/`; Windows: the install directory; Linux: the Flatpak, exported via `flatpak run` wrappers, and the AppImage). Settings has an "Install command line tools" action that symlinks them onto `PATH`. One artifact per platform, one auto-update path (Sparkle on macOS, MSIX App Installer on Windows, Flathub on Linux), so the CLI, MCP server, and host always update together.
- **Version handshake.** Client and host exchange the IPC protocol version on connect. A mismatch (typically an update that has not restarted the host yet) returns a clear "restart Liste to finish updating" message instead of a confusing failure.

### What lives in the core
- Data model (tasks, subtasks, lists/projects, tags, priorities, due dates, reminders, notes, statuses, spaces)
- Local database (SQLite, same engine on every platform, including web)
- Sync engine (operation log, see Section 6)
- End-to-end encryption (see Section 7)
- Search (local, instant, via SQLite FTS5)
- Natural-language parsing for quick capture (e.g. `call mom tomorrow 5pm #family !high`)
- Recurrence logic ("every 2nd Tuesday", "3 days after completion", timezone edge cases)
- Undo/redo history (inverse ops)
- Importers (Todoist, Things, TickTick), so import logic is tested once
- The `host` module: IPC endpoint, store ownership, sync runner

### What lives in each native app
- All UI and interaction
- Platform integrations (widgets, shortcuts, notifications, global hotkeys, tray/menu bar, etc.)
- Local notification scheduling via each platform's APIs (required, because the server cannot read reminder content under E2EE)
- Secure key storage via the platform keystore, called through the core (Section 7)
- On desktop: hosting the core's `host` module and auto-starting quietly when a CLI/MCP client needs it

### What lives in CLI and MCP
- Argument parsing, MCP tool schemas, stdio transport, IPC client. Nothing else.

## 4. Core performance principles

1. **Local-first.** Every action writes to the local database and renders immediately. Sync is always in the background. No spinners, no waiting on the network, full offline support. This matters more for perceived speed than native UI does.
2. **Sync is the most important piece of engineering in the product.** It must be robust, conflict-tolerant, and invisible.
3. **Never block the UI thread** with database, sync, crypto, or search work, on any platform. CLI and MCP must not block on the network to accept a capture.
4. **Keyboard-first on desktop:** command palette, shortcuts for every action, optional vim-style navigation.
5. **Virtualize long lists** everywhere.
6. **Capture latency budgets:** global quick-capture window on desktop appears in under ~50 ms. Adding a task from anywhere (hotkey, CLI, MCP, share sheet) takes under 2 seconds end to end, and the local write itself is immediate. A CLI/MCP round trip to a running host adds well under 1 ms.
7. **These numbers are budgets, not slogans, and they are the only form the speed goal takes in public.** A bench fails on regression. Start the large-DB fixture at 50,000 tasks. Search must still return on every keystroke against that fixture. WASM core load must not block first paint of the shell.
8. **Assume capture volume is high.** If capture is actually instant, people will create far more tasks and ops than they do in older to-do apps. Snapshots, virtualization, FTS, and op-log growth have to be designed for that, not for a demo account. Measure real op-log growth in beta before treating the garbage-collection window as final.

## 5. Features

### Essential (v1)
- **Instant capture:** global hotkey (desktop), home/lock-screen widgets (mobile), share-sheet/share extensions, natural-language parsing of dates, tags, priorities, and lists. Same parser in the GUI, the CLI, and MCP.
- **Local CLI and local MCP server** (Section 13). Phase 1. Not Pro-only.
- **Tasks and structure:** tasks, subtasks, lists/projects, due dates, reminders, priorities, tags, notes.
- **Recurring tasks**, with robust rules and timezone handling.
- **List view and Kanban view.** Two views of the same data, not separate concepts. Kanban columns can be grouped by status, priority, project, or tag. Drag-and-drop must be smooth on every platform, including keyboard-driven moves on desktop.
- **Instant local search** with results on every keystroke. FTS5 on device. No server-side search index. On-device semantic search is not v1.
- **Saved filters / smart lists:** Today, Upcoming, Overdue, plus user-defined filters.
- **Undo everywhere.**
- **Offline support** everywhere, with background sync.
- **End-to-end encryption** (Section 7).
- **Importers** from Todoist, Things, and TickTick. Import logic lives in the core; each client only supplies a file picker.

### Platform integrations (the payoff for going native)
- **iOS/macOS:** Siri & Shortcuts, widgets, Spotlight indexing, menu bar app (macOS). Notification Service Extension decrypts push payloads.
- **Android:** widgets, Quick Settings tile, actionable notifications (complete/snooze without opening the app).
- **Windows:** jump lists, system tray quick capture.
- **Linux:** tray icon, D-Bus integration. The user-facing CLI is not waiting on this app.
- **Web:** installable PWA, full offline support (also covers platforms without a native app, e.g. ChromeOS). The web app does not embed the MCP server; a desktop agent talks to the local host, and other devices see the result through normal sync.
- **CLI / MCP:** the agent-facing surface. See Section 13.

### Deferred (not v1, but designed for)
- **Sharing/collaboration.** The data model (spaces), the shard key, and the key-wrapping scheme are all designed so collaboration can be added without migration. Do not build the UI, membership flows, or invite screens for v1.
- **First-class public API.** A cloud HTTP API that accepts task text is not a substitute for the local MCP server, and it fights E2EE. Do not add one.

## 6. Sync (settled)

**Server-ordered operation log with CRDT-style merge semantics per field.** Not a general-purpose document CRDT library over the whole dataset.

### The model
- Every user change is an **op**: `{op_id (UUIDv7), space_id, device_id, hlc, entity_type, entity_id, mutation}`. `hlc` is a hybrid logical clock (wall time + logical counter + device id) so ops are totally ordered even across offline devices.
- Ops are **idempotent** and **commutative under the merge rules**, so any device can apply any op in any order and converge.
- The host writes each op to local SQLite (op log + materialized tables) **in one transaction**, renders immediately, and queues the op for push. CLI and MCP use this same write path through the host. There is no special "agent write" that skips the op log.
- The **server assigns a monotonically increasing `seq` per space** on receipt and stores the op. Clients pull "everything in space X after seq N". The server never interprets op contents (they are encrypted; Section 7).
- **Snapshots:** the client periodically uploads an encrypted snapshot of a space's materialized state at a given `seq`, so new devices bootstrap from snapshot + tail instead of replaying the full log. High capture volume makes snapshots mandatory. A new device must not depend on replaying an unbounded log.
- **Realtime:** a WebSocket (or SSE) stream of new ops per space. Not required for v1 correctness, but the design assumes it, and it is the same mechanism later collaboration will use.
- **One sync runner per device:** the host process (Section 3). No other process pushes or pulls.
- **Snapshot cadence:** a client uploads a new snapshot for a space after every 2,000 ops or 7 days, whichever comes first. The server keeps the latest two snapshots per space. Ops older than the second-latest snapshot become eligible for garbage collection after the retention window.
- **Notes text CRDT (when needed):** the designated upgrade path for the notes field is **Loro** (Rust-native, performant, embeds as a per-field CRDT). Only the notes field ever uses it; everything else stays on the merge rules above. Trigger: shipping collaboration, or measured user complaints about note conflicts.

### Merge rules (fixed per field type)
| Field type | Rule |
|---|---|
| Scalars (title, due date, priority, status, completed, etc.) | Last-writer-wins by `hlc`, **per field** (not per entity) |
| Sets (tags on a task, members of a space) | Add-wins observed-remove set |
| Manual ordering (position in a list, in a kanban column) | Fractional indexing (string keys), with periodic rebalancing |
| Deletes | Tombstones with `deleted_at`; garbage-collected after the retention window (Section 20) |
| Notes (free text) | Whole-field LWW in v1, but stored as its own op type so it can be upgraded to a per-field text CRDT later without touching any other field |

### Why this over a CRDT library
- The data model is mostly scalar fields; per-field LWW is what users expect and is easy to explain and debug.
- Ops are compact, human-readable (before encryption), and give **undo, history, and audit for free**.
- Works cleanly under E2EE: the server only orders opaque ciphertext.
- Extends to realtime collaboration by streaming the same ops.
- No dependency on a large third-party CRDT library for the core of the product; full control over performance and schema evolution.

### Schema evolution
- Every op carries a `schema_version`. Clients must be able to apply ops from older versions forever. Unknown newer op types are stored and skipped until the client updates. Never break old ops.

## 7. End-to-end encryption (settled: yes, in v1)

E2EE is the one property that cannot be retrofitted, so it ships in v1. The server stores only ciphertext and minimal routing metadata.

### Key hierarchy
- **Account root key:** random 256-bit key generated on the first device. Never leaves a device unencrypted. Used to wrap the user's private keys.
- **User keypair:** X25519 (encryption) + Ed25519 (signing), wrapped under the root key. Public keys are published to the server.
- **Space key:** random 256-bit symmetric key per space. **Wrapped for each member under that member's X25519 public key.** This is exactly the structure sharing will need: adding a collaborator later is "wrap the space key for one more public key."
- **Ops and snapshots** are encrypted with **XChaCha20-Poly1305** under the space key, with `space_id`, op id, and `schema_version` in the authenticated associated data.

### Key storage per platform (always through the core, only in the host process on desktop)
- iOS/macOS: Keychain (Secure Enclave-backed where possible)
- Android: Android Keystore
- Windows: DPAPI / Credential Manager
- Linux: libsecret (Secret Service)
- Web: WebCrypto non-extractable keys in IndexedDB; optionally the WebAuthn PRF extension where available

### New device onboarding (no passwords exist; see Section 8)
1. **Approval from an existing device:** new device shows a short code / QR; existing device verifies it and sends the wrapped root key over an authenticated channel.
2. **Recovery key:** a mandatory, randomly generated key shown at signup (user must confirm they saved it). It wraps the root key. Losing all devices *and* the recovery key means the data is unrecoverable, and the UI must say so plainly.

### What E2EE protects, and what it does not (state this in the product, too)
- E2EE protects task content from **Liste's servers** and from anyone who compromises them.
- It does **not** protect content from software the user runs on their own device. In particular, **an agent the user connects to the local MCP server sees plaintext**, and if that agent is a cloud model, the content goes to that model's provider. That is the user's choice. The MCP server shows a one-time notice on first connection saying exactly this. Do not "fix" this by encrypting what the MCP server returns; it would make the feature useless without changing the security model.

### Plaintext rule for the server
- **Liste clients (GUI, CLI, MCP, web) never send plaintext task content to the sync server.** There is no server endpoint that accepts task text from a Liste client.
- **Third-party inbound channels** (a future email-to-task address, for example) are the only exception: the server may receive plaintext from a third party, must encrypt it **in memory** with the space's public key (sealed-box style) before writing anything, and must never persist or log it in plaintext. Such channels are opt-in per space.

### Consequences designed in
- Reminders are scheduled **locally** on each device.
- Push notifications are **silent wake-ups with an encrypted payload**; the client (or a notification extension on iOS) decrypts and displays.
- Search runs on-device only. The server never indexes content.
- Server metadata visible in plaintext is limited to: account/device identifiers, space ids, seq numbers, ciphertext sizes, timestamps.

### Implementation rules
- Use audited **RustCrypto** crates (`chacha20poly1305`, `x25519-dalek`, `ed25519-dalek`, `hkdf`, `sha2`). Never implement primitives by hand, and never "simplify" a cipher.
- All crypto lives in the core, so every client uses identical code.
- The core is not a black box. Crypto and sync changes are reviewed by reading them, not only by running the app. **Commission an external security audit** of the crypto and sync design before public launch.

## 8. Authentication (settled: passwordless)

- **Primary:** passkeys (WebAuthn / FIDO2).
- **Fallback:** email one-time code (also used for signup).
- **Social:** Sign in with Apple and Sign in with Google. (Apple requires offering Sign in with Apple on iOS if any third-party login is offered.)
- **No passwords, ever.** There is no password-derived encryption key to attack and no password-breach liability.
- Authentication is **separate from encryption**: logging in proves identity to the server and yields a per-device session token. Encryption keys reach a device only via device approval or the recovery key (Section 7). A stolen session token alone cannot decrypt anything.
- Sessions: opaque per-device tokens, revocable from a device list in settings. CLI and MCP have no separate account system; they act through the host's device session.
- Implemented in the Axum server (own tables, Section 10), not delegated to a third-party auth SaaS, to keep the account/key model under full control.
- **Self-hosted servers:** passkeys in the *native* apps depend on Apple/Google associated-domain files that name Liste's app identifiers, which a self-hoster's domain cannot provide. Against a self-hosted server the native apps sign in with the email one-time code; passkeys still work in the web app and on the self-hoster's own domain. Sign in with Apple/Google are Liste Cloud only. The email-code path is therefore a first-class login, not a degraded one.

## 9. Identifiers (settled)

- **UUIDv7** for every entity and every op, generated **client-side by the core** (offline creation must work).
- Stored as 16-byte `BLOB` in SQLite and `uuid` in Postgres.
- Time-sortable, globally unique across shards, no server round-trip needed.

## 10. Database

### Clients: SQLite
- SQLite on every client, accessed only through the Rust core, so every client shares one schema, one set of queries, and one set of migrations.
- **Native platforms:** `rusqlite`. WAL mode. On desktop, only the host process opens the file (Section 3).
- **Web (settled):** SQLite is **compiled into the Rust WASM module** using the `sqlite-wasm-rs` crate, which integrates with `rusqlite`. Use its **OPFS "sahpool" VFS only**, in the dedicated worker where the core already runs. Browsers without OPFS are unsupported for persistence and get a clear "unsupported browser" message. Do not reintroduce an IndexedDB VFS: its asynchronous persistence does not meet SQLite's durability requirements. This keeps one core binary everywhere with FTS5 and migrations in one place. The alternative (letting the official SQLite WASM build own storage on the JS side) is rejected because it splits the core.
- **Search:** SQLite **FTS5** for instant local search. No separate search engine.
- **Two kinds of tables:** the encrypted-at-rest op log (mirrors what the server has) and plaintext materialized tables for querying. The local database relies on OS-level disk encryption plus keystore-protected keys. **No SQLite page-level encryption** (settled): the materialized tables must be queryable and FTS-indexed in plaintext anyway, page-level encryption adds a second key to manage in every client, and the platform keystore plus full-disk encryption is the same model the major password managers rely on for their local caches.

### Server: PostgreSQL
- PostgreSQL, accessed from the Rust sync server with **sqlx** (compile-time checked queries).
- The server's job is narrow: durable store of encrypted ops and snapshots, plus accounts, devices, memberships, auth, push tokens, and billing entitlements.
- **Launch on standard managed Postgres** (PlanetScale's regular Postgres offering is a natural fit; any reliable managed Postgres works). Add a read replica before considering sharding.

### Server schema (settled outline; shard key is `space_id` everywhere it applies)
```
spaces           (space_id, kind [personal|shared], created_at)
space_members    (space_id, user_id, role, wrapped_space_key, added_at)
ops              (space_id, seq BIGINT, op_id UUID, device_id, hlc, schema_version,
                  ciphertext BYTEA, received_at)          -- PK (space_id, seq); UNIQUE (space_id, op_id)
snapshots        (space_id, seq, ciphertext BYTEA, created_by_device, created_at)
-- global (small, not sharded by space):
accounts         (user_id, email, public_enc_key, public_sign_key, created_at, plan, plan_expires_at)
devices          (device_id, user_id, name, platform, push_token, last_seen, session_token_hash)
auth_passkeys    (credential_id, user_id, public_key, sign_count, ...)
auth_email_codes (email, code_hash, expires_at)
user_spaces      (user_id, space_id)   -- directory for "which spaces does this user belong to"
```
- `ops.seq` is assigned by the server, per space, strictly increasing.
- Idempotent ingest: re-sending an op with an existing `(space_id, op_id)` is a no-op.

### Shard-ready rules (apply from day one)
1. **Every space-scoped table carries `space_id`**, and every sync query filters on it.
2. **The shard key is the space, not the user.** Every user starts with a personal space; future shared lists live in shared spaces, so collaborative data stays on one shard.
3. **No cross-space queries or joins in hot paths.** Admin, analytics, and reporting stay off the sync path.
4. Global tables (accounts, devices, directory) stay small and are not sharded by space.

### Future scaling: PlanetScale Neki (settled trigger)
- **Neki** (https://planetscale.com/docs/neki) is PlanetScale's horizontal sharding layer for Postgres: a router in front of Postgres nodes, behind a single connection string speaking the Postgres wire protocol. A database starts unsharded and can be sharded later.
- **Status as of September 2026:** Platform Preview (beta), no SLA. **Do not launch on it.**
- **Move to Neki only when both hold:** (a) it is generally available, and (b) a concrete trigger is hit: primary database approaching ~1 TB, sustained write throughput or connection limits despite pooling and a read replica, or a multi-region requirement.
- Because Neki is real Postgres over the standard wire protocol and the schema follows the shard-ready rules above, the move should be an **operational migration, not an application rewrite**.

## 11. Web app specifics

- **Framework: SvelteKit** (chosen over SolidStart and TanStack Start for fine-grained reactivity, small bundles, and handling marketing site + app in one project).
  - **Marketing/docs/pricing pages:** prerendered or SSR.
  - **The app itself:** SPA mode (SSR gets in the way of a local-first client).
- **Styling:** Tailwind CSS + **shadcn-svelte** (community port of shadcn/ui on Bits UI). Svelte's scoped `<style>` blocks are fine where Tailwind gets awkward. Setup: `npx sv add tailwindcss`, then shadcn-svelte init. shadcn is a starting point; reshape it into Liste's design language.
- **The Rust core (including SQLite and crypto) runs as WASM inside a dedicated Web Worker.** The UI talks to it through a thin message-passing layer; the main thread stays free.
- **WASM bundle size:** build with size optimizations, strip unused code, load the core in parallel with the UI shell so the app appears instantly. This is a budget (Section 4), not a hope.
- **Service worker** for full offline use and instant repeat loads; installable as a PWA.
- Test **Safari** carefully (OPFS availability, storage eviction, WebCrypto key persistence).
- The web app is a Phase 1 client. It is not dropped in favor of native shells. Link-and-go and machines that will not install an app still matter.
- Rejected: Next.js (server-centric complexity a local-first app doesn't need), Rust web UI frameworks like Leptos/Dioxus (JS has better browser APIs, accessibility tooling, drag-and-drop libraries, hiring; Rust stays for logic).

## 12. Backend

- **Rust sync server using Axum**, Postgres via sqlx.
- Responsibilities: op ingest and ordering, snapshot storage, realtime fan-out, authentication, device management, push notification dispatch (encrypted payloads), billing entitlements.
- Shares the sync protocol and data types with the core via the Cargo workspace.
- Stateless server processes; all state in Postgres. Horizontal scaling of the server tier is trivial; the database is the only stateful component.
- The server does not grow an agent API. Agent access is local (Section 13).
- Deployed on Cloudflare Containers behind a Worker, with Hyperdrive to Postgres (Section 19).
- **Push delivery (settled):** the server talks to **APNs, FCM, and WNS directly**, behind one internal `PushSender` trait. No third-party push SaaS: payloads are encrypted anyway, but routing metadata and delivery timing stay under Liste's control, and there is one fewer vendor in the trust chain.
- **Push relay for self-hosters (settled):** store-signed apps can only receive push from Liste's own APNs/FCM/WNS credentials, so a self-hosted server cannot push to them directly. Liste Cloud runs a free **push relay**: a self-hosted server registers once, then forwards `{device push token, encrypted payload}` to the relay, which delivers it. The relay sees a token and ciphertext, nothing else, and is optional; without it, self-hosted clients fall back to background polling. (Bitwarden uses the same design for its self-hosted instances.)
- **One binary, one image.** The same server binary runs Liste Cloud and every self-hosted instance. Cloud-only behavior (billing, the push relay's server side, Sign in with Apple/Google) is enabled by configuration, never by a separate codebase or a license key.

## 13. Local agent interface (settled: Phase 1)

The user's own agent must be able to use Liste without opening a GUI. That interface is local, because the server cannot read task content.

Two small Rust binaries in the Cargo workspace. **Both are thin IPC clients of the host process (Section 3).** They never open the database, never touch the keystore, and never embed the core's logic. If no host is running, they start one.

### CLI
- Binary name: `liste`.
- **User commands (v1 minimum):** `capture` (natural-language string), `search`, `today`, `upcoming`, `complete`, `uncomplete`, `show`.
- **`liste daemon`:** runs the core's `host` module headless, for machines without the GUI (Section 3). This is the one place the CLI binary links the core, and it is a host, not a client.
- **Harness commands** under `liste debug`: inspect the local op log, a materialized row, and the sync cursor; apply a fixture; run a named two-device scenario. Served by the host over IPC. They exist so a developer can see what the core did without clicking through a GUI. Keep them behind `debug` so the user surface stays small.
- Capture returns as soon as the host commits the local transaction. Sync is background, same as the apps.

### MCP server
- A tiny local MCP server. Phase 1.
- **Transport (v1):** stdio. No network listener. A local agent starts it and talks over stdin/stdout; it forwards to the host over IPC.
- **Transport (Phase 2):** MCP streamable HTTP bound to loopback only, protected by a per-install token the app shows in settings, for agent hosts that cannot spawn stdio processes. Never bound to a non-loopback interface.
- **Tools (v1 minimum, all required):**
  - `capture`: natural-language string in, task out. Same parser as the GUI and CLI (it is the host's parser).
  - `search`: local FTS only.
  - `list_today`
  - `list_upcoming`
  - `complete`
  - `uncomplete`
  - `get_task`
  - `update_task`: change title, due date, priority, list, tags, notes by task id. Without this an agent cannot fix a capture it got wrong.
  - `list_lists`: the user's lists/projects, so an agent can target one.
- Tool handlers are thin wrappers over host calls. No second task model, no second date parser, no direct SQL.
- If the store is locked, tools return a `locked` error. They do not prompt for credentials and do not call the server.
- On first connection from a new agent client, returns a one-time notice that the connected agent sees plaintext (Section 7).
- The only bytes that leave the machine are encrypted ops from the host's normal sync path.
- Same crate on macOS, Windows, and Linux. Do not rewrite it per OS.

### What this is not
- Not a cloud MCP endpoint.
- Not the deferred public HTTP API.
- Not a chatbot inside the Liste GUI.
- Not gated behind Pro. Speed of capture includes this path.

iOS does not run the MCP server in v1. Phone capture is widgets, Shortcuts, and the share sheet. A desktop agent and the phone converge through sync.

## 14. Bindings

- **Swift, Kotlin:** official Mozilla **UniFFI**.
- **C# (Windows):** **uniffi-bindgen-cs** by NordSecurity, versioned in lockstep with UniFFI and used in NordSecurity's production apps. **Fallback if it blocks:** expose a thin C ABI from the core and call it via P/Invoke.
- **Linux GUI and `liste daemon`:** none needed; they are Rust and use the core crate directly.
- **CLI and MCP (client mode):** none; they speak the IPC protocol, whose message types are a small Rust crate shared with the core.
- **Web:** `wasm-bindgen` wrapper around the core.
- Bindings are **generated at build time from the current core**; nothing is published or pinned.

## 15. Quality bar

Prose in this document is not enough. The core suite is the definition of done. A core change that does not pass it is wrong, even if the UI looks fine.

### Core suite (required before Phase 1 is "done," and on every core change)
- **Natural-language fixtures.** `call mom tomorrow 5pm #family !high` and the ugly cases: ambiguous dates, missing year, timezone edges, tags and priorities in either order, text that is not a date.
- **Sync simulations.** Two devices offline. Same field edited on both (LWW by `hlc`). Different fields edited on both (both survive). Delete vs edit. Duplicate `(space_id, op_id)` is a no-op. Snapshot plus tail equals full replay. Unknown future op type is stored and skipped, then applied after the client understands it.
- **Crypto fixtures.** Ciphertext and metadata do not contain title, note, or tag substrings. Associated data is `space_id`, op id, and `schema_version`. A session token cannot decrypt. Round-trip through wrap and unwrap of the space key.
- **Recurrence.** "every 2nd Tuesday", "3 days after completion", DST boundaries.
- **Undo.** Inverse op restores the previous materialized state, including order.
- **Host / IPC.** A second process cannot open the store while a host holds it. CLI capture works with the GUI running and with no host running (auto-start, including the 3 s readiness handshake). A locked store returns `locked` over IPC. A protocol-version mismatch returns the restart message. Exactly one process ever pushes to the sync server during a scenario.
- **Background mode (macOS driver).** Launch with `--background`: no window appears, focus is not stolen, the socket is ready, capture works. Close the last window: process survives, menu bar item remains, next CLI call succeeds without a relaunch.

These live next to the core and run in CI without launching a GUI. The `liste debug` harness commands are a valid driver.

### Acceptance scripts (one behavior, every Phase 1 client)
Write each behavior once. macOS, iOS, web, CLI, and MCP each get a thin driver. Phase 2 clients must pass the same scripts before they ship. Minimum set:

1. Natural-language capture creates the right task (date, tag, priority, list).
2. The create is visible immediately while offline.
3. After sync, another device shows it.
4. A per-field conflict converges to the merge rule, not to a duplicated task.
5. Undo restores the previous state.
6. Search returns on the first keystroke offline, including against the large fixture.
7. Complete via CLI or MCP shows completed in the GUI after sync, and the reverse.
8. A captured title does not appear in what the server stored.

A client is not done because a screen looks plausible. It is done when its driver passes these scripts.

### Performance benches
Fail on regression against Section 4: quick-capture appearance, local insert, keystroke search on the 50,000-task fixture, UI thread not blocked, WASM core loaded beside the shell rather than in front of it, CLI/MCP round trip to a running host.

## 16. Repository structure (one monorepo)

```
liste/
├── README.md        # What Liste is, status, how to build, how to self-host, license
├── LICENSE          # AGPL-3.0-only
├── CONTRIBUTING.md  # Engineering non-negotiables, CLA, how to contribute. Points at docs/ARCHITECTURE.md.
├── SECURITY.md      # Responsible disclosure for an E2EE product; PGP key; what is in scope
├── TRADEMARK.md     # Name and logo policy for forks and self-hosters
├── CODE_OF_CONDUCT.md
├── core/            # Rust: data model, SQLite, sync, crypto, search, parsing, recurrence, undo, importers, host
│   └── tests/       # Parse, sync, crypto, recurrence, undo, host fixtures (Section 15)
├── ipc/             # Rust: IPC message types + client, shared by core (host side), cli, mcp
├── bindings/        # UniFFI config, uniffi-bindgen-cs config, generated bindings, WASM wrapper
├── server/          # Rust sync server (Axum + sqlx)
├── cli/             # Phase 1. `liste` CLI (IPC client) + `liste daemon` (headless host)
├── mcp/             # Phase 1. Local MCP server (stdio ↔ IPC client)
├── apps/
│   ├── apple/       # iOS + macOS (one Xcode project, shared Swift code)
│   ├── android/     # Kotlin + Jetpack Compose
│   ├── windows/     # C# + WinUI 3
│   ├── linux/       # Rust + GTK4 + libadwaita
│   └── web/         # SvelteKit
├── design/          # Shared design tokens (colors, spacing, radii, type scale), exported per platform
├── infra/           # Liste Cloud deployment (Section 19): wrangler configs, R2/appcast tooling, migrations runner
├── deploy/
│   └── self-host/   # Dockerfile (server), docker-compose.yml, Coolify template, .env.example, docs
├── .github/
│   └── workflows/   # Path-filtered CI + release pipelines per channel (Section 19)
├── justfile         # Root task runner (e.g. `just build-core`, `just web-dev`, `just cli`, `just mcp`)
└── Cargo.toml       # Cargo workspace: core, ipc, bindings, server, cli, mcp, apps/linux
```

### Why a monorepo
- **Atomic changes:** a change to the data model, sync protocol, or IPC protocol, plus matching updates to bindings, CLI, MCP, and all clients, lands in one PR. Every commit is a consistent state of the whole system.
- Bindings are generated from the current core at build time.
- Avoids clients running mismatched core versions.

### Monorepo rules
- **Path-filtered CI:** each app's pipeline runs only when its folder, the core, the IPC crate, or the bindings change. Core suite runs on every core change.
- **Each platform keeps its native tooling** (Xcode, Gradle, Visual Studio/MSBuild, Cargo, pnpm). Do not force a single build system over everything.
- A simple root task runner (`just`) provides common commands.

### `CONTRIBUTING.md`
Required at the repo root. It must state, in the first file any contributor reads:

- Business rules, merge rules, crypto, parsing, recurrence, importers, and undo go in `core`. If you are about to encode any of those in Swift, Kotlin, C#, Svelte, the CLI, or an MCP tool handler, stop.
- On desktop, only the host process opens SQLite, touches the keystore, or syncs. CLI and MCP are IPC clients. They do not open the database or embed core logic.
- The desktop app must run windowless when launched with `--background`, must not quit when the last window closes, and must expose its IPC socket only after the store is open.
- Never weaken E2EE. No plaintext task content from a Liste client to the server. No cloud agent API.
- Phase 1 is macOS, iOS, web, CLI, and MCP. Do not start Android, Windows, or the Linux GUI until asked.
- Do not build collaboration UI or a public HTTP API in v1.
- A core change that fails the Section 15 suite is not done. A client path is not done until its acceptance driver is green.
- Deployment of Liste Cloud is Cloudflare for web, server, downloads, and DNS (Section 19). Do not add another hosting provider for Cloud. Do not hand-deploy; every release goes through the tagged pipeline.
- Every feature must work on a self-hosted instance unless it is on the explicit Cloud-only list in Section 18. Nothing in the server depends on Cloudflare-specific APIs.
- Contributions require the CLA (Section 18). No code enters `main` without it.

## 17. Design system

- Liste has **its own design language** that adapts to each platform's conventions rather than looking identical everywhere.
- **Design tokens** live in `design/` and are exported to each platform's format (Tailwind theme variables on web; Swift/Kotlin/XAML/GTK CSS resources on native).
- Do not force the six GUIs to share a component implementation so they look the same. Shared tokens, platform idioms. The thing that must not drift is behavior (Section 15), not pixels.
- Accessibility is a requirement on every platform (screen readers, keyboard navigation, dynamic type, contrast).

## 18. Open source, self-hosting, and business model (settled)

### The model in one paragraph
Liste's code is open source under the **AGPL-3.0**. Anyone can read it, build it, run their own server, and use the native apps against that server, with every feature. **Liste Cloud** is the hosted service run by the maintainers: it is what most people will use, and it is what pays for development. Revenue comes from convenience (hosting, accounts, push, updates, stores) and from Pro on Cloud, never from withholding code. This is the Bitwarden / Plausible / Cal.com model, chosen because it is the one that has repeatedly worked for small teams and solo developers with an E2EE or privacy-focused product.

### Why open source from the first commit
- **E2EE is only credible when it is verifiable.** "Trust us" is a weaker claim than "read `core/`." Security researchers and auditors can inspect the crypto and sync code that runs on users' devices.
- **Self-hosting is a feature users pay attention to** even when they never use it: it is the guarantee that their data and workflow survive the company.
- **Outside review of a small team's code** finds bugs the team would miss, which matters when much of the UI is thin and the core is where mistakes cost data.
- **Opening later is always harder** than opening now: history has to be clean, secrets never committed, contributors' terms settled. Starting open makes all of that the default.

### License
- **AGPL-3.0-only** for the entire repository: core, all six clients, CLI, MCP server, sync server, deploy tooling. One license, no split.
- **Why AGPL** and not MIT/Apache: the value Liste sells is a hosted service. AGPL's network clause means anyone who runs a modified Liste as a service must publish their modifications, which prevents a competitor from taking the code, adding closed features, and hosting it against Liste Cloud. MIT/Apache would allow exactly that. Why not a source-available license (BSL, FSL, SSPL): those are not open source, lose the trust and contribution benefits above, and Liste's moat is the hosted service, not the code, so the extra protection buys little.
- **Brand assets are excluded:** the name "Liste," the logo, and the app icons are trademarks, not licensed under the AGPL. `TRADEMARK.md` states the policy: forks and self-hosted instances may say "powered by Liste" or "Liste (self-hosted)," but a redistributed or rebranded fork must not use the name or icon as its own product identity, and must not present itself as the official app in any store.

### Contributor License Agreement (required)
- Every outside contribution is accepted only under a **CLA**, enforced by the **cla-assistant** GitHub app on pull requests (one-time click per contributor), granting the maintainers the right to relicense the contribution. Not a copyright assignment; a license grant.
- **Why this is necessary, not optional:** the App Store and Google Play terms are widely considered incompatible with the (A)GPL for third-party code. As copyright holder, Liste's maintainers can distribute their own AGPL code through the stores under the stores' terms; they cannot do that for contributors' AGPL code without a grant. The CLA is what makes the store builds legally clean. It also keeps a future license change possible if the ecosystem shifts, without tracking down every past contributor.
- No separate DCO sign-off requirement; the CLA covers provenance and one signature step is enough friction.

### What self-hosters get
- **Everything.** All features, all clients, unlimited lists, collaboration when it ships. No license keys, no feature flags gated on payment, no "community edition."
- The native apps from the stores connect to any server: the sign-in screen has a "Use a self-hosted server" option that takes a URL. The apps are the same binaries Liste Cloud users run.
- A **single Docker image** for the server plus PostgreSQL, a `docker-compose.yml`, a **Coolify** template (and the same compose works on any Docker host: Dokploy, Portainer, a VPS), and a documented `.env.example`. Self-hosting must be a 15-minute job for someone who has run Docker before.
- The web app as a second image (SvelteKit `adapter-node`), or served by the same compose. The marketing pages are not part of the self-host image; only the app shell.
- The free push relay (Section 12), optional.
- Migrations run automatically on container start for self-hosters (Cloud runs them from CI, Section 19). Every release notes any migration that needs attention.

### What is Cloud-only (the complete list)
1. Billing and Pro entitlements (Stripe, store IAP).
2. Sign in with Apple and Google.
3. Native-app passkeys (Section 8; a self-hoster's domain cannot vouch for Liste's app identifiers).
4. The push relay's server side (self-hosters use it; they do not run it).
5. Future third-party inbound channels that require Liste's domains (e.g. an `@<project-domain>` email-to-task address). A self-hoster can point their own inbound domain at their instance.

Anything not on this list must work self-hosted. Adding to this list is a Section 18 change and needs a written reason.

### Revenue
- **Liste Cloud Pro subscription** (primary): **$48/year** (shown as $4/month) or **$6 month-to-month**, 14-day trial, regional pricing via the Stripe and store price tiers.
- **Cloud free tier:** the full app on every platform, sync across devices, E2EE, local CLI, local MCP server, one personal space, up to 5 lists/projects, unlimited tasks. Generous enough that most individuals never need Pro, which is deliberate: those users are the funnel, the reviews, and the contributors.
- **Cloud Pro:** unlimited lists, attachments/file storage, shared spaces and collaboration (when shipped), calendar integrations, unlimited saved filters, `@<project-domain>` inbound channels (when shipped), priority support.
- **GitHub Sponsors / Open Collective** for people who self-host and want to fund the project anyway. Linked from the README and the self-host docs, never nagged in the product.
- **Later, if demand appears:** paid self-hosting support or managed instances for teams. Not v1.
- **Billing plumbing:** Stripe on web; App Store and Google Play in-app purchase where store rules require it. Direct-download desktop builds use the web checkout. Entitlement is stored on the account (`accounts.plan`) so it applies on every platform regardless of where it was purchased.
- **Never** gate speed, sync, encryption, the CLI, the MCP server, or self-hosting behind Pro. Those are the product's promise.

### Why free self-hosting does not undercut Cloud
Most people do not want to run a server, keep it patched, manage backups, and debug push notifications; they want the app to work. Bitwarden's business grew while its server was free to self-host and Vaultwarden, a third-party reimplementation, was popular. Self-hosters are a small, loyal, technically skilled group who file the best bug reports and contribute code. The right posture is to make them successful, not to tax them.

### Community surface
- The code lives in a **GitHub organization owned by the maintainer**, not a personal account, so the project, its container images, its sponsors profile, and its future companion repositories (Coolify template, Flathub manifest, Homebrew tap) have one stable home that can gain maintainers without moving. Public repository, issues, discussions, and a public roadmap from day one.
- `SECURITY.md` names a private disclosure channel (GitHub private vulnerability reporting, enabled on the repository, plus a security email) and a public PGP key. An E2EE product with no disclosure path is not credible.
- `CODE_OF_CONDUCT.md` is the Contributor Covenant, unmodified.
- Funding page: GitHub Sponsors on the organization, linked from `README.md` and `FUNDING.yml`.
- `CONTRIBUTING.md` states plainly that the settled decisions in this document are not reopened in issues. Open projects attract endless "why not Flutter" threads; the answer is already written.
- Pre-1.0, the repository is public but the README says clearly: **early development, not usable yet, do not self-host in production.** Being open early is fine; pretending it is ready is not.

## 19. Deployment and operations (settled)

This section covers **Liste Cloud**, the maintainers' hosted service. Self-hosting is covered at the end of the section and in `deploy/self-host/`. Everything server-side and every download for Liste Cloud lives on **Cloudflare**. Native apps ship through their platform stores plus Liste's own download channel. One Git tag produces every artifact for every channel.

### Environments
- Three environments: `dev` (local: `wrangler dev`, a local Postgres, the server in Docker), `staging`, and `production`. Wrangler environments for the Worker-side pieces; separate PlanetScale branches/databases for staging and production.
- Every pull request gets a preview deployment of the web app (Workers preview URL) against staging.
- Secrets live in Cloudflare (Wrangler secrets / Secrets Store) for runtime and in GitHub Actions encrypted secrets for build-time signing material. Nothing secret is ever committed, including in `infra/`.

### Web app: Cloudflare Workers
- SvelteKit with `@sveltejs/adapter-cloudflare`. Prerendered marketing/docs/pricing pages and the SPA app shell are served as Workers static assets on the edge.
- **Caching:** the WASM core, the worker bundle, and all JS/CSS get content-hashed filenames with `Cache-Control: public, max-age=31536000, immutable`. `index.html` and the service worker are `no-cache`, so a deploy is atomic and instant for returning users.
- **WASM delivery:** serve `.wasm` as `application/wasm` so browsers stream-compile it. Precompress with Brotli. The core loads in parallel with the shell (Section 4 budget).
- **No cross-origin isolation required.** The OPFS sahpool VFS does not use SharedArrayBuffer, so no COOP/COEP headers. Do not add them; they break third-party embeds and fonts for no gain.
- Domains: `<project-domain>` (marketing + app), `api.<project-domain>` (sync), `downloads.<project-domain>` (R2).

### Sync server: Cloudflare Containers, unchanged Axum
- The Axum server from Section 12 runs **as-is** in a Cloudflare Container (Docker image built in CI; multi-stage build, distroless or scratch base, statically linked). No Cloudflare-specific code inside the server: if Liste ever leaves Cloudflare, the same image runs anywhere.
- A thin **Worker** in front handles routing to the container, TLS termination, rate limiting, and WebSocket upgrade pass-through. It contains no business logic.
- **Hyperdrive** provides pooled, cached connections from the container/Worker layer to PlanetScale Postgres. sqlx connects to the Hyperdrive connection string.
- **Placement:** pin the container-owning Durable Objects with a location hint in the same region as the Postgres primary. Every sync request touches the database, so container-to-database latency dominates; edge proximity to the user does not.
- **Warm pool:** keep a minimum number of instances warm (start with 2 per environment) so the first sync after a quiet period does not pay a container cold start. Scale by request volume.
- **Realtime (v1):** WebSocket/SSE fan-out is served by the container itself, one codebase. **Revisit (Section 20):** if idle-connection cost becomes material, move fan-out to one Durable Object per space using the WebSocket Hibernation API; the DO relays, the container keeps ingest and ordering, and the per-space DO maps 1:1 onto the per-space shard key.
- **Rejected:** replacing Axum + Postgres with a Durable Object per space as sequencer and store. It fits the op-log model well and would make Neki unnecessary, but it trades a portable server and standard Postgres for full runtime lock-in and a less mature Rust runtime. Containers give Liste Cloudflare's network without that bet.

### Database operations
- PlanetScale Postgres (Section 10), production in one region, with a read replica added before any sharding discussion.
- Migrations run from CI via `sqlx migrate` against staging on every merge to main and against production as an explicit, manual-approval step in the release pipeline. Migrations are forward-only and must be backward compatible with the currently deployed server for one release (expand/contract pattern), because containers roll gradually.
- Backups: PlanetScale automated backups plus a nightly logical dump to a private R2 bucket in a different account/region. Restore is rehearsed quarterly against staging. Data on the server is ciphertext, so a backup leak is not a content leak, but it is still treated as sensitive metadata.

### Downloads and updates: R2
- All desktop artifacts live in an R2 bucket served at `downloads.<project-domain>` behind Cloudflare's CDN: macOS DMGs and the Sparkle appcast, Windows MSIX bundles and the `.appinstaller` manifest, Linux AppImages. R2 has no egress fees, which matters at update-volume scale.
- Stable "latest" URLs (`/mac/latest.dmg`, `/win/Liste.appinstaller`, `/linux/latest.AppImage`) are served by a tiny Worker that redirects to the current versioned object, so links on the site never rot.
- Every artifact is versioned, immutable, and never overwritten. Roll back by pointing "latest" at the previous version.

### DNS, email, webhooks, push
- **DNS:** move the domain's nameservers to Cloudflare so site, API, and downloads share one control plane. GoDaddy stays as registrar (transfer later if convenient). Enable Cloudflare's proxy on everything except the raw R2 origin.
- **Outbound email** (one-time sign-in codes, receipts): a transactional provider (Postmark or Resend), called from the server. Revisit if Cloudflare's own email-sending product is generally available and equivalent (Section 20).
- **Inbound email** (future email-to-task, Section 7): Cloudflare Email Workers receive mail, encrypt in memory with the space's public key, and forward ciphertext to the server. Not v1.
- **Webhooks:** Stripe and store server-notifications (App Store Server Notifications, Google Play RTDN) land on small Workers that verify signatures and forward to the server's entitlement endpoint.
- **Push:** APNs, FCM, and WNS directly from the server (Section 12). Credentials are Cloudflare secrets.

### Observability
- **Clients:** Sentry SDKs in every native app and the web app, with crash reports scrubbed of task content (never send titles, notes, tags, or op payloads in breadcrumbs).
- **Server:** structured JSON logs from Axum, shipped via Cloudflare Logpush; Workers Analytics for the edge layer; one external uptime check on `api.<project-domain>/health`.
- **Metrics that matter:** sync ingest latency p50/p99, op-log growth per space, snapshot age, container cold-start rate, WebSocket connection count, failed push deliveries.

### CI/CD: GitHub Actions
- One workflow set at `.github/workflows/`. Path-filtered (Section 16). macOS runners build the Apple targets; ubuntu runners build core, server, web, Linux, and Android; windows runners build the Windows app.
- **Release process:** a version tag (`vX.Y.Z`) on `main` triggers every channel from the same commit. No hand-built artifacts, ever.

### Per-platform release pipelines
| Channel | Pipeline |
|---|---|
| **macOS direct (primary)** | Xcode archive → codesign with Developer ID → `notarytool` submit and staple → DMG → upload to R2 → regenerate Sparkle appcast (Ed25519-signed) → update `latest` redirect. Sparkle updates in-app. |
| **macOS App Store** | Same archive, App Store signing, App Group entitlement → fastlane → TestFlight (beta) → App Store Connect (release). |
| **iOS** | fastlane → TestFlight → App Store Connect. Beta testers via TestFlight groups. |
| **Windows direct (primary)** | MSIX bundle → sign with **Azure Trusted Signing** → upload MSIX + `.appinstaller` to R2. App Installer auto-updates on launch; no custom updater. |
| **Microsoft Store** | Same MSIX → Partner Center via the Store submission action. |
| **Android** | Gradle Play Publisher (or fastlane `supply`) → Play Console tracks: internal → closed → open → production. Play App Signing holds the release key; the upload key is a CI secret. |
| **Linux** | Flathub: manifest in the Flathub repo, updated by PR on each tag (bot-assisted). AppImage built in CI and uploaded to R2. The CLI and MCP binaries ship inside the Flatpak with exported wrappers. |
| **Web** | `wrangler deploy` to staging on every merge to `main`; to production on tag after the server deploy succeeds. |
| **Server** | Docker image built and pushed on tag → migrations to production (manual approval) → `wrangler deploy` of the container-backed Worker with gradual rollout → smoke test against `/health` and one authenticated sync round-trip. |

### Order of operations on a release
1. Tag → build all artifacts in parallel.
2. Server: migrate (expand only), deploy container, smoke test.
3. Web: deploy production.
4. Desktop and mobile: upload to stores and R2; publish appcast/appinstaller last so clients only see an update once the server that supports it is live.
5. Client versions must tolerate one server version of skew in each direction (Section 6 schema evolution makes this natural).

### Cloudflare tooling
The Cloudflare CLI (`wrangler`) and Cloudflare's documentation are the reference for creating R2 buckets, deploying Workers, and inspecting logs. **Check the current documentation before relying on any limit or API.** Cloudflare's platform moves quickly; verify, then build.

### Self-hosting (the same artifacts, no Cloudflare)
- **Images:** `ghcr.io/<organization>/liste-server` and `ghcr.io/<organization>/liste-web` (the project's GitHub organization), built by the same tagged release pipeline that deploys Cloud, published to the GitHub Container Registry, multi-arch (amd64, arm64). The server image is the exact image Cloud runs.
- **Compose:** `deploy/self-host/docker-compose.yml` with three services: `server`, `web`, `postgres` (with a persistent volume). Reverse proxy and TLS are the host's job (Coolify, Caddy, Traefik, or a tunnel), documented with examples.
- **Coolify:** a one-click template in `deploy/self-host/coolify/`, and a PR to Coolify's service catalog once 1.0 ships. The template is just the compose file plus sane defaults.
- **Configuration:** environment variables only, documented in `.env.example`: database URL, public base URL, email provider credentials (for sign-in codes), optional push-relay registration, optional S3-compatible storage for attachments. No config file formats to learn.
- **Requirements in the server:** no dependency on Cloudflare bindings, Hyperdrive, Workers, or R2. Hyperdrive is just a Postgres connection string to the server; a self-hoster passes a plain one. Realtime is served by the server process itself (Section 19, realtime v1), which is why fan-out is not moved into Durable Objects by default.
- **Migrations:** the server runs pending migrations on start when `LISTE_AUTO_MIGRATE=true` (the compose default). Cloud sets it false and migrates from CI.
- **Backups:** the self-host docs show a one-line `pg_dump` cron and state plainly that a lost database with no backup is unrecoverable, and that attachments live outside Postgres.
- **Upgrades:** pull the new tag, restart. Every release's notes list breaking configuration changes, if any. The server refuses to start against a database newer than itself, with a clear message.
- **Web app for self-hosters:** the `liste-web` image serves the SPA shell with `adapter-node` and points at the server URL from one environment variable. The Cloudflare build (`adapter-cloudflare`) and the self-host build come from the same SvelteKit source with two adapters; nothing in the app code is adapter-specific.

## 20. Settled decisions with a scheduled revisit

Everything here is decided and in force. Each has one named revisit trigger. Until that trigger fires, build to the decision as written.

| Decision | In force now | Revisit when |
|---|---|---|
| Pro pricing | $48/year or $6/month, 14-day trial | Beta conversion data clearly contradicts it |
| Tombstone and op retention | 90 days after the second-latest snapshot | Measured op-log growth in beta; may shorten the snapshot cadence, not weaken the merge rules |
| Notes text CRDT | Whole-field LWW; Loro designated for the upgrade | Collaboration ships, or measured note-conflict complaints |
| MCP transport | stdio only | Phase 2 adds loopback streamable HTTP with a per-install token |
| Extra MCP tools | The v1 set in Section 13 | A real Phase 1 workflow is awkward without one; never delay the server for it |
| Neki | Standard managed Postgres | Neki is GA **and** a Section 10 trigger is hit |
| Linux GUI | Not started | Phase 3, when asked |
| Realtime fan-out | Served by the sync container | Idle WebSocket cost becomes material; move to one Durable Object per space with hibernation |
| Outbound email provider | Postmark or Resend | Cloudflare email sending is GA and equivalent |
| Container warm pool | 2 per environment | Cold-start rate metric above 1% of syncs |
| License | AGPL-3.0-only, CLA for contributions | Never planned to change; the CLA exists so a change stays possible if the ecosystem forces one |
| Paid self-host support / managed instances | Not offered | Sustained inbound demand after 1.0 |
| Coolify catalog listing | Template in repo only | Submit after 1.0 |

## 21. Engineering rules

- Optimize for **speed** first, then correctness of sync and crypto, then everything else.
- Keep logic in the Rust core and presentation in native code. If you're about to write business logic in a UI layer, the CLI, or an MCP handler, stop and put it in the core.
- CLI and MCP are thin IPC clients of the host. A tool that reimplements parsing, opens SQLite, or reads a key is a bug.
- The desktop app is a background-capable host from its first build. Windowless launch, menu bar/tray presence, launch-at-login, and the readiness handshake are Phase 1 work, not polish.
- Phase 1 scope is macOS, iOS, web, the local CLI, and the local MCP server. Do not start Android, Windows, or the Linux GUI until asked, but keep the core and bindings ready for them.
- Every server schema change must respect the shard-ready rules in Section 10.
- Every op format change must respect the schema-evolution rule in Section 6 (old ops must remain applicable forever).
- Never weaken E2EE for a feature, including agent access. If a feature needs plaintext on the server from a Liste client, redesign the feature. Third-party inbound plaintext follows the rule in Section 7.
- Do not pull collaboration or a public HTTP API into v1. The data model is already ready. That is enough.
- Prefer each platform's idioms over forced visual consistency across platforms. Prefer one shared behavior suite over six hand-waved "it works on my machine" checks.
- A change to the core is not done until the Section 15 suite is green. A new or changed client path is not done until its acceptance driver is green.
- Read crypto, sync, and host/IPC code. Do not accept "it compiled and the app launched" as review.
- Deployment of Liste Cloud is Section 19. Cloudflare for web, server, downloads, DNS, and webhooks; stores plus R2 for native apps; one tag releases everything, including the self-host images. Do not introduce another hosting provider for Cloud or a hand-deploy step.
- Everything works self-hosted unless it is on the Cloud-only list in Section 18. The server never imports a Cloudflare-specific API. If a feature only works on Cloud and is not on that list, it is a bug.
- The repository is public. Never commit secrets, credentials, customer data, or internal URLs. Assume every commit is read by strangers the moment it is pushed.
- Never put task content in logs, crash reports, or metrics. The server sees ciphertext; observability must too.
- Verify current library versions and APIs before relying on them. The ecosystem (UniFFI, uniffi-bindgen-cs, WinUI 3, SvelteKit, sqlite-wasm-rs, shadcn-svelte, PlanetScale Neki, MCP, Cloudflare Workers/Containers/Hyperdrive/R2) evolves quickly.
