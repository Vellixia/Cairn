//! Local cross-encoder reranking trait (P4.2). Thin wrapper around `cairn_rerank` so
//! `cairn-memory` doesn't need to depend on the optional `local` feature of `cairn-rerank`
//! (which pulls in fastembed + ORT).
//!
//! `MemoryEngine::with_reranker` accepts any `Arc<dyn Reranker>`. The default
//! `NullReranker` is provided for the common "no model, no rerank" case.

use std::sync::Arc;

// Re-export of the `cairn-rerank` trait + key types, so the rest of the engine
// doesn't need to depend on the optional `local` feature.
pub use cairn_rerank::{RerankError, RerankOutcome, Reranker};

// Re-export of the null (no-op) backend. Always available, zero cost.
pub use cairn_rerank::NullReranker;

// Re-export of the dispatcher (`provider=none` => NullReranker; `provider=local` => LocalReranker).
pub use cairn_rerank::from_config;

// Re-export the config struct so callers don't need a second dep.
pub use cairn_core::RerankConfig;

// Convenience: `Arc<dyn Reranker>` shorthand.
pub type RerankerRef = Arc<dyn Reranker>;
