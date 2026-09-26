//! The Liste sync server (Section 12).
//!
//! A stateless Axum process in front of PostgreSQL. It stores encrypted ops
//! and snapshots, assigns a per-space `seq`, and handles accounts, devices,
//! authentication, push dispatch, and billing entitlements. It never
//! interprets op contents. The same binary runs Liste Cloud and every
//! self-hosted instance; there is no Cloudflare-specific code in it.

use std::net::SocketAddr;

use axum::{Router, routing::get};

#[tokio::main]
async fn main() {
    let addr: SocketAddr = std::env::var("LISTE_BIND_ADDR")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or_else(|| SocketAddr::from(([0, 0, 0, 0], 8080)));

    let app = Router::new().route("/health", get(|| async { "ok" }));

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("bind listener");
    axum::serve(listener, app).await.expect("serve");
}
