//! Cross-encoder reranking for Cairn hybrid search (P4.2).
//!
//! ## Architecture
//!
//! - [`Reranker`] - the trait, with a single `rerank(query, docs) -> Vec<RerankOutcome>`
//!   method. Implementations batch internally.
//! - [`NullReranker`] - the no-op default. Returns docs in original order. Active when
//!   `RerankConfig::provider == "none"` or `enabled == false`. Zero cost.
//! - [`LocalReranker`] - in-process fastembed-based cross-encoder. Built only when the
//!   `local` feature is enabled. Downloads the model on first use (cached at the HF hub).
//! - [`from_config`] - dispatch on `RerankConfig` to the right backend, with safe
//!   fallback to `NullReranker` on any failure (so a broken model never 500s).
//!
//! ## Pipeline integration
//!
//! `cairn-memory::MemoryEngine::hybrid_search_with_rerank` runs MMR first, then re-scores
//! the top-K results with the cross-encoder, and finally blends the two scores:
//! `final = α * cross + (1-α) * hybrid` (α = `RerankConfig::blend_weight`).

use cairn_core::RerankConfig;
use std::sync::Arc;
use thiserror::Error;

/// One outcome of a rerank call: the original input position and the cross-encoder score
/// (higher = better).
#[derive(Debug, Clone, Copy)]
pub struct RerankOutcome {
    pub original_index: usize,
    pub score: f32,
}

/// The trait. Cheap to construct; the first `rerank` call may pay model-load latency.
pub trait Reranker: Send + Sync {
    /// Score each (query, doc) pair. Implementations should batch internally for
    /// efficiency. Output length always equals `docs.len()`, in the same order (results
    /// sorted separately by the caller, not by this trait).
    fn rerank(&self, query: &str, docs: &[&str]) -> Result<Vec<RerankOutcome>, RerankError>;
}

/// Errors a reranker can surface. Most are recoverable - the `from_config` dispatcher
/// falls back to `NullReranker` on `Recoverable` variants so a broken model degrades
/// gracefully to no-op.
#[derive(Debug, Error)]
pub enum RerankError {
    #[error("model load failed: {0}")]
    ModelLoad(String),
    #[error("inference failed: {0}")]
    Inference(String),
    #[error("invalid configuration: {0}")]
    Config(String),
}

/// The no-op default. Returns docs in original order with descending scores
/// (1.0, 0.99, 0.98, ...) so the pipeline never sees ties.
#[derive(Debug, Default, Clone)]
pub struct NullReranker;

impl Reranker for NullReranker {
    fn rerank(&self, _query: &str, docs: &[&str]) -> Result<Vec<RerankOutcome>, RerankError> {
        let n = docs.len();
        // Scores descending by position so MMR-passing results stay near the top if
        // someone else later reads the score. The exact magnitudes are unused by
        // the blend formula (hybrid dominates when rerank is null).
        Ok((0..n)
            .map(|i| RerankOutcome {
                original_index: i,
                score: 1.0 - (i as f32) * 0.001,
            })
            .collect())
    }
}

/// In-process fastembed cross-encoder reranker. Built only when the `local` feature is
/// enabled. Model is fetched on first construction; subsequent calls reuse the
/// already-loaded session.
#[cfg(feature = "local")]
pub struct LocalReranker {
    /// `Mutex` because `TextRerank::rerank` takes `&mut self`.
    model: std::sync::Mutex<fastembed::TextRerank>,
    batch_size: usize,
    max_length: usize,
}

#[cfg(feature = "local")]
impl std::fmt::Debug for LocalReranker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalReranker")
            .field("batch_size", &self.batch_size)
            .field("max_length", &self.max_length)
            .finish()
    }
}

#[cfg(feature = "local")]
impl LocalReranker {
    /// Build a `LocalReranker` with the default Jina cross-encoder. Downloads the model
    /// on first use (~110MB cached at `~/.cache/huggingface/hub/`).
    pub fn new() -> Result<Self, RerankError> {
        Self::with_model("jina-reranker-v1-turbo-en")
    }

    /// Build a `LocalReranker` for a specific model name. Currently only the default is
    /// wired (fastembed's `RerankerModel` enum maps a small set of supported models).
    pub fn with_model(model: &str) -> Result<Self, RerankError> {
        use fastembed::{RerankInitOptions, RerankerModel, TextRerank};
        let m = match model {
            // Map a few friendly names to fastembed's RerankerModel enum. Unknown names
            // fall through to the default.
            "bge-reranker-base" | "BAAI/bge-reranker-base" => RerankerModel::BGERerankerBase,
            "jina-reranker-v1-turbo-en" | "jinaai/jina-reranker-v1-turbo-en" => {
                RerankerModel::JINARerankerV1TurboEn
            }
            // Default to the Jina Turbo variant - best English benchmark at the time
            // of writing.
            _ => RerankerModel::JINARerankerV1TurboEn,
        };
        let model = TextRerank::try_new(RerankInitOptions::new(m))
            .map_err(|e| RerankError::ModelLoad(e.to_string()))?;
        Ok(Self {
            model: std::sync::Mutex::new(model),
            batch_size: 16,
            max_length: 512,
        })
    }
}

#[cfg(feature = "local")]
impl Reranker for LocalReranker {
    fn rerank(&self, query: &str, docs: &[&str]) -> Result<Vec<RerankOutcome>, RerankError> {
        let model = self
            .model
            .lock()
            .map_err(|e| RerankError::Inference(format!("model poisoned: {e}")))?;
        let docs_owned: Vec<&str> = docs.to_vec();
        let results = model
            .rerank(query, docs_owned, false, Some(self.batch_size))
            .map_err(|e| RerankError::Inference(e.to_string()))?;
        Ok(results
            .into_iter()
            .map(|r| RerankOutcome {
                original_index: r.index,
                score: r.score,
            })
            .collect())
    }
}

/// Dispatch on `RerankConfig` to the right backend. **Always returns a usable
/// `Arc<dyn Reranker>`** - on any failure to construct the configured backend, this
/// falls back to `NullReranker` and logs a warning. The intent is that operators can
/// flip `CAIRN_RERANKER_ENABLED=true` with a broken config and the server still serves
/// search (just without reranking).
pub fn from_config(config: &RerankConfig) -> Arc<dyn Reranker> {
    if !config.enabled {
        return Arc::new(NullReranker);
    }
    match config.provider.as_str() {
        "none" | "null" | "" => Arc::new(NullReranker),
        "local" => build_local(config),
        other => {
            tracing::warn!(
                provider = %other,
                "unknown reranker provider; falling back to no-op"
            );
            Arc::new(NullReranker)
        }
    }
}

#[cfg(feature = "local")]
fn build_local(config: &RerankConfig) -> Arc<dyn Reranker> {
    let model = config
        .model
        .as_deref()
        .unwrap_or("jina-reranker-v1-turbo-en");
    match LocalReranker::with_model(model) {
        Ok(r) => {
            tracing::info!(model = %model, "local reranker loaded");
            Arc::new(r)
        }
        Err(e) => {
            tracing::warn!(
                error = %e,
                model = %model,
                "failed to load local reranker; falling back to no-op"
            );
            Arc::new(NullReranker)
        }
    }
}

#[cfg(not(feature = "local"))]
fn build_local(config: &RerankConfig) -> Arc<dyn Reranker> {
    let _ = config;
    tracing::warn!(
        "provider=local requested but cairn-rerank was built without the `local` feature; \
         falling back to no-op. Rebuild with `--features local` to enable."
    );
    Arc::new(NullReranker)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_reranker_preserves_order() {
        let r = NullReranker;
        let docs = vec!["first", "second", "third"];
        let out = r.rerank("query", &docs).unwrap();
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].original_index, 0);
        assert_eq!(out[1].original_index, 1);
        assert_eq!(out[2].original_index, 2);
        // Scores descending so callers using a sort-by-score path get the original
        // ordering back.
        assert!(out[0].score >= out[1].score);
        assert!(out[1].score >= out[2].score);
    }

    #[test]
    fn null_reranker_handles_empty() {
        let r = NullReranker;
        let out = r.rerank("query", &[]).unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn from_config_disabled_returns_null() {
        let cfg = RerankConfig {
            enabled: false,
            ..RerankConfig::default()
        };
        let r = from_config(&cfg);
        // The trait's type-erased Arc doesn't expose Debug, but a sanity check:
        // run a rerank on the result and confirm pass-through.
        let out = r.rerank("q", &["a", "b"]).unwrap();
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn from_config_provider_none_returns_null() {
        let cfg = RerankConfig {
            enabled: true, // gate is open, but provider is "none" - still no-op
            provider: "none".into(),
            ..RerankConfig::default()
        };
        let r = from_config(&cfg);
        let out = r.rerank("q", &["a"]).unwrap();
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn from_config_unknown_provider_falls_back() {
        let cfg = RerankConfig {
            enabled: true,
            provider: "magic-ai".into(),
            ..RerankConfig::default()
        };
        let r = from_config(&cfg);
        // Should fall back to NullReranker - same observable behavior.
        let out = r.rerank("q", &["a", "b", "c"]).unwrap();
        assert_eq!(out.len(), 3);
    }

    #[test]
    fn rerank_config_defaults_are_off() {
        let cfg = RerankConfig::default();
        assert_eq!(cfg.provider, "none");
        assert!(!cfg.enabled);
        assert_eq!(cfg.top_k, 20);
        assert!((cfg.blend_weight - 0.6).abs() < 0.001);
    }

    #[test]
    fn rerank_config_debug_redacts_api_key() {
        let cfg = RerankConfig {
            provider: "http".into(),
            model: Some("bge-reranker".into()),
            api_key: Some("super-secret-key".into()),
            enabled: true,
            top_k: 10,
            blend_weight: 0.5,
        };
        let dbg = format!("{cfg:?}");
        assert!(dbg.contains("[REDACTED]"));
        assert!(!dbg.contains("super-secret-key"));
    }
}
