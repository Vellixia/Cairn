//! Deterministic snapshot fingerprints (research R2).
//!
//! Canonical form: entries sorted bytewise by path, serialized as
//! newline-terminated fields, hashed with BLAKE3. A schema-version prefix on
//! the final fingerprint lets the algorithm evolve without silent collisions.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Version prefix baked into every final snapshot fingerprint.
pub const FP_SCHEMA_VERSION: u32 = 1;

/// Sentinel hash value used for deleted files (no content to hash).
pub const DELETED_SENTINEL: &str = "deleted";

/// Marker used in the final fingerprint when HEAD is detached.
pub const DETACHED_MARKER: &str = "DETACHED";

/// One index (staged) entry from `git ls-files -s`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexEntry {
    pub mode: String,
    pub stage: u8,
    pub oid: String,
    pub path: String,
}

/// Working-tree change classification for fingerprint entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WorkStatus {
    Modified,
    Deleted,
    Added,
}

impl WorkStatus {
    fn as_str(self) -> &'static str {
        match self {
            WorkStatus::Modified => "modified",
            WorkStatus::Deleted => "deleted",
            WorkStatus::Added => "added",
        }
    }
}

/// One working-tree entry (unstaged change or untracked file).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkEntry {
    pub path: String,
    pub status: WorkStatus,
    /// BLAKE3 hex of file contents; `None` for deleted files (sentinel used).
    pub content_hash: Option<String>,
}

fn blake3_hex(input: &[u8]) -> String {
    blake3::hash(input).to_hex().to_string()
}

/// Fingerprint of the full index state. Any staging change alters it.
pub fn staged_fingerprint(entries: &[IndexEntry]) -> String {
    let mut sorted: Vec<&IndexEntry> = entries.iter().collect();
    sorted.sort_by(|a, b| a.path.as_bytes().cmp(b.path.as_bytes()));
    let mut buf = String::new();
    for e in sorted {
        buf.push_str(&format!("{} {} {} {}\n", e.mode, e.stage, e.oid, e.path));
    }
    blake3_hex(buf.as_bytes())
}

/// Fingerprint of a working-tree entry set (unstaged or untracked).
pub fn work_fingerprint(entries: &[WorkEntry]) -> String {
    let mut sorted: Vec<&WorkEntry> = entries.iter().collect();
    sorted.sort_by(|a, b| a.path.as_bytes().cmp(b.path.as_bytes()));
    let mut buf = String::new();
    for e in sorted {
        let hash = e.content_hash.as_deref().unwrap_or(DELETED_SENTINEL);
        buf.push_str(&format!("{}\n{}\n{}\n", e.path, e.status.as_str(), hash));
    }
    blake3_hex(buf.as_bytes())
}

/// Compose the final snapshot fingerprint from its components (FR-008).
pub fn snapshot_fingerprint(
    branch: Option<&str>,
    head_commit: &str,
    staged_fp: &str,
    unstaged_fp: &str,
    untracked_fp: &str,
) -> String {
    let branch_part = branch.unwrap_or(DETACHED_MARKER);
    let buf = format!(
        "{FP_SCHEMA_VERSION}\n{branch_part}\n{head_commit}\n{staged_fp}\n{unstaged_fp}\n{untracked_fp}\n"
    );
    blake3_hex(buf.as_bytes())
}

/// The persisted components of one snapshot (metadata + hashes only, FR-011).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotComponents {
    pub branch: Option<String>,
    pub head_commit: String,
    pub staged_fp: String,
    pub unstaged_fp: String,
    pub untracked_fp: String,
}

impl SnapshotComponents {
    pub fn final_fingerprint(&self) -> String {
        snapshot_fingerprint(
            self.branch.as_deref(),
            &self.head_commit,
            &self.staged_fp,
            &self.unstaged_fp,
            &self.untracked_fp,
        )
    }
}
