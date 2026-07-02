//! The shell-pattern registry. One slice, the single source of truth for which commands
//! get which compression treatment. Adding a new pattern: drop a `Pattern` literal into
//! the appropriate `patterns/*.rs` and re-export it from `patterns/mod.rs`.
//!
//! Lookup is linear (O(n)) but `n` is bounded by the number of patterns (~30 in v1), so
//! no hashmap needed. The first matching pattern wins.

use crate::category::Category;
use crate::patterns;
use crate::pipeline;

/// One compression rule.
#[derive(Debug, Clone, Copy)]
pub struct Pattern {
    /// Stable name for diagnostics + dashboard. Lowercase, kebab-case.
    pub name: &'static str,
    pub category: Category,
    /// Tokens that must all appear in the lowercased command line (word-bounded). The
    /// pattern matches when every required token is present.
    pub matchers: &'static [&'static str],
    /// Lines to KEEP (filter_keep). Mutually exclusive with `drop`.
    pub keep: Option<&'static [&'static str]>,
    /// Lines to DROP (filter_drop). Mutually exclusive with `keep`.
    pub drop: Option<&'static [&'static str]>,
}

/// The full registry. Aggregated from `patterns/*.rs` so each category file owns its own
/// patterns. Order matters: first match wins.
pub const REGISTRY: &[Pattern] = &[
    // ---- Vcs (version control) ----
    patterns::vcs::GIT_STATUS,
    patterns::vcs::GIT_DIFF_STAT,
    // ---- Build ----
    patterns::build::CARGO_TEST,
    patterns::build::CARGO_BUILD,
    patterns::build::MAKE,
    // ---- Package managers ----
    patterns::package::NPM_INSTALL,
    patterns::package::PNPM_INSTALL,
    patterns::package::PIP_INSTALL,
    // ---- Lint ----
    patterns::lint::ESLINT,
    patterns::lint::TSC,
    // ---- Infra ----
    patterns::infra::DOCKER_PS,
    patterns::infra::KUBECTL_GET,
    // ---- Http ----
    patterns::http::CURL,
    // ---- Search ----
    patterns::search::RG,
    patterns::search::GREP_RN,
    // ---- File read ----
    patterns::fileread::CAT,
    // ---- Dir list ----
    patterns::dirlist::TREE,
    patterns::dirlist::LS,
];

/// Find the first pattern matching `command` (lowercased, word-bounded tokens). Returns
/// `None` for unrecognised commands - caller falls through to the generic pipeline.
pub fn find_match(command: &str) -> Option<&'static Pattern> {
    let c = command.to_lowercase();
    REGISTRY
        .iter()
        .find(|p| p.matchers.iter().all(|w| has_token(&c, w)))
}

/// True when `command` (lowercased) contains `token` as a whole word.
fn has_token(command: &str, token: &str) -> bool {
    command
        .split(|ch: char| !ch.is_alphanumeric())
        .any(|t| t == token)
}

/// Apply a pattern to `output` and return the compressed lines.
pub fn apply(pattern: &Pattern, output: &str) -> String {
    let lines: Vec<&str> = output.lines().collect();
    let filtered: Vec<String> = if let Some(keep) = pattern.keep {
        pipeline::filter_keep(&lines, keep)
    } else if let Some(drop) = pattern.drop {
        pipeline::filter_drop(&lines, drop)
    } else {
        lines.iter().map(|s| s.to_string()).collect()
    };
    let deduped = pipeline::dedup_consecutive(&filtered);
    pipeline::truncate_head_tail(&deduped, 200, 60, 40).join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_status_matches_git_status_command() {
        let p = find_match("git status").unwrap();
        assert_eq!(p.name, "git-status");
        assert_eq!(p.category, Category::Vcs);
    }

    #[test]
    fn cargo_test_matches_cargo_test() {
        let p = find_match("cargo test --workspace").unwrap();
        assert_eq!(p.name, "cargo-test");
        assert_eq!(p.category, Category::Build);
    }

    #[test]
    fn cargo_build_matches_clippy() {
        let p = find_match("cargo clippy --all-targets").unwrap();
        assert_eq!(p.name, "cargo-build");
    }

    #[test]
    fn npm_install_matches_pnpm_run() {
        // pnpm is distinct from npm
        let p = find_match("npm install").unwrap();
        assert_eq!(p.name, "npm-install");
        let p = find_match("pnpm install").unwrap();
        assert_eq!(p.name, "pnpm-install");
    }

    #[test]
    fn unknown_command_falls_through() {
        assert!(find_match("totally-unknown-tool xyz").is_none());
        assert!(find_match("").is_none());
    }

    #[test]
    fn word_boundary_works() {
        // "categorical" must not match "cat"
        assert!(find_match("categorical --help").is_none());
        // but real cat does
        let p = find_match("cat foo.txt").unwrap();
        assert_eq!(p.name, "cat");
    }

    #[test]
    fn first_match_wins() {
        // More specific patterns earlier in REGISTRY win over later generic ones.
        // git status before git diff --stat etc.
        let p = find_match("git status").unwrap();
        assert_eq!(p.name, "git-status");
    }
}
