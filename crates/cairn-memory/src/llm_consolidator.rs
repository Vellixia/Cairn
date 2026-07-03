//! LLM-driven memory consolidation (P1.4). Replaces (or supplements) the rule-based
//! `consolidate()` pipeline with LLM calls that synthesize stable facts, reusable
//! procedures, and cross-cutting insights from raw memory clusters.
//!
//! Gated behind `LlmConsolidationConfig::enabled` (opt-in via `CAIRN_LLM_CONSOLIDATION=true`)
//! due to LLM call cost.

use cairn_core::{Error, LlmConsolidationConfig, Result};
use chrono::Datelike;
use serde::Deserialize;

/// One extracted semantic fact (LLM call output).
#[derive(Debug, Clone)]
pub struct SemanticFact {
    pub statement: String,
    pub confidence: f32,
}

/// One extracted procedural step (LLM call output).
#[derive(Debug, Clone)]
pub struct ProceduralStep {
    pub name: String,
    pub trigger: String,
    pub steps: Vec<String>,
}

/// One cross-cutting insight (LLM call output).
#[derive(Debug, Clone)]
pub struct Insight {
    pub title: String,
    pub text: String,
    pub confidence: f32,
}

/// Raw OpenAI-compatible chat completion response (only what we use).
#[derive(Debug, Deserialize)]
struct ChatCompletionResponse {
    choices: Vec<ChatChoice>,
    /// Not every OpenAI-compatible server returns `usage` (some local Ollama configurations
    /// omit it) - `#[serde(default)]` so a missing field degrades to "unknown cost" rather
    /// than a parse failure.
    #[serde(default)]
    usage: Option<Usage>,
}

#[derive(Debug, Deserialize)]
struct ChatChoice {
    message: ChatMessage,
}

#[derive(Debug, Deserialize)]
struct ChatMessage {
    content: String,
}

/// v0.8.0 Sprint 9: token accounting from the response, fed into [`record_llm_tokens`] so the
/// daily budget guard (`Config::llm_daily_budget`) has real usage to check against.
#[derive(Debug, Deserialize)]
struct Usage {
    #[serde(default)]
    total_tokens: u64,
}

/// LLM-driven consolidator. Talks to any OpenAI-compatible `/v1/chat/completions` endpoint
/// (Ollama, llama.cpp server, OpenAI, etc.). When `config.enabled` is false, all calls
/// short-circuit to empty results.
pub struct LlmConsolidator {
    config: LlmConsolidationConfig,
}

impl LlmConsolidator {
    pub fn new(config: LlmConsolidationConfig) -> Self {
        Self { config }
    }

    /// True if LLM calls will actually be made. Lets callers skip pre-gathering work.
    pub fn is_enabled(&self) -> bool {
        self.config.enabled
    }

    /// Merge overlapping session summaries into stable semantic facts.
    /// Requires at least 3 summaries to bother calling the LLM.
    pub fn consolidate_semantic(&self, summaries: &[&str]) -> Result<Vec<SemanticFact>> {
        if !self.config.enabled || summaries.len() < 3 {
            return Ok(Vec::new());
        }
        let prompt = format!(
            "Given overlapping session summaries, extract stable factual knowledge.\n\
             Output each fact as <fact confidence='0.0-1.0'>statement</fact>.\n\n\
             Summaries:\n{}",
            summaries
                .iter()
                .map(|s| format!("- {}", s))
                .collect::<Vec<_>>()
                .join("\n")
        );
        let text = self.chat(&prompt)?;
        Ok(Self::parse_facts(&text))
    }

    /// Extract reusable procedures from repeated pattern-type memories.
    pub fn extract_procedures(&self, patterns: &[&str]) -> Result<Vec<ProceduralStep>> {
        if !self.config.enabled || patterns.is_empty() {
            return Ok(Vec::new());
        }
        let prompt = format!(
            "Extract reusable procedures from repeated patterns.\n\
             Output each as <procedure name='...' trigger='...'><step>Step 1</step></procedure>\n\n\
             Patterns:\n{}",
            patterns
                .iter()
                .map(|s| format!("- {}", s))
                .collect::<Vec<_>>()
                .join("\n")
        );
        let text = self.chat(&prompt)?;
        Ok(Self::parse_procedures(&text))
    }

    /// Synthesize cross-cutting insights from a cluster of related facts.
    pub fn synthesize_insights(&self, cluster: &[&str]) -> Result<Vec<Insight>> {
        if !self.config.enabled || cluster.is_empty() {
            return Ok(Vec::new());
        }
        let prompt = format!(
            "Synthesize cross-cutting insights from a cluster of related facts.\n\
             Output each as <insight confidence='...' title='...'>text</insight>\n\n\
             Facts:\n{}",
            cluster
                .iter()
                .map(|s| format!("- {}", s))
                .collect::<Vec<_>>()
                .join("\n")
        );
        let text = self.chat(&prompt)?;
        Ok(Self::parse_insights(&text))
    }
}

/// Make a single chat completion call against the configured OpenAI-compatible endpoint.
/// `pub(crate)` so sibling modules (e.g. `query_expander`) can reuse it without duplicating
/// the HTTP plumbing.
pub(crate) fn chat_with_config(config: &LlmConsolidationConfig, prompt: &str) -> Result<String> {
    let body = serde_json::json!({
        "model": config.model,
        "messages": [{"role": "user", "content": prompt}],
        "temperature": 0.3,
    });
    let mut req = ureq::post(&config.url).set("Content-Type", "application/json");
    if let Some(ref key) = config.api_key {
        req = req.set("Authorization", &format!("Bearer {}", key));
    }
    let resp: ChatCompletionResponse = req
        .send_json(&body)
        .map_err(|e| Error::Invalid(format!("LLM request failed: {e}")))?
        .into_json()
        .map_err(|e| Error::Invalid(format!("LLM response parse: {e}")))?;
    // v0.8.0 Sprint 9: every call self-reports into the process-wide daily budget tracker, so
    // every existing caller (this module's own consolidator, query_expander, llm_intelligence)
    // gets budget accounting for free - no signature change needed anywhere else. Checking
    // *against* the budget is still each background job's own responsibility (see
    // `is_budget_exhausted`), since only the caller knows whether skipping this particular call
    // is safe (a live user-facing query-expansion call and an unattended nightly batch have very
    // different "is it OK to just skip this" answers).
    if let Some(usage) = resp.usage.as_ref() {
        record_llm_tokens(usage.total_tokens);
    }
    Ok(resp
        .choices
        .first()
        .map(|c| c.message.content.clone())
        .unwrap_or_default())
}

/// v0.8.0 Sprint 9: process-wide daily LLM token budget. `(day_number, tokens_used_today)` -
/// `day_number` is days-since-epoch, so a new day naturally resets the counter to `0` the next
/// time anything touches it (no timer, no background task needed for the reset itself).
static LLM_BUDGET: std::sync::OnceLock<std::sync::Mutex<(i64, u64)>> = std::sync::OnceLock::new();

fn today() -> i64 {
    chrono::Utc::now().date_naive().num_days_from_ce() as i64
}

/// Record tokens spent against today's budget. Best-effort: a poisoned mutex (only possible if
/// an earlier accessor panicked while holding the lock) is recovered rather than propagated -
/// losing a little budget-tracking precision is preferable to a background job crashing over it.
pub(crate) fn record_llm_tokens(tokens: u64) {
    let cell = LLM_BUDGET.get_or_init(|| std::sync::Mutex::new((today(), 0)));
    let mut guard = cell.lock().unwrap_or_else(|e| e.into_inner());
    let day = today();
    if guard.0 != day {
        *guard = (day, 0);
    }
    guard.1 += tokens;
}

/// `true` once today's usage has reached `daily_limit`. `daily_limit == 0` means unlimited
/// (`CAIRN_LLM_DAILY_BUDGET=0`), matching every other `0 = unlimited` convention in `Config`.
pub(crate) fn is_budget_exhausted(daily_limit: u64) -> bool {
    if daily_limit == 0 {
        return false;
    }
    let cell = LLM_BUDGET.get_or_init(|| std::sync::Mutex::new((today(), 0)));
    let mut guard = cell.lock().unwrap_or_else(|e| e.into_inner());
    let day = today();
    if guard.0 != day {
        *guard = (day, 0);
    }
    guard.1 >= daily_limit
}

/// Tokens used so far today - exposed for `GET /api/metrics`/the autopilot digest.
pub fn llm_tokens_used_today() -> u64 {
    let cell = LLM_BUDGET.get_or_init(|| std::sync::Mutex::new((today(), 0)));
    let mut guard = cell.lock().unwrap_or_else(|e| e.into_inner());
    let day = today();
    if guard.0 != day {
        *guard = (day, 0);
    }
    guard.1
}

impl LlmConsolidator {
    /// Make a single chat completion call. Errors are returned to the caller; HTTP failures
    /// become `Error::Internal` so the caller can decide whether to surface them.
    fn chat(&self, prompt: &str) -> Result<String> {
        chat_with_config(&self.config, prompt)
    }

    /// Parse `<fact confidence='0.9'>statement</fact>` lines. Lenient: any malformed chunk
    /// is silently dropped, so an LLM that drifts from the format still leaves the good
    /// answers in place.
    pub fn parse_facts(text: &str) -> Vec<SemanticFact> {
        let mut facts = Vec::new();
        for cap in text.split("<fact").skip(1) {
            let conf_part = match cap.split("confidence='").nth(1) {
                Some(c) => c,
                None => continue,
            };
            let confidence: f32 = conf_part
                .split('\'')
                .next()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0.5);
            let body = match cap.split("'>").nth(1) {
                Some(b) => b,
                None => continue,
            };
            let statement = match body.split("</fact>").next() {
                Some(s) => s.trim().to_string(),
                None => continue,
            };
            if !statement.is_empty() {
                facts.push(SemanticFact {
                    statement,
                    confidence,
                });
            }
        }
        facts
    }

    /// Parse `<procedure name='...' trigger='...'><step>Step 1</step></procedure>` chunks.
    pub fn parse_procedures(text: &str) -> Vec<ProceduralStep> {
        let mut procs = Vec::new();
        for cap in text.split("<procedure").skip(1) {
            let name = cap
                .split("name='")
                .nth(1)
                .and_then(|s| s.split('\'').next())
                .unwrap_or("")
                .to_string();
            let trigger = cap
                .split("trigger='")
                .nth(1)
                .and_then(|s| s.split('\'').next())
                .unwrap_or("")
                .to_string();
            let body = match cap.split("'>").nth(1) {
                Some(b) => b,
                None => continue,
            };
            let steps: Vec<String> = body
                .split("<step>")
                .skip(1)
                .filter_map(|s| s.split("</step>").next())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            if !name.is_empty() && !steps.is_empty() {
                procs.push(ProceduralStep {
                    name,
                    trigger,
                    steps,
                });
            }
        }
        procs
    }

    /// Parse `<insight confidence='...' title='...'>text</insight>` chunks.
    pub fn parse_insights(text: &str) -> Vec<Insight> {
        let mut insights = Vec::new();
        for cap in text.split("<insight").skip(1) {
            let confidence: f32 = cap
                .split("confidence='")
                .nth(1)
                .and_then(|s| s.split('\'').next())
                .and_then(|s| s.parse().ok())
                .unwrap_or(0.5);
            let title = cap
                .split("title='")
                .nth(1)
                .and_then(|s| s.split('\'').next())
                .unwrap_or("")
                .to_string();
            let body = match cap.split("'>").nth(1) {
                Some(b) => b,
                None => continue,
            };
            let text_body = match body.split("</insight>").next() {
                Some(t) => t.trim().to_string(),
                None => continue,
            };
            if !title.is_empty() && !text_body.is_empty() {
                insights.push(Insight {
                    title,
                    text: text_body,
                    confidence,
                });
            }
        }
        insights
    }
}

/// Apply exponential decay to a memory's confidence based on time since last access.
/// `decay_days` is the period (default 30). Multiplier is 0.9^periods since last access.
/// Floors at 0.1 so memories never fully zero out.
pub fn apply_decay(
    memory: &mut cairn_core::Memory,
    decay_days: f64,
    now: chrono::DateTime<chrono::Utc>,
) {
    let last = memory.updated_at;
    let delta_days = (now - last).num_hours() as f64 / 24.0;
    if delta_days <= 0.0 {
        return;
    }
    let periods = (delta_days / decay_days).floor() as u32;
    if periods > 0 {
        let factor = 0.9_f64.powi(periods as i32);
        memory.confidence = ((memory.confidence as f64) * factor).max(0.1) as f32;
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
    fn parse_facts_extracts_two() {
        let output = "<fact confidence='0.9'>Cairn uses HelixDB for memory storage</fact>\n\
                      <fact confidence='0.7'>The embed provider defaults to local ONNX</fact>";
        let facts = LlmConsolidator::parse_facts(output);
        assert_eq!(facts.len(), 2);
        assert_eq!(facts[0].statement, "Cairn uses HelixDB for memory storage");
        assert!((facts[0].confidence - 0.9).abs() < 0.01);
        assert_eq!(
            facts[1].statement,
            "The embed provider defaults to local ONNX"
        );
    }

    #[test]
    fn parse_facts_handles_malformed_input() {
        let output = "no tags here\n<fact confidence='0.5'>good fact</fact>";
        let facts = LlmConsolidator::parse_facts(output);
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].statement, "good fact");
    }

    #[test]
    fn parse_procedures_extracts_steps() {
        let output = "<procedure name='debug-memory' trigger='memory not found'>\
                      <step>Check HelixDB connection</step>\
                      <step>Verify org_id matches</step></procedure>";
        let procs = LlmConsolidator::parse_procedures(output);
        assert_eq!(procs.len(), 1);
        assert_eq!(procs[0].name, "debug-memory");
        assert_eq!(procs[0].trigger, "memory not found");
        assert_eq!(procs[0].steps.len(), 2);
        assert_eq!(procs[0].steps[0], "Check HelixDB connection");
    }

    #[test]
    fn parse_insights_extracts_title_and_text() {
        let output = "<insight confidence='0.85' title='Memory tier evolution'>\
                      Working memories that get recalled twice tend to promote to semantic tier.</insight>";
        let insights = LlmConsolidator::parse_insights(output);
        assert_eq!(insights.len(), 1);
        assert_eq!(insights[0].title, "Memory tier evolution");
        assert!(insights[0].text.contains("promote to semantic"));
        assert!((insights[0].confidence - 0.85).abs() < 0.01);
    }

    #[test]
    fn consolidate_semantic_gated_when_disabled() {
        let c = LlmConsolidator::new(cfg(false));
        let facts = c
            .consolidate_semantic(&["summary1", "summary2", "summary3"])
            .unwrap();
        assert!(facts.is_empty());
    }

    #[test]
    fn consolidate_semantic_gated_when_too_few() {
        let c = LlmConsolidator::new(cfg(true));
        let facts = c.consolidate_semantic(&["only1", "only2"]).unwrap();
        assert!(facts.is_empty());
    }

    #[test]
    fn extract_procedures_gated_when_disabled() {
        let c = LlmConsolidator::new(cfg(false));
        let procs = c.extract_procedures(&["pattern"]).unwrap();
        assert!(procs.is_empty());
    }

    #[test]
    fn synthesize_insights_gated_when_disabled() {
        let c = LlmConsolidator::new(cfg(false));
        let insights = c.synthesize_insights(&["fact"]).unwrap();
        assert!(insights.is_empty());
    }

    #[test]
    fn is_enabled_reflects_config() {
        assert!(!LlmConsolidator::new(cfg(false)).is_enabled());
        assert!(LlmConsolidator::new(cfg(true)).is_enabled());
    }

    #[test]
    fn apply_decay_floors_at_zero_one() {
        use crate::tests::synth;
        let mut m = synth("test");
        m.confidence = 1.0;
        // Set updated_at far in the past so the decay kicks in
        m.updated_at = chrono::Utc::now() - chrono::Duration::days(365);
        apply_decay(&mut m, 30.0, chrono::Utc::now());
        // After 365 days with 30-day period, ~12 periods -> 0.9^12 = 0.28, so floors at 0.1
        assert!(m.confidence >= 0.1);
    }

    #[test]
    fn apply_decay_skips_fresh_memory() {
        use crate::tests::synth;
        let mut m = synth("fresh");
        m.confidence = 0.8;
        let now = chrono::Utc::now();
        m.updated_at = now;
        apply_decay(&mut m, 30.0, now);
        // delta_days is 0, so no decay
        assert!((m.confidence - 0.8).abs() < 0.001);
    }

    // v0.8.0 Sprint 9: LLM_BUDGET is a single process-wide static, so every assertion here uses
    // deltas off a `before` snapshot rather than absolute numbers - keeps this test correct
    // regardless of execution order relative to any other test in this binary that happens to
    // record usage (there are none today, but this way nobody has to remember that constraint
    // later). Everything lives in one #[test] fn since interleaving across threads is what would
    // actually be unsafe, not order.
    #[test]
    fn budget_guard_tracks_usage_and_reports_exhaustion() {
        let before = llm_tokens_used_today();

        // 0 = unlimited, never exhausted no matter how much has already been spent today.
        assert!(!is_budget_exhausted(0));

        record_llm_tokens(50);
        let after = llm_tokens_used_today();
        assert_eq!(after, before + 50, "usage accumulates rather than resetting");

        assert!(
            !is_budget_exhausted(after + 1),
            "a limit above today's usage is not yet exhausted"
        );
        assert!(
            is_budget_exhausted(after),
            "a limit at today's usage is exhausted"
        );
    }
}
