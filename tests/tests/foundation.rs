//! T014 — the foundation works against a real repository.

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
fn a_non_repository_fails_cleanly_and_creates_no_state() {
    let home = tempfile::TempDir::new().unwrap();
    let plain = tempfile::TempDir::new().unwrap();
    let socket = cairn_e2e::sandbox_socket();

    let out = Command::new(binary("cairn"))
        .args(["--json", "status"])
        .current_dir(plain.path())
        .env("CAIRN_HOME", home.path())
        .env("CAIRN_SOCKET", &socket)
        .env("CAIRND_BIN", binary("cairnd"))
        .output()
        .expect("cairn runs");

    let envelope: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("envelope");
    assert_eq!(envelope["ok"], false);
    assert_eq!(envelope["error"]["code"], "not_a_repository");
    assert_eq!(out.status.code(), Some(1), "a user error exits 1");

    // No partial state: no project row was written (FR-005).
    let db = home.path().join("cairn.sqlite3");
    if db.exists() {
        let bytes = std::fs::read(&db).unwrap_or_default();
        let text = String::from_utf8_lossy(&bytes);
        assert!(
            !text.contains(&plain.path().display().to_string()),
            "a project was registered for a non-repository"
        );
    }

    let _ = Command::new(binary("cairn"))
        .args(["daemon", "stop"])
        .env("CAIRN_HOME", home.path())
        .env("CAIRN_SOCKET", &socket)
        .output();
    #[cfg(unix)]
    {
        let _ = std::fs::remove_file(&socket);
    }
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
fn concurrent_first_use_of_one_fresh_repository_never_races() {
    // H1 / FR-002: register-or-reuse must be atomic. Check-then-insert used to
    // answer this with `UNIQUE constraint failed: projects.git_common_dir`.
    let s = Sandbox::new();

    // A cold store, so every thread below is racing the very first insert.
    s.must(&["daemon", "stop"]);
    std::thread::sleep(std::time::Duration::from_millis(200));

    let threads: Vec<_> = (0..12)
        .map(|_| {
            let repo = s.repo_path().to_path_buf();
            let home = s.home.path().to_path_buf();
            let socket = s.socket.clone();
            std::thread::spawn(move || {
                let out = Command::new(binary("cairn"))
                    .args(["--json", "status"])
                    .current_dir(&repo)
                    .env("CAIRN_HOME", &home)
                    .env("CAIRN_SOCKET", &socket)
                    .env("CAIRND_BIN", binary("cairnd"))
                    .output()
                    .expect("cairn runs");
                String::from_utf8_lossy(&out.stdout).to_string()
            })
        })
        .collect();

    let mut project_ids = std::collections::HashSet::new();
    for (i, t) in threads.into_iter().enumerate() {
        let raw = t.join().expect("thread");
        let envelope: serde_json::Value =
            serde_json::from_str(&raw).unwrap_or_else(|e| panic!("thread {i}: {e}\n{raw}"));
        assert_eq!(
            envelope["ok"],
            true,
            "concurrent first use failed: {}",
            serde_json::to_string(&envelope["error"]).unwrap_or_default()
        );
        project_ids.insert(
            envelope["data"]["project"]["id"]
                .as_str()
                .unwrap()
                .to_string(),
        );
    }
    assert_eq!(
        project_ids.len(),
        1,
        "one repository must map to one project"
    );
}

#[test]
fn a_corrupt_database_is_detected_and_reported() {
    let s = Sandbox::new();
    s.must(&["daemon", "stop"]);
    std::thread::sleep(std::time::Duration::from_millis(200));

    // Overwrite the database with garbage.
    std::fs::write(s.db_path(), b"not a valid sqlite database").expect("write");

    let out = s.cairn(&["status"]);
    assert!(
        !out.ok(),
        "a corrupt database must not report success: {}",
        out.stderr
    );
}

#[test]
fn an_unwritable_cairn_home_is_reported_cleanly() {
    let repo = tempfile::TempDir::new().expect("repo");
    let socket = cairn_e2e::sandbox_socket();

    // Initialize a git repo.
    Command::new("git")
        .args(["init", "--initial-branch=main"])
        .current_dir(repo.path())
        .output()
        .expect("git init");
    Command::new("git")
        .args(["config", "user.email", "test@example.com"])
        .current_dir(repo.path())
        .output()
        .expect("git config");
    Command::new("git")
        .args(["config", "user.name", "Cairn Test"])
        .current_dir(repo.path())
        .output()
        .expect("git config");
    std::fs::write(repo.path().join("README.md"), "# fixture\n").expect("write");
    Command::new("git")
        .args(["add", "."])
        .current_dir(repo.path())
        .output()
        .expect("git add");
    Command::new("git")
        .args(["commit", "-m", "init", "--no-gpg-sign"])
        .current_dir(repo.path())
        .output()
        .expect("git commit");

    // Use a file as CAIRN_HOME — writing inside it must fail on every platform.
    let home_file = tempfile::NamedTempFile::new().expect("file");
    let invalid_home = home_file.path().to_path_buf();

    let out = Command::new(binary("cairn"))
        .args(["--json", "init"])
        .current_dir(repo.path())
        .env("CAIRN_HOME", &invalid_home)
        .env("CAIRN_SOCKET", &socket)
        .env("CAIRND_BIN", binary("cairnd"))
        .output()
        .expect("cairn runs");

    // The command should fail, not panic or hang.
    let envelope: serde_json::Value = serde_json::from_str(&String::from_utf8_lossy(&out.stdout))
        .unwrap_or(serde_json::Value::Null);
    assert!(
        !out.status.success() || envelope["ok"] == serde_json::Value::Bool(false),
        "an unwritable home must fail: {}",
        String::from_utf8_lossy(&out.stderr)
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

#[test]
fn git_not_on_path_is_reported_cleanly() {
    let home = tempfile::TempDir::new().expect("home");
    let repo = tempfile::TempDir::new().expect("repo");
    let socket = cairn_e2e::sandbox_socket();

    // Initialize a git repo so the directory is valid.
    Command::new("git")
        .args(["init", "--initial-branch=main"])
        .current_dir(repo.path())
        .output()
        .expect("git init");

    // Run cairn with a PATH that does not include git.
    let out = Command::new(binary("cairn"))
        .args(["--json", "init"])
        .current_dir(repo.path())
        .env("CAIRN_HOME", home.path())
        .env("CAIRN_SOCKET", &socket)
        .env("CAIRND_BIN", binary("cairnd"))
        .env("PATH", "") // Empty PATH — git won't be found.
        .output()
        .expect("cairn runs");

    let envelope: serde_json::Value = serde_json::from_str(&String::from_utf8_lossy(&out.stdout))
        .unwrap_or(serde_json::Value::Null);
    assert!(
        !out.status.success() || envelope["ok"] == serde_json::Value::Bool(false),
        "missing git must fail: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}
