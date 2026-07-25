//! T045: US3 live tracking — edits reflected after quiescence (SC-003 bound),
//! branch changes evented, bursts coalesced losslessly, touch = no-op,
//! delete + rebase survive without corruption.

mod support;

use cairn_protocol::methods;
use fixtures_repositories::FixtureRepo;
use serde_json::json;
use support::TestDaemon;

struct Live {
    daemon: TestDaemon,
    repo: FixtureRepo,
    repo_id: String,
    session_id: String,
    agent_instance_id: String,
}

fn path_of(repo: &FixtureRepo) -> String {
    repo.root().to_string_lossy().to_string()
}

async fn start_live() -> Live {
    let daemon = TestDaemon::start().await;
    let repo = FixtureRepo::new().unwrap();
    let reg = daemon
        .call(
            methods::REPOSITORY_REGISTER,
            &json!({"path": path_of(&repo)}),
        )
        .await
        .unwrap();
    let repo_id = reg["repository"]["repository_id"]
        .as_str()
        .unwrap()
        .to_string();
    let inst = uuid::Uuid::new_v4().to_string();
    let started = daemon
        .call(
            methods::SESSION_START,
            &json!({"path": path_of(&repo), "agent_type": "watcher-test",
                    "agent_instance_id": inst}),
        )
        .await
        .unwrap();
    let session_id = started["session"]["session_id"]
        .as_str()
        .unwrap()
        .to_string();
    Live {
        daemon,
        repo,
        repo_id,
        session_id,
        agent_instance_id: inst,
    }
}

async fn current_fp(live: &Live) -> String {
    let get = live
        .daemon
        .call(
            methods::SESSION_GET,
            &json!({"session_id": live.session_id}),
        )
        .await
        .unwrap();
    get["session"]["current_snapshot"]["snapshot_fp"]
        .as_str()
        .unwrap()
        .to_string()
}

/// Wait (≤5 s per SC-003) for the current fingerprint to differ from `from`.
async fn wait_fp_change(live: &Live, from: &str) -> String {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let fp = current_fp(live).await;
        if fp != from {
            return fp;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "current snapshot did not update within the 5s SC-003 bound"
        );
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

async fn event_types(live: &Live) -> Vec<String> {
    let events = live
        .daemon
        .call(
            methods::EVENTS_LIST,
            &json!({"repository_id": live.repo_id, "limit": 1000}),
        )
        .await
        .unwrap();
    events["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["event_type"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn file_edit_updates_current_snapshot_within_bound() {
    let live = start_live().await;
    let fp0 = current_fp(&live).await;
    let controls = live.daemon.watcher_controls();
    let baseline = controls.reconciled_count();
    live.repo.write("tracked-change.txt", "hello\n").unwrap();
    controls.wait_reconciled_after(baseline).await;
    let fp1 = current_fp(&live).await;
    assert_ne!(fp0, fp1);
    assert!(event_types(&live)
        .await
        .contains(&"repository.state_changed".to_string()));
    live.daemon.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn branch_switch_emits_branch_changed_and_updates_snapshot() {
    let live = start_live().await;
    let fp0 = current_fp(&live).await;
    live.repo.checkout_new_branch("feature-x").unwrap();
    // Branch creation alone doesn't change HEAD content; commit to move HEAD.
    live.repo
        .commit_file("on-feature.txt", "x\n", "feature commit")
        .unwrap();
    let _fp1 = wait_fp_change(&live, &fp0).await;

    let get = live
        .daemon
        .call(
            methods::SESSION_GET,
            &json!({"session_id": live.session_id}),
        )
        .await
        .unwrap();
    assert_eq!(
        get["session"]["current_snapshot"]["branch"],
        json!("feature-x")
    );
    let events = event_types(&live).await;
    assert!(events.contains(&"branch.changed".to_string()));
    eprintln!(
        "feature001_scenario=3 result=pass repository_state_changed={} branch_changed={}",
        events
            .iter()
            .filter(|kind| kind.as_str() == "repository.state_changed")
            .count(),
        events
            .iter()
            .filter(|kind| kind.as_str() == "branch.changed")
            .count(),
    );
    live.daemon.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn burst_of_writes_coalesces_but_final_state_is_correct() {
    let live = start_live().await;
    let controls = live.daemon.watcher_controls();
    let baseline = controls.reconciled_count();
    for i in 0..100 {
        live.repo
            .write("burst.txt", &format!("line {i}\n"))
            .unwrap();
    }
    controls.wait_reconciled_after(baseline).await;

    let authoritative = cairn_git::fingerprint::fingerprint_state(live.repo.root())
        .await
        .unwrap()
        .components
        .final_fingerprint();
    let tracked = current_fp(&live).await;
    assert_eq!(
        tracked, authoritative,
        "final tracked state must match git reality (FR-023)"
    );

    // Coalescing: far fewer state-change events than writes.
    let changes = event_types(&live)
        .await
        .iter()
        .filter(|t| *t == "repository.state_changed")
        .count();
    assert!(
        changes < 20,
        "100 writes must coalesce (got {changes} state changes)"
    );
    live.daemon.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn touch_without_content_change_creates_no_snapshot() {
    let live = start_live().await;
    let controls = live.daemon.watcher_controls();
    let baseline = controls.reconciled_count();
    live.repo.write("stable.txt", "same\n").unwrap();
    controls.wait_reconciled_after(baseline).await;
    let fp1 = current_fp(&live).await;
    let events_before = event_types(&live).await.len();

    // Rewrite identical content: hint fires, reconciliation finds no change.
    let baseline = controls.reconciled_count();
    live.repo.write("stable.txt", "same\n").unwrap();
    controls.wait_reconciled_after(baseline).await;

    assert_eq!(
        current_fp(&live).await,
        fp1,
        "touch must not change the fingerprint (FR-022)"
    );
    let events_after = event_types(&live).await.len();
    assert_eq!(events_before, events_after, "no events for a non-change");
    live.daemon.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn session_start_waits_for_install_and_reconciles_installation_window_changes() {
    let daemon = TestDaemon::start().await;
    let controls = daemon.watcher_controls();
    let repo = FixtureRepo::new().unwrap();
    let reg = daemon
        .call(
            methods::REPOSITORY_REGISTER,
            &json!({"path": path_of(&repo)}),
        )
        .await
        .unwrap();
    let repo_id = reg["repository"]["repository_id"]
        .as_str()
        .unwrap()
        .to_string();

    controls.pause_before_install();
    controls.pause_before_reconcile();
    let mut client = daemon.client().await;
    let path = path_of(&repo);
    let instance = uuid::Uuid::new_v4().to_string();
    let start = tokio::spawn(async move {
        client
            .call(
                methods::SESSION_START,
                &json!({"path": path, "agent_type": "install-window",
                        "agent_instance_id": instance}),
            )
            .await
    });

    controls.wait_before_install().await;
    repo.write("README.md", "modified during install\n")
        .unwrap();
    repo.write("created.txt", "created\n").unwrap();
    repo.write("rename-source.txt", "renamed\n").unwrap();
    std::fs::rename(
        repo.root().join("rename-source.txt"),
        repo.root().join("rename-target.txt"),
    )
    .unwrap();
    controls.release_install();

    controls.wait_before_reconcile().await;
    // README.md is present in the seed commit and therefore in the initial
    // authoritative session snapshot. Delete it after the OS watcher is
    // installed but before readiness/reconciliation completes: this both
    // creates an unambiguous Git deletion and queues an advisory notification.
    repo.delete("README.md").unwrap();
    assert!(
        !start.is_finished(),
        "session-start must not return before post-install reconciliation"
    );
    let reconciled_before_release = controls.reconciled_count();
    controls.release_reconcile();
    let started = start.await.unwrap().unwrap();

    let authoritative = cairn_git::fingerprint::fingerprint_state(repo.root())
        .await
        .unwrap()
        .components
        .final_fingerprint();
    assert_eq!(
        started["session"]["current_snapshot"]["snapshot_fp"],
        authoritative
    );
    assert_ne!(
        started["session"]["start_snapshot"]["snapshot_fp"],
        started["session"]["current_snapshot"]["snapshot_fp"],
        "the initially committed-file deletion must change the returned session snapshot"
    );
    let status = repo.git(&["status", "--porcelain=v1"]).unwrap();
    assert!(
        status.lines().any(|line| line == " D README.md"),
        "Git must authoritatively report deletion of the initially committed file: {status:?}"
    );

    // Wait for the queued advisory notification to trigger its own reconcile.
    // Both that path and the readiness reconcile must converge to one logical
    // repository.state_changed event, without correctness sleeps.
    controls
        .wait_reconciled_after(reconciled_before_release + 1)
        .await;
    let events = daemon
        .call(
            methods::EVENTS_LIST,
            &json!({"repository_id": repo_id, "limit": 1000}),
        )
        .await
        .unwrap();
    let changes: Vec<&serde_json::Value> = events["events"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|event| event["event_type"] == "repository.state_changed")
        .collect();
    assert_eq!(
        changes.len(),
        1,
        "the authoritative installation-window delta is recorded once"
    );
    assert_eq!(
        changes[0]["payload"]["from_snapshot_id"],
        started["session"]["start_snapshot"]["snapshot_id"]
    );
    assert_eq!(
        changes[0]["payload"]["to_snapshot_id"],
        started["session"]["current_snapshot"]["snapshot_id"]
    );
    daemon.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn dropped_notification_is_recovered_by_authoritative_reconciliation() {
    let live = start_live().await;
    let controls = live.daemon.watcher_controls();
    controls.drop_notifications();
    live.repo
        .write("dropped-hint.txt", "still observed\n")
        .unwrap();

    let before = event_types(&live)
        .await
        .iter()
        .filter(|kind| *kind == "repository.state_changed")
        .count();
    let restarted = live
        .daemon
        .call(
            methods::SESSION_START,
            &json!({"path": path_of(&live.repo), "agent_type": "watcher-test",
                    "agent_instance_id": live.agent_instance_id}),
        )
        .await
        .unwrap();
    controls.resume_notifications();
    assert_eq!(restarted["outcome"], "existing");

    let authoritative = cairn_git::fingerprint::fingerprint_state(live.repo.root())
        .await
        .unwrap()
        .components
        .final_fingerprint();
    assert_eq!(
        restarted["session"]["current_snapshot"]["snapshot_fp"],
        authoritative
    );
    let after = event_types(&live)
        .await
        .iter()
        .filter(|kind| *kind == "repository.state_changed")
        .count();
    assert_eq!(after, before + 1);
    live.daemon.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn notification_and_explicit_reconciliation_do_not_duplicate_change_events() {
    let live = start_live().await;
    let controls = live.daemon.watcher_controls();
    let baseline = controls.reconciled_count();
    live.repo.write("deduplicated.txt", "one change\n").unwrap();
    controls.wait_reconciled_after(baseline).await;
    let before = event_types(&live)
        .await
        .iter()
        .filter(|kind| *kind == "repository.state_changed")
        .count();

    live.daemon
        .call(
            methods::SESSION_START,
            &json!({"path": path_of(&live.repo), "agent_type": "watcher-test",
                    "agent_instance_id": live.agent_instance_id}),
        )
        .await
        .unwrap();
    let after = event_types(&live)
        .await
        .iter()
        .filter(|kind| *kind == "repository.state_changed")
        .count();
    assert_eq!(after, before, "unchanged Git state must be idempotent");
    live.daemon.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn watcher_installation_failure_interrupts_session_and_returns_stable_error() {
    let daemon = TestDaemon::start().await;
    let controls = daemon.watcher_controls();
    let repo = FixtureRepo::new().unwrap();
    daemon
        .call(
            methods::REPOSITORY_REGISTER,
            &json!({"path": path_of(&repo)}),
        )
        .await
        .unwrap();
    controls.force_install_failure();

    let error = daemon
        .call(
            methods::SESSION_START,
            &json!({"path": path_of(&repo), "agent_type": "install-failure",
                    "agent_instance_id": uuid::Uuid::new_v4()}),
        )
        .await
        .unwrap_err();
    assert_eq!(
        serde_json::to_value(error.code).unwrap(),
        json!("WATCHER_START_FAILED")
    );
    let data = serde_json::to_value(error.data.unwrap()).unwrap();
    assert_eq!(
        data,
        json!({"kind": "watcher_start_failure", "stage": "install"})
    );
    assert!(!error.message.contains(&path_of(&repo)));
    assert!(!error.message.contains("deterministic test injection"));

    let listed = daemon
        .call(methods::SESSION_LIST, &json!({}))
        .await
        .unwrap();
    let sessions = listed["sessions"].as_array().unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0]["state"], json!("interrupted"));
    let session_id = sessions[0]["session_id"].as_str().unwrap();
    let events = daemon
        .call(
            methods::EVENTS_LIST,
            &json!({"session_id": session_id, "limit": 1000}),
        )
        .await
        .unwrap();
    let interrupted = events["events"]
        .as_array()
        .unwrap()
        .iter()
        .find(|event| event["event_type"] == "session.interrupted")
        .expect("watcher failure interruption event");
    assert_eq!(interrupted["payload"]["reason"], "watcher_start_failed");
    assert_eq!(interrupted["payload"]["watcher_stage"], "install");
    daemon.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn post_install_reconciliation_failure_interrupts_session_with_stage() {
    let daemon = TestDaemon::start().await;
    let controls = daemon.watcher_controls();
    let repo = FixtureRepo::new().unwrap();
    daemon
        .call(
            methods::REPOSITORY_REGISTER,
            &json!({"path": path_of(&repo)}),
        )
        .await
        .unwrap();
    controls.force_reconcile_failure();

    let error = daemon
        .call(
            methods::SESSION_START,
            &json!({"path": path_of(&repo), "agent_type": "reconcile-failure",
                    "agent_instance_id": uuid::Uuid::new_v4()}),
        )
        .await
        .unwrap_err();
    assert_eq!(
        serde_json::to_value(error.code).unwrap(),
        json!("WATCHER_START_FAILED")
    );
    let data = serde_json::to_value(error.data.unwrap()).unwrap();
    assert_eq!(
        data,
        json!({"kind": "watcher_start_failure", "stage": "reconcile"})
    );
    assert!(!error.message.contains(&path_of(&repo)));
    assert!(!error.message.contains("deterministic test injection"));

    let listed = daemon
        .call(methods::SESSION_LIST, &json!({}))
        .await
        .unwrap();
    let session = &listed["sessions"].as_array().unwrap()[0];
    assert_eq!(session["state"], json!("interrupted"));
    let events = daemon
        .call(
            methods::EVENTS_LIST,
            &json!({"session_id": session["session_id"], "limit": 1000}),
        )
        .await
        .unwrap();
    let interrupted = events["events"]
        .as_array()
        .unwrap()
        .iter()
        .find(|event| event["event_type"] == "session.interrupted")
        .expect("reconciliation failure interruption event");
    assert_eq!(interrupted["payload"]["reason"], "watcher_start_failed");
    assert_eq!(interrupted["payload"]["watcher_stage"], "reconcile");
    daemon.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn delete_and_rebase_are_tracked_without_corruption() {
    let live = start_live().await;

    // Deletion.
    live.repo
        .commit_file("victim.txt", "bye\n", "add victim")
        .unwrap();
    let fp_committed = wait_fp_change(&live, &current_fp(&live).await).await;
    live.repo.delete("victim.txt").unwrap();
    let fp_deleted = wait_fp_change(&live, &fp_committed).await;
    assert_ne!(fp_committed, fp_deleted);
    live.repo.git(&["checkout", "--", "victim.txt"]).unwrap();

    // Rebase rewriting history.
    let fp_before = wait_fp_change(&live, &fp_deleted).await;
    live.repo.checkout_new_branch("side").unwrap();
    live.repo
        .commit_file("side.txt", "s\n", "side work")
        .unwrap();
    live.repo.checkout("main").unwrap();
    live.repo
        .commit_file("main.txt", "m\n", "main work")
        .unwrap();
    live.repo.checkout("side").unwrap();
    live.repo.git(&["rebase", "main"]).unwrap();
    let fp_after = wait_fp_change(&live, &fp_before).await;
    assert_ne!(fp_before, fp_after);

    // Ground truth still matches after the churn settles.
    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
    let authoritative = cairn_git::fingerprint::fingerprint_state(live.repo.root())
        .await
        .unwrap()
        .components
        .final_fingerprint();
    assert_eq!(current_fp(&live).await, authoritative);
    live.daemon.stop().await;
}
