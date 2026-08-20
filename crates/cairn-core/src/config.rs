//! Configuration: capture bounds, context budget, hook deadlines, exclusions
//! (FR-013, FR-029, FR-041, FR-050).

use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CairnConfig {
    /// Maximum stored bytes per observation (FR-013).
    pub payload_cap_bytes: usize,
    /// Briefing budget in Cairn-estimated tokens (FR-029).
    pub context_budget_tokens: usize,
    /// Capture-class hook deadline, milliseconds (FR-041, D15).
    pub capture_deadline_ms: u64,
    /// Context-class hook deadline, milliseconds (FR-041, D15).
    pub context_deadline_ms: u64,
    /// Glob patterns; matching paths are never captured (FR-050).
    pub excluded_paths: Vec<String>,
    /// Glob patterns; matching commands are never captured (FR-050).
    pub excluded_commands: Vec<String>,
    /// Base URL of the Cairn server, when one is configured.
    pub server_url: Option<String>,

    // -----------------------------------------------------------------------
    // Feature 003 bounds (FR-500, research D75)
    //
    // Every bound this feature relies on has a documented default, is asserted
    // by test rather than assumed, and is adjustable here. D75 counts sixteen
    // *classes*; there are seventeen fields, because the class "the stored
    // bounds on an evidence value and locator" is one clause covering two
    // values that are bounded separately.
    //
    // A bound with no test is a bound that drifts, so
    // `defaults_are_the_documented_ones` below pins every one of them and
    // `tests/tests/bounds.rs` (T140) asserts the behaviour at each.
    // -----------------------------------------------------------------------
    /// Share of the context budget Level 1 and Level 2 may **not** take.
    ///
    /// A cap on the lower levels, not a floor Level 0 must spend: unspent
    /// reserve returns to the general pool, so a project with no task, no
    /// warnings and no pins delivers exactly what it delivers today (FR-442).
    pub min_safe_context_fraction: f64,
    /// Below this budget the briefing is still produced, truncated in Level 0's
    /// documented admission order. It is never rejected for size (FR-445).
    pub min_context_budget_tokens: usize,
    /// A task goal is truncated to this in Tier 0a, which is what keeps the
    /// guaranteed tier O(1) in the size of the task.
    pub goal_max_tokens: usize,
    /// Pins per project. Exceeding it refuses and names the current pins;
    /// nothing is ever silently unpinned (FR-454).
    pub pin_budget_project: usize,
    /// Pins per scope.
    pub pin_budget_per_scope: usize,
    /// Applicable pins admitted to a briefing.
    pub pins_in_context_max: usize,
    /// Warnings admitted to a briefing, highest precedence first.
    pub warnings_in_context_max: usize,
    /// Signal-matched reusable patterns admitted to a briefing.
    pub patterns_in_context_max: usize,
    /// Subject members examined per write before reconciliation defers to the
    /// maintenance tick and reports `reconciliation_deferred` (FR-474).
    pub reconcile_members_max: usize,
    /// Topic-keyed memories scanned for warnings on the session-open path,
    /// highest-precedence scopes first. Beyond it, assembly reports `degraded`.
    pub subject_warning_scan_max: usize,
    /// Indexed evidence lookups per captured event. Exceeding it defers to the
    /// background pass and is **not** an error (FR-374).
    pub evidence_lookups_per_event_max: usize,
    /// Evidence facts examined per bounded background verification pass.
    pub verify_pass_evidence_max: usize,
    /// Verifier runs per bounded background pass.
    pub verify_pass_runs_max: usize,
    /// Wall-clock share of one bounded background pass, milliseconds.
    pub verify_pass_wall_ms: u64,
    /// Stored size of an evidence fact's observed value, **after** redaction
    /// (FR-354).
    pub evidence_value_max_bytes: usize,
    /// Stored size of an evidence fact's source locator.
    pub evidence_locator_max_bytes: usize,
    /// Signals a reusable pattern must define, and the overlap a suggestion
    /// needs. A pattern that matches indiscriminately is worse than none.
    pub pattern_signals_min: usize,
}

impl Default for CairnConfig {
    fn default() -> Self {
        Self {
            payload_cap_bytes: crate::bound::DEFAULT_PAYLOAD_CAP_BYTES,
            context_budget_tokens: 3000,
            capture_deadline_ms: 250,
            context_deadline_ms: 1500,
            excluded_paths: Vec::new(),
            excluded_commands: Vec::new(),
            server_url: None,

            min_safe_context_fraction: 0.40,
            min_context_budget_tokens: 600,
            goal_max_tokens: 60,
            pin_budget_project: 12,
            pin_budget_per_scope: 4,
            pins_in_context_max: 4,
            warnings_in_context_max: 5,
            patterns_in_context_max: 2,
            reconcile_members_max: 64,
            subject_warning_scan_max: 256,
            evidence_lookups_per_event_max: 8,
            verify_pass_evidence_max: 200,
            verify_pass_runs_max: 50,
            verify_pass_wall_ms: 2000,
            evidence_value_max_bytes: 256,
            evidence_locator_max_bytes: 256,
            pattern_signals_min: 2,
        }
    }
}

impl CairnConfig {
    /// The reserved share of a budget, in estimated tokens.
    ///
    /// `floor(limit * min_safe_context_fraction)`
    /// (`contracts/continuity-context.md` §The budget reserve).
    pub fn context_reserve(&self, limit: usize) -> usize {
        (limit as f64 * self.min_safe_context_fraction).floor() as usize
    }
}

impl CairnConfig {
    /// Load from `CAIRN_HOME/config.json`, falling back to defaults.
    ///
    /// A malformed config must not stop Cairn from running: it is reported by
    /// the caller and defaults are used.
    pub fn load() -> Self {
        Self::load_from(&crate::paths::config_path()).unwrap_or_default()
    }

    pub fn load_from(path: &Path) -> Option<Self> {
        let text = std::fs::read_to_string(path).ok()?;
        serde_json::from_str(&text).ok()
    }

    pub fn save(&self) -> std::io::Result<()> {
        crate::paths::ensure_home()?;
        self.save_to(&crate::paths::config_path())
    }

    pub fn save_to(&self, path: &Path) -> std::io::Result<()> {
        let text = serde_json::to_string_pretty(self).expect("config serializes");
        std::fs::write(path, text)
    }

    /// True when this path must never be captured.
    pub fn is_path_excluded(&self, path: &str) -> bool {
        self.excluded_paths.iter().any(|p| glob_match(p, path))
    }

    /// True when this command must never be captured.
    pub fn is_command_excluded(&self, command: &str) -> bool {
        let trimmed = command.trim();
        self.excluded_commands
            .iter()
            .any(|p| glob_match(p, trimmed))
    }
}

/// Minimal glob matcher: `*` matches within a path segment, `**` across
/// segments, `?` matches one character.
///
/// Deliberately small — exclusions are user-facing patterns like
/// `secrets/**` and `aws sts*`, not a full shell globbing language.
pub fn glob_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    matches_from(&p, 0, &t, 0)
}

fn matches_from(p: &[char], mut pi: usize, t: &[char], mut ti: usize) -> bool {
    while pi < p.len() {
        match p[pi] {
            '*' => {
                let double = pi + 1 < p.len() && p[pi + 1] == '*';
                let next = if double { pi + 2 } else { pi + 1 };
                // `**` crosses separators; a single `*` does not.
                for skip in ti..=t.len() {
                    if !double && t[ti..skip].contains(&'/') {
                        break;
                    }
                    if matches_from(p, next, t, skip) {
                        return true;
                    }
                }
                return false;
            }
            '?' => {
                if ti >= t.len() {
                    return false;
                }
                pi += 1;
                ti += 1;
            }
            c => {
                if ti >= t.len() || t[ti] != c {
                    return false;
                }
                pi += 1;
                ti += 1;
            }
        }
    }
    ti == t.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_the_documented_ones() {
        let c = CairnConfig::default();
        assert_eq!(c.payload_cap_bytes, 4096);
        assert_eq!(c.capture_deadline_ms, 250);
        assert_eq!(c.context_deadline_ms, 1500);
        assert!(c.context_budget_tokens >= 2000 && c.context_budget_tokens <= 4000);
    }

    #[test]
    fn every_feature_003_bound_is_at_its_documented_default() {
        // FR-500 and SC-320: every bound this feature relies on has a
        // documented default asserted by test rather than assumed. Seventeen
        // fields, sixteen D75 classes — the evidence value and locator bounds
        // are one clause covering two separately bounded values.
        let c = CairnConfig::default();
        assert_eq!(c.min_safe_context_fraction, 0.40);
        assert_eq!(c.min_context_budget_tokens, 600);
        assert_eq!(c.goal_max_tokens, 60);
        assert_eq!(c.pin_budget_project, 12);
        assert_eq!(c.pin_budget_per_scope, 4);
        assert_eq!(c.pins_in_context_max, 4);
        assert_eq!(c.warnings_in_context_max, 5);
        assert_eq!(c.patterns_in_context_max, 2);
        assert_eq!(c.reconcile_members_max, 64);
        assert_eq!(c.subject_warning_scan_max, 256);
        assert_eq!(c.evidence_lookups_per_event_max, 8);
        assert_eq!(c.verify_pass_evidence_max, 200);
        assert_eq!(c.verify_pass_runs_max, 50);
        assert_eq!(c.verify_pass_wall_ms, 2000);
        assert_eq!(c.evidence_value_max_bytes, 256);
        assert_eq!(c.evidence_locator_max_bytes, 256);
        assert_eq!(c.pattern_signals_min, 2);

        // Relationships that would be defects if they inverted.
        assert!(
            c.pin_budget_per_scope < c.pin_budget_project,
            "a per-scope budget at or above the project budget makes the project one unreachable"
        );
        assert!(
            c.pins_in_context_max <= c.pin_budget_per_scope,
            "context would offer more pins than a scope can hold"
        );
        assert!(
            c.min_context_budget_tokens < c.context_budget_tokens,
            "the documented minimum must be below the default budget"
        );
        assert!(
            c.min_safe_context_fraction > 0.0 && c.min_safe_context_fraction < 1.0,
            "a reserve of the whole budget would leave Level 1 nothing"
        );
    }

    #[test]
    fn the_reserve_is_a_floor_of_the_fraction() {
        let c = CairnConfig::default();
        assert_eq!(c.context_reserve(3000), 1200);
        assert_eq!(c.context_reserve(600), 240);
        assert_eq!(c.context_reserve(0), 0);
        // Never more than the budget, whatever the arithmetic rounds to.
        for limit in [1usize, 7, 999, 12_000] {
            assert!(c.context_reserve(limit) <= limit);
        }
    }

    #[test]
    fn a_config_written_before_feature_003_still_loads() {
        // Backward compatibility: a `config.json` from alpha.4 has none of the
        // new keys, and must load with every bound at its default rather than
        // failing (FR-497's spirit, applied to configuration).
        let old = r#"{"payload_cap_bytes": 8192, "excluded_paths": ["secrets/**"]}"#;
        let c: CairnConfig = serde_json::from_str(old).expect("an alpha.4 config still parses");
        assert_eq!(c.payload_cap_bytes, 8192);
        assert_eq!(c.excluded_paths, vec!["secrets/**".to_string()]);
        assert_eq!(c.reconcile_members_max, 64);
        assert_eq!(c.min_safe_context_fraction, 0.40);
    }

    #[test]
    fn a_bound_can_be_overridden_through_the_config_file() {
        // FR-500 requires every bound be adjustable through the existing file.
        let tuned = r#"{"warnings_in_context_max": 2, "pattern_signals_min": 3}"#;
        let c: CairnConfig = serde_json::from_str(tuned).expect("parses");
        assert_eq!(c.warnings_in_context_max, 2);
        assert_eq!(c.pattern_signals_min, 3);
        assert_eq!(
            c.pins_in_context_max, 4,
            "untouched bounds keep their default"
        );
    }

    #[test]
    fn double_star_crosses_directories_single_star_does_not() {
        assert!(glob_match("secrets/**", "secrets/prod.env"));
        assert!(glob_match("secrets/**", "secrets/nested/deep/prod.env"));
        assert!(glob_match("src/*.rs", "src/main.rs"));
        assert!(!glob_match("src/*.rs", "src/deep/main.rs"));
    }

    #[test]
    fn command_exclusions_match_prefixes() {
        let c = CairnConfig {
            excluded_commands: vec!["aws sts*".into()],
            ..Default::default()
        };
        assert!(c.is_command_excluded("aws sts get-caller-identity"));
        assert!(!c.is_command_excluded("cargo test"));
    }

    #[test]
    fn path_exclusion_is_honoured() {
        let c = CairnConfig {
            excluded_paths: vec!["secrets/**".into()],
            ..Default::default()
        };
        assert!(c.is_path_excluded("secrets/prod.env"));
        assert!(!c.is_path_excluded("src/lib.rs"));
    }

    #[test]
    fn roundtrips_through_disk() {
        let dir = std::env::temp_dir().join(format!("cairn-cfg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.json");
        let mut c = CairnConfig::default();
        c.excluded_paths.push("secrets/**".into());
        c.save_to(&path).unwrap();
        let back = CairnConfig::load_from(&path).unwrap();
        assert_eq!(back.excluded_paths, c.excluded_paths);
        std::fs::remove_dir_all(&dir).ok();
    }
}
