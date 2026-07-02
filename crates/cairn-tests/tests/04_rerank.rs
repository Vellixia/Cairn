//! 04 — Reranking: NullReranker order, `from_config` fallback chain, redacted
//! Debug, end-to-end hybrid_search_with_rerank through the real engine.
//!
//! `LocalReranker` itself downloads a model on first use, so the tests
//! exercise the *contract* and the engine integration — not the model
//! output. The pure-math assertions (alpha-blend, min-max) used to live
//! here but were tautological: they re-derived the formula without
//! calling any cairn code. They have been replaced by an integration
//! test against `MemoryEngine::hybrid_search_with_rerank` driven via the
//! in-memory `Store`.

use cairn_core::{MemoryKind, NewMemory, RerankConfig};
use cairn_rerank::{from_config, NullReranker, RerankOutcome, Reranker};
use cairn_store::Store;
use std::sync::Arc;

fn engine() -> Option<(cairn_memory::MemoryEngine, tempfile::TempDir)> {
    let dir = tempfile::tempdir().ok()?;
    let store = Arc::new(Store::open_in_memory(dir.path().join("blobs")).ok()?);
    Some((cairn_memory::MemoryEngine::new(store), dir))
}

#[test]
fn null_reranker_preserves_input_order() {
    let r = NullReranker;
    let docs: Vec<&str> = vec!["first", "second", "third"];
    let out = r.rerank("q", &docs).expect("NullReranker is total");
    assert_eq!(out.len(), 3);
    assert_eq!(out[0].original_index, 0);
    assert_eq!(out[1].original_index, 1);
    assert_eq!(out[2].original_index, 2);
    assert!(out[0].score >= out[1].score);
    assert!(out[1].score >= out[2].score);
}

#[test]
fn null_reranker_handles_empty_input() {
    let r = NullReranker;
    let out = r.rerank("q", &[]).expect("NullReranker is total");
    assert!(out.is_empty());
}

#[test]
fn from_config_disabled_returns_a_pass_through_reranker() {
    let cfg = RerankConfig {
        enabled: false,
        provider: "local".into(),
        model: Some("anything".into()),
        api_key: None,
        top_k: 20,
        blend_weight: 0.6,
    };
    let r = from_config(&cfg);
    let docs = vec!["a", "b"];
    let out = r.rerank("q", &docs).expect("disabled reranker is total");
    assert_eq!(out.len(), 2);
}

#[test]
fn from_config_provider_none_returns_a_pass_through_reranker() {
    let cfg = RerankConfig {
        enabled: true,
        provider: "none".into(),
        model: None,
        api_key: None,
        top_k: 20,
        blend_weight: 0.6,
    };
    let r = from_config(&cfg);
    let out = r.rerank("q", &["only"]).expect("none-provider is total");
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].original_index, 0);
}

#[test]
fn from_config_unknown_provider_falls_back_without_panicking() {
    let cfg = RerankConfig {
        enabled: true,
        provider: "magic-ai-9000".into(),
        model: None,
        api_key: None,
        top_k: 20,
        blend_weight: 0.6,
    };
    let r = from_config(&cfg);
    let out = r.rerank("q", &["a", "b", "c"]).expect("fallback is total");
    assert_eq!(out.len(), 3);
}

#[test]
fn rerank_config_defaults_are_safe() {
    let cfg = RerankConfig::default();
    assert_eq!(cfg.provider, "none");
    assert!(!cfg.enabled);
    assert_eq!(cfg.top_k, 20);
    assert!((cfg.blend_weight - 0.6).abs() < 1e-6);
}

#[test]
fn rerank_config_debug_redacts_api_key() {
    let cfg = RerankConfig {
        enabled: true,
        provider: "http".into(),
        model: Some("bge-reranker".into()),
        api_key: Some("super-secret-key".into()),
        top_k: 10,
        blend_weight: 0.5,
    };
    let dbg = format!("{cfg:?}");
    assert!(dbg.contains("[REDACTED]"));
    assert!(!dbg.contains("super-secret-key"));
}

#[test]
fn rerank_outcome_is_copy_and_total() {
    fn is_copy<T: Copy>() {}
    is_copy::<RerankOutcome>();
}

#[test]
fn hybrid_search_with_rerank_runs_end_to_end_on_in_memory_store() {
    // End-to-end: seed the in-memory store via the engine, run a hybrid
    // search with the NullReranker attached, and assert the pipeline
    // returns scored hits whose scores stay in [0, 1].
    let Some((mem, _dir)) = engine() else { return };
    mem.remember(NewMemory {
        content: "rust uses ownership for memory safety".into(),
        kind: Some(MemoryKind::Fact),
        ..Default::default()
    })
    .expect("remember");
    mem.remember(NewMemory {
        content: "the weather today is sunny and warm".into(),
        kind: Some(MemoryKind::Note),
        ..Default::default()
    })
    .expect("remember");

    let hits = mem
        .hybrid_search_with_rerank("rust ownership memory", 5, 10, &NullReranker, 0.6)
        .expect("hybrid search with rerank");
    assert!(!hits.is_empty(), "the rust memory must surface");
    for h in &hits {
        assert!(
            (0.0..=1.0).contains(&h.score),
            "blended score must be in [0, 1]: {}",
            h.score
        );
    }
    // The rust memory must rank above the weather note for this query.
    assert!(
        hits[0].memory.content.contains("rust"),
        "rust memory should be first; got {:?}",
        hits[0].memory.content
    );
}
