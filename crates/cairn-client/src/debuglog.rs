//! `CAIRN_DEBUG=1` / `[hooks] debug = true` file logging for `cairn hook`.
//!
//! Hook errors have always gone to stderr, which the calling agent swallows -
//! nobody ever sees them, so a broken hook was undiagnosable without editing
//! the source. This gives a durable, opt-in trail: `~/.cairn/logs/hook.log`,
//! one block per hook invocation, rotated once at 1 MiB to `hook.log.1` (one
//! generation kept) so it can never grow unbounded.

use std::io::Write;
use std::path::PathBuf;

const MAX_LOG_BYTES: u64 = 1024 * 1024;

pub struct DebugLog {
    enabled: bool,
    lines: Vec<String>,
}

impl DebugLog {
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            lines: Vec::new(),
        }
    }

    /// Record one line if logging is enabled; a cheap no-op otherwise (the
    /// caller still pays for building the `line` argument, so hot paths
    /// should check `is_enabled()` first if that cost matters).
    pub fn record(&mut self, line: impl Into<String>) {
        if self.enabled {
            self.lines.push(line.into());
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Append everything recorded so far as one timestamped block, rotating
    /// the file first if it's grown past `MAX_LOG_BYTES`. Best-effort: a
    /// logging failure must never surface as a hook failure.
    pub fn flush(&self, event: &str, project: Option<&str>, elapsed_ms: u128) {
        if !self.enabled || self.lines.is_empty() {
            return;
        }
        let Some(path) = log_path() else { return };
        let Some(parent) = path.parent() else { return };
        if std::fs::create_dir_all(parent).is_err() {
            return;
        }
        rotate_if_large(&path);

        let mut block = format!(
            "[{}] event={event} project={} total={elapsed_ms}ms\n",
            chrono::Utc::now().to_rfc3339(),
            project.unwrap_or("(none)")
        );
        for line in &self.lines {
            block.push_str("  ");
            block.push_str(line);
            block.push('\n');
        }

        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
        {
            let _ = f.write_all(block.as_bytes());
        }
    }
}

fn log_path() -> Option<PathBuf> {
    crate::paths::cairn_home().map(|h| h.join("logs").join("hook.log"))
}

fn rotate_if_large(path: &std::path::Path) {
    let Ok(meta) = std::fs::metadata(path) else {
        return;
    };
    if meta.len() < MAX_LOG_BYTES {
        return;
    }
    if let Some(parent) = path.parent() {
        let _ = std::fs::rename(path, parent.join("hook.log.1"));
    }
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
    fn disabled_log_writes_nothing() {
        let (home, home_str) = temp_home();
        with_env(
            &[("HOME", Some(&home_str)), ("USERPROFILE", Some(&home_str))],
            || {
                let mut log = DebugLog::new(false);
                log.record("GET /api/x -> 200 (12ms)");
                log.flush("SessionStart", Some("proj"), 12);
                assert!(!home.path().join(".cairn/logs/hook.log").exists());
            },
        );
    }

    #[test]
    fn enabled_log_writes_a_block_with_event_and_lines() {
        let (home, home_str) = temp_home();
        with_env(
            &[("HOME", Some(&home_str)), ("USERPROFILE", Some(&home_str))],
            || {
                let mut log = DebugLog::new(true);
                assert!(log.is_enabled());
                log.record("GET /api/guard/anchor -> 200 (10ms)");
                log.record("GET /api/memory/wakeup -> 200 (15ms)");
                log.flush("SessionStart", Some("proj-abc"), 25);

                let text =
                    std::fs::read_to_string(home.path().join(".cairn/logs/hook.log")).unwrap();
                assert!(text.contains("event=SessionStart"));
                assert!(text.contains("project=proj-abc"));
                assert!(text.contains("total=25ms"));
                assert!(text.contains("GET /api/guard/anchor -> 200 (10ms)"));
                assert!(text.contains("GET /api/memory/wakeup -> 200 (15ms)"));
            },
        );
    }

    #[test]
    fn empty_recording_flushes_nothing() {
        let (home, home_str) = temp_home();
        with_env(
            &[("HOME", Some(&home_str)), ("USERPROFILE", Some(&home_str))],
            || {
                let log = DebugLog::new(true);
                log.flush("PostToolUse", None, 0);
                assert!(!home.path().join(".cairn/logs/hook.log").exists());
            },
        );
    }

    #[test]
    fn appends_across_multiple_flushes() {
        let (home, home_str) = temp_home();
        with_env(
            &[("HOME", Some(&home_str)), ("USERPROFILE", Some(&home_str))],
            || {
                let mut first = DebugLog::new(true);
                first.record("a");
                first.flush("SessionStart", None, 1);

                let mut second = DebugLog::new(true);
                second.record("b");
                second.flush("SessionEnd", None, 2);

                let text =
                    std::fs::read_to_string(home.path().join(".cairn/logs/hook.log")).unwrap();
                assert!(text.contains("event=SessionStart"));
                assert!(text.contains("event=SessionEnd"));
            },
        );
    }

    #[test]
    fn rotates_when_past_max_size() {
        let (home, home_str) = temp_home();
        with_env(
            &[("HOME", Some(&home_str)), ("USERPROFILE", Some(&home_str))],
            || {
                let log_path = home.path().join(".cairn/logs/hook.log");
                std::fs::create_dir_all(log_path.parent().unwrap()).unwrap();
                // Pre-fill past the 1 MiB rotation threshold.
                std::fs::write(&log_path, vec![b'x'; (MAX_LOG_BYTES + 1) as usize]).unwrap();

                let mut log = DebugLog::new(true);
                log.record("fresh entry");
                log.flush("SessionStart", None, 1);

                assert!(
                    home.path().join(".cairn/logs/hook.log.1").exists(),
                    "the oversized file must be rotated to hook.log.1"
                );
                let text = std::fs::read_to_string(&log_path).unwrap();
                assert!(
                    text.contains("fresh entry"),
                    "the new log file must contain only the fresh block, not the old bulk"
                );
                assert!(
                    text.len() < (MAX_LOG_BYTES as usize),
                    "new file must not carry over the old bulk"
                );
            },
        );
    }
}
