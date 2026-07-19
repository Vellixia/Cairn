//! T033: fingerprint pipeline (research R2) with read-verify-retry
//! consistency (research R3, FR-012).

use std::path::Path;

use cairn_domain::snapshot::{
    staged_fingerprint, work_fingerprint, SnapshotComponents, WorkEntry, WorkStatus,
};

use crate::error::GitError;
use crate::ignored::IgnoreStack;
use crate::runner::GitRunner;
use crate::status::{self, ChangeKind, StatusReport};

const MAX_ATTEMPTS: u32 = 3;

/// Sentinel head value for a repository whose HEAD is unborn (no commits).
pub const UNBORN_HEAD: &str = "UNBORN";

/// One consistent snapshot of the exact repository state plus its report.
#[derive(Debug, Clone)]
pub struct FingerprintedState {
    pub components: SnapshotComponents,
    pub report: StatusReport,
}

/// Hash file contents with BLAKE3 via a blocking task (files can be large).
async fn hash_file(path: std::path::PathBuf) -> Option<String> {
    tokio::task::spawn_blocking(move || {
        use std::io::Read;
        let mut file = std::fs::File::open(&path).ok()?;
        let mut hasher = blake3::Hasher::new();
        let mut buf = [0u8; 64 * 1024];
        loop {
            match file.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    hasher.update(&buf[..n]);
                }
                Err(_) => return None, // vanished mid-scan → deleted sentinel
            }
        }
        Some(hasher.finalize().to_hex().to_string())
    })
    .await
    .ok()
    .flatten()
}

async fn work_entries(root: &Path, changes: &[status::FileChange]) -> Vec<WorkEntry> {
    let mut entries = Vec::with_capacity(changes.len());
    for c in changes {
        let status = match c.status {
            ChangeKind::Deleted => WorkStatus::Deleted,
            ChangeKind::Added => WorkStatus::Added,
            _ => WorkStatus::Modified,
        };
        // Files that vanish mid-scan hash to None (deleted sentinel).
        let content_hash = if status == WorkStatus::Deleted {
            None
        } else {
            hash_file(root.join(&c.path)).await
        };
        entries.push(WorkEntry {
            path: c.path.clone(),
            status,
            content_hash,
        });
    }
    entries
}

async fn untracked_entries(
    root: &Path,
    untracked: &[String],
    stack: &IgnoreStack,
) -> Vec<WorkEntry> {
    let mut entries = Vec::with_capacity(untracked.len());
    for path in untracked {
        // .cairnignore filtering: excluded paths never enter fingerprints
        // (FR-026); git already applied .gitignore.
        if stack.is_excluded(Path::new(path), false) {
            continue;
        }
        let content_hash = hash_file(root.join(path)).await;
        entries.push(WorkEntry {
            path: path.clone(),
            status: WorkStatus::Added,
            content_hash,
        });
    }
    entries
}

/// Cheap consistency probe: HEAD OID + raw hashes of status and index output.
async fn probe(runner: &GitRunner) -> Result<(String, String), GitError> {
    let status_raw = runner
        .run(&[
            "status",
            "--porcelain=v2",
            "--branch",
            "--untracked-files=all",
            "-z",
        ])
        .await?;
    let index_raw = runner.run(&["ls-files", "-s", "-z"]).await?;
    Ok((
        blake3::hash(&status_raw).to_hex().to_string(),
        blake3::hash(&index_raw).to_hex().to_string(),
    ))
}

/// Compute a consistent fingerprinted state, retrying on concurrent mutation
/// (FR-012). Returns `SnapshotContention` after `MAX_ATTEMPTS` torn reads.
pub async fn fingerprint_state(root: &Path) -> Result<FingerprintedState, GitError> {
    let runner = GitRunner::new(root);
    let stack = IgnoreStack::load(root)?;

    for attempt in 1..=MAX_ATTEMPTS {
        let before = probe(&runner).await?;

        let report = status::status(&runner).await?;
        let index_raw = runner.run(&["ls-files", "-s", "-z"]).await?;
        let index_entries = status::parse_ls_files(&index_raw)?;

        let staged_fp = staged_fingerprint(&index_entries);
        let unstaged = work_entries(root, &report.unstaged).await;
        let untracked = untracked_entries(root, &report.untracked, &stack).await;
        let unstaged_fp = work_fingerprint(&unstaged);
        let untracked_fp = work_fingerprint(&untracked);

        let after = probe(&runner).await?;
        if before != after {
            tracing::debug!(attempt, "repository changed during snapshot; retrying");
            tokio::time::sleep(std::time::Duration::from_millis(50 * u64::from(attempt))).await;
            continue;
        }

        let head_commit = report
            .head_oid
            .clone()
            .unwrap_or_else(|| UNBORN_HEAD.to_string());
        let components = SnapshotComponents {
            branch: report.branch.clone(),
            head_commit,
            staged_fp,
            unstaged_fp,
            untracked_fp,
        };
        return Ok(FingerprintedState { components, report });
    }
    Err(GitError::SnapshotContention(MAX_ATTEMPTS))
}
