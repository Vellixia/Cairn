//! Per-session touched-file buffer for `PostToolUse` (v0.8.0 client redesign).
//!
//! `PostToolUse` used to be a documented no-op - every single edit spawned a
//! `cairn hook PostToolUse` process that did nothing, since nothing consumed
//! the event. This makes it useful with ZERO network cost on the edit hot
//! path: append the touched file path to a local per-session buffer, and let
//! `SessionEnd`/`PreCompact` (which already make network calls) flush the
//! deduped list as one session-scoped memory that `/api/memory/session-summary`
//! folds in.

use std::collections::HashSet;
use std::io::Write;
use std::path::PathBuf;

fn buffer_path(session_id: &str) -> Option<PathBuf> {
    crate::paths::cairn_home().map(|h| h.join("sessions").join(format!("{session_id}.files")))
}

/// Append `file_path` to the session's touched-file buffer. Best-effort - a
/// hook must never fail the agent turn over its own bookkeeping.
pub fn record_touch(session_id: &str, file_path: &str) {
    let Some(path) = buffer_path(session_id) else {
        return;
    };
    let Some(parent) = path.parent() else { return };
    if std::fs::create_dir_all(parent).is_err() {
        return;
    }
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let _ = writeln!(f, "{file_path}");
    }
}

/// Read back the deduped, order-preserving list of files touched this
/// session, and delete the buffer. Returns an empty vec (not an error) when
/// there's nothing to flush - e.g. a session with no file edits.
pub fn drain(session_id: &str) -> Vec<String> {
    let Some(path) = buffer_path(session_id) else {
        return Vec::new();
    };
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    let _ = std::fs::remove_file(&path);
    let mut seen = HashSet::new();
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .filter(|l| seen.insert(*l))
        .map(String::from)
        .collect()
}

/// Delete session-file buffers older than 7 days. A session that never
/// reaches `SessionEnd`/`PreCompact` (crash, force-quit) would otherwise leak
/// its buffer file forever. Called once at `SessionStart`.
pub fn sweep_orphans() {
    sweep_orphans_older_than(std::time::Duration::from_secs(7 * 24 * 3600));
}

/// `max_age` is a parameter (rather than the 7-day constant being inlined)
/// purely so tests can exercise the boundary with a millisecond-scale age
/// instead of needing to backdate a real file's mtime (no `filetime`
/// dependency needed - a short real sleep gives genuine mtime separation).
fn sweep_orphans_older_than(max_age: std::time::Duration) {
    let Some(dir) = crate::paths::cairn_home().map(|h| h.join("sessions")) else {
        return;
    };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return;
    };
    let cutoff = std::time::SystemTime::now()
        .checked_sub(max_age)
        .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("files") {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        let Ok(modified) = meta.modified() else {
            continue;
        };
        if modified < cutoff {
            let _ = std::fs::remove_file(&path);
        }
    }
}

/// Best-effort extraction of the file path a tool call touched, tolerant of
/// the field-name differences across agents: Claude Code/Codex send
/// `tool_input.file_path`; some tools use a bare `path`; OpenCode's plugin
/// forwards whatever the underlying tool's input shape is verbatim. Returns
/// `None` (never a network call, never a panic) when nothing recognizable is
/// found - PostToolUse simply records nothing for that call.
pub fn extract_touched_path(payload: &serde_json::Value) -> Option<String> {
    let input = payload.get("tool_input").unwrap_or(payload);
    for key in ["file_path", "filePath", "path", "notebook_path"] {
        if let Some(p) = input.get(key).and_then(serde_json::Value::as_str) {
            return Some(p.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::env_guard::with_env;

    fn temp_home() -> (tempfile::TempDir, String) {
        let dir = tempfile::tempdir().unwrap();
        let s = dir.path().to_string_lossy().into_owned();
        (dir, s)
    }

    #[test]
    fn record_and_drain_dedups_preserving_order() {
        let (_home, home_str) = temp_home();
        with_env(
            &[("HOME", Some(&home_str)), ("USERPROFILE", Some(&home_str))],
            || {
                record_touch("sess-1", "a.rs");
                record_touch("sess-1", "b.rs");
                record_touch("sess-1", "a.rs");
                let files = drain("sess-1");
                assert_eq!(files, vec!["a.rs".to_string(), "b.rs".to_string()]);
            },
        );
    }

    #[test]
    fn drain_deletes_the_buffer() {
        let (_home, home_str) = temp_home();
        with_env(
            &[("HOME", Some(&home_str)), ("USERPROFILE", Some(&home_str))],
            || {
                record_touch("sess-2", "a.rs");
                assert!(!drain("sess-2").is_empty());
                assert!(
                    drain("sess-2").is_empty(),
                    "second drain must find nothing left"
                );
            },
        );
    }

    #[test]
    fn drain_with_no_prior_touches_is_empty() {
        let (_home, home_str) = temp_home();
        with_env(
            &[("HOME", Some(&home_str)), ("USERPROFILE", Some(&home_str))],
            || {
                assert!(drain("never-touched").is_empty());
            },
        );
    }

    #[test]
    fn sweep_orphans_older_than_removes_entries_past_max_age() {
        let (home, home_str) = temp_home();
        with_env(
            &[("HOME", Some(&home_str)), ("USERPROFILE", Some(&home_str))],
            || {
                record_touch("old-sess", "a.rs");
                std::thread::sleep(std::time::Duration::from_millis(50));
                sweep_orphans_older_than(std::time::Duration::from_millis(10));
                assert!(
                    !home.path().join(".cairn/sessions/old-sess.files").exists(),
                    "an entry older than max_age must be swept"
                );
            },
        );
    }

    #[test]
    fn sweep_orphans_older_than_keeps_entries_within_max_age() {
        let (home, home_str) = temp_home();
        with_env(
            &[("HOME", Some(&home_str)), ("USERPROFILE", Some(&home_str))],
            || {
                record_touch("fresh-sess", "b.rs");
                sweep_orphans_older_than(std::time::Duration::from_secs(3600));
                assert!(
                    home.path()
                        .join(".cairn/sessions/fresh-sess.files")
                        .exists(),
                    "an entry within max_age must survive the sweep"
                );
            },
        );
    }

    #[test]
    fn extract_touched_path_tries_known_field_names() {
        let payload = serde_json::json!({ "tool_input": { "file_path": "a.rs" } });
        assert_eq!(extract_touched_path(&payload).as_deref(), Some("a.rs"));

        let payload = serde_json::json!({ "tool_input": { "filePath": "b.rs" } });
        assert_eq!(extract_touched_path(&payload).as_deref(), Some("b.rs"));

        let payload = serde_json::json!({ "path": "c.rs" });
        assert_eq!(extract_touched_path(&payload).as_deref(), Some("c.rs"));

        let payload = serde_json::json!({ "tool_input": { "command": "ls" } });
        assert_eq!(extract_touched_path(&payload), None);
    }
}
