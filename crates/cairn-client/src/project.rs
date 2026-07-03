//! Shared project detection (v0.8.0 Sprint 3). Used by `hook.rs` (so every hook invocation can
//! attach `X-Cairn-Project`, scoping memories to the current repo automatically - see the v0.8.0
//! Sprint 2 scope model) and by `doctor.rs` (diagnostic display of what would be detected).

use cairn_core::ContentHash;

/// Detect the current project. Priority: `CAIRN_PROJECT` env override, then the git repo root
/// (`git rev-parse --show-toplevel`), then the current directory's basename. Returns `(id,
/// name)` - `id` is a 16-hex-char hash of the resolved *path*, not the repo's identity, so the
/// same repo cloned to two different paths (or opened as two different worktrees) gets two
/// different ids. That's an accepted tradeoff: it needs no network call and no `.git/config`
/// parsing, and in practice an agent works from one path per machine. `None` only when every
/// detection path fails (e.g. `$PWD` itself no longer exists).
pub fn detect_project() -> (Option<String>, String) {
    if let Ok(name) = std::env::var("CAIRN_PROJECT") {
        let name = name.trim();
        if !name.is_empty() {
            return (Some(project_hash(name)), name.to_string());
        }
    }

    if let Ok(out) = std::process::Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()
    {
        if out.status.success() {
            let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !path.is_empty() {
                let canonical =
                    std::fs::canonicalize(&path).unwrap_or_else(|_| std::path::PathBuf::from(&path));
                let name = canonical
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "unknown".to_string());
                return (Some(project_hash(&canonical.to_string_lossy())), name);
            }
        }
    }

    if let Ok(cwd) = std::env::current_dir() {
        let name = cwd
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "unknown".to_string());
        return (Some(project_hash(&cwd.to_string_lossy())), name);
    }

    (None, "unknown".to_string())
}

/// 16 hex chars (64 bits) of a SHA-256 over the resolved path - collision-resistant enough for
/// scoping, short enough to be a readable `scope_id`. Reuses `cairn_core::ContentHash` rather
/// than pulling `sha2`/`hex` into `cairn-client` directly.
fn project_hash(path: &str) -> String {
    ContentHash::of_str(path).as_str()[..16].to_string()
}

/// The current working directory as a string, empty on failure - used as the `path` field when
/// registering a detected project with the server.
pub fn current_dir_str() -> String {
    std::env::current_dir()
        .ok()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::env_guard::with_env;

    #[test]
    fn env_override_wins_and_is_stable() {
        with_env(&[("CAIRN_PROJECT", Some("my-explicit-project"))], || {
            let (id1, name1) = detect_project();
            let (id2, name2) = detect_project();
            assert_eq!(name1, "my-explicit-project");
            assert_eq!(name2, "my-explicit-project");
            assert_eq!(id1, id2, "hashing the same name must be stable");
            assert_eq!(id1.unwrap().len(), 16);
        });
    }

    #[test]
    fn blank_env_override_falls_through() {
        with_env(&[("CAIRN_PROJECT", Some("   "))], || {
            let (id, name) = detect_project();
            // Falls through to git/cwd detection - some id, and definitely not the blank string.
            assert!(id.is_some());
            assert_ne!(name, "   ");
        });
    }

    #[test]
    fn project_hash_is_16_hex_chars() {
        let h = project_hash("/some/path");
        assert_eq!(h.len(), 16);
        assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
