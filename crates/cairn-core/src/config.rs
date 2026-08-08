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
        }
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
