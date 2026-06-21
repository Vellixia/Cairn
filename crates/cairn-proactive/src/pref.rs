//! Per-project opt-out preference (v0.5.0 Sprint 18c).
//!
//! Stored as a `Memory` with `kind = Preference` and `applies_to = [project_root]`.
//! A `ProactivePref` is the in-memory view of the set of opted-out project
//! roots — refreshed on each `cairn-cli prefer` call. The match is prefix-based
//! so opting out of `/work/foo` also opts out of `/work/foo/subdir`.
//!
//! The `PROJECT_OPT_OUT` constant is the magic content string the agentmemory
//! pattern uses to find a "remember to disable proactive recall for project X"
//! preference. Any other matching preference is treated equivalently.

use serde::{Deserialize, Serialize};

/// The canonical content marker for a proactive-recall opt-out preference.
pub const PROJECT_OPT_OUT: &str = "cairn.proactive_recall=false";

/// In-memory list of opted-out project roots. Cheap to clone (small).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProactivePref {
    opted_out: Vec<String>,
}

impl ProactivePref {
    /// Mark `project_root` (or any subdirectory of it) as opted out.
    pub fn with_opt_out(mut self, project_root: impl Into<String>) -> Self {
        let r = project_root.into();
        if !self.opted_out.iter().any(|p| p == &r) {
            self.opted_out.push(r);
        }
        self
    }

    /// Drop a previously-set opt-out (rare — the agentmemory store is append-only
    /// in spirit, so this is mostly for tests).
    pub fn without_opt_out(mut self, project_root: &str) -> Self {
        self.opted_out.retain(|p| p != project_root);
        self
    }

    /// True if `project_root` (or any ancestor) is in the opted-out set.
    pub fn is_opted_out(&self, project_root: &str) -> bool {
        self.opted_out
            .iter()
            .any(|prefix| project_root == prefix || project_root.starts_with(&format!("{prefix}/")))
    }

    /// Build a ProactivePref by scanning a slice of (content, applies_to) tuples.
    /// A preference memory whose content contains `PROJECT_OPT_OUT` and has any
    /// non-empty `applies_to` opts out that prefix.
    pub fn from_memories<'a, I>(mems: I) -> Self
    where
        I: IntoIterator<Item = &'a (String, Vec<String>)>,
    {
        let mut out = ProactivePref::default();
        for (content, applies_to) in mems {
            if content.contains(PROJECT_OPT_OUT) {
                for root in applies_to {
                    out = out.with_opt_out(root.clone());
                }
            }
        }
        out
    }

    /// All opted-out prefixes (for diagnostics).
    pub fn opted_out_roots(&self) -> &[String] {
        &self.opted_out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opt_out_matches_exact_and_prefix() {
        let p = ProactivePref::default().with_opt_out("/work/foo");
        assert!(p.is_opted_out("/work/foo"));
        assert!(p.is_opted_out("/work/foo/sub"));
        assert!(p.is_opted_out("/work/foo/sub/deep"));
        assert!(!p.is_opted_out("/work/bar"));
        assert!(!p.is_opted_out("/work"));
        assert!(!p.is_opted_out("/work-foo"));
    }

    #[test]
    fn without_opt_out_drops_a_root() {
        let p = ProactivePref::default()
            .with_opt_out("/work/a")
            .with_opt_out("/work/b")
            .without_opt_out("/work/a");
        assert!(!p.is_opted_out("/work/a"));
        assert!(p.is_opted_out("/work/b"));
    }

    #[test]
    fn from_memories_picks_up_marker_in_content() {
        let mems = vec![
            (
                "cairn.proactive_recall=false for noisy repo".to_string(),
                vec!["/repos/noisy".to_string()],
            ),
            (
                "remember tabs over spaces".to_string(),
                vec!["/work/foo".to_string()],
            ),
        ];
        let p = ProactivePref::from_memories(&mems);
        assert!(p.is_opted_out("/repos/noisy"));
        assert!(p.is_opted_out("/repos/noisy/sub"));
        assert!(!p.is_opted_out("/work/foo"));
    }
}
