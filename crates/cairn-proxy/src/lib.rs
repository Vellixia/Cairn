//! `cairn.sh` reverse proxy (v0.5.0 Sprint 19b).
//!
//! A small axum binary that sits in front of one or more self-hosted Cairn
//! registries and exposes a unified `GET /registry/packs`, `GET /registry/search`,
//! etc. surface. The proxy doesn't own any state - it fans out queries to
//! configured backends in parallel and merges the results.
//!
//! ## Protocol
//!
//! Each upstream registry is the same `cairn-registry` HTTP API we already
//! expose on a single cairn-server. The proxy speaks the same protocol, so:
//!
//! - **Browse**: GET /registry/packs merges `index.json` from each upstream.
//! - **Search**: GET /registry/search?q=... sends the query to every upstream in
//!   parallel and dedups by `<name>@<version>`.
//! - **Download**: GET /registry/packs/:name/:version/download streams from the
//!   upstream that first reports the version in its index.
//! - **Federation pull**: GET /registry/federation/pull?since=<unix> merges
//!   revocations from every upstream into a single stream.
//!
//! ## Trust model
//!
//! The proxy trusts the operator's `peers.toml`. Each upstream is reached over
//! TLS; the operator is responsible for keeping the bearer tokens fresh. The
//! proxy does NOT itself validate pack signatures - that's the caller's job.
//!
//! See ADR-025 for the rationale on why this lives in its own crate vs. being
//! embedded in `cairn-server`.

pub mod config;
pub mod fanout;
pub mod router;

pub use config::{PeerEntry, ProxyConfig};
pub use fanout::{FanoutResult, MergedPack};
pub use router::build as proxy_router;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum ProxyError {
    #[error("config: {0}")]
    Config(String),
    #[error("upstream unreachable: {0}")]
    Upstream(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}
