//! Cairn's domain: types, wire format, and the pure logic that turns recorded
//! state into memory, briefings and handoffs.
//!
//! No I/O lives here. Git access is `cairn-git`, storage is `cairn-store`.

pub mod applicability;
pub mod bound;
pub mod budget;
pub mod config;
pub mod context;
pub mod continuity;
pub mod corpus;
pub mod domain;
/// Feature 005's canonical event model — the one record that crosses the
/// machine boundary.
pub mod event;
/// Feature 005's deterministic identities: events, commands, candidates,
/// refusals, corroborations and patterns.
pub mod eventid;
pub mod global;
pub mod handoff;
pub mod knowledge;
pub mod lifecycle;
pub mod paths;
pub mod patterns;
pub mod promotion;
pub mod redact;
pub mod release;
pub mod startup;
pub mod tasks;
pub mod tools;
pub mod validate;
pub mod verify;
/// The session vocabulary a semantic signal must justify its tokens against —
/// one implementation, called by the client and the server independently.
pub mod vocabulary;
pub mod wire;

pub use config::CairnConfig;
pub use domain::*;
pub use lifecycle::{CanonicalEvent, CanonicalLifecycleEvent};
pub use wire::{Envelope, Request, WireError};

/// Digest used for evidence references and idempotency keys.
pub fn digest(input: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(input.as_bytes());
    hex::encode(h.finalize())
}
