//! Cairn storage: a SurrealDB-backed structured store (graph + vectors) plus a content-addressed
//! blob store that retains every full-fidelity original. The blob store is what makes Cairn's
//! compression lossless - any compact view handed to an agent can be expanded back to the exact
//! bytes.

mod blob;
mod db;
mod surreal;
pub mod memory_backend;

pub use blob::BlobStore;
pub use db::{
    AuditRecord, DocumentChunkRecord, DocumentSummary, ProjectRecord, PromotionLogEntry, Store,
};
