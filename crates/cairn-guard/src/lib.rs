//! Active guardrails: catch silent corruption in agent edits.
//!
//! The research motivating Cairn found frontier models silently delete ~25% of content over long
//! delegated edits. Two checks guard against that, both backed by the content-addressed blob store
//! so the pre-edit state is always recoverable:
//!
//! - [`Guard::verify_edit`] compares a *proposed* new version against the current file (pre-write).
//! - [`Guard::verify_against_baseline`] compares the *current* file against the version Cairn
//!   recorded when the agent last read it — the PostToolUse check that catches damage after a read.

use cairn_core::{ContentHash, Result};
use cairn_store::Store;
use chrono::{DateTime, Utc};
use serde::Serialize;
use similar::{ChangeTag, TextDiff};
use std::path::Path;
use std::sync::Arc;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Risk {
    Ok,
    Warn,
    Danger,
}

/// The outcome of verifying an edit.
#[derive(Debug, Clone, Serialize)]
pub struct VerifyReport {
    pub path: String,
    /// Handle to the retained pre-edit original (None if the file didn't exist).
    pub baseline_hash: Option<String>,
    pub baseline_lines: usize,
    pub new_lines: usize,
    pub added: usize,
    pub removed: usize,
    pub removed_ratio: f32,
    pub risk: Risk,
    pub message: String,
}

impl VerifyReport {
    /// Whether the edit looks safe (no large unreplaced deletion).
    pub fn is_clean(&self) -> bool {
        matches!(self.risk, Risk::Ok)
    }
}

/// A snapshot of the tracked file state you can roll back to.
#[derive(Debug, Clone, Serialize)]
pub struct Checkpoint {
    pub id: String,
    pub label: String,
    pub created_at: DateTime<Utc>,
    pub files: usize,
}

/// The outcome of rolling back to a checkpoint.
#[derive(Debug, Clone, Serialize)]
pub struct RollbackReport {
    pub checkpoint_id: String,
    pub restored: Vec<String>,
    pub skipped: Vec<String>,
}

pub struct Guard {
    store: Arc<Store>,
}

impl Guard {
    pub fn new(store: Arc<Store>) -> Self {
        Self { store }
    }

    /// Compare a proposed new version of `path` against the current on-disk file. The original is
    /// snapshotted into the blob store first so it is always recoverable.
    pub fn verify_edit(&self, path: &Path, new_content: &str) -> Result<VerifyReport> {
        let (baseline, baseline_hash) = match std::fs::read(path) {
            Ok(bytes) => {
                let hash = self.store.blobs().put(&bytes)?;
                (String::from_utf8_lossy(&bytes).into_owned(), Some(hash.0))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (String::new(), None),
            Err(e) => return Err(e.into()),
        };
        Ok(assess(
            &path.to_string_lossy(),
            &baseline,
            baseline_hash,
            new_content,
        ))
    }

    /// Verify the *current* on-disk file against the version Cairn recorded when the agent last
    /// read it (its edit baseline) — catching silent corruption introduced after the read. Returns
    /// `None` if Cairn has no baseline for this path (it was never read through Cairn).
    pub fn verify_against_baseline(&self, path: &Path) -> Result<Option<VerifyReport>> {
        let key = std::fs::canonicalize(path)
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| path.to_string_lossy().into_owned());

        let Some((hash, _lines)) = self.store.latest_file_version(&key)? else {
            return Ok(None);
        };
        let Some(baseline) = self.store.blobs().get_str(&ContentHash(hash.clone()))? else {
            return Ok(None);
        };
        let current = match std::fs::read(path) {
            Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(e.into()),
        };
        Ok(Some(assess(&key, &baseline, Some(hash), &current)))
    }

    /// Set the current task anchor — the goal Cairn re-injects at session start to keep the agent
    /// on track (anti-drift). A single current goal.
    pub fn set_anchor(&self, goal: &str) -> Result<()> {
        self.store.set_meta("task_anchor", goal.trim())
    }

    /// The current task anchor, if one is set.
    pub fn anchor(&self) -> Result<Option<String>> {
        self.store.get_meta("task_anchor")
    }

    /// Snapshot the files Cairn has tracked (path → content hash) as a named checkpoint.
    pub fn checkpoint(&self, label: &str) -> Result<Checkpoint> {
        let map: std::collections::BTreeMap<String, String> = self
            .store
            .all_file_versions()?
            .into_iter()
            .map(|(path, hash, _lines)| (path, hash))
            .collect();
        let id = Uuid::new_v4().simple().to_string();
        let created_at = Utc::now();
        let files = map.len();
        self.store.insert_checkpoint(
            &id,
            label,
            &created_at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            &serde_json::to_string(&map)?,
        )?;
        Ok(Checkpoint {
            id,
            label: label.to_string(),
            created_at,
            files,
        })
    }

    /// List checkpoints, newest first.
    pub fn list_checkpoints(&self) -> Result<Vec<Checkpoint>> {
        let mut out = Vec::new();
        for (id, label, created) in self.store.list_checkpoints()? {
            let files = self
                .store
                .get_checkpoint(&id)?
                .and_then(|(_, _, j)| {
                    serde_json::from_str::<std::collections::BTreeMap<String, String>>(&j).ok()
                })
                .map(|m| m.len())
                .unwrap_or(0);
            out.push(Checkpoint {
                id,
                label,
                created_at: DateTime::parse_from_rfc3339(&created)
                    .map(|d| d.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
                files,
            });
        }
        Ok(out)
    }

    /// Roll back to a checkpoint, restoring each tracked file's content from the blob store.
    pub fn rollback(&self, checkpoint_id: &str) -> Result<RollbackReport> {
        let (_, _, files_json) = self
            .store
            .get_checkpoint(checkpoint_id)?
            .ok_or_else(|| cairn_core::Error::NotFound(format!("checkpoint {checkpoint_id}")))?;
        let map: std::collections::BTreeMap<String, String> = serde_json::from_str(&files_json)?;
        let mut restored = Vec::new();
        let mut skipped = Vec::new();
        for (path, hash) in map {
            match self.store.blobs().get(&ContentHash(hash))? {
                Some(bytes) if std::fs::write(std::path::Path::new(&path), &bytes).is_ok() => {
                    restored.push(path)
                }
                _ => skipped.push(path),
            }
        }
        Ok(RollbackReport {
            checkpoint_id: checkpoint_id.to_string(),
            restored,
            skipped,
        })
    }
}

/// Diff `baseline` → `new_content` and judge the risk of a large, unreplaced deletion.
fn assess(
    path: &str,
    baseline: &str,
    baseline_hash: Option<String>,
    new_content: &str,
) -> VerifyReport {
    let diff = TextDiff::from_lines(baseline, new_content);
    let mut added = 0usize;
    let mut removed = 0usize;
    for change in diff.iter_all_changes() {
        match change.tag() {
            ChangeTag::Insert => added += 1,
            ChangeTag::Delete => removed += 1,
            ChangeTag::Equal => {}
        }
    }

    let baseline_lines = baseline.lines().count();
    let new_lines = new_content.lines().count();
    let removed_ratio = if baseline_lines == 0 {
        0.0
    } else {
        removed as f32 / baseline_lines as f32
    };

    let (risk, message) = if baseline.is_empty() {
        (Risk::Ok, format!("new file: {new_lines} lines"))
    } else if added == 0 && removed == 0 {
        (Risk::Ok, "no changes".to_string())
    } else if removed_ratio >= 0.5 && added < removed / 2 {
        (
            Risk::Danger,
            format!(
                "removes {removed} of {baseline_lines} lines ({:.0}%) with little replacement — possible silent corruption",
                removed_ratio * 100.0
            ),
        )
    } else if removed_ratio >= 0.2 {
        (
            Risk::Warn,
            format!(
                "removes {removed} of {baseline_lines} lines ({:.0}%) — review before accepting",
                removed_ratio * 100.0
            ),
        )
    } else {
        (Risk::Ok, format!("+{added} / -{removed} lines"))
    };

    VerifyReport {
        path: path.to_string(),
        baseline_hash,
        baseline_lines,
        new_lines,
        added,
        removed,
        removed_ratio,
        risk,
        message,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_core::Config;

    fn guard() -> (Guard, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let cfg = Config::resolve(Some(dir.path().join("data"))).unwrap();
        (Guard::new(Arc::new(Store::open(&cfg).unwrap())), dir)
    }

    fn hundred_lines() -> String {
        (0..100).map(|i| format!("line {i}\n")).collect()
    }

    #[test]
    fn flags_large_unreplaced_deletion() {
        let (g, dir) = guard();
        let f = dir.path().join("f.txt");
        std::fs::write(&f, hundred_lines()).unwrap();
        let report = g.verify_edit(&f, "line 0\n").unwrap();
        assert_eq!(report.risk, Risk::Danger);
        assert!(!report.is_clean());
        assert!(report.baseline_hash.is_some(), "original must be retained");
    }

    #[test]
    fn small_append_is_ok() {
        let (g, dir) = guard();
        let f = dir.path().join("f.txt");
        let original = hundred_lines();
        std::fs::write(&f, &original).unwrap();
        let report = g.verify_edit(&f, &format!("{original}line 100\n")).unwrap();
        assert!(report.is_clean());
    }

    #[test]
    fn new_file_is_ok_with_no_baseline() {
        let (g, dir) = guard();
        let f = dir.path().join("new.txt");
        let report = g.verify_edit(&f, "hello\nworld\n").unwrap();
        assert_eq!(report.risk, Risk::Ok);
        assert!(report.baseline_hash.is_none());
    }

    #[test]
    fn verify_against_baseline_flags_post_read_corruption() {
        let (g, dir) = guard();
        let f = dir.path().join("f.txt");
        std::fs::write(&f, hundred_lines()).unwrap();

        // Simulate Cairn having read the file: retain the bytes + record the version baseline.
        let key = std::fs::canonicalize(&f)
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let bytes = std::fs::read(&f).unwrap();
        let hash = g.store.blobs().put(&bytes).unwrap();
        g.store.record_file_version(&key, &hash.0, 100).unwrap();

        // The agent then guts the file.
        std::fs::write(&f, "line 0\n").unwrap();
        let report = g.verify_against_baseline(&f).unwrap().unwrap();
        assert_eq!(report.risk, Risk::Danger);

        // A file Cairn never read has no baseline -> no judgment.
        let f2 = dir.path().join("never_read.txt");
        std::fs::write(&f2, "x").unwrap();
        assert!(g.verify_against_baseline(&f2).unwrap().is_none());
    }

    #[test]
    fn anchor_set_and_get() {
        let (g, _dir) = guard();
        assert!(g.anchor().unwrap().is_none());
        g.set_anchor("  ship Cairn v0.2  ").unwrap();
        assert_eq!(g.anchor().unwrap().unwrap(), "ship Cairn v0.2");
    }

    #[test]
    fn checkpoint_and_rollback_restores_files() {
        let (g, dir) = guard();
        let f = dir.path().join("f.txt");
        std::fs::write(&f, "original content\nline2\n").unwrap();

        // Simulate Cairn having read the file: retain the bytes + record the version.
        let key = std::fs::canonicalize(&f)
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let bytes = std::fs::read(&f).unwrap();
        let hash = g.store.blobs().put(&bytes).unwrap();
        g.store.record_file_version(&key, &hash.0, 2).unwrap();

        let cp = g.checkpoint("before edits").unwrap();
        assert_eq!(cp.files, 1);
        assert_eq!(g.list_checkpoints().unwrap().len(), 1);

        // The agent guts the file, then we roll back.
        std::fs::write(&f, "gutted\n").unwrap();
        let report = g.rollback(&cp.id).unwrap();
        assert_eq!(report.restored.len(), 1);
        assert!(report.skipped.is_empty());
        assert_eq!(
            std::fs::read_to_string(&f).unwrap(),
            "original content\nline2\n"
        );
    }
}
