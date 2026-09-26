# Root task runner for the Liste monorepo. Each platform app keeps its own
# native tooling; these recipes cover the Rust workspace and the common
# cross-cutting commands (Section 16 of docs/ARCHITECTURE.md).

set shell := ["bash", "-euo", "pipefail", "-c"]

default:
    @just --list

# Build the whole Rust workspace.
build:
    cargo build --workspace --all-targets

# Build only the core.
build-core:
    cargo build -p liste-core

# Run every test in the workspace, including the core suite in core/tests/.
test:
    cargo test --workspace --no-fail-fast

# Run only the core suite (Section 15).
test-core:
    cargo test -p liste-core --no-fail-fast

# Format check and clippy with warnings denied.
lint:
    cargo fmt --all --check
    cargo clippy --workspace --all-targets --all-features -- -D warnings

# Format the Rust workspace.
fmt:
    cargo fmt --all

# Run the CLI. Example: just cli capture "call mom tomorrow 5pm"
cli *ARGS:
    cargo run -q -p liste-cli -- {{ARGS}}

# Run the local MCP server on stdio.
mcp:
    cargo run -q -p liste-mcp

# Run the headless host.
daemon:
    cargo run -q -p liste-cli -- daemon

# Run the sync server locally.
server:
    cargo run -p liste-server

# Type-check the core for the web target.
check-wasm:
    cargo check -p liste-core --target wasm32-unknown-unknown

# Generate Swift bindings from the current core into bindings/generated/swift.
bindings-swift:
    cargo build -p liste-bindings --release
    cargo run -p liste-bindings --features cli --bin uniffi-bindgen -- \
        generate --library target/release/libliste_bindings.dylib \
        --language swift --out-dir bindings/generated/swift

# Start the web app dev server (once apps/web exists).
web-dev:
    cd apps/web && pnpm install && pnpm dev

# Build the self-host server image from the repository root.
docker-server:
    docker build -f deploy/self-host/Dockerfile -t liste-server .
