mod support;

use cairn_protocol::{CliEnvelope, ErrorCode, SessionBindResult};
use fixtures_repositories::FixtureRepo;
use support::CliHarness;

fn contract(goal: &str) -> String {
    serde_json::json!({
        "schema_version":1,
        "goal":goal,
        "included_scope":["binding"],
        "excluded_scope":[],
        "acceptance_criteria":["one binding"],
        "constraints":["append-only"]
    })
    .to_string()
}

#[tokio::test(flavor = "multi_thread")]
async fn session_bind_uses_daemon_ipc_and_preserves_exact_retry_flags() {
    let harness = CliHarness::start().await;
    let repository = FixtureRepo::new().unwrap();
    let (registered, code) = harness
        .cairn_json(&["init", "--json"], Some(repository.root()))
        .await;
    assert_eq!(code, 0);
    let repository_id = registered["data"]["repository"]["repository_id"]
        .as_str()
        .unwrap()
        .to_string();
    let (started, code) = harness
        .cairn_json(
            &["session", "start", "--agent", "binding-test", "--json"],
            Some(repository.root()),
        )
        .await;
    assert_eq!(code, 0);
    let session_id = started["data"]["session"]["session_id"]
        .as_str()
        .unwrap()
        .to_string();
    let (project, code) = harness
        .cairn_json(&["project", "create", "--name", "Bound", "--json"], None)
        .await;
    assert_eq!(code, 0);
    let project_id = project["data"]["project"]["project_id"]
        .as_str()
        .unwrap()
        .to_string();
    let (_, code) = harness
        .cairn_json(
            &[
                "project",
                "repository",
                "add",
                "--project-id",
                &project_id,
                "--repository-id",
                &repository_id,
                "--json",
            ],
            None,
        )
        .await;
    assert_eq!(code, 0);
    let goal = harness.dir.path().join("binding-goal.json");
    std::fs::write(&goal, contract("revision one")).unwrap();
    let goal = goal.to_string_lossy().to_string();
    let (task, code) = harness
        .cairn_json(
            &[
                "task",
                "create",
                "--project-id",
                &project_id,
                "--title",
                "Bind",
                "--goal-contract",
                &goal,
                "--json",
            ],
            None,
        )
        .await;
    assert_eq!(code, 0);
    let revision_id = task["data"]["revision"]["revision_id"]
        .as_str()
        .unwrap()
        .to_string();
    let key = uuid::Uuid::now_v7().to_string();
    let args = [
        "session",
        "bind",
        "--session",
        &session_id,
        "--project-id",
        &project_id,
        "--task-revision-id",
        &revision_id,
        "--idempotency-key",
        &key,
        "--json",
    ];
    let (created, code) = harness.cairn_json(&args, None).await;
    assert_eq!(code, 0);
    let first: SessionBindResult = serde_json::from_value(created["data"].clone()).unwrap();
    assert!(first.created);
    let (retried, code) = harness.cairn_json(&args, None).await;
    assert_eq!(code, 0);
    let retry: SessionBindResult = serde_json::from_value(retried["data"].clone()).unwrap();
    assert_eq!(retry, first);

    let (existing, code) = harness
        .cairn_json(
            &[
                "session",
                "bind",
                "--session",
                &session_id,
                "--project-id",
                &project_id,
                "--task-revision-id",
                &revision_id,
                "--json",
            ],
            None,
        )
        .await;
    assert_eq!(code, 0);
    let existing: SessionBindResult = serde_json::from_value(existing["data"].clone()).unwrap();
    assert!(!existing.created);
    assert_eq!(existing.binding_identity(), first.binding_identity());

    let human = harness
        .cairn(
            &[
                "session",
                "bind",
                "--session",
                &session_id,
                "--project-id",
                &project_id,
                "--task-revision-id",
                &revision_id,
            ],
            None,
        )
        .await;
    assert_eq!(human.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&human.stdout);
    assert!(stdout.contains("project_bound"));
    assert!(stdout.contains(&session_id));
    assert!(stdout.contains(&project_id));
    assert!(stdout.contains(&revision_id));
    assert!(!stdout.contains("resume_token"));

    let revised_goal = harness.dir.path().join("binding-goal-two.json");
    std::fs::write(&revised_goal, contract("revision two")).unwrap();
    let revised_goal = revised_goal.to_string_lossy().to_string();
    let task_id = task["data"]["task"]["task_id"].as_str().unwrap();
    let (revised, code) = harness
        .cairn_json(
            &[
                "task",
                "revise",
                "--task-id",
                task_id,
                "--goal-contract",
                &revised_goal,
                "--json",
            ],
            None,
        )
        .await;
    assert_eq!(code, 0);
    let revision_two = revised["data"]["revision"]["revision_id"].as_str().unwrap();
    let (key_conflict, code) = harness
        .cairn_json(
            &[
                "session",
                "bind",
                "--session",
                &session_id,
                "--project-id",
                &project_id,
                "--task-revision-id",
                revision_two,
                "--idempotency-key",
                &key,
                "--json",
            ],
            None,
        )
        .await;
    assert_eq!(code, 1);
    assert_eq!(
        key_conflict["error"]["code"],
        serde_json::json!("IDEMPOTENCY_CONFLICT")
    );
    let (binding_conflict, code) = harness
        .cairn_json(
            &[
                "session",
                "bind",
                "--session",
                &session_id,
                "--project-id",
                &project_id,
                "--task-revision-id",
                revision_two,
                "--json",
            ],
            None,
        )
        .await;
    assert_eq!(code, 1);
    assert_eq!(
        binding_conflict["error"]["code"],
        serde_json::json!("SESSION_BINDING_CONFLICT")
    );

    let invalid = harness
        .cairn_full(
            &[
                "session",
                "bind",
                "--session",
                "not-an-id",
                "--project-id",
                &project_id,
                "--task-revision-id",
                &revision_id,
                "--json",
            ],
            None,
            None,
            &[("CAIRN_SOCKET_PATH", "/definitely-not-an-ipc-endpoint")],
        )
        .await;
    assert_eq!(invalid.status.code(), Some(2));
    let invalid: serde_json::Value = serde_json::from_slice(invalid.stdout.as_slice()).unwrap();
    assert_eq!(invalid["error"]["code"], serde_json::json!("USAGE"));
    harness.stop();
}

#[test]
fn checked_in_session_bind_cli_goldens_are_typed_and_private() {
    for file in [
        "session-bind-created.json",
        "session-bind-existing.json",
        "session-bind-conflict.json",
    ] {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/goldens")
            .join(file);
        let value: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let envelope: CliEnvelope = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(serde_json::to_value(envelope).unwrap(), value);
        let rendered = serde_json::to_string(&value).unwrap();
        for forbidden in ["resume_token", "/private/", "goal_contract", "SELECT "] {
            assert!(!rendered.contains(forbidden));
        }
    }
    assert_eq!(ErrorCode::SessionBindingConflict.exit_code(), 1);
    let manifest = include_str!("../Cargo.toml");
    assert!(!manifest
        .lines()
        .any(|line| line.trim_start().starts_with("sqlx")));
}

trait BindingIdentity {
    fn binding_identity(&self) -> (String, String, String, String);
}

impl BindingIdentity for SessionBindResult {
    fn binding_identity(&self) -> (String, String, String, String) {
        let (project, revision) = match self.scope {
            cairn_protocol::SessionScopeDto::ProjectBound {
                project_id,
                task_revision_id,
            } => (project_id.to_string(), task_revision_id.to_string()),
            cairn_protocol::SessionScopeDto::LocalUnbound => panic!("bind returned unbound"),
        };
        (
            self.session_id.to_string(),
            project,
            revision,
            self.bound_at.to_rfc3339(),
        )
    }
}
