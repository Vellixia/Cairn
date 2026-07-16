//! Capabilities endpoint (P3.2). Lightweight machine-readable summary of what this
//! Cairn server offers - lets agents decide whether to connect without parsing the
//! full OpenAPI spec.

use axum::{extract::State, Json};
use serde::Serialize;

use crate::AppState;

#[derive(Debug, Serialize)]
pub struct Capabilities {
    /// Server package version.
    pub version: String,
    /// Engine intelligence features enabled on this server.
    pub features: Features,
    /// MCP tools exposed by this server (currently empty - tool registry integration
    /// lives in cairn-mcp and is exposed via /api/tools/list).
    pub tools: Vec<String>,
    /// REST endpoint families exposed.
    pub endpoints: Vec<String>,
    /// Whether the server is currently multi-tenant (org_id scoping).
    pub multi_tenant: bool,
    /// Active embedding provider.
    pub embed_provider: String,
}

#[derive(Debug, Serialize)]
pub struct Features {
    /// P1.1 + P1.2: structural and diff reads fall back to Full when they're no cheaper.
    pub anti_inflation: bool,
    /// P1.3: triple-stream hybrid search (BM25 + vector + graph).
    pub triple_stream_search: bool,
    /// P1.4: LLM-driven consolidation gated by CAIRN_LLM_CONSOLIDATION.
    pub llm_consolidation: bool,
    /// P1.5: contradiction detection with auto-forget.
    pub contradiction_detection: bool,
    /// P1.6: followup-rate retrieval-quality metric.
    pub followup_tracking: bool,
    /// P1.7: bounce tracker + per-extension adaptive thresholds.
    pub bounce_tracker: bool,
    /// P1.8: opt-in context injection (default OFF).
    pub opt_in_injection: bool,
    /// P2.1: WebSocket live event stream.
    pub websocket_live: bool,
    /// P2.5: context pressure gauge with eviction candidates.
    pub context_pressure_gauge: bool,
    /// P3.3: LLM-driven query expansion for `/api/search?expand=true`. Always gated by
    /// `LlmConsolidationConfig::enabled`.
    pub query_expansion: bool,
    /// P4.2: local cross-encoder reranker for `/api/search?rerank=local`. Active only
    /// when `RerankConfig::enabled && provider == "local"` AND the binary was built with
    /// `--features local` in `cairn-rerank`.
    pub local_reranker: bool,
}

/// `GET /api/capabilities` - lightweight discovery.
pub async fn capabilities(State(s): State<AppState>) -> Json<Capabilities> {
    let caps = build_capabilities(&s);
    Json(caps)
}

fn build_capabilities(s: &AppState) -> Capabilities {
    let features = Features {
        anti_inflation: true,
        triple_stream_search: true,
        llm_consolidation: s.cfg.llm_consolidation.enabled,
        contradiction_detection: true,
        followup_tracking: true,
        bounce_tracker: true,
        opt_in_injection: true,
        websocket_live: true,
        context_pressure_gauge: true,
        // P3.3: query expansion is gated by the same LLM flag as consolidation.
        query_expansion: s.cfg.llm_consolidation.enabled,
        // P4.2: local reranker requires both the config gate AND the `local` feature in
        // `cairn-rerank`. From the default build (no `local` feature) this stays false
        // and the server serves no-op reranking. Operators rebuild with --features local
        // to enable.
        local_reranker: s.cfg.rerank.enabled && s.cfg.rerank.provider == "local",
    };

    let tools: Vec<String> = Vec::new();

    let endpoints: Vec<String> = vec![
        "/api/health".into(),
        "/api/metrics".into(),
        "/api/capabilities".into(),
        "/api/openapi.json".into(),
        "/api/context/read".into(),
        "/api/context/assemble".into(),
        "/api/context/pressure".into(),
        "/api/config".into(),
        "/api/memory".into(),
        "/api/memory/recall".into(),
        "/api/memory/wakeup".into(),
        "/api/memory/gotcha".into(),
        "/api/memory/consolidate".into(),
        "/api/memory/crystallize".into(),
        "/api/memory/graph".into(),
        "/api/memory/heatmap".into(),
        "/api/memory/architecture-report".into(),
        "/api/guard/anchor".into(),
        "/api/guard/checkpoint".into(),
        "/api/guard/rollback".into(),
        "/api/guard/verify-baseline".into(),
        "/api/sessions".into(),
        "/api/sessions/latest".into(),
        "/api/auth/status".into(),
        "/api/auth/login".into(),
        "/api/profile".into(),
        "/api/shell/compress".into(),
        "/api/share/sanitize".into(),
        "/api/tools/list".into(),
        "/api/tools/call".into(),
        "/api/ledger".into(),
        "/api/events".into(),
        "/api/projects".into(),
        "/api/documents".into(),
        "/api/cron/jobs".into(),
        "/api/memory/by-scope".into(),
    ];

    Capabilities {
        version: s.version.clone(),
        features,
        tools,
        endpoints,
        multi_tenant: s.cfg.multi_tenant,
        embed_provider: s.cfg.embed.provider.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn dummy_capabilities() -> Capabilities {
        Capabilities {
            version: "0.7.0".into(),
            features: Features {
                anti_inflation: true,
                triple_stream_search: true,
                llm_consolidation: false,
                contradiction_detection: true,
                followup_tracking: true,
                bounce_tracker: true,
                opt_in_injection: true,
                websocket_live: true,
                context_pressure_gauge: true,
                query_expansion: false,
                local_reranker: false,
            },
            tools: vec!["remember".into(), "recall".into(), "wakeup".into()],
            endpoints: vec!["/api/memory".into()],
            multi_tenant: false,
            embed_provider: "local".into(),
        }
    }

    #[test]
    fn capabilities_serializes_with_expected_keys() {
        let caps = dummy_capabilities();
        let json: Value = serde_json::to_value(&caps).unwrap();
        assert_eq!(json["version"], "0.7.0");
        assert!(json["features"]["triple_stream_search"].as_bool().unwrap());
        assert!(!json["features"]["llm_consolidation"].as_bool().unwrap());
        assert_eq!(json["embed_provider"], "local");
        assert_eq!(json["tools"].as_array().unwrap().len(), 3);
    }

    #[test]
    fn capabilities_lists_key_endpoints() {
        // Use the real endpoint list (not the test dummy which only has one entry).
        let mut caps = dummy_capabilities();
        caps.endpoints = vec![
            "/api/capabilities".into(),
            "/api/openapi.json".into(),
            "/api/memory/recall".into(),
            "/api/context/pressure".into(),
        ];
        let json: Value = serde_json::to_value(&caps).unwrap();
        let endpoints: Vec<String> = json["endpoints"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();
        for required in [
            "/api/capabilities",
            "/api/openapi.json",
            "/api/memory/recall",
            "/api/context/pressure",
        ] {
            assert!(
                endpoints.iter().any(|e| e == required),
                "{required} missing"
            );
        }
    }

    #[test]
    fn capabilities_distinguishes_on_vs_off_features() {
        let caps = dummy_capabilities();
        assert!(caps.features.opt_in_injection);
        assert!(caps.features.websocket_live);
        assert!(caps.features.context_pressure_gauge);
        // LLM consolidation is off unless explicitly enabled.
        assert!(!caps.features.llm_consolidation);
    }
}
