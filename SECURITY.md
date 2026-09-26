# Security Policy

Liste is an end-to-end encrypted product. A security issue in the core, the sync protocol, or the server can put users' data at risk, so we take reports seriously and want to hear about them privately first.

## Reporting a vulnerability

Please do not open a public issue for anything security-related.

Use **GitHub private vulnerability reporting** on this repository (Security tab, "Report a vulnerability").

Include enough detail to reproduce the issue: affected component, version or commit, steps, and impact. If you have a proof of concept, attach it.

We will acknowledge a report within three working days, keep you informed as we investigate, and credit you in the release notes when the fix ships unless you ask otherwise.

## Scope

In scope:

- The Rust core (`core/`): encryption, key handling, sync merge logic, the op log, the IPC host.
- The IPC protocol and the CLI and MCP clients (`ipc/`, `cli/`, `mcp/`).
- The sync server (`server/`): authentication, session handling, op ingest, anything that could expose plaintext or metadata beyond what Section 7 of the architecture document permits.
- The native apps and the web app (`apps/`), including key storage on each platform.
- The self-hosting images and compose files (`deploy/`).

Out of scope:

- **Plaintext visible to software running on the user's own device.** By design, an agent the user connects to the local MCP server sees plaintext, and the local materialized SQLite tables are plaintext protected by OS disk encryption and the platform keystore. Reports that amount to "a local process can read local data" are not vulnerabilities.
- Denial of service against Liste Cloud, rate-limit testing, or automated scanning of production infrastructure.
- Issues in third-party services (Cloudflare, PlanetScale, Apple, Google, Microsoft) that are not caused by Liste's use of them.
- Social engineering of maintainers or users.

## Safe harbor

If you make a good-faith effort to follow this policy, we will not pursue legal action against you for your research. Please avoid accessing, modifying, or deleting data that is not your own, and do not test against other users' accounts on Liste Cloud. Use a self-hosted instance or your own accounts.

## Supported versions

Liste is in early development and has no supported releases yet. Once releases exist, this section will list which versions receive security fixes.
