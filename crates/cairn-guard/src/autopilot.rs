//! Drift autopilot policy (v0.8.0 Sprint 8): decide whether a drift event auto-approves or
//! waits for a human. `Risk::Danger` **never** auto-approves - that's a hard rule, not a
//! config knob, regardless of `DriftAutopilot` mode.

use crate::Risk;
use regex::Regex;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriftAutopilot {
    /// Fully manual - every `warn`/`danger` event holds for a human (pre-Sprint-8 behavior).
    Off,
    /// `ok` always auto-approves; `warn` auto-approves only under a safe-path glob.
    Safe,
    /// `ok` and `warn` always auto-approve. `danger` still never does.
    All,
}

impl DriftAutopilot {
    /// Parse `CAIRN_DRIFT_AUTOPILOT`'s raw value. Anything unrecognized (including empty/unset,
    /// which resolves to `Config::drift_autopilot`'s default) falls back to `Safe`.
    pub fn parse(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "off" => Self::Off,
            "all" => Self::All,
            _ => Self::Safe,
        }
    }
}

/// Decide whether a drift event auto-approves. `Some(reason)` means auto-approve, tagging the
/// audit trail with `reason` instead of a human's identity; `None` means hold for a human.
pub fn autopilot_decision(
    mode: DriftAutopilot,
    risk: Risk,
    path: &str,
    safe_globs: &[String],
) -> Option<&'static str> {
    if risk == Risk::Danger {
        return None;
    }
    match mode {
        DriftAutopilot::Off => None,
        DriftAutopilot::All => Some("autopilot:all"),
        DriftAutopilot::Safe => match risk {
            Risk::Ok => Some("autopilot:ok"),
            Risk::Warn if glob_match_any(path, safe_globs) => Some("autopilot:safe-path"),
            _ => None,
        },
    }
}

/// `true` if `path` matches any of `globs`. Supports `*` (anything but `/`) and `**` (anything
/// including `/`) - the two wildcards `CAIRN_DRIFT_SAFE_GLOBS`'s defaults actually use. No new
/// dependency: each glob is converted to a small anchored regex (`regex` is already a
/// workspace dep, used elsewhere for exactly this kind of pattern matching).
///
/// A glob with no `/` at all (e.g. `*.md`) matches the path's basename anywhere in the tree,
/// not just at the root - the common "any markdown file, wherever it lives" intent. A glob
/// containing `/` matches the whole (backslash-normalized) path.
pub fn glob_match_any(path: &str, globs: &[String]) -> bool {
    let normalized = path.replace('\\', "/");
    globs.iter().any(|g| glob_matches(&normalized, g))
}

fn glob_matches(path: &str, glob: &str) -> bool {
    let re = glob_to_regex(glob);
    if glob.contains('/') {
        re.is_match(path)
    } else {
        let basename = path.rsplit('/').next().unwrap_or(path);
        re.is_match(basename)
    }
}

fn glob_to_regex(glob: &str) -> Regex {
    let mut pattern = String::from("^");
    let mut chars = glob.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '*' => {
                if chars.peek() == Some(&'*') {
                    chars.next();
                    // `**/` at a segment boundary must also match *zero* segments - standard
                    // glob convention (`**/tests/**` matches `tests/foo.rs`, not just
                    // `a/tests/foo.rs`) - so it's `(directory/)*` optional-group, not a bare
                    // `.*` that would force at least the literal `/` right after it.
                    if chars.peek() == Some(&'/') {
                        chars.next();
                        pattern.push_str("(?:.*/)?");
                    } else {
                        pattern.push_str(".*");
                    }
                } else {
                    pattern.push_str("[^/]*");
                }
            }
            '.' | '+' | '(' | ')' | '[' | ']' | '{' | '}' | '^' | '$' | '|' | '\\' => {
                pattern.push('\\');
                pattern.push(c);
            }
            _ => pattern.push(c),
        }
    }
    pattern.push('$');
    // An operator-controlled config glob is expected to compile; a bad one fails safe
    // (matches nothing) rather than panicking the caller.
    Regex::new(&pattern).unwrap_or_else(|_| Regex::new("$^").expect("trivial unmatchable regex"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn globs(patterns: &[&str]) -> Vec<String> {
        patterns.iter().map(|s| s.to_string()).collect()
    }

    fn default_safe_globs() -> Vec<String> {
        globs(&["docs/**", "*.md", "**/tests/**", "**/*.test.*"])
    }

    // --- DriftAutopilot::parse ---

    #[test]
    fn parse_recognizes_off_and_all_case_insensitively() {
        assert_eq!(DriftAutopilot::parse("off"), DriftAutopilot::Off);
        assert_eq!(DriftAutopilot::parse("OFF"), DriftAutopilot::Off);
        assert_eq!(DriftAutopilot::parse("all"), DriftAutopilot::All);
        assert_eq!(DriftAutopilot::parse("ALL"), DriftAutopilot::All);
    }

    #[test]
    fn parse_defaults_to_safe_for_anything_else() {
        assert_eq!(DriftAutopilot::parse("safe"), DriftAutopilot::Safe);
        assert_eq!(DriftAutopilot::parse(""), DriftAutopilot::Safe);
        assert_eq!(DriftAutopilot::parse("bogus"), DriftAutopilot::Safe);
    }

    // --- autopilot_decision ---

    #[test]
    fn danger_never_auto_approves_regardless_of_mode() {
        let safe_globs = default_safe_globs();
        for mode in [
            DriftAutopilot::Off,
            DriftAutopilot::Safe,
            DriftAutopilot::All,
        ] {
            assert_eq!(
                autopilot_decision(mode, Risk::Danger, "docs/readme.md", &safe_globs),
                None,
                "danger must hold in {mode:?} mode"
            );
        }
    }

    #[test]
    fn off_mode_never_auto_approves_ok_or_warn() {
        let safe_globs = default_safe_globs();
        assert_eq!(
            autopilot_decision(DriftAutopilot::Off, Risk::Ok, "docs/readme.md", &safe_globs),
            None
        );
        assert_eq!(
            autopilot_decision(
                DriftAutopilot::Off,
                Risk::Warn,
                "docs/readme.md",
                &safe_globs
            ),
            None
        );
    }

    #[test]
    fn safe_mode_always_approves_ok() {
        let safe_globs = default_safe_globs();
        assert_eq!(
            autopilot_decision(
                DriftAutopilot::Safe,
                Risk::Ok,
                "crates/cairn-core/src/lib.rs",
                &safe_globs
            ),
            Some("autopilot:ok")
        );
    }

    #[test]
    fn safe_mode_approves_warn_under_a_safe_glob() {
        let safe_globs = default_safe_globs();
        assert_eq!(
            autopilot_decision(
                DriftAutopilot::Safe,
                Risk::Warn,
                "docs/architecture.md",
                &safe_globs
            ),
            Some("autopilot:safe-path")
        );
    }

    #[test]
    fn safe_mode_holds_warn_outside_safe_globs() {
        let safe_globs = default_safe_globs();
        assert_eq!(
            autopilot_decision(
                DriftAutopilot::Safe,
                Risk::Warn,
                "crates/cairn-core/src/lib.rs",
                &safe_globs
            ),
            None
        );
    }

    #[test]
    fn all_mode_approves_ok_and_warn_everywhere() {
        let safe_globs = default_safe_globs();
        assert_eq!(
            autopilot_decision(DriftAutopilot::All, Risk::Ok, "crates/x.rs", &safe_globs),
            Some("autopilot:all")
        );
        assert_eq!(
            autopilot_decision(DriftAutopilot::All, Risk::Warn, "crates/x.rs", &safe_globs),
            Some("autopilot:all")
        );
    }

    // --- glob_match_any ---

    #[test]
    fn bare_extension_glob_matches_anywhere_in_the_tree() {
        let g = globs(&["*.md"]);
        assert!(glob_match_any("readme.md", &g));
        assert!(glob_match_any("docs/architecture.md", &g));
        assert!(glob_match_any("a/b/c/notes.md", &g));
        assert!(!glob_match_any("readme.mdx", &g));
        assert!(!glob_match_any("lib.rs", &g));
    }

    #[test]
    fn directory_double_star_glob_matches_nested_paths() {
        let g = globs(&["docs/**"]);
        assert!(glob_match_any("docs/architecture.md", &g));
        assert!(glob_match_any("docs/guides/upgrading.md", &g));
        assert!(
            !glob_match_any("crates/docs/x.rs", &g),
            "must anchor at the start"
        );
        assert!(!glob_match_any("other/docs.md", &g));
    }

    #[test]
    fn middle_double_star_glob_matches_a_tests_segment_anywhere() {
        let g = globs(&["**/tests/**"]);
        assert!(glob_match_any("crates/cairn-tests/tests/foo.rs", &g));
        assert!(glob_match_any("tests/unit/bar.rs", &g));
        assert!(!glob_match_any("crates/cairn-tests/src/lib.rs", &g));
    }

    #[test]
    fn windows_backslash_paths_are_normalized() {
        let g = globs(&["docs/**"]);
        assert!(glob_match_any(r"docs\architecture.md", &g));
    }

    #[test]
    fn glob_matches_none_of_the_defaults_for_a_core_source_file() {
        assert!(!glob_match_any(
            "crates/cairn-core/src/config.rs",
            &default_safe_globs()
        ));
    }
}
