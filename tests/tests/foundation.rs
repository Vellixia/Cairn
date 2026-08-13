//! T014 — the foundation works against a real repository (tier 3, journey).
//!
//! What a user does on a normal repository: register it, see real Git state
//! reported back, and have every worktree of one repository resolve to one
//! project. The cases where the *environment* is broken — no repository, a
//! corrupt database, an unwritable home, no `git` — live in
//! `hostile_environment.rs`, and the first-use race lives in `concurrency.rs`.
//!
//! That split is not tidiness. Those tests each spawn a `cairn` that cannot
//! reach a daemon and wait out its start timeout, which is why this file used
//! to take 14s and set the floor for every e2e run, including the ones that
//! only wanted the journeys (see `docs/testing.md`).

use cairn_e2e::{binary, Sandbox};
use std::process::Command;

#[test]
fn init_is_idempotent_and_status_reports_real_repository_state() {
    let s = Sandbox::new();

    let first = s.json(&["init"]);
    let second = s.json(&["init"]);
    assert_eq!(
        first["project"]["id"], second["project"]["id"],
        "init must reuse the project it already registered (FR-002)"
    );

    // Dirty the working tree in three distinct ways.
    s.write_file("README.md", "# fixture\nchanged\n");
    s.write_file("staged.txt", "staged\n");
    s.git(&["add", "staged.txt"]);
    s.write_file("untracked.txt", "untracked\n");

    let status = s.json(&["status"]);
    assert_eq!(status["repository"]["branch"], "main");
    assert!(status["repository"]["commit_sha"].is_string());
    assert_eq!(status["repository"]["staged"], 1);
    assert_eq!(status["repository"]["unstaged"], 1);
    assert_eq!(status["repository"]["untracked"], 1);
    assert_eq!(status["daemon"], "running");
}

#[test]
fn two_worktrees_of_one_repository_are_one_project() {
    // FR-004: same project, distinct working contexts.
    let s = Sandbox::new();
    let project = s.json(&["init"])["project"]["id"].clone();

    let second = tempfile::TempDir::new().unwrap();
    let wt = second.path().join("wt");
    s.git(&["worktree", "add", wt.to_str().unwrap(), "-b", "second"]);

    let out = Command::new(binary("cairn"))
        .args(["--json", "init"])
        .current_dir(&wt)
        .env("CAIRN_HOME", s.home.path())
        .env("CAIRN_SOCKET", &s.socket)
        .env("CAIRND_BIN", binary("cairnd"))
        .output()
        .expect("cairn runs");
    let envelope: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("envelope");

    assert_eq!(envelope["data"]["project"]["id"], project);
    assert_ne!(
        envelope["data"]["worktree_path"],
        serde_json::json!(s.repo_path().canonicalize().unwrap().display().to_string()),
        "worktrees are distinct working contexts"
    );
}

#[test]
fn a_repository_with_a_gitdir_file_is_discovered() {
    // Some setups (submodules, worktrees with older git) use a `.git` file
    // containing `gitdir: /path/to/actual/.git` rather than a directory.
    let s = Sandbox::new();
    let git_dir = s.repo_path().join(".git");

    // Move .git to .real-git and replace with a gitdir file.
    let moved = s.repo_path().join(".real-git");
    std::fs::rename(&git_dir, &moved).expect("rename");
    std::fs::write(&git_dir, format!("gitdir: {}\n", moved.display())).expect("write gitdir");

    let status = s.json(&["status"]);
    assert_eq!(status["repository"]["branch"], "main");
    assert!(status["repository"]["commit_sha"].is_string());
}
