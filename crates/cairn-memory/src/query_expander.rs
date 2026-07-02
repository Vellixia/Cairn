//! Query expansion (P3.3). Asks an LLM for 3-5 reformulations + temporal concretizations +
//! entity extractions of a search query, then runs the engine's triple-stream hybrid search
//! for each reformulation and merges by max combinedScore. The pipeline slot is "before
//! recall, after query" - expanding the candidate pool without changing ranking math.
//!
//! Gated behind the existing `LlmConsolidationConfig::enabled` flag (the same env var that
//! controls consolidation). When disabled, `expand()` short-circuits to a single-query
//! `ExpandedQuery` so the rest of the pipeline runs unchanged.
//!
//! Cost: one LLM call per expanded search (not per reformulation - the prompt asks for all
//! reformulations in one go). LLM errors degrade gracefully to no-expansion.

use cairn_core::{LlmConsolidationConfig, Result};
use serde::Serialize;

use crate::llm_consolidator::chat_with_config;

/// Maximum reformulations to keep. The plan spec says "3-5" - cap at 5 to bound cost.
const MAX_REFORMULATIONS: usize = 5;

/// LLM-produced expansion of a single query. `reformulations` are alternate phrasings the
/// LLM thinks would surface related memories; `concretizations` are time anchors it extracted
/// (e.g. "last week" -> "2026-06-22..2026-06-29"); `entities` are named entities it pulled out
/// (already used by the engine's graph leg via `extract_entities`).
#[derive(Debug, Clone, Default, Serialize)]
pub struct Expansion {
    pub reformulations: Vec<String>,
    pub concretizations: Vec<String>,
    pub entities: Vec<String>,
}

/// What the rest of the pipeline gets. `queries` is the dedup'd list to feed to `recall()`;
/// `entities` is the union of all entities (caller can blend into the BM25/graph leg).
#[derive(Debug, Clone, Serialize)]
pub struct ExpandedQuery {
    pub queries: Vec<String>,
    pub entities: Vec<String>,
}

impl ExpandedQuery {
    /// Trivial expansion with a single query (no reformulations, no entities). Returned
    /// when LLM is disabled or the expansion call fails - lets the caller use the same
    /// code path regardless.
    pub fn single(query: &str) -> Self {
        Self {
            queries: vec![query.to_string()],
            entities: Vec::new(),
        }
    }

    /// True when the expansion yielded more than just the original query.
    pub fn is_expanded(&self) -> bool {
        self.queries.len() > 1 || !self.entities.is_empty()
    }
}

/// LLM-driven query expander. Cheap to construct (no network IO at construction).
pub struct QueryExpander {
    config: LlmConsolidationConfig,
}

impl QueryExpander {
    pub fn new(config: LlmConsolidationConfig) -> Self {
        Self { config }
    }

    /// True if LLM calls will actually be made. Lets callers skip pre-gathering work.
    pub fn is_enabled(&self) -> bool {
        self.config.enabled
    }

    /// Expand `query` via the LLM. Returns `ExpandedQuery::single(query)` when:
    /// - LLM is disabled
    /// - LLM call errors (so the rest of the pipeline still works)
    /// - LLM output is empty / unparseable
    pub fn expand(&self, query: &str) -> Result<ExpandedQuery> {
        if !self.config.enabled {
            return Ok(ExpandedQuery::single(query));
        }
        let prompt = format!(
            "Given a search query, produce:\n\
             - 3-5 alternate phrasings as <reformulation>...</reformulation>\n\
             - any concrete time anchors as <temporal>...</temporal>\n\
             - named entities as <entity>...</entity>\n\n\
             Query: {query}"
        );
        let text = match chat_with_config(&self.config, &prompt) {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!(error = %e, "query expansion LLM call failed; falling back to single query");
                return Ok(ExpandedQuery::single(query));
            }
        };
        let expansion = Self::parse_expansion(&text);
        let mut queries: Vec<String> = Vec::with_capacity(1 + expansion.reformulations.len());
        queries.push(query.to_string());
        for r in expansion
            .reformulations
            .into_iter()
            .take(MAX_REFORMULATIONS)
        {
            let r = r.trim().to_string();
            if !r.is_empty() && !queries.iter().any(|q| q.eq_ignore_ascii_case(&r)) {
                queries.push(r);
            }
        }
        for c in expansion.concretizations {
            let c = c.trim().to_string();
            if !c.is_empty() && !queries.iter().any(|q| q.eq_ignore_ascii_case(&c)) {
                queries.push(c);
            }
        }
        let entities: Vec<String> = expansion
            .entities
            .into_iter()
            .map(|e| e.trim().to_string())
            .filter(|e| !e.is_empty())
            .collect();
        Ok(ExpandedQuery { queries, entities })
    }

    /// Parse `<reformulation>...</reformulation>`, `<temporal>...</temporal>`, and
    /// `<entity>...</entity>` lines. Lenient: any malformed chunk is silently dropped.
    pub fn parse_expansion(text: &str) -> Expansion {
        let mut out = Expansion::default();
        for cap in text.split("<reformulation").skip(1) {
            // `cap` starts with `>body</reformulation>...`. The opening `>` is at byte 0;
            // there may be another `>` in the closing tag, so use a fixed offset (skip
            // exactly one char past the split boundary) instead of `split(">").nth(1)`.
            let rest = cap.get(1..).unwrap_or("");
            if let Some(end) = rest.find("</reformulation>") {
                let body = rest[..end].trim().to_string();
                if !body.is_empty() {
                    out.reformulations.push(body);
                }
            }
        }
        for cap in text.split("<temporal").skip(1) {
            let rest = cap.get(1..).unwrap_or("");
            if let Some(end) = rest.find("</temporal>") {
                let body = rest[..end].trim().to_string();
                if !body.is_empty() {
                    out.concretizations.push(body);
                }
            }
        }
        for cap in text.split("<entity").skip(1) {
            let rest = cap.get(1..).unwrap_or("");
            if let Some(end) = rest.find("</entity>") {
                let body = rest[..end].trim().to_string();
                if !body.is_empty() {
                    out.entities.push(body);
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(enabled: bool) -> LlmConsolidationConfig {
        LlmConsolidationConfig {
            enabled,
            url: "http://localhost:11434/v1/chat/completions".to_string(),
            model: "llama3.2".to_string(),
            api_key: None,
        }
    }

    #[test]
    fn parse_expansion_extracts_three_kinds() {
        let output = r#"
            <reformulation>how does auth work in cairn</reformulation>
            <reformulation>auth flow explained</reformulation>
            <reformulation>how is authentication implemented</reformulation>
            <temporal>2026-06-22..2026-06-29</temporal>
            <entity>cairn</entity>
            <entity>helixdb</entity>
        "#;
        let e = QueryExpander::parse_expansion(output);
        assert_eq!(e.reformulations.len(), 3);
        assert_eq!(e.concretizations.len(), 1);
        assert_eq!(e.entities.len(), 2);
        assert!(e.reformulations[0].contains("auth"));
    }

    #[test]
    fn parse_expansion_handles_malformed_input() {
        // Garbage between good tags - good rows preserved.
        let output = "noise\n<not_a_tag>ignored</not_a_tag>\n<reformulation>good one</reformulation>\nrandom text";
        let e = QueryExpander::parse_expansion(output);
        assert_eq!(e.reformulations.len(), 1);
        assert_eq!(e.reformulations[0], "good one");
        assert!(e.concretizations.is_empty());
        assert!(e.entities.is_empty());
    }

    #[test]
    fn expand_returns_single_query_when_disabled() {
        let e = QueryExpander::new(cfg(false));
        let out = e.expand("how does auth work").unwrap();
        assert!(!out.is_expanded());
        assert_eq!(out.queries, vec!["how does auth work".to_string()]);
    }

    #[test]
    fn expand_is_enabled_reflects_config() {
        assert!(!QueryExpander::new(cfg(false)).is_enabled());
        assert!(QueryExpander::new(cfg(true)).is_enabled());
    }

    #[test]
    fn single_query_helper() {
        let q = ExpandedQuery::single("foo bar");
        assert!(!q.is_expanded());
        assert_eq!(q.queries, vec!["foo bar".to_string()]);
        assert!(q.entities.is_empty());
    }

    #[test]
    fn single_query_helper_with_entities_is_expanded() {
        let mut q = ExpandedQuery::single("foo");
        q.entities.push("bar".into());
        assert!(q.is_expanded());
    }
}
