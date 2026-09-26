# Sync server image. Multi-stage: build with the Rust toolchain, run on a
# distroless static base. The same image runs Liste Cloud and self-hosted
# instances (Section 12 of docs/ARCHITECTURE.md).
#
# Build from the repository root:
#   docker build -f deploy/self-host/Dockerfile -t liste-server .

FROM rust:1.97-bookworm AS builder
WORKDIR /src
RUN rustup target add x86_64-unknown-linux-musl \
    && apt-get update && apt-get install -y --no-install-recommends musl-tools \
    && rm -rf /var/lib/apt/lists/*
COPY . .
RUN cargo build --release -p liste-server --target x86_64-unknown-linux-musl

FROM gcr.io/distroless/static-debian12:nonroot
COPY --from=builder /src/target/x86_64-unknown-linux-musl/release/liste-server /liste-server
EXPOSE 8080
ENTRYPOINT ["/liste-server"]
