//! T019: repository discovery via `git rev-parse`, worktree listing, default
//! remote, and in-progress operation detection. Bare repos are rejected with
//! `NotAWorktree` (analysis U2).

use std::path::{Path, PathBuf};

use crate::error::GitError;
use crate::runner::GitRunner;

/// Resolved layout of the repository containing a given path.
#[derive(Debug, Clone)]
pub struct RepoLayout {
    /// Absolute working-tree root.
    pub worktree_root: PathBuf,
    /// Absolute Git common directory (shared across linked worktrees).
    pub git_common_dir: PathBuf,
    /// Absolute Git directory for THIS worktree (== common dir for main).
    pub git_dir: PathBuf,
    /// Whether this worktree is the main worktree.
    pub is_main: bool,
}

fn absolutize(base: &Path, p: &str) -> PathBuf {
    let path = PathBuf::from(p);
    let abs = if path.is_absolute() {
        path
    } else {
        base.join(path)
    };
    // Normalize without touching the filesystem beyond canonicalize; fall back
    // to the joined path if canonicalize fails (e.g., pending deletion).
    dunce_canonicalize(&abs).unwrap_or(abs)
}

/// Canonicalize without Windows `\\?\` prefixes (keeps paths comparable).
pub fn dunce_canonicalize(p: &Path) -> std::io::Result<PathBuf> {
    let c = p.canonicalize()?;
    let s = c.to_string_lossy();
    if let Some(stripped) = s.strip_prefix(r"\\?\") {
        Ok(PathBuf::from(stripped))
    } else {
        Ok(c)
    }
}

/// Discover the repository layout for `path`.
pub async fn discover(path: &Path) -> Result<RepoLayout, GitError> {
    let runner = GitRunner::new(path);
    let bare = runner
        .run_text(&["rev-parse", "--is-bare-repository"])
        .await?;
    if bare.trim() == "true" {
        return Err(GitError::NotAWorktree(path.display().to_string()));
    }
    let out = runner
        .run_text(&[
            "rev-parse",
            "--show-toplevel",
            "--git-common-dir",
            "--absolute-git-dir",
        ])
        .await?;
    let mut lines = out.lines();
    let toplevel = lines
        .next()
        .ok_or_else(|| GitError::Parse("missing --show-toplevel output".into()))?;
    let common = lines
        .next()
        .ok_or_else(|| GitError::Parse("missing --git-common-dir output".into()))?;
    let git_dir = lines
        .next()
        .ok_or_else(|| GitError::Parse("missing --absolute-git-dir output".into()))?;

    let worktree_root = absolutize(path, toplevel);
    let git_common_dir = absolutize(&worktree_root, common);
    let git_dir = absolutize(&worktree_root, git_dir);
    let is_main = git_common_dir == git_dir;
    Ok(RepoLayout {
        worktree_root,
        git_common_dir,
        git_dir,
        is_main,
    })
}

/// Default remote: `origin` when present, else the first remote, else None.
pub async fn default_remote(runner: &GitRunner) -> Result<Option<(String, String)>, GitError> {
    let remotes = runner.run_text(&["remote"]).await?;
    let names: Vec<&str> = remotes.lines().filter(|l| !l.is_empty()).collect();
    let chosen = if names.contains(&"origin") {
        Some("origin")
    } else {
        names.first().copied()
    };
    match chosen {
        None => Ok(None),
        Some(name) => {
            let url = runner.run_text(&["remote", "get-url", name]).await?;
            Ok(Some((name.to_string(), url)))
        }
    }
}

/// Detect an in-progress rebase or merge (FR-032 / edge cases).
pub fn in_progress_operation(layout: &RepoLayout) -> Option<&'static str> {
    if layout.git_dir.join("rebase-merge").exists() || layout.git_dir.join("rebase-apply").exists()
    {
        Some("rebase")
    } else if layout.git_dir.join("MERGE_HEAD").exists() {
        Some("merge")
    } else {
        None
    }
}

/// One entry from `git worktree list --porcelain`.
#[derive(Debug, Clone)]
pub struct WorktreeListEntry {
    pub path: PathBuf,
    pub is_bare: bool,
}

pub async fn worktree_list(runner: &GitRunner) -> Result<Vec<WorktreeListEntry>, GitError> {
    let out = runner
        .run_text(&["worktree", "list", "--porcelain"])
        .await?;
    let mut entries = Vec::new();
    let mut current: Option<WorktreeListEntry> = None;
    for line in out.lines() {
        if let Some(p) = line.strip_prefix("worktree ") {
            if let Some(e) = current.take() {
                entries.push(e);
            }
            current = Some(WorktreeListEntry {
                path: PathBuf::from(p),
                is_bare: false,
            });
        } else if line == "bare" {
            if let Some(e) = current.as_mut() {
                e.is_bare = true;
            }
        }
    }
    if let Some(e) = current.take() {
        entries.push(e);
    }
    Ok(entries)
}
