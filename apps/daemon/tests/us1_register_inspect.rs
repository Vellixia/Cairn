//! T031: US1 acceptance — registration idempotence, non-Git rejection, bare
//! rejection, marker restore/ambiguity, inspection matching git ground truth.

mod support;

use cairn_protocol::methods;
use fixtures_repositories::FixtureRepo;
use serde_json::json;
use support::TestDaemon;

fn path_of(repo: &FixtureRepo) -> String {
    repo.root().to_string_lossy().to_string()
}

#[tokio::test(flavor = "multi_thread")]
async fn fresh_init_registers_once_and_reinit_is_idempotent() {
    let daemon = TestDaemon::start().await;
    let repo = FixtureRepo::new().unwrap();

    let first = daemon
        .call(
            methods::REPOSITORY_REGISTER,
            &json!({"path": path_of(&repo)}),
        )
        .await
        .unwrap();
    assert_eq!(first["created"], json!(true));
    assert_eq!(first["identity_outcome"], json!("created"));
    let repo_id = first["repository"]["repository_id"]
        .as_str()
        .unwrap()
        .to_string();

    // Identity marker written into Git-private metadata, not the tracked tree.
    assert!(repo.root().join(".git/cairn/repository-id").exists());
    assert!(repo.root().join(".git/cairn/worktree-id").exists());

    let second = daemon
        .call(
            methods::REPOSITORY_REGISTER,
            &json!({"path": path_of(&repo)}),
        )
        .await
        .unwrap();
    assert_eq!(second["created"], json!(false));
    assert_eq!(second["identity_outcome"], json!("existing"));
    assert_eq!(
        second["repository"]["repository_id"].as_str().unwrap(),
        repo_id
    );

    // Exactly one registration event, exactly one repository row implied by
    // identical id; verify via events stream.
    let events = daemon
        .call(methods::EVENTS_LIST, &json!({"repository_id": repo_id}))
        .await
        .unwrap();
    let reg_events: Vec<_> = events["events"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["event_type"] == "repository.registered")
        .collect();
    assert_eq!(
        reg_events.len(),
        1,
        "re-init must not emit another registration event"
    );
    daemon.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn non_git_directory_is_rejected_with_zero_writes() {
    let daemon = TestDaemon::start().await;
    let dir = tempfile::TempDir::new().unwrap();

    let err = daemon
        .call(
            methods::REPOSITORY_REGISTER,
            &json!({"path": dir.path().to_string_lossy()}),
        )
        .await
        .unwrap_err();
    assert_eq!(
        serde_json::to_value(err.code).unwrap(),
        json!("NOT_A_REPOSITORY")
    );

    let events = daemon.call(methods::EVENTS_LIST, &json!({})).await.unwrap();
    assert_eq!(
        events["events"].as_array().unwrap().len(),
        0,
        "no partial registration"
    );
    daemon.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn bare_repository_is_rejected_not_a_worktree() {
    let daemon = TestDaemon::start().await;
    let repo = FixtureRepo::bare().unwrap();

    let err = daemon
        .call(
            methods::REPOSITORY_REGISTER,
            &json!({"path": repo.root().to_string_lossy()}),
        )
        .await
        .unwrap_err();
    assert_eq!(
        serde_json::to_value(err.code).unwrap(),
        json!("NOT_A_WORKTREE")
    );

    // Zero rows, markers, or events.
    assert!(!repo.root().join("cairn").exists());
    let events = daemon.call(methods::EVENTS_LIST, &json!({})).await.unwrap();
    assert_eq!(events["events"].as_array().unwrap().len(), 0);
    daemon.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn marker_deletion_restores_unique_identity() {
    let daemon = TestDaemon::start().await;
    let repo = FixtureRepo::new().unwrap();

    let first = daemon
        .call(
            methods::REPOSITORY_REGISTER,
            &json!({"path": path_of(&repo)}),
        )
        .await
        .unwrap();
    let repo_id = first["repository"]["repository_id"]
        .as_str()
        .unwrap()
        .to_string();
    let repo_uuid = first["repository"]["repo_uuid"]
        .as_str()
        .unwrap()
        .to_string();

    repo.delete_identity_markers().unwrap();

    let restored = daemon
        .call(
            methods::REPOSITORY_REGISTER,
            &json!({"path": path_of(&repo)}),
        )
        .await
        .unwrap();
    assert_eq!(restored["identity_outcome"], json!("restored"));
    assert_eq!(
        restored["repository"]["repository_id"].as_str().unwrap(),
        repo_id
    );
    assert_eq!(
        restored["repository"]["repo_uuid"].as_str().unwrap(),
        repo_uuid
    );

    let events = daemon
        .call(methods::EVENTS_LIST, &json!({"repository_id": repo_id}))
        .await
        .unwrap();
    assert!(
        events["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["event_type"] == "identity.marker_restored"),
        "restoration must be recorded as an event"
    );
    daemon.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn inspection_matches_git_ground_truth() {
    let daemon = TestDaemon::start().await;
    let repo = FixtureRepo::new().unwrap();
    daemon
        .call(
            methods::REPOSITORY_REGISTER,
            &json!({"path": path_of(&repo)}),
        )
        .await
        .unwrap();

    // Dirty state: staged add, unstaged edit, untracked file, ignored file.
    repo.write("staged.txt", "s\n").unwrap();
    repo.stage("staged.txt").unwrap();
    repo.write("README.md", "edited\n").unwrap();
    repo.write("untracked.txt", "u\n").unwrap();
    repo.ignored_secret("sekret-value-123").unwrap();

    let ins = daemon
        .call(
            methods::REPOSITORY_INSPECT,
            &json!({"path": path_of(&repo)}),
        )
        .await
        .unwrap();

    assert_eq!(ins["branch"], json!("main"));
    assert_eq!(ins["detached"], json!(false));
    assert_eq!(
        ins["head_commit"].as_str().unwrap(),
        repo.head_oid().unwrap()
    );
    assert!(ins["staged"]
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c["path"] == "staged.txt"));
    assert!(ins["unstaged"]
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c["path"] == "README.md"));
    let untracked: Vec<&str> = ins["untracked"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert!(untracked.contains(&"untracked.txt"));
    assert!(
        untracked.contains(&".gitignore"),
        ".gitignore itself is untracked here"
    );
    assert!(
        !untracked.contains(&".env"),
        "ignored file must not appear as untracked"
    );
    assert!(ins["ignored_summary"]["total_count"].as_u64().unwrap() >= 1);
    assert_eq!(ins["worktree"]["is_main"], json!(true));
    assert_eq!(ins["in_progress"], json!(null));
    daemon.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn detached_head_and_no_remote_inspect_cleanly() {
    let daemon = TestDaemon::start().await;
    let repo = FixtureRepo::new().unwrap();
    daemon
        .call(
            methods::REPOSITORY_REGISTER,
            &json!({"path": path_of(&repo)}),
        )
        .await
        .unwrap();
    repo.detach_head().unwrap();

    let ins = daemon
        .call(
            methods::REPOSITORY_INSPECT,
            &json!({"path": path_of(&repo)}),
        )
        .await
        .unwrap();
    assert_eq!(ins["branch"], json!(null));
    assert_eq!(ins["detached"], json!(true));
    assert_eq!(ins["default_remote"], json!(null));
    daemon.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn ignored_files_paginate_bounded() {
    let daemon = TestDaemon::start().await;
    let repo = FixtureRepo::new().unwrap();
    repo.huge_ignored_tree("vendor", 250).unwrap();
    let reg = daemon
        .call(
            methods::REPOSITORY_REGISTER,
            &json!({"path": path_of(&repo)}),
        )
        .await
        .unwrap();
    let repo_id = reg["repository"]["repository_id"].as_str().unwrap();

    let page1 = daemon
        .call(
            methods::REPOSITORY_IGNORED_FILES,
            &json!({"repository_id": repo_id, "limit": 100}),
        )
        .await
        .unwrap();
    assert_eq!(page1["paths"].as_array().unwrap().len(), 100);
    let cursor = page1["next_cursor"].as_str().unwrap().to_string();

    let page2 = daemon
        .call(
            methods::REPOSITORY_IGNORED_FILES,
            &json!({"repository_id": repo_id, "limit": 100, "cursor": cursor}),
        )
        .await
        .unwrap();
    assert_eq!(page2["paths"].as_array().unwrap().len(), 100);
    // No overlap between pages.
    let p1: std::collections::HashSet<String> = page1["paths"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert!(page2["paths"]
        .as_array()
        .unwrap()
        .iter()
        .all(|v| !p1.contains(v.as_str().unwrap())));

    // Summary is bounded regardless of set size.
    let ins = daemon
        .call(
            methods::REPOSITORY_INSPECT,
            &json!({"path": path_of(&repo)}),
        )
        .await
        .unwrap();
    assert_eq!(ins["ignored_summary"]["total_count"], json!(250));
    assert!(ins["ignored_summary"]["samples"].as_array().unwrap().len() <= 20);
    daemon.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn daemon_status_reports_health() {
    let daemon = TestDaemon::start().await;
    let status = daemon
        .call(methods::DAEMON_STATUS, &json!({}))
        .await
        .unwrap();
    assert_eq!(status["db_healthy"], json!(true));
    assert!(status["version"].as_str().unwrap().contains('.'));
    daemon.stop().await;
}
