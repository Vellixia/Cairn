//! Offline hook spool (v0.8.0 Sprint 9). Every hook call today is fire-and-forget
//! (`let _ = self.post(...)`) - if the server is unreachable, the content (most importantly,
//! the memory `UserPromptSubmit` records on every prompt) is silently and permanently lost.
//!
//! `RemoteClient::post_spooled` queues a request here on a genuine connectivity failure
//! (`ureq::Error::Transport` - the request never got a response at all, unlike an HTTP error
//! status which means the server *was* reached). [`replay`] drains the queue at the top of the
//! next `SessionStart` that can reach the server.
//!
//! Deliberately POST-only: every spooled endpoint is a mutation the hook already fires and
//! forgets. GET calls exist to inject *live* context into the current prompt - replaying a
//! stale one later has no meaning, so they're never spooled.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::Write;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SpoolEntry {
    pub path: String,
    pub body: Value,
    pub project_id: Option<String>,
    pub session_id: Option<String>,
    pub ts: chrono::DateTime<chrono::Utc>,
}

fn spool_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)?;
    Some(home.join(".cairn").join("spool.jsonl"))
}

/// Append one failed request to the spool. Best-effort: if `~/.cairn` can't be created or
/// written (read-only home, disk full), the request is dropped exactly like it always was
/// before Sprint 9 - a hook must never fail the agent turn over its own bookkeeping.
pub(crate) fn append(entry: &SpoolEntry) {
    let Some(path) = spool_path() else { return };
    let Some(parent) = path.parent() else { return };
    if std::fs::create_dir_all(parent).is_err() {
        return;
    }
    let Ok(line) = serde_json::to_string(entry) else {
        return;
    };
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(f, "{line}");
    }
}

/// Drain the spool against `server`/`token`. Replays each entry with the scope headers it was
/// originally queued with (which may belong to an earlier session/project than the current
/// one) so it lands in the same place it would have if the original call had succeeded.
///
/// Only entries that get a real response - success or a hard HTTP error - are removed from the
/// queue; an entry that hits another transport failure (still offline) is kept for the next
/// attempt, so a spool never drains data on a bad connection.
pub(crate) fn replay(server: &str, token: &str) {
    let Some(path) = spool_path() else { return };
    let Ok(contents) = std::fs::read_to_string(&path) else {
        return;
    };
    if contents.trim().is_empty() {
        return;
    }
    let server = server.trim_end_matches('/');
    let mut remaining = Vec::new();
    for line in contents.lines() {
        if line.trim().is_empty() {
            continue;
        }
        // An unparseable line (hand-edited file, truncated write) is dropped rather than
        // jamming every entry behind it in the queue forever.
        let Ok(entry) = serde_json::from_str::<SpoolEntry>(line) else {
            continue;
        };
        let req = ureq::post(&format!("{server}{}", entry.path))
            .set("Authorization", &format!("Bearer {token}"));
        let req = match &entry.project_id {
            Some(pid) => req.set("X-Cairn-Project", pid),
            None => req,
        };
        let req = match &entry.session_id {
            Some(sid) => req.set("X-Cairn-Session", sid),
            None => req,
        };
        if let Err(ureq::Error::Transport(_)) = req.send_json(entry.body.clone()) {
            remaining.push(entry);
        }
    }
    let _ = if remaining.is_empty() {
        std::fs::remove_file(&path)
    } else {
        let joined = remaining
            .iter()
            .filter_map(|e| serde_json::to_string(e).ok())
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(&path, joined + "\n")
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// `spool_path` reads `HOME`/`USERPROFILE`; serialize tests that touch them.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn with_temp_home<T>(f: impl FnOnce(&std::path::Path) -> T) -> T {
        let _guard = ENV_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let prev_home = std::env::var_os("HOME");
        let prev_userprofile = std::env::var_os("USERPROFILE");
        std::env::set_var("HOME", dir.path());
        std::env::set_var("USERPROFILE", dir.path());
        let result = f(dir.path());
        match prev_home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
        match prev_userprofile {
            Some(v) => std::env::set_var("USERPROFILE", v),
            None => std::env::remove_var("USERPROFILE"),
        }
        result
    }

    fn entry(path: &str) -> SpoolEntry {
        SpoolEntry {
            path: path.to_string(),
            body: serde_json::json!({"content": "test"}),
            project_id: Some("proj-a".to_string()),
            session_id: Some("sess-1".to_string()),
            ts: chrono::Utc::now(),
        }
    }

    #[test]
    fn append_creates_dir_and_file_on_first_write() {
        with_temp_home(|home| {
            append(&entry("/api/memory"));
            let contents = std::fs::read_to_string(home.join(".cairn").join("spool.jsonl")).unwrap();
            assert!(contents.contains("/api/memory"));
            assert!(contents.contains("proj-a"));
        });
    }

    #[test]
    fn append_is_additive_across_multiple_calls() {
        with_temp_home(|home| {
            append(&entry("/api/memory"));
            append(&entry("/api/guard/anchor/auto"));
            let contents = std::fs::read_to_string(home.join(".cairn").join("spool.jsonl")).unwrap();
            assert_eq!(contents.lines().count(), 2);
        });
    }

    #[test]
    fn replay_against_an_unreachable_server_keeps_every_entry_queued() {
        with_temp_home(|home| {
            append(&entry("/api/memory"));
            append(&entry("/api/projects/upsert"));
            // Port 1 is reserved and never accepts connections - a fast, deterministic
            // "unreachable" without depending on any real network state.
            replay("http://127.0.0.1:1", "test-token");
            let contents = std::fs::read_to_string(home.join(".cairn").join("spool.jsonl")).unwrap();
            assert_eq!(contents.lines().count(), 2, "still-offline entries must stay queued");
        });
    }

    #[test]
    fn replay_with_no_spool_file_is_a_silent_noop() {
        with_temp_home(|home| {
            replay("http://127.0.0.1:1", "test-token");
            assert!(!home.join(".cairn").join("spool.jsonl").exists());
        });
    }
}
