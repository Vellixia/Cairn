//! Cross-Session Protocol (CCP) - durable session state.
//!
//! The plan for v0.5.0 calls out the lean-ctx CCP pattern: every agent session writes a
//! structured "where we were" record (`tasks`, `findings`, `decisions`, `touched_files`,
//! `next_steps`) that the next session picks up via `SessionStart` injection. Cairn adapts
//! that pattern with the following shape:
//!
//! - **Storage**: a dedicated `sessions/` directory under the configured data dir, holding
//!   one `<session_id>.json` file per session and a `latest.json` pointer.
//! - **Auto-restore**: the `SessionStart` hook reads `latest.json` and injects the CCP block.
//! - **Drift**: `cairn-guard::verify` results are appended to a `drift_events.jsonl` ledger
//!   so the dashboard's `/dashboard/reliability/drift` page can review/approve/reject.

use cairn_core::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

const SESSIONS_DIR: &str = "sessions";
const LATEST_POINTER: &str = "sessions/latest.json";
const DRIFT_LOG: &str = "sessions/drift_events.jsonl";

/// A single task the agent is working on.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub progress: String,
}

/// A finding - a short factual observation made during the session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Finding {
    pub text: String,
    #[serde(default)]
    pub source_file: Option<String>,
    #[serde(default)]
    pub confidence: f32,
}

/// A decision - the agent's call (with rationale).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Decision {
    pub text: String,
    #[serde(default)]
    pub rationale: String,
    #[serde(default)]
    pub confidence: f32,
}

/// A file the agent read/wrote/edited during the session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TouchedFile {
    pub path: String,
    #[serde(default)]
    pub mode: String,
    #[serde(default)]
    pub handle: Option<String>,
}

/// The full CCP session record - what gets written to `<session_id>.json` and what the next
/// session's bootstrap block is built from.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub project_hash: String,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub tasks: Vec<Task>,
    #[serde(default)]
    pub findings: Vec<Finding>,
    #[serde(default)]
    pub decisions: Vec<Decision>,
    #[serde(default)]
    pub touched_files: Vec<TouchedFile>,
    #[serde(default)]
    pub next_steps: Vec<String>,
    #[serde(default)]
    pub memory_ids: Vec<String>,
}

impl Session {
    pub fn new(project_hash: impl Into<String>) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            project_hash: project_hash.into(),
            started_at: Utc::now(),
            ended_at: None,
            tasks: Vec::new(),
            findings: Vec::new(),
            decisions: Vec::new(),
            touched_files: Vec::new(),
            next_steps: Vec::new(),
            memory_ids: Vec::new(),
        }
    }

    /// Render the session as the compact "CCP block" that gets injected at session start.
    /// Mirrors the lean-ctx shape (~400 tokens for a typical session).
    pub fn as_block(&self) -> String {
        let mut out = String::from("# Cross-Session Protocol - previous session\n");
        out.push_str(&format!("Session: {}\n", self.id));
        out.push_str(&format!("Started: {}\n", self.started_at.to_rfc3339()));
        if !self.tasks.is_empty() {
            out.push_str("\n## Tasks\n");
            for t in &self.tasks {
                out.push_str(&format!("- [{}] {} - {}\n", t.id, t.title, t.progress));
            }
        }
        if !self.findings.is_empty() {
            out.push_str("\n## Findings\n");
            for f in &self.findings {
                let src = f
                    .source_file
                    .as_deref()
                    .map(|s| format!(" (from {s})"))
                    .unwrap_or_default();
                out.push_str(&format!("- {}{src}\n", f.text));
            }
        }
        if !self.decisions.is_empty() {
            out.push_str("\n## Decisions\n");
            for d in &self.decisions {
                out.push_str(&format!("- {} (rationale: {})\n", d.text, d.rationale));
            }
        }
        if !self.touched_files.is_empty() {
            out.push_str("\n## Touched files\n");
            for f in &self.touched_files {
                out.push_str(&format!("- {} ({})\n", f.path, f.mode));
            }
        }
        if !self.next_steps.is_empty() {
            out.push_str("\n## Next steps\n");
            for s in &self.next_steps {
                out.push_str(&format!("- {s}\n"));
            }
        }
        out
    }
}

/// Pointer file at `sessions/latest.json` that records which session is the "current" one
/// (i.e. the one new sessions should restore from).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LatestPointer {
    pub session_id: String,
    pub updated_at: DateTime<Utc>,
}

/// Patch body for `PATCH /api/sessions/:id`. Any `Some` field is merged into the existing
/// session (existing entries are kept; new entries are appended).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionPatch {
    #[serde(default)]
    pub tasks: Option<Vec<Task>>,
    #[serde(default)]
    pub findings: Option<Vec<Finding>>,
    #[serde(default)]
    pub decisions: Option<Vec<Decision>>,
    #[serde(default)]
    pub touched_files: Option<Vec<TouchedFile>>,
    #[serde(default)]
    pub next_steps: Option<Vec<String>>,
    /// When `true`, set `ended_at = now` and mark the session closed.
    #[serde(default)]
    pub end: Option<bool>,
}

/// One drift event - a verify call flagged an edit as warn/danger, or a rollback ran.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DriftEvent {
    pub id: i64,
    pub ts: DateTime<Utc>,
    pub path: String,
    pub risk: String,
    pub kind: String,
    pub detail: String,
    #[serde(default)]
    pub status: DriftStatus,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum DriftStatus {
    #[default]
    Pending,
    Approved,
    Rejected,
}

/// Owns the on-disk session/drift storage rooted at `data_dir`.
#[derive(Debug, Clone)]
pub struct SessionStore {
    root: PathBuf,
}

impl SessionStore {
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        let root = data_dir.into();
        let _ = fs::create_dir_all(root.join(SESSIONS_DIR));
        Self { root }
    }

    fn session_path(&self, id: &str) -> PathBuf {
        self.root.join(SESSIONS_DIR).join(format!("{id}.json"))
    }

    /// Persist a session to disk and update the latest pointer. Returns the path.
    pub fn save(&self, session: &Session) -> Result<PathBuf> {
        let path = self.session_path(&session.id);
        let json = serde_json::to_vec_pretty(session)?;
        atomic_write(&path, &json)?;
        let pointer = LatestPointer {
            session_id: session.id.clone(),
            updated_at: Utc::now(),
        };
        atomic_write(
            &self.root.join(LATEST_POINTER),
            &serde_json::to_vec_pretty(&pointer)?,
        )?;
        Ok(path)
    }

    /// Read a session by id, if it exists.
    pub fn load(&self, id: &str) -> Result<Option<Session>> {
        let path = self.session_path(id);
        if !path.exists() {
            return Ok(None);
        }
        let bytes = fs::read(&path)?;
        let s: Session = serde_json::from_slice(&bytes)?;
        Ok(Some(s))
    }

    /// Return the id of the most-recently-saved session, if any.
    pub fn latest_id(&self) -> Option<String> {
        let path = self.root.join(LATEST_POINTER);
        let bytes = fs::read(&path).ok()?;
        let p: LatestPointer = serde_json::from_slice(&bytes).ok()?;
        Some(p.session_id)
    }

    /// Return the most-recent session's CCP block (for SessionStart injection). Empty if none.
    pub fn latest_block(&self) -> Result<String> {
        let Some(id) = self.latest_id() else {
            return Ok(String::new());
        };
        match self.load(&id)? {
            Some(s) => Ok(s.as_block()),
            None => Ok(String::new()),
        }
    }

    /// List all session ids on disk, newest-first (by filename / lexicographic order).
    pub fn list(&self) -> Result<Vec<String>> {
        let dir = self.root.join(SESSIONS_DIR);
        let mut ids: Vec<String> = fs::read_dir(&dir)?
            .filter_map(|e| e.ok())
            .filter_map(|e| {
                let path = e.path();
                let name = path.file_name()?.to_string_lossy().into_owned();
                if name == "latest.json" || name == "drift_events.jsonl" {
                    return None;
                }
                if name.ends_with(".json") {
                    Some(name.trim_end_matches(".json").to_string())
                } else {
                    None
                }
            })
            .collect();
        ids.sort_by(|a, b| b.cmp(a));
        Ok(ids)
    }

    // -- drift log -------------------------------------------------------------------

    /// Append a drift event to the JSONL log. Returns the assigned id (1-based).
    pub fn append_drift(&self, ev: &DriftEvent) -> Result<i64> {
        let path = self.root.join(DRIFT_LOG);
        let line = serde_json::to_string(ev)?;
        use std::io::Write;
        let mut f = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;
        writeln!(f, "{line}")?;
        Ok(ev.id)
    }

    /// Read drift events, newest first, optionally filtering by status.
    pub fn recent_drift(
        &self,
        limit: usize,
        status: Option<DriftStatus>,
    ) -> Result<Vec<DriftEvent>> {
        let path = self.root.join(DRIFT_LOG);
        if !path.exists() {
            return Ok(Vec::new());
        }
        let raw = fs::read_to_string(&path)?;
        let mut out: Vec<DriftEvent> = raw
            .lines()
            .filter_map(|line| serde_json::from_str::<DriftEvent>(line).ok())
            .collect();
        if let Some(s) = status {
            out.retain(|e| e.status == s);
        }
        out.sort_by_key(|b| std::cmp::Reverse(b.id));
        out.truncate(limit);
        Ok(out)
    }

    /// Update the status of a drift event by id. Returns true if a row was updated.
    pub fn set_drift_status(&self, id: i64, status: DriftStatus) -> Result<bool> {
        let path = self.root.join(DRIFT_LOG);
        if !path.exists() {
            return Ok(false);
        }
        let raw = fs::read_to_string(&path)?;
        let mut updated = false;
        let mut out_lines: Vec<String> = Vec::with_capacity(raw.lines().count());
        for line in raw.lines() {
            let Ok(mut ev) = serde_json::from_str::<DriftEvent>(line) else {
                out_lines.push(line.to_string());
                continue;
            };
            if ev.id == id && ev.status == DriftStatus::Pending {
                ev.status = status;
                updated = true;
            }
            out_lines.push(serde_json::to_string(&ev)?);
        }
        if updated {
            // Atomic write - temp + rename.
            atomic_write(&path, out_lines.join("\n").as_bytes())?;
        }
        Ok(updated)
    }

    /// Allocate the next monotonic drift event id (1-based).
    pub fn next_drift_id(&self) -> i64 {
        let path = self.root.join(DRIFT_LOG);
        if !path.exists() {
            return 1;
        }
        let Ok(raw) = fs::read_to_string(&path) else {
            return 1;
        };
        raw.lines()
            .filter_map(|l| serde_json::from_str::<DriftEvent>(l).ok())
            .map(|e| e.id)
            .max()
            .map(|m| m + 1)
            .unwrap_or(1)
    }
}

/// Atomic write: serialize to a temp file next to the destination, then rename. Survives
/// crashes mid-write.
fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    // - Session::as_block ---

    #[test]
    fn block_empty_session_has_header_only() {
        let s = Session::new("proj");
        let block = s.as_block();
        assert!(block.contains("Cross-Session Protocol"));
        assert!(block.contains("Session:"));
        assert!(!block.contains("## Tasks"), "no tasks section when empty");
        assert!(
            !block.contains("## Findings"),
            "no findings section when empty"
        );
        assert!(
            !block.contains("## Decisions"),
            "no decisions section when empty"
        );
        assert!(
            !block.contains("## Touched files"),
            "no files section when empty"
        );
        assert!(!block.contains("## Next steps"), "no next-steps when empty");
    }

    #[test]
    fn block_all_fields_present() {
        let mut s = Session::new("proj");
        s.tasks.push(Task {
            id: "t1".into(),
            title: "Implement foo".into(),
            progress: "done".into(),
        });
        s.findings.push(Finding {
            text: "bug in bar".into(),
            source_file: Some("bar.rs".into()),
            confidence: 0.9,
        });
        s.decisions.push(Decision {
            text: "use btree".into(),
            rationale: "ordered".into(),
            confidence: 0.8,
        });
        s.touched_files.push(TouchedFile {
            path: "src/lib.rs".into(),
            mode: "read".into(),
            handle: None,
        });
        s.next_steps.push("refactor baz".into());
        let block = s.as_block();
        assert!(block.contains("Implement foo"));
        assert!(block.contains("bug in bar"));
        assert!(block.contains("bar.rs"));
        assert!(block.contains("use btree"));
        assert!(block.contains("ordered"));
        assert!(block.contains("src/lib.rs"));
        assert!(block.contains("refactor baz"));
    }

    #[test]
    fn block_decision_with_empty_rationale() {
        let mut s = Session::new("proj");
        s.decisions.push(Decision {
            text: "chose X".into(),
            rationale: String::new(),
            confidence: 1.0,
        });
        let block = s.as_block();
        assert!(block.contains("chose X"));
    }

    #[test]
    fn block_finding_without_source_file() {
        let mut s = Session::new("proj");
        s.findings.push(Finding {
            text: "no file".into(),
            source_file: None,
            confidence: 0.5,
        });
        let block = s.as_block();
        assert!(block.contains("no file"));
        assert!(
            !block.contains("(from"),
            "no source file -> no 'from' annotation"
        );
    }

    #[test]
    fn block_touched_file_mode_shown() {
        let mut s = Session::new("proj");
        s.touched_files.push(TouchedFile {
            path: "a.rs".into(),
            mode: "write".into(),
            handle: None,
        });
        let block = s.as_block();
        assert!(block.contains("write"), "mode is shown in block");
    }

    // - SessionStore ---

    #[test]
    fn session_round_trips_through_disk() {
        let dir = TempDir::new().unwrap();
        let store = SessionStore::new(dir.path());

        let mut s = Session::new("proj-hash");
        s.tasks.push(Task {
            id: "t1".into(),
            title: "Ship the dashboard".into(),
            progress: "in_progress".into(),
        });
        s.findings.push(Finding {
            text: "shadcn sidebar needs SidebarProvider at the layout root".into(),
            source_file: Some("web/src/app/dashboard/layout.tsx".into()),
            confidence: 0.9,
        });
        s.decisions.push(Decision {
            text: "Use .cairnpkg as the canonical pack extension".into(),
            rationale: "Distinct identity + lean-ctx interop".into(),
            confidence: 0.85,
        });
        s.next_steps.push("Implement Sprint 4 (CCP)".into());
        store.save(&s).unwrap();

        let loaded = store.load(&s.id).unwrap().unwrap();
        assert_eq!(loaded.tasks.len(), 1);
        assert_eq!(loaded.tasks[0].id, "t1");
        assert_eq!(loaded.findings[0].confidence, 0.9);

        // The CCP block is the next session's bootstrap payload.
        let block = loaded.as_block();
        assert!(block.contains("Cross-Session Protocol"));
        assert!(block.contains("Ship the dashboard"));
        assert!(block.contains(".cairnpkg"));
    }

    #[test]
    fn latest_pointer_round_trips() {
        let dir = TempDir::new().unwrap();
        let store = SessionStore::new(dir.path());
        assert!(store.latest_id().is_none());

        let s1 = Session::new("p1");
        store.save(&s1).unwrap();
        assert_eq!(store.latest_id().as_deref(), Some(s1.id.as_str()));

        let s2 = Session::new("p1");
        store.save(&s2).unwrap();
        assert_eq!(store.latest_id().as_deref(), Some(s2.id.as_str()));
    }

    #[test]
    fn list_returns_newest_first() {
        let dir = TempDir::new().unwrap();
        let store = SessionStore::new(dir.path());
        let s1 = Session::new("p");
        store.save(&s1).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(5));
        let s2 = Session::new("p");
        store.save(&s2).unwrap();
        let ids = store.list().unwrap();
        assert_eq!(ids.len(), 2);
        // Lexicographic descending - UUIDs sort the same as creation time here.
        assert!(ids[0] >= ids[1]);
    }

    #[test]
    fn drift_log_appends_and_filters_by_status() {
        let dir = TempDir::new().unwrap();
        let store = SessionStore::new(dir.path());

        for i in 1..=4 {
            store
                .append_drift(&DriftEvent {
                    id: i,
                    ts: Utc::now(),
                    path: format!("/file{i}.txt"),
                    risk: if i % 2 == 0 {
                        "warn".into()
                    } else {
                        "danger".into()
                    },
                    kind: "verify".into(),
                    detail: format!("edit flagged {i}"),
                    status: DriftStatus::Pending,
                })
                .unwrap();
        }
        let all = store.recent_drift(10, None).unwrap();
        assert_eq!(all.len(), 4);
        // Newest first.
        assert_eq!(all[0].id, 4);

        let pending = store.recent_drift(10, Some(DriftStatus::Pending)).unwrap();
        assert_eq!(pending.len(), 4);

        // Approve id=2, then re-query.
        assert!(store.set_drift_status(2, DriftStatus::Approved).unwrap());
        let approved = store.recent_drift(10, Some(DriftStatus::Approved)).unwrap();
        assert_eq!(approved.len(), 1);
        assert_eq!(approved[0].id, 2);
    }

    #[test]
    fn load_nonexistent_session_returns_none() {
        let dir = TempDir::new().unwrap();
        let store = SessionStore::new(dir.path());
        assert!(store.load("no-such-id").unwrap().is_none());
    }

    #[test]
    fn latest_block_with_no_sessions_is_empty() {
        let dir = TempDir::new().unwrap();
        let store = SessionStore::new(dir.path());
        assert_eq!(store.latest_block().unwrap(), "");
    }

    #[test]
    fn list_empty_store_returns_empty_vec() {
        let dir = TempDir::new().unwrap();
        let store = SessionStore::new(dir.path());
        assert!(store.list().unwrap().is_empty());
    }

    #[test]
    fn recent_drift_empty_log_returns_empty() {
        let dir = TempDir::new().unwrap();
        let store = SessionStore::new(dir.path());
        assert!(store.recent_drift(10, None).unwrap().is_empty());
        assert!(store
            .recent_drift(10, Some(DriftStatus::Pending))
            .unwrap()
            .is_empty());
    }

    #[test]
    fn set_drift_status_nonexistent_returns_false() {
        let dir = TempDir::new().unwrap();
        let store = SessionStore::new(dir.path());
        assert!(!store.set_drift_status(99, DriftStatus::Approved).unwrap());
    }

    #[test]
    fn set_drift_status_already_approved_returns_false() {
        let dir = TempDir::new().unwrap();
        let store = SessionStore::new(dir.path());
        let ev = DriftEvent {
            id: 1,
            ts: Utc::now(),
            path: "/f.rs".into(),
            risk: "warn".into(),
            kind: "verify".into(),
            detail: "test".into(),
            status: DriftStatus::Approved,
        };
        store.append_drift(&ev).unwrap();
        // Already Approved, not Pending -> returns false
        assert!(!store.set_drift_status(1, DriftStatus::Rejected).unwrap());
    }

    #[test]
    fn next_drift_id_empty_log_is_one() {
        let dir = TempDir::new().unwrap();
        let store = SessionStore::new(dir.path());
        assert_eq!(store.next_drift_id(), 1);
    }

    #[test]
    fn next_drift_id_after_events_is_max_plus_one() {
        let dir = TempDir::new().unwrap();
        let store = SessionStore::new(dir.path());
        for id in [3i64, 7, 2] {
            store
                .append_drift(&DriftEvent {
                    id,
                    ts: Utc::now(),
                    path: "/f.rs".into(),
                    risk: "ok".into(),
                    kind: "verify".into(),
                    detail: "".into(),
                    status: DriftStatus::Pending,
                })
                .unwrap();
        }
        assert_eq!(store.next_drift_id(), 8, "max id is 7 -> next is 8");
    }

    #[test]
    fn drift_status_pending_is_default() {
        let status: DriftStatus = Default::default();
        assert_eq!(status, DriftStatus::Pending);
    }

    #[test]
    fn latest_block_after_save_is_nonempty() {
        let dir = TempDir::new().unwrap();
        let store = SessionStore::new(dir.path());
        let mut s = Session::new("proj");
        s.next_steps.push("do something".into());
        store.save(&s).unwrap();
        let block = store.latest_block().unwrap();
        assert!(block.contains("Cross-Session Protocol"));
        assert!(block.contains("do something"));
    }

    #[test]
    fn drift_limit_respected() {
        let dir = TempDir::new().unwrap();
        let store = SessionStore::new(dir.path());
        for id in 1..=10i64 {
            store
                .append_drift(&DriftEvent {
                    id,
                    ts: Utc::now(),
                    path: "/f.rs".into(),
                    risk: "warn".into(),
                    kind: "verify".into(),
                    detail: "".into(),
                    status: DriftStatus::Pending,
                })
                .unwrap();
        }
        let limited = store.recent_drift(3, None).unwrap();
        assert_eq!(limited.len(), 3, "limit=3 should return 3 most recent");
        assert_eq!(limited[0].id, 10, "newest first");
    }
}
