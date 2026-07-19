//! T025: Git-private identity markers (research R4, clarification Q1).
//!
//! Repository identity lives at `<git-common-dir>/cairn/repository-id`;
//! per-worktree identity at `<absolute-git-dir>/cairn/worktree-id`. Never in
//! the tracked working tree. Marker-loss restoration policy (analysis U1) is
//! decided by the caller (daemon) which owns DB access; this module only
//! reads/writes markers atomically.

use std::path::{Path, PathBuf};

use uuid::Uuid;

use crate::discover::RepoLayout;
use crate::error::GitError;

const REPO_MARKER: &str = "repository-id";
const WORKTREE_MARKER: &str = "worktree-id";

/// Current on-disk marker state for a repository layout.
#[derive(Debug, Clone, Default)]
pub struct MarkerState {
    pub repo_uuid: Option<Uuid>,
    pub worktree_uuid: Option<Uuid>,
}

fn marker_dir(base: &Path) -> PathBuf {
    base.join("cairn")
}

fn read_marker(path: &Path) -> Result<Option<Uuid>, GitError> {
    match std::fs::read_to_string(path) {
        Ok(s) => {
            let trimmed = s.trim();
            Uuid::parse_str(trimmed)
                .map(Some)
                .map_err(|e| GitError::Parse(format!("corrupt marker {}: {e}", path.display())))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(GitError::Io(e)),
    }
}

/// Atomically write a marker file (temp file + rename).
fn write_marker(path: &Path, id: Uuid) -> Result<(), GitError> {
    let dir = path.parent().expect("marker has parent");
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(format!(".{}.tmp", uuid::Uuid::new_v4()));
    std::fs::write(&tmp, format!("{id}\n"))?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

pub fn repo_marker_path(layout: &RepoLayout) -> PathBuf {
    marker_dir(&layout.git_common_dir).join(REPO_MARKER)
}

pub fn worktree_marker_path(layout: &RepoLayout) -> PathBuf {
    marker_dir(&layout.git_dir).join(WORKTREE_MARKER)
}

/// Read current markers (either may be absent).
pub fn read_markers(layout: &RepoLayout) -> Result<MarkerState, GitError> {
    Ok(MarkerState {
        repo_uuid: read_marker(&repo_marker_path(layout))?,
        worktree_uuid: read_marker(&worktree_marker_path(layout))?,
    })
}

/// Write the repository marker (used for create and restore).
pub fn write_repo_marker(layout: &RepoLayout, id: Uuid) -> Result<(), GitError> {
    write_marker(&repo_marker_path(layout), id)
}

/// Write the worktree marker (used for create and restore).
pub fn write_worktree_marker(layout: &RepoLayout, id: Uuid) -> Result<(), GitError> {
    write_marker(&worktree_marker_path(layout), id)
}
