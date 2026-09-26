# Contributing to Liste

Thank you for your interest in Liste. This file is the first thing to read before writing code. It restates the non-negotiable rules from [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md), which is the complete architecture and decision record. If this file and that document ever disagree, the architecture document wins and this file is corrected in the same change.

## Settled decisions are settled

Every decision in the architecture document is final unless the maintainers explicitly reopen it. Section 20 lists the decisions that have a scheduled revisit trigger; until that trigger fires, they are settled too.

Please do not open issues or discussions proposing a different UI framework, a different sync model, a different database, a different license, a different hosting provider, or a different launch order. Those questions have been answered, and the reasoning is written down. Issues that reopen a settled decision will be closed with a pointer to the relevant section.

If you believe something in the document is impossible as written, open an issue with the concrete reason and the smallest change that would fix it. That is welcome.

## The non-negotiables

1. **Logic lives in the core.** Business rules, merge rules, cryptography, natural-language parsing, recurrence, importers, and undo go in `core/`. If you are about to encode any of those in Swift, Kotlin, C#, Svelte, the CLI, or an MCP tool handler, stop and put it in the core.
2. **One host per device on desktop.** Only the host process opens SQLite, touches the platform keystore, or syncs. The CLI and the MCP server are IPC clients. They do not open the database and do not embed core logic. A tool that reimplements parsing, opens SQLite, or reads a key is a bug.
3. **The desktop app is a background-capable host from its first build.** It must run windowless when launched with `--background`, must not quit when the last window closes, and must expose its IPC socket only after the store is open. This is Phase 1 work, not polish.
4. **Never weaken end-to-end encryption.** No plaintext task content ever travels from a Liste client to the server. There is no cloud agent API. If a feature needs plaintext on the server from a Liste client, redesign the feature. Use the audited RustCrypto crates; never implement a primitive by hand.
5. **Phase 1 is macOS, iOS, web, the CLI, and the MCP server.** Do not start Android, Windows, or the Linux GUI until asked, however cheap another shell looks.
6. **No collaboration UI and no public HTTP API in v1.** The data model is already designed for both. That is enough.
7. **The Section 15 suite is the definition of done.** A core change that fails it is not done. A new or changed client path is not done until its acceptance driver is green. "It compiled and the app launched" is not a review.
8. **Liste Cloud deploys on Cloudflare** for web, server, downloads, DNS, and webhooks. Do not add another hosting provider for Cloud. Do not hand-deploy; every release goes through the tagged pipeline.
9. **Everything works self-hosted** unless it is on the explicit Cloud-only list in Section 18. The server never imports a Cloudflare-specific API. A feature that only works on Cloud and is not on that list is a bug.
10. **Every op format change must keep old ops applicable forever**, and every server schema change must respect the shard-ready rules in Section 10.
11. **The repository is public.** Never commit secrets, credentials, customer data, or internal URLs. Never put task content in logs, crash reports, or metrics.
12. **Contributions require the CLA.** No code enters `main` without it.

## Contributor License Agreement

Every outside contribution is accepted only under a Contributor License Agreement. It is a license grant, not a copyright assignment: you keep your copyright and grant the maintainers the right to relicense your contribution.

This is necessary, not optional. The App Store and Google Play terms are widely considered incompatible with the AGPL for third-party code. As copyright holder, the maintainers can distribute their own AGPL code through the stores; they cannot do that for contributors' code without a grant. The CLA is what keeps the store builds legally clean.

The CLA is enforced by the cla-assistant GitHub app. When you open your first pull request, a bot will ask you to sign it once. There is no separate DCO sign-off requirement.

## How to contribute

1. **Look for an issue first.** Bug reports and small fixes are welcome at any time. For anything larger, open an issue describing the change before writing code so it can be checked against the architecture document.
2. **Read the relevant sections** of the architecture document for the area you are touching. Sync is Section 6, encryption is Section 7, the host and IPC model is Section 3, the CLI and MCP server are Section 13, and the quality bar is Section 15.
3. **Fork and branch** from `main`.
4. **Write tests.** Core changes must extend or pass the suites in `core/tests/`. Changes to crypto, sync, or host/IPC code are reviewed by reading them, so keep them small and well described.
5. **Run the checks locally** before opening a pull request:

   ```
   just lint
   just test
   ```

6. **Open a pull request** with a clear description of what changed and why. Use conventional commit messages (`feat:`, `fix:`, `docs:`, `test:`, `build:`, `ci:`, `chore:`). Sign the CLA when the bot asks.
7. **Expect review to read the code.** Crypto, sync, and IPC changes in particular are reviewed line by line.

## Code layout

See Section 16 of the architecture document for the full monorepo layout. In short: `core/`, `ipc/`, `bindings/`, `server/`, `cli/`, and `mcp/` are Rust crates in one Cargo workspace; `apps/` holds one folder per platform with that platform's native tooling; `design/`, `infra/`, and `deploy/` hold tokens, Cloud deployment tooling, and self-hosting files.

CI is path-filtered: each app's pipeline runs only when its folder, the core, the IPC crate, or the bindings change. The core suite runs on every core change.

## Style

- Rust: `rustfmt` defaults and `clippy` with no warnings.
- Each platform app follows that platform's conventions and formatter.
- Commit messages describe the change, not the tooling used to make it.

## Code of conduct

This project follows the [Contributor Covenant](CODE_OF_CONDUCT.md). By participating you agree to abide by it.

## Security

Do not open public issues for vulnerabilities. See [SECURITY.md](SECURITY.md).
