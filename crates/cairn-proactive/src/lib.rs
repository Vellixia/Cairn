//! Proactive recall (v0.5.0 Sprint 18).
//!
//! Sits between an MCP agent and its tools. On each turn the hook receives the
//! pending prompt (e.g. the user's chat message + any tool descriptions the
//! model has surfaced so far). It runs a lightweight **intent classifier** to
//! decide whether the prompt is a question or task that would benefit from
//! memory recall. If yes, it calls the local [`cairn_memory::MemoryEngine`]
//! to fetch the top-K relevant memories and prepends them to the model's
//! context.
//!
//! ## Intent classifier
//!
//! A tiny, fully-local heuristic - no LLM call. We score a prompt on:
//!
//! - **Question markers** (`?`, `what/why/how/when/where`, `which`).
//! - **Recall cues** (`remember`, `decided`, `agreed`, `last time`,
//!   `previously`, `earlier`).
//! - **File / path mentions** - trigger memory recall for the file's
//!   `applies_to` edges.
//! - **Reference phrases** (`this`, `that`, `the api`, `the model`, pronouns
//!   without clear antecedents - usually mean "the thing we talked about").
//!
//! Score >= threshold (default 0.4) -> fire recall. Anything below is left
//! alone (the agent will still call `cairn_recall` explicitly when it
//! wants to).
//!
//! ## Per-project opt-out
//!
//! A `proactive_recall: false` preference stored in the memory store disables
//! the hook for the matching project (workspace root path prefix). The hook
//! checks this on every invocation; toggling the preference takes effect on
//! the next agent turn.
//!
//! See ADR-025 for the rationale on keeping the classifier local + cheap.

pub mod intent;
pub mod pref;

pub use intent::{IntentDecision, IntentFeatures, IntentSignal};
pub use pref::{ProactivePref, PROJECT_OPT_OUT};

use cairn_core::Memory;

/// Outcome of running [`ProactiveHook::on_turn`]. Either we fired recall
/// (returning the memories we'd prepend) or we left the prompt alone.
#[derive(Debug, Clone)]
pub enum HookOutcome {
    /// Recall fired - caller should prepend these memories to the prompt.
    Recalled(Vec<Memory>),
    /// Recall skipped - either the intent classifier said no, or the per-project
    /// opt-out is on. `reason` is a short string for diagnostics.
    Skipped { reason: &'static str },
}

/// P1.8: opt-in gate. Mirrors `cairn-client/src/hook.rs` - the same env var controls
/// both layers so users only need to set it once. Default OFF.
fn inject_context_enabled() -> bool {
    matches!(
        std::env::var("CAIRN_INJECT_CONTEXT").ok().as_deref(),
        Some("1" | "true" | "yes" | "on")
    )
}

/// The hook. Construct with a `MemoryEngine` (or any callable that returns
/// ranked memories) and an opt-out pref; call `on_turn(prompt)` per agent
/// turn.
pub struct ProactiveHook<F>
where
    F: Fn(&str, usize) -> Vec<Memory>,
{
    pub recall: F,
    pub pref: ProactivePref,
    pub max_inject: usize,
    pub threshold: f32,
}

impl<F> ProactiveHook<F>
where
    F: Fn(&str, usize) -> Vec<Memory>,
{
    pub fn new(recall: F) -> Self {
        // P1.8: default-off. Proactive injection only fires when the user has explicitly
        // opted in via `CAIRN_INJECT_CONTEXT=true`. Without this, every `on_turn` call
        // classifies intent and may inject memories - silent burn.
        let default_threshold = if inject_context_enabled() {
            0.4
        } else {
            f32::INFINITY
        };
        Self {
            recall,
            pref: ProactivePref::default(),
            max_inject: 3,
            threshold: default_threshold,
        }
    }

    pub fn with_pref(mut self, pref: ProactivePref) -> Self {
        self.pref = pref;
        self
    }

    pub fn with_max_inject(mut self, n: usize) -> Self {
        self.max_inject = n.max(1);
        self
    }

    pub fn with_threshold(mut self, t: f32) -> Self {
        self.threshold = t.clamp(0.0, 1.0);
        self
    }

    /// Decide whether to recall for `prompt`. `project_root` is the workspace
    /// root the agent is operating in; used to check per-project opt-out.
    pub fn on_turn(&self, prompt: &str, project_root: Option<&str>) -> HookOutcome {
        if let Some(root) = project_root {
            if self.pref.is_opted_out(root) {
                return HookOutcome::Skipped {
                    reason: "project opted out",
                };
            }
        }
        let decision = intent::classify(prompt, self.threshold);
        match decision {
            IntentDecision::Skip(reason) => HookOutcome::Skipped { reason },
            IntentDecision::Fire { score, .. } => {
                let k = self.max_inject;
                let mut mems = (self.recall)(prompt, k);
                mems.truncate(k);
                if mems.is_empty() {
                    HookOutcome::Skipped {
                        reason: "no memories matched",
                    }
                } else {
                    tracing::debug!(
                        intent_score = score,
                        recalled = mems.len(),
                        "proactive recall fired"
                    );
                    HookOutcome::Recalled(mems)
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_core::Memory;
    use chrono::Utc;
    use std::sync::Mutex;

    /// Serialize tests that touch the env so they don't race with each other.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn mk_mem(id: &str, content: &str) -> Memory {
        Memory {
            id: id.into(),
            kind: cairn_core::MemoryKind::Note,
            tier: cairn_core::MemoryTier::Working,
            content: content.into(),
            concepts: vec![],
            files: vec![],
            session_id: None,
            importance: 0.5,
            access_count: 0,
            org_id: cairn_core::OrgId::default(),
            suspicious: false,
            confidence: 0.5,
            pinned: false,
            derived_from: vec![],
            contradicts: vec![],
            supersedes: vec![],
            applies_to: vec![],
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    /// No-op recall fn that returns a fixed memory for any prompt - lets the
    /// tests focus on intent + opt-out logic without needing a real engine.
    fn fake_recall(_prompt: &str, _k: usize) -> Vec<Memory> {
        vec![mk_mem("m1", "the team's prior decision")]
    }

    /// Lock the env-mutex and force `CAIRN_INJECT_CONTEXT` to a known value,
    /// restoring the previous value when `f` returns. Required for tests that
    /// construct `ProactiveHook::new` (since `new` reads the env).
    fn with_inject_env<T>(value: Option<&str>, f: impl FnOnce() -> T) -> T {
        let _guard = ENV_LOCK.lock().unwrap();
        let prev = std::env::var("CAIRN_INJECT_CONTEXT").ok();
        match value {
            Some(v) => std::env::set_var("CAIRN_INJECT_CONTEXT", v),
            None => std::env::remove_var("CAIRN_INJECT_CONTEXT"),
        }
        let result = f();
        match prev {
            Some(v) => std::env::set_var("CAIRN_INJECT_CONTEXT", v),
            None => std::env::remove_var("CAIRN_INJECT_CONTEXT"),
        }
        result
    }

    #[test]
    fn recall_fires_on_question_with_recall_cue() {
        with_inject_env(Some("true"), || {
            let h = ProactiveHook::new(fake_recall);
            let out = h.on_turn("What did we decide about the auth layer last time?", None);
            assert!(matches!(out, HookOutcome::Recalled(_)));
        });
    }

    #[test]
    fn recall_skips_on_plain_statement() {
        with_inject_env(Some("true"), || {
            let h = ProactiveHook::new(fake_recall);
            let out = h.on_turn("Add a print statement", None);
            match out {
                HookOutcome::Skipped { reason } => assert_ne!(reason, "no memories matched"),
                _ => panic!("expected Skipped"),
            }
        });
    }

    #[test]
    fn per_project_opt_out_disables_recall() {
        with_inject_env(Some("true"), || {
            let h = ProactiveHook::new(fake_recall)
                .with_pref(ProactivePref::default().with_opt_out("/work/excluded"));
            let out = h.on_turn("What did we decide?", Some("/work/excluded"));
            assert!(matches!(
                out,
                HookOutcome::Skipped {
                    reason: "project opted out"
                }
            ));
        });
    }

    #[test]
    fn opt_out_does_not_apply_to_other_projects() {
        with_inject_env(Some("true"), || {
            let h = ProactiveHook::new(fake_recall)
                .with_pref(ProactivePref::default().with_opt_out("/work/excluded"));
            let out = h.on_turn("What did we decide?", Some("/work/other"));
            assert!(matches!(out, HookOutcome::Recalled(_)));
        });
    }

    #[test]
    fn max_inject_caps_the_injection() {
        with_inject_env(Some("true"), || {
            let h = ProactiveHook::new(|_, _| {
                (0..10)
                    .map(|i| mk_mem(&format!("m{i}"), &format!("memory {i}")))
                    .collect()
            })
            .with_max_inject(2);
            if let HookOutcome::Recalled(mems) = h.on_turn("What did we decide?", None) {
                assert_eq!(mems.len(), 2);
            } else {
                panic!("expected recall");
            }
        });
    }

    /// P1.8: when `CAIRN_INJECT_CONTEXT` is unset, the default threshold is +inf
    /// (no recall ever fires). When set to `true`, the threshold drops to 0.4.
    #[test]
    fn default_threshold_is_infinity_when_env_unset() {
        with_inject_env(None, || {
            let h = ProactiveHook::new(fake_recall);
            assert!(h.threshold.is_infinite());
            let out = h.on_turn("What did we decide?", None);
            assert!(
                matches!(out, HookOutcome::Skipped { .. }),
                "default-off must skip recall"
            );
        });
    }

    #[test]
    fn threshold_drops_to_0_4_when_env_true() {
        with_inject_env(Some("true"), || {
            let h = ProactiveHook::new(fake_recall);
            assert!((h.threshold - 0.4).abs() < 1e-6);
        });
    }
}
