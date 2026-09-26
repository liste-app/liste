# Liste

Liste is a to-do app. It is designed around one goal: capture, interaction, search, and sync should all feel instant. That goal is expressed as measured budgets in the architecture document, not as a claim about being faster than anything else.

Liste is local-first, end-to-end encrypted, open source under the AGPL-3.0, and self-hostable. There is one Rust core (data model, SQLite, sync, encryption, search, natural-language parsing, recurrence, undo, importers) and a fully native app on each platform. A local command-line tool and a local MCP server let your own scripts and agents use Liste without a GUI and without the server ever seeing plaintext.

## Status

**Early development. Liste is not usable yet.**

Nothing here is ready for real tasks. The sync protocol, encryption, and data formats will change without migration paths until the first release. **Do not self-host Liste in production.** The self-hosting files in `deploy/self-host/` exist so the pieces are built together from the start, not because they work today.

The full design and decision record is in [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md). The decisions in it are settled; see [CONTRIBUTING.md](CONTRIBUTING.md) before opening an issue about one.

## Platforms

| Platform | Status |
|---|---|
| macOS (SwiftUI, direct download and App Store) | Phase 1, in development |
| iOS (SwiftUI) | Phase 1, in development |
| Web (SvelteKit, installable PWA) | Phase 1, in development |
| Local CLI (`liste`) | Phase 1, in development |
| Local MCP server (`liste-mcp`) | Phase 1, in development |
| Android (Jetpack Compose) | Phase 2, not started |
| Windows (WinUI 3) | Phase 2, not started |
| Linux (GTK4 + libadwaita) | Phase 3, not started |

## Repository layout

```
core/       Rust core: data model, SQLite, sync, crypto, search, parsing, recurrence, undo, importers, host
ipc/        IPC message types and client shared by the host, the CLI, and the MCP server
bindings/   UniFFI and uniffi-bindgen-cs configuration, generated bindings, WASM wrapper
server/     Sync server (Axum + sqlx + PostgreSQL)
cli/        `liste` command-line tool and `liste daemon` headless host
mcp/        Local MCP server (stdio)
apps/       Native apps and the web app, one folder per platform
design/     Shared design tokens, exported per platform
infra/      Liste Cloud deployment tooling (no secrets)
deploy/     Self-hosting: Dockerfile, docker-compose, Coolify template
```

## Building

Requirements: a current stable Rust toolchain and [`just`](https://github.com/casey/just). Each platform app keeps its own native tooling (Xcode, Gradle, Visual Studio, pnpm); see the README in each `apps/` folder.

```
just build      # build the Rust workspace
just test       # run the core suite and all crate tests
just lint       # rustfmt and clippy
```

The core test suite in `core/tests/` is the definition of done for core changes. Cases that are specified but not yet implemented are marked ignored; `just test-pending` runs them and shows what is still missing.

## Self-hosting

Not yet. When Liste reaches its first release, `deploy/self-host/` will contain a single server image, a `docker-compose.yml`, a Coolify template, and a documented `.env.example`. Every feature works on a self-hosted instance except the short Cloud-only list in Section 18 of the architecture document.

## Liste Cloud

Liste Cloud is the hosted service run by the maintainers and is what funds development. Self-hosters get every feature; Cloud sells convenience. If you self-host and want to support the project anyway, see [GitHub Sponsors](https://github.com/sponsors/liste-app).

## Support

Questions go to [GitHub Discussions](https://github.com/liste-app/liste/discussions); bugs go to [Issues](https://github.com/liste-app/liste/issues).

## Security

Liste is end-to-end encrypted. Please report vulnerabilities privately as described in [SECURITY.md](SECURITY.md).

## License

Liste is licensed under the [AGPL-3.0-only](LICENSE). The name "Liste", the logo, and the app icons are trademarks and are not covered by that license; see [TRADEMARK.md](TRADEMARK.md).

Contributions are accepted under a Contributor License Agreement; see [CONTRIBUTING.md](CONTRIBUTING.md).
