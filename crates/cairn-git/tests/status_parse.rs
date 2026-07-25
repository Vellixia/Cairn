//! T020: parser + discovery against real fixtures — detached HEAD, no-remote,
//! dirty tree, deletions, renames, linked worktree, rebase-in-progress, bare
//! rejection (FR-032 matrix).

use cairn_git::{discover, status, GitError, GitRunner};
use fixtures_repositories::FixtureRepo;

#[tokio::test]
async fn clean_repo_reports_branch_and_head() {
    let repo = FixtureRepo::new().unwrap();
    let runner = GitRunner::new(repo.root());
    let report = status::status(&runner).await.unwrap();
    assert_eq!(report.branch.as_deref(), Some("main"));
    assert_eq!(
        report.head_oid.as_deref(),
        Some(repo.head_oid().unwrap().as_str())
    );
    assert!(report.staged.is_empty());
    assert!(report.unstaged.is_empty());
    assert!(report.untracked.is_empty());
}

#[tokio::test]
async fn dirty_tree_classifies_staged_unstaged_untracked() {
    let repo = FixtureRepo::new().unwrap();
    repo.write("staged.txt", "s\n").unwrap();
    repo.stage("staged.txt").unwrap();
    repo.write("README.md", "changed\n").unwrap(); // tracked, unstaged edit
    repo.write("untracked.txt", "u\n").unwrap();

    let runner = GitRunner::new(repo.root());
    let report = status::status(&runner).await.unwrap();

    assert!(report.staged.iter().any(|c| c.path == "staged.txt"));
    assert!(report.unstaged.iter().any(|c| c.path == "README.md"));
    assert_eq!(report.untracked, vec!["untracked.txt".to_string()]);
}

#[tokio::test]
async fn deletion_is_reported() {
    let repo = FixtureRepo::new().unwrap();
    repo.commit_file("doomed.txt", "bye\n", "add doomed")
        .unwrap();
    repo.delete("doomed.txt").unwrap();
    let runner = GitRunner::new(repo.root());
    let report = status::status(&runner).await.unwrap();
    assert!(report
        .unstaged
        .iter()
        .any(|c| c.path == "doomed.txt" && c.status == status::ChangeKind::Deleted));
}

#[tokio::test]
async fn rename_carries_orig_path() {
    let repo = FixtureRepo::new().unwrap();
    repo.commit_file(
        "old-name.txt",
        "content that is long enough to match\n",
        "add",
    )
    .unwrap();
    repo.git(&["mv", "old-name.txt", "new-name.txt"]).unwrap();
    let runner = GitRunner::new(repo.root());
    let report = status::status(&runner).await.unwrap();
    let renamed = report
        .staged
        .iter()
        .find(|c| c.status == status::ChangeKind::Renamed)
        .expect("rename entry");
    assert_eq!(renamed.path, "new-name.txt");
    assert_eq!(renamed.orig_path.as_deref(), Some("old-name.txt"));
}

#[tokio::test]
async fn detached_head_reports_no_branch() {
    let repo = FixtureRepo::new().unwrap();
    repo.detach_head().unwrap();
    let runner = GitRunner::new(repo.root());
    let report = status::status(&runner).await.unwrap();
    assert_eq!(
        report.branch, None,
        "detached HEAD must not report a branch"
    );
    assert!(report.head_oid.is_some());
}

#[tokio::test]
async fn no_remote_is_none_not_error() {
    let repo = FixtureRepo::new().unwrap();
    let runner = GitRunner::new(repo.root());
    assert_eq!(discover::default_remote(&runner).await.unwrap(), None);

    repo.add_origin("git@example.invalid:demo.git").unwrap();
    let remote = discover::default_remote(&runner).await.unwrap().unwrap();
    assert_eq!(remote.0, "origin");
    assert_eq!(remote.1, "git@example.invalid:demo.git");
}

#[tokio::test]
async fn rebase_in_progress_is_detected() {
    let repo = FixtureRepo::new().unwrap();
    repo.start_conflicted_rebase().unwrap();
    let layout = discover::discover(repo.root()).await.unwrap();
    assert_eq!(discover::in_progress_operation(&layout), Some("rebase"));
}

#[tokio::test]
async fn linked_worktree_has_distinct_git_dir() {
    let repo = FixtureRepo::new().unwrap();
    let wt = repo.add_linked_worktree("feature-wt").unwrap();
    let main_layout = discover::discover(repo.root()).await.unwrap();
    let wt_layout = discover::discover(&wt).await.unwrap();
    assert!(main_layout.is_main);
    assert!(!wt_layout.is_main);
    assert_eq!(main_layout.git_common_dir, wt_layout.git_common_dir);
    assert_ne!(main_layout.git_dir, wt_layout.git_dir);
}

#[tokio::test]
async fn bare_repository_is_rejected_not_a_worktree() {
    let repo = FixtureRepo::bare().unwrap();
    let err = discover::discover(repo.root()).await.unwrap_err();
    assert!(matches!(err, GitError::NotAWorktree(_)), "got {err:?}");
}

#[tokio::test]
async fn non_git_directory_is_not_a_repository() {
    let dir = tempfile::TempDir::new().unwrap();
    let err = discover::discover(dir.path()).await.unwrap_err();
    assert!(matches!(err, GitError::NotARepository(_)), "got {err:?}");
}

#[tokio::test]
async fn ls_files_parse_covers_index() {
    let repo = FixtureRepo::new().unwrap();
    repo.write("a dir/spaced file.txt", "x\n").unwrap();
    repo.stage("a dir/spaced file.txt").unwrap();
    let runner = GitRunner::new(repo.root());
    let raw = runner.run(&["ls-files", "-s", "-z"]).await.unwrap();
    let entries = status::parse_ls_files(&raw).unwrap();
    assert!(entries.iter().any(|e| e.path == "a dir/spaced file.txt"));
    assert!(entries
        .iter()
        .all(|e| e.oid.len() == 40 && !e.mode.is_empty()));
}
