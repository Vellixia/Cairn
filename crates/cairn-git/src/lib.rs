//! Git adapter (D4, FR-001, FR-003, FR-005, FR-064).
//!
//! Shells out to the `git` command-line tool, which is the definition of
//! correct Git behaviour. What this module identifies is the **local
//! repository instance** — Git common directory, worktree path, branch,
//! commit, status. That is not the identity of a shared Cairn project: the
//! same repository is `/Users/a/project` on one machine and `/home/a/project`
//! on another, so shared identity is server-assigned at `cairn link` (D14).

use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, thiserror::Error)]
pub enum GitError {
    #[error("git is not available on PATH: {0}")]
    GitMissing(String),
    #[error("not a git repository: {0}")]
    NotARepository(String),
    #[error("git {command} failed: {stderr}")]
    CommandFailed { command: String, stderr: String },
}

/// A local checkout. Every field here stays on this machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoInstance {
    /// Resolved Git common directory — groups worktrees into one local project.
    pub git_common_dir: PathBuf,
    /// Absolute path of this worktree.
    pub worktree_path: PathBuf,
    /// Suggested project name: the worktree directory name.
    pub name: String,
    /// Normalized first remote, a discovery *hint* only (FR-064).
    pub remote: Option<String>,
}

/// Working-tree state at a point in time (FR-003).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GitStatus {
    pub branch: String,
    pub commit_sha: Option<String>,
    pub staged: usize,
    pub unstaged: usize,
    pub untracked: usize,
    /// Every path Git currently reports as changed, including untracked.
    pub changed_files: Vec<String>,
}

/// Branch name reported when `HEAD` is detached.
pub const DETACHED: &str = "(detached)";

fn run(cwd: &Path, args: &[&str]) -> Result<String, GitError> {
    let out = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(args)
        .output()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                GitError::GitMissing(e.to_string())
            } else {
                GitError::CommandFailed {
                    command: args.join(" "),
                    stderr: e.to_string(),
                }
            }
        })?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        if stderr.contains("not a git repository") {
            return Err(GitError::NotARepository(cwd.display().to_string()));
        }
        return Err(GitError::CommandFailed {
            command: args.join(" "),
            stderr,
        });
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

/// Same as `run`, but a non-zero exit is an expected "no value" rather than an
/// error — used for HEAD and remote lookups that are legitimately absent.
fn run_opt(cwd: &Path, args: &[&str]) -> Result<Option<String>, GitError> {
    match run(cwd, args) {
        Ok(s) => {
            let t = s.trim().to_string();
            Ok(if t.is_empty() { None } else { Some(t) })
        }
        Err(GitError::CommandFailed { .. }) => Ok(None),
        Err(e) => Err(e),
    }
}

/// Identify the local repository instance containing `cwd`.
///
/// Reports a clear, actionable error and creates nothing when `cwd` is not a
/// repository or Git is unavailable (FR-005).
pub fn discover(cwd: &Path) -> Result<RepoInstance, GitError> {
    if !cwd.exists() {
        return Err(GitError::NotARepository(cwd.display().to_string()));
    }
    let common = run(
        cwd,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )?;
    let git_common_dir = canonical(Path::new(common.trim()))?;

    let top = run(
        cwd,
        &["rev-parse", "--path-format=absolute", "--show-toplevel"],
    )?;
    let worktree_path = canonical(Path::new(top.trim()))?;

    let name = worktree_path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "repository".to_string());

    Ok(RepoInstance {
        git_common_dir,
        worktree_path,
        name,
        remote: first_remote(cwd)?,
    })
}

/// Resolve a path to its canonical form, deterministically.
///
/// Local project identity is keyed on this string, so a silent fallback to the
/// uncanonicalised path would let one repository register twice — once as
/// `/var/...` and once as `/private/var/...`. A failure here is reported, not
/// papered over (FR-002).
fn canonical(p: &Path) -> Result<PathBuf, GitError> {
    p.canonicalize().map_err(|e| GitError::CommandFailed {
        command: format!("canonicalize {}", p.display()),
        stderr: e.to_string(),
    })
}

/// The normalized URL of `origin`, or of the first remote if there is no
/// `origin`. A repository with no remote simply has none (FR-064).
pub fn first_remote(cwd: &Path) -> Result<Option<String>, GitError> {
    if let Some(url) = run_opt(cwd, &["remote", "get-url", "origin"])? {
        return Ok(Some(normalize_remote(&url)));
    }
    let Some(list) = run_opt(cwd, &["remote"])? else {
        return Ok(None);
    };
    let Some(first) = list.lines().next().map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    Ok(run_opt(cwd, &["remote", "get-url", first])?.map(|u| normalize_remote(&u)))
}

/// Reduce a remote URL to `host/owner/repo`.
///
/// Only a comparison hint: forks share an upstream, mirrors duplicate it, and
/// plenty of repositories have no remote — which is why this can never be the
/// authority for shared project identity (D14).
pub fn normalize_remote(url: &str) -> String {
    let mut s = url.trim().to_string();
    if let Some(rest) = s.strip_suffix('/') {
        s = rest.to_string();
    }
    if let Some(rest) = s.strip_suffix(".git") {
        s = rest.to_string();
    }
    // scp-like: git@host:owner/repo
    if !s.contains("://") {
        if let Some((prefix, path)) = s.split_once(':') {
            let host = prefix.rsplit('@').next().unwrap_or(prefix);
            return format!("{}/{}", host.to_lowercase(), path.trim_start_matches('/'));
        }
    }
    if let Some((_scheme, rest)) = s.split_once("://") {
        // Drop any embedded credentials.
        let rest = rest.rsplit('@').next().unwrap_or(rest);
        let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
        return format!("{}/{}", host.to_lowercase(), path);
    }
    s
}

/// Current branch, commit and working-tree counts.
pub fn status(worktree: &Path) -> Result<GitStatus, GitError> {
    let branch = run_opt(worktree, &["symbolic-ref", "--quiet", "--short", "HEAD"])?
        .unwrap_or_else(|| DETACHED.to_string());
    let commit_sha = run_opt(worktree, &["rev-parse", "HEAD"])?;

    let raw = run(
        worktree,
        &["status", "--porcelain=v1", "-z", "--untracked-files=normal"],
    )?;
    let mut st = GitStatus {
        branch,
        commit_sha,
        ..Default::default()
    };

    // NUL-separated. Rename entries ("R  new\0old") consume an extra field.
    let mut fields = raw.split('\0').filter(|f| !f.is_empty()).peekable();
    while let Some(entry) = fields.next() {
        if entry.len() < 3 {
            continue;
        }
        let bytes: Vec<char> = entry.chars().collect();
        let (x, y) = (bytes[0], bytes[1]);
        let path: String = entry[3..].to_string();

        if x == 'R' || x == 'C' {
            // The following field is the original path; skip it.
            fields.next();
        }

        if x == '?' && y == '?' {
            st.untracked += 1;
        } else {
            if x != ' ' && x != '?' {
                st.staged += 1;
            }
            if y != ' ' && y != '?' {
                st.unstaged += 1;
            }
        }
        st.changed_files.push(path);
    }
    st.changed_files.sort();
    st.changed_files.dedup();
    Ok(st)
}

/// Every local branch name.
///
/// Feeds stale-scope reconciliation: memory scoped to a branch that no longer
/// exists is marked `stale`, never deleted (FR-018, US3 scenario 5).
pub fn local_branches(worktree: &Path) -> Result<Vec<String>, GitError> {
    let raw = run(worktree, &["branch", "--format=%(refname:short)"])?;
    Ok(raw
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect())
}

/// Whether `branch` has been merged into `target`.
///
/// True when the branch tip is an ancestor of the target tip — which is what
/// "merged" means to Git and what Cairn needs to know before it may offer a
/// branch's knowledge as an **elevation candidate** (FR-382).
///
/// It is only ever a candidate. Branch-scoped knowledge never becomes project
/// knowledge automatically when a branch merges: a merge may *produce* a
/// candidate, which is then verified against the current target branch and
/// applied only on an explicit decision.
///
/// An unresolvable ref is `Ok(false)` rather than an error: a branch that no
/// longer exists has not been merged, and a status read must not fail because
/// of it (FR-476).
pub fn is_merged_into(worktree: &Path, branch: &str, target: &str) -> Result<bool, GitError> {
    if branch == target {
        return Ok(false);
    }
    let Some(branch_tip) = resolve_ref(worktree, branch)? else {
        return Ok(false);
    };
    let Some(target_tip) = resolve_ref(worktree, target)? else {
        return Ok(false);
    };
    if branch_tip == target_tip {
        return Ok(true);
    }
    // `merge-base --is-ancestor` exits 0 when it is, 1 when it is not, and
    // something else on a real failure. Only the last is an error.
    let out = Command::new("git")
        .arg("-C")
        .arg(worktree)
        .args(["merge-base", "--is-ancestor", &branch_tip, &target_tip])
        .output()
        .map_err(|e| GitError::CommandFailed {
            command: "rev-parse".into(),
            stderr: e.to_string(),
        })?;
    match out.status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => Ok(false),
    }
}

/// Resolve a ref to an object id, or `None` when it does not resolve.
///
/// Unresolvable is a distinguishable outcome rather than an error, because
/// every caller treats it as "inconclusive" rather than as a failure (FR-366).
pub fn resolve_ref(worktree: &Path, name: &str) -> Result<Option<String>, GitError> {
    let out = Command::new("git")
        .arg("-C")
        .arg(worktree)
        .args([
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{name}^{{commit}}"),
        ])
        .output()
        .map_err(|e| GitError::CommandFailed {
            command: "rev-parse".into(),
            stderr: e.to_string(),
        })?;
    if !out.status.success() {
        return Ok(None);
    }
    let id = String::from_utf8_lossy(&out.stdout).trim().to_string();
    Ok((!id.is_empty()).then_some(id))
}

/// Whether a commit is present in this clone.
///
/// Absent is `Ok(false)`: a commit this clone has never fetched is not an
/// error, it is a check that cannot conclude.
pub fn commit_present(worktree: &Path, commit: &str) -> Result<bool, GitError> {
    Ok(resolve_ref(worktree, commit)?.is_some())
}

/// True when `git` can be executed at all.
pub fn git_available() -> bool {
    Command::new("git")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn git(dir: &Path, args: &[&str]) {
        let out = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .expect("git runs");
        assert!(
            out.status.success(),
            "git {:?}: {}",
            args,
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn init_repo() -> TempDir {
        let dir = TempDir::new().unwrap();
        git(dir.path(), &["init", "--initial-branch=main"]);
        git(dir.path(), &["config", "user.email", "t@example.com"]);
        git(dir.path(), &["config", "user.name", "t"]);
        git(dir.path(), &["config", "commit.gpgsign", "false"]);
        dir
    }

    #[test]
    fn discovers_a_fresh_repository_with_no_commits() {
        let dir = init_repo();
        let inst = discover(dir.path()).unwrap();
        assert!(inst.git_common_dir.ends_with(".git"));
        assert_eq!(inst.worktree_path, canonical(dir.path()).unwrap());
        assert!(inst.remote.is_none());

        let st = status(dir.path()).unwrap();
        assert_eq!(st.branch, "main");
        assert!(st.commit_sha.is_none(), "no commits yet");
    }

    #[test]
    fn reports_working_tree_counts() {
        let dir = init_repo();
        fs::write(dir.path().join("a.txt"), "one").unwrap();
        git(dir.path(), &["add", "a.txt"]);
        git(dir.path(), &["commit", "-m", "init", "--no-gpg-sign"]);

        fs::write(dir.path().join("a.txt"), "two").unwrap();
        fs::write(dir.path().join("b.txt"), "new").unwrap();
        git(dir.path(), &["add", "b.txt"]);
        fs::write(dir.path().join("c.txt"), "untracked").unwrap();

        let st = status(dir.path()).unwrap();
        assert!(st.commit_sha.is_some());
        assert_eq!(st.staged, 1, "b.txt staged");
        assert_eq!(st.unstaged, 1, "a.txt modified");
        assert_eq!(st.untracked, 1, "c.txt untracked");
        assert_eq!(st.changed_files.len(), 3);
    }

    #[test]
    fn detached_head_still_resolves() {
        let dir = init_repo();
        fs::write(dir.path().join("a.txt"), "one").unwrap();
        git(dir.path(), &["add", "."]);
        git(dir.path(), &["commit", "-m", "init", "--no-gpg-sign"]);
        let head = run(dir.path(), &["rev-parse", "HEAD"])
            .unwrap()
            .trim()
            .to_string();
        git(dir.path(), &["checkout", "--detach", &head]);

        let st = status(dir.path()).unwrap();
        assert_eq!(st.branch, DETACHED);
        assert_eq!(st.commit_sha.as_deref(), Some(head.as_str()));
    }

    #[test]
    fn two_worktrees_share_one_git_common_dir() {
        // This is what makes them one local project (FR-004).
        let dir = init_repo();
        fs::write(dir.path().join("a.txt"), "one").unwrap();
        git(dir.path(), &["add", "."]);
        git(dir.path(), &["commit", "-m", "init", "--no-gpg-sign"]);

        let wt = TempDir::new().unwrap();
        let wt_path = wt.path().join("second");
        git(
            dir.path(),
            &["worktree", "add", wt_path.to_str().unwrap(), "-b", "second"],
        );

        let a = discover(dir.path()).unwrap();
        let b = discover(&wt_path).unwrap();
        assert_eq!(a.git_common_dir, b.git_common_dir);
        assert_ne!(
            a.worktree_path, b.worktree_path,
            "distinct working contexts"
        );
    }

    #[test]
    fn a_clone_at_another_path_is_a_different_local_instance() {
        // Path can never be shared identity — that is server-assigned (FR-064).
        let origin = init_repo();
        fs::write(origin.path().join("a.txt"), "one").unwrap();
        git(origin.path(), &["add", "."]);
        git(origin.path(), &["commit", "-m", "init", "--no-gpg-sign"]);

        let dest = TempDir::new().unwrap();
        let clone_path = dest.path().join("clone");
        let out = Command::new("git")
            .args([
                "clone",
                origin.path().to_str().unwrap(),
                clone_path.to_str().unwrap(),
            ])
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );

        let a = discover(origin.path()).unwrap();
        let b = discover(&clone_path).unwrap();
        assert_ne!(a.git_common_dir, b.git_common_dir);
        assert!(b.remote.is_some(), "clone has an origin remote");
    }

    #[test]
    fn not_a_repository_is_a_clean_error() {
        let dir = TempDir::new().unwrap();
        match discover(dir.path()) {
            Err(GitError::NotARepository(_)) => {}
            other => panic!("expected NotARepository, got {other:?}"),
        }
    }

    #[test]
    fn lists_local_branches() {
        let dir = init_repo();
        fs::write(dir.path().join("a.txt"), "one").unwrap();
        git(dir.path(), &["add", "."]);
        git(dir.path(), &["commit", "-m", "init", "--no-gpg-sign"]);
        git(dir.path(), &["branch", "feature/x"]);

        let branches = local_branches(dir.path()).unwrap();
        assert!(branches.contains(&"main".to_string()), "{branches:?}");
        assert!(branches.contains(&"feature/x".to_string()), "{branches:?}");
    }

    #[test]
    fn remote_normalization_is_shape_independent() {
        assert_eq!(
            normalize_remote("git@github.com:Vellixia/Cairn.git"),
            "github.com/Vellixia/Cairn"
        );
        assert_eq!(
            normalize_remote("https://github.com/Vellixia/Cairn.git"),
            "github.com/Vellixia/Cairn"
        );
        assert_eq!(
            normalize_remote("https://user:pass@github.com/Vellixia/Cairn"),
            "github.com/Vellixia/Cairn"
        );
    }
}

#[cfg(test)]
mod merge_tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn git(dir: &Path, args: &[&str]) {
        let out = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .expect("git runs");
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn repo() -> TempDir {
        let dir = TempDir::new().expect("dir");
        let p = dir.path();
        git(p, &["init", "--initial-branch=main"]);
        git(p, &["config", "user.email", "t@example.com"]);
        git(p, &["config", "user.name", "T"]);
        git(p, &["config", "commit.gpgsign", "false"]);
        fs::write(p.join("a.txt"), "one\n").expect("write");
        git(p, &["add", "."]);
        git(p, &["commit", "-m", "one", "--no-gpg-sign"]);
        dir
    }

    #[test]
    fn a_merged_branch_is_an_ancestor_of_the_target() {
        let dir = repo();
        let p = dir.path();
        git(p, &["checkout", "-b", "feature/x"]);
        fs::write(p.join("b.txt"), "two\n").expect("write");
        git(p, &["add", "."]);
        git(p, &["commit", "-m", "two", "--no-gpg-sign"]);

        // Not merged yet.
        assert!(!is_merged_into(p, "feature/x", "main").expect("check"));

        git(p, &["checkout", "main"]);
        git(
            p,
            &[
                "merge",
                "--no-ff",
                "feature/x",
                "-m",
                "merge",
                "--no-gpg-sign",
            ],
        );
        assert!(is_merged_into(p, "feature/x", "main").expect("check"));
    }

    #[test]
    fn a_branch_is_never_merged_into_itself() {
        let dir = repo();
        assert!(!is_merged_into(dir.path(), "main", "main").expect("check"));
    }

    #[test]
    fn an_unresolvable_ref_is_not_merged_rather_than_an_error() {
        // A branch that no longer exists has not been merged, and a status read
        // must not fail because of it.
        let dir = repo();
        assert!(!is_merged_into(dir.path(), "branch/gone", "main").expect("check"));
        assert!(!is_merged_into(dir.path(), "main", "branch/gone").expect("check"));
        assert_eq!(
            resolve_ref(dir.path(), "branch/gone").expect("resolve"),
            None
        );
    }

    #[test]
    fn a_ref_resolves_to_a_commit_and_a_missing_commit_is_absent() {
        let dir = repo();
        let head = resolve_ref(dir.path(), "HEAD")
            .expect("resolve")
            .expect("head");
        assert_eq!(head.len(), 40, "{head}");
        assert!(commit_present(dir.path(), &head).expect("present"));
        assert!(
            !commit_present(dir.path(), "0000000000000000000000000000000000000000")
                .expect("absent")
        );
    }
}
