//! Offline-first sync via CRDTs (v0.5.0 Sprint 15a).
//!
//! The pre-0.5.0 sync path used **last-write-wins** - two devices editing the same
//! memory offline would silently drop one of the edits when they reconnected. That was
//! fine for the simple "remember once, recall forever" model, but it lost data the
//! moment a user edited a memory from two devices without an active connection.
//!
//! Cairn now uses two CRDTs that map cleanly onto the existing memory model:
//!
//! - **GCounter** (grow-only counter) for `access_count` and `confidence`. Both
//!   monotonically increase per memory, so concurrent additions are simply summed ---
//!   no data loss, no conflict.
//! - **OR-Set** (observed-remove set) for `tags` and `concepts`. Each side adds and
//!   removes elements with a unique causal marker; concurrent add + remove resolve
//!   to add (the element is back in the set), preserving the user's intent on both
//!   sides.
//!
//! What we **don't** CRDT-ify: the memory body itself (`content`, `importance`,
//! `files`, `description`). Those fields are owned by a single logical "latest write"
//! - but the comparison uses vector clocks instead of wall-clock timestamps, so a
//!   write from a device with a fresh clock doesn't silently overwrite an older write
//!   that just happened to land later.
//!
//! See ADR-019 for the rationale on why we picked these two CRDTs over a full
//! automerge/automerge-style document CRDT (binary size + dependency weight).

pub mod counter;
pub mod crypto;
pub mod orset;
pub mod sync;

pub use counter::GCounter;
pub use crypto::{decrypt_envelope, encrypt_envelope, EncryptedEnvelope, Header, KdfParams};
pub use orset::ORSet;
pub use sync::{MemoryOp, SyncEnvelope, SyncPeer, SyncResult, VectorClock};

#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    #[error("serialization: {0}")]
    Json(#[from] serde_json::Error),
    #[error("peer rejected the sync envelope: {0}")]
    Rejected(String),
    #[error("I/O: {0}")]
    Io(#[from] std::io::Error),
}
