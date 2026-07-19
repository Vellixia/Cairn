//! Deterministic Git repository fixtures for Cairn integration tests.
//!
//! Every fixture is built by driving the real `git` CLI inside an isolated
//! temporary directory with pinned author/committer identity and timestamps,
//! so repeated builds produce identical commit graphs.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};
use tempfile::TempDir;

/// A scripted Git repository rooted in its own temporary directory.
pub struct FixtureRepo {
    dir: TempDir,
    root: PathBuf,
}

impl FixtureRepo {
    /// Initialize a fresh repository on branch `main` with one seed commit.
    pub fn new() -> Result<Self> {
        let repo = Self::empty()?;
        repo.commit_file("README.md", "seed\n", "seed commit")?;
        Ok(repo)
    }

    /// Initialize a repository with no commits (unborn HEAD).
    pub fn empty() -> Result<Self> {
        let dir = TempDir::new().context("create fixture tempdir")?;
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root)?;
        let repo = Self { dir, root };
        repo.git(&["init", "-b", "main"])?;
        repo.git(&["config", "user.name", "Cairn Fixture"])?;
        repo.git(&["config", "user.email", "fixture@cairn.invalid"])?;
        repo.git(&["config", "commit.gpgsign", "false"])?;
        repo.git(&["config", "core.autocrlf", "false"])?;
        Ok(repo)
    }

    /// Initialize a bare repository (no working tree).
    pub fn bare() -> Result<Self> {
        let dir = TempDir::new().context("create fixture tempdir")?;
        let root = dir.path().join("repo.git");
        std::fs::create_dir_all(&root)?;
        let repo = Self { dir, root };
        repo.git(&["init", "--bare"])?;
        Ok(repo)
    }

    /// Working-tree root of the repository.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Directory that owns the whole fixture (parent of the repo root).
    pub fn fixture_dir(&self) -> &Path {
        self.dir.path()
    }

    /// Run a git subcommand inside the repository, asserting success.
    pub fn git(&self, args: &[&str]) -> Result<String> {
        let out = Command::new("git")
            .args(args)
            .current_dir(&self.root)
            .env("GIT_AUTHOR_DATE", "2026-01-02T03:04:05Z")
            .env("GIT_COMMITTER_DATE", "2026-01-02T03:04:05Z")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .context("spawn git")?;
        if !out.status.success() {
            bail!(
                "git {:?} failed: {}\n{}",
                args,
                out.status,
                String::from_utf8_lossy(&out.stderr)
            );
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    /// Write a file (creating parent dirs) relative to the repo root.
    pub fn write(&self, rel: &str, contents: &str) -> Result<()> {
        let path = self.root.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, contents)?;
        Ok(())
    }

    /// Delete a file relative to the repo root.
    pub fn delete(&self, rel: &str) -> Result<()> {
        std::fs::remove_file(self.root.join(rel))?;
        Ok(())
    }

    /// Stage a path.
    pub fn stage(&self, rel: &str) -> Result<()> {
        self.git(&["add", "--", rel])?;
        Ok(())
    }

    /// Write, stage, and commit a file. Returns the new HEAD OID.
    pub fn commit_file(&self, rel: &str, contents: &str, message: &str) -> Result<String> {
        self.write(rel, contents)?;
        self.stage(rel)?;
        self.git(&["commit", "-m", message])?;
        self.head_oid()
    }

    /// Current HEAD commit OID.
    pub fn head_oid(&self) -> Result<String> {
        Ok(self.git(&["rev-parse", "HEAD"])?.trim().to_string())
    }

    /// Create and check out a branch.
    pub fn checkout_new_branch(&self, name: &str) -> Result<()> {
        self.git(&["checkout", "-q", "-b", name])?;
        Ok(())
    }

    /// Check out an existing ref.
    pub fn checkout(&self, name: &str) -> Result<()> {
        self.git(&["checkout", "-q", name])?;
        Ok(())
    }

    /// Detach HEAD at the current commit.
    pub fn detach_head(&self) -> Result<()> {
        self.git(&["checkout", "-q", "--detach"])?;
        Ok(())
    }

    /// Add a remote named `origin`.
    pub fn add_origin(&self, url: &str) -> Result<()> {
        self.git(&["remote", "add", "origin", url])?;
        Ok(())
    }

    /// Start a rebase that stops on a conflict, leaving rebase-in-progress state.
    pub fn start_conflicted_rebase(&self) -> Result<()> {
        self.commit_file("conflict.txt", "base\n", "base")?;
        self.checkout_new_branch("side")?;
        self.commit_file("conflict.txt", "side\n", "side change")?;
        self.checkout("main")?;
        self.commit_file("conflict.txt", "main\n", "main change")?;
        self.checkout("side")?;
        // Expected to fail with a conflict; that is the fixture state we want.
        let out = Command::new("git")
            .args(["rebase", "main"])
            .current_dir(&self.root)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()?;
        if out.status.success() {
            bail!("expected rebase conflict, but rebase succeeded");
        }
        Ok(())
    }

    /// Add a linked worktree for `branch`, returning its path.
    pub fn add_linked_worktree(&self, branch: &str) -> Result<PathBuf> {
        let wt = self.dir.path().join(format!("wt-{branch}"));
        self.git(&["worktree", "add", "-b", branch, wt.to_str().unwrap()])?;
        Ok(wt)
    }

    /// Populate `n` files under an ignored directory plus a matching .gitignore.
    pub fn huge_ignored_tree(&self, dir: &str, n: usize) -> Result<()> {
        self.write(".gitignore", &format!("{dir}/\n"))?;
        self.stage(".gitignore")?;
        self.git(&["commit", "-m", "add gitignore"])?;
        for i in 0..n {
            self.write(
                &format!("{dir}/sub{}/f{}.txt", i % 20, i),
                &format!("x{i}\n"),
            )?;
        }
        Ok(())
    }

    /// Create an ignored file containing a secret marker value.
    pub fn ignored_secret(&self, marker: &str) -> Result<()> {
        let gitignore = self.root.join(".gitignore");
        let mut current = std::fs::read_to_string(&gitignore).unwrap_or_default();
        current.push_str(".env\n");
        std::fs::write(gitignore, current)?;
        self.write(".env", &format!("SECRET_TOKEN={marker}\n"))?;
        Ok(())
    }

    /// Delete Cairn identity markers under the git common dir (marker-loss fixture).
    pub fn delete_identity_markers(&self) -> Result<()> {
        let common = self.git(&["rev-parse", "--git-common-dir"])?;
        let mut dir = PathBuf::from(common.trim());
        if dir.is_relative() {
            dir = self.root.join(dir);
        }
        let cairn = dir.join("cairn");
        if cairn.exists() {
            std::fs::remove_dir_all(&cairn)?;
        }
        Ok(())
    }

    /// Copy the entire repository directory (including .git data) to a sibling
    /// path, simulating a manual `cp -r` copy. Returns the copy's root.
    pub fn copy_with_git_data(&self) -> Result<PathBuf> {
        let dest = self.dir.path().join("repo-copy");
        copy_dir_recursive(&self.root, &dest)?;
        Ok(dest)
    }
}

fn copy_dir_recursive(src: &Path, dest: &Path) -> Result<()> {
    std::fs::create_dir_all(dest)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let to = dest.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_recursive(&entry.path(), &to)?;
        } else {
            std::fs::copy(entry.path(), &to)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_seeded_repo() {
        let repo = FixtureRepo::new().unwrap();
        assert!(repo.root().join(".git").exists());
        assert_eq!(repo.head_oid().unwrap().len(), 40);
    }

    #[test]
    fn builds_bare_repo() {
        let repo = FixtureRepo::bare().unwrap();
        assert!(repo.root().join("HEAD").exists());
    }
}
