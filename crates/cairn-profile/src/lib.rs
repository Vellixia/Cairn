//! The preference/behavior profile — Cairn's "make any model smart" engine.
//!
//! It captures the user's standing preferences (preferred stack, style, do/don'ts, corrections)
//! and surfaces them so even a small, cheap model honors how *you* work. Preferences are durable
//! `Preference`-kind memories; this crate adds explicit recording, conservative auto-capture from
//! prompts, and a compact profile block for injection.

use cairn_core::{Memory, MemoryKind, MemoryTier, NewMemory, Result};
use cairn_memory::MemoryEngine;
use std::sync::Arc;

pub struct Profile {
    mem: Arc<MemoryEngine>,
}

impl Profile {
    pub fn new(mem: Arc<MemoryEngine>) -> Self {
        Self { mem }
    }

    /// Record a standing preference (durable, high-importance). Dedup handles repeats.
    pub fn prefer(&self, rule: &str) -> Result<Memory> {
        let mut nm = NewMemory::new(rule.trim());
        nm.kind = Some(MemoryKind::Preference);
        nm.tier = Some(MemoryTier::Semantic);
        nm.importance = Some(0.85);
        self.mem.remember(nm)
    }

    /// All recorded preferences (the profile), newest first.
    pub fn preferences(&self) -> Result<Vec<Memory>> {
        self.mem.by_kind(MemoryKind::Preference)
    }

    /// Detect clear preference directives in a user prompt and record them. Returns what was
    /// captured. Conservative by design — only unambiguous coding directives are captured.
    pub fn capture_from_prompt(&self, prompt: &str) -> Result<Vec<Memory>> {
        let mut captured = Vec::new();
        for rule in detect_preferences(prompt) {
            captured.push(self.prefer(&rule)?);
        }
        Ok(captured)
    }

    /// A compact block of the user's preferences for injecting into context. Empty if none.
    pub fn block(&self) -> Result<String> {
        let prefs = self.preferences()?;
        if prefs.is_empty() {
            return Ok(String::new());
        }
        let mut out = String::from("Your preferences (honor these):\n");
        for p in prefs {
            out.push_str(&format!("- {}\n", p.content));
        }
        Ok(out)
    }
}

/// Extract clear preference directives from a prompt. High-precision: a clause is captured only if
/// it contains a strong directive cue. Clauses are split on sentence/clause boundaries.
fn detect_preferences(prompt: &str) -> Vec<String> {
    const CUES: &[&str] = &[
        "always use ",
        "never use ",
        "don't use ",
        "do not use ",
        "prefer using ",
        "prefer to use ",
        "instead of ",
    ];
    let mut out = Vec::new();
    for raw in prompt.split(['.', '!', '?', '\n', ';']) {
        let frag = raw.trim();
        if frag.len() < 5 || frag.len() > 160 {
            continue;
        }
        let low = frag.to_lowercase();
        let hit = CUES.iter().any(|cue| {
            if *cue == "instead of " {
                // "instead of" only counts as a directive when paired with "use".
                low.contains("instead of ") && low.contains("use ")
            } else {
                low.contains(cue)
            }
        });
        if hit {
            out.push(frag.to_string());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_core::Config;
    use cairn_store::Store;

    fn profile() -> (Profile, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let cfg = Config::resolve(Some(dir.path().join("data"))).unwrap();
        let mem = Arc::new(MemoryEngine::new(Arc::new(Store::open(&cfg).unwrap())));
        (Profile::new(mem), dir)
    }

    #[test]
    fn detects_clear_directives_only() {
        let prompt = "Please refactor this. Always use ripgrep instead of grep. I was thinking about lunch. Never use unwrap in library code.";
        let found = detect_preferences(prompt);
        assert!(found.iter().any(|f| f.to_lowercase().contains("ripgrep")));
        assert!(found
            .iter()
            .any(|f| f.to_lowercase().contains("never use unwrap")));
        assert!(!found.iter().any(|f| f.to_lowercase().contains("lunch")));
    }

    #[test]
    fn prefer_lists_and_blocks_with_dedup() {
        let (p, _d) = profile();
        p.prefer("always use 4-space indentation").unwrap();
        p.prefer("prefer using axum for HTTP").unwrap();
        p.prefer("always use 4-space indentation").unwrap(); // dedup
        let prefs = p.preferences().unwrap();
        assert_eq!(prefs.len(), 2);
        let block = p.block().unwrap();
        assert!(block.contains("4-space"));
        assert!(block.contains("axum"));
    }

    #[test]
    fn capture_from_prompt_stores_directives() {
        let (p, _d) = profile();
        let captured = p.capture_from_prompt("always use tabs not spaces").unwrap();
        assert_eq!(captured.len(), 1);
        assert_eq!(p.preferences().unwrap().len(), 1);
    }
}
