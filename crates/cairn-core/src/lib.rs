//! Cairn's domain: types, wire format, and the pure logic that turns recorded
//! state into memory, briefings and handoffs.
//!
//! No I/O lives here. Git access is `cairn-git`, storage is `cairn-store`.

pub mod bound;
pub mod budget;
pub mod config;
pub mod context;
pub mod domain;
pub mod handoff;
pub mod paths;
pub mod redact;
pub mod release;
pub mod tools;
pub mod wire;

pub use config::CairnConfig;
pub use domain::*;
pub use wire::{Envelope, Request, WireError};

/// Digest used for evidence references and idempotency keys.
pub fn digest(input: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(input.as_bytes());
    hex::encode(h.finalize())
}
