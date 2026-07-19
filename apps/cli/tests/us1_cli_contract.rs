//! T032: US1 CLI contract goldens — `--json` outputs deserialize into the
//! cairn-protocol DTOs (single source of truth) and exit codes hold.

mod support;

use cairn_protocol::{DaemonStatusResult, InspectionDto, RegisterResult};
use fixtures_repositories::FixtureRepo;
use support::CliHarness;

#[tokio::test(flavor = "multi_thread")]
async fn init_json_contract_and_idempotence() {
    let h = CliHarness::start().await;
    let repo = FixtureRepo::new().unwrap();

    let (env1, code1) = h.cairn_json(&["init", "--json"], Some(repo.root())).await;
    assert_eq!(code1, 0);
    assert_eq!(env1["ok"], serde_json::json!(true));
    assert_eq!(env1["command"], serde_json::json!("init"));
    let data1: RegisterResult = serde_json::from_value(env1["data"].clone())
        .expect("data validates against RegisterResult DTO");
    assert!(data1.created);

    let (env2, code2) = h.cairn_json(&["init", "--json"], Some(repo.root())).await;
    assert_eq!(code2, 0);
    let data2: RegisterResult = serde_json::from_value(env2["data"].clone()).unwrap();
    assert!(!data2.created);
    assert_eq!(
        data1.repository.repository_id,
        data2.repository.repository_id
    );
    h.stop();
}

#[tokio::test(flavor = "multi_thread")]
async fn init_outside_git_repo_exits_3() {
    let h = CliHarness::start().await;
    let dir = tempfile::TempDir::new().unwrap();
    let (envelope, code) = h.cairn_json(&["init", "--json"], Some(dir.path())).await;
    assert_eq!(code, 3);
    assert_eq!(envelope["ok"], serde_json::json!(false));
    assert_eq!(
        envelope["error"]["code"],
        serde_json::json!("NOT_A_REPOSITORY")
    );
    h.stop();
}

#[tokio::test(flavor = "multi_thread")]
async fn init_in_bare_repo_exits_3_not_a_worktree() {
    let h = CliHarness::start().await;
    let repo = FixtureRepo::bare().unwrap();
    let (envelope, code) = h.cairn_json(&["init", "--json"], Some(repo.root())).await;
    assert_eq!(code, 3);
    assert_eq!(
        envelope["error"]["code"],
        serde_json::json!("NOT_A_WORKTREE")
    );
    h.stop();
}

#[tokio::test(flavor = "multi_thread")]
async fn status_json_validates_inspection_dto() {
    let h = CliHarness::start().await;
    let repo = FixtureRepo::new().unwrap();
    h.cairn_json(&["init", "--json"], Some(repo.root())).await;
    repo.write("new.txt", "x\n").unwrap();

    let (envelope, code) = h.cairn_json(&["status", "--json"], Some(repo.root())).await;
    assert_eq!(code, 0);
    let data: InspectionDto = serde_json::from_value(envelope["data"].clone())
        .expect("data validates against InspectionDto");
    assert_eq!(data.branch.as_deref(), Some("main"));
    assert!(data.untracked.contains(&"new.txt".to_string()));
    h.stop();
}

#[tokio::test(flavor = "multi_thread")]
async fn daemon_status_json_validates_dto() {
    let h = CliHarness::start().await;
    let (envelope, code) = h.cairn_json(&["daemon", "status", "--json"], None).await;
    assert_eq!(code, 0);
    let data: DaemonStatusResult =
        serde_json::from_value(envelope["data"].clone()).expect("DaemonStatusResult");
    assert!(data.db_healthy);
    h.stop();
}

#[tokio::test(flavor = "multi_thread")]
async fn human_mode_emits_no_json_envelope() {
    let h = CliHarness::start().await;
    let repo = FixtureRepo::new().unwrap();
    let out = h.cairn(&["init"], Some(repo.root())).await;
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.contains("\"schema\""),
        "human mode must not print the envelope"
    );
    assert!(stdout.contains("Registered repository"));
    h.stop();
}
