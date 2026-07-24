mod support;

use cairn_protocol::{
    CliEnvelope, ErrorCode, SessionGetResult, SessionListResult, SessionStartResult,
};
use fixtures_repositories::FixtureRepo;
use support::CliHarness;

fn golden(name: &str) -> serde_json::Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/goldens")
        .join(name);
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn goal_contract() -> String {
    serde_json::json!({
        "schema_version":1,
        "goal":"start one correctly scoped session",
        "included_scope":["session scope"],
        "excluded_scope":[],
        "acceptance_criteria":["binding persists"],
        "constraints":["append-only"]
    })
    .to_string()
}

async fn create_selectable_scope(
    harness: &CliHarness,
    repository: &FixtureRepo,
) -> (String, String, String) {
    let (registered, code) = harness
        .cairn_json(&["init", "--json"], Some(repository.root()))
        .await;
    assert_eq!(code, 0);
    let repository_id = registered["data"]["repository"]["repository_id"]
        .as_str()
        .unwrap()
        .to_string();
    let (project, code) = harness
        .cairn_json(
            &["project", "create", "--name", "Scoped sessions", "--json"],
            None,
        )
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
    let path = harness.dir.path().join("session-scope-goal.json");
    std::fs::write(&path, goal_contract()).unwrap();
    let path = path.to_string_lossy().to_string();
    let (task, code) = harness
        .cairn_json(
            &[
                "task",
                "create",
                "--project-id",
                &project_id,
                "--title",
                "Scoped start",
                "--goal-contract",
                &path,
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
    (repository_id, project_id, revision_id)
}

#[tokio::test(flavor = "multi_thread")]
async fn cli_starts_renders_filters_and_rejects_scopes_without_inference() {
    let harness = CliHarness::start().await;
    let bootstrap = FixtureRepo::new().unwrap();
    harness
        .cairn_json(&["init", "--json"], Some(bootstrap.root()))
        .await;
    for (instance, explicit) in [
        (uuid::Uuid::now_v7().to_string(), true),
        (uuid::Uuid::now_v7().to_string(), false),
    ] {
        let mut args = vec![
            "session",
            "start",
            "--agent",
            "bootstrap",
            "--agent-instance",
            &instance,
        ];
        if explicit {
            args.push("--local-unbound");
        }
        args.push("--json");
        let (envelope, code) = harness.cairn_json(&args, Some(bootstrap.root())).await;
        assert_eq!(code, 0);
        let result: SessionStartResult = serde_json::from_value(envelope["data"].clone()).unwrap();
        assert_eq!(
            result.session.scope,
            cairn_protocol::SessionScopeDto::LocalUnbound
        );
    }

    let repository = FixtureRepo::new().unwrap();
    let (repository_id, project_id, revision_id) =
        create_selectable_scope(&harness, &repository).await;
    for explicit in [false, true] {
        let instance = uuid::Uuid::now_v7().to_string();
        let mut args = vec![
            "session",
            "start",
            "--agent",
            "scope-required",
            "--agent-instance",
            &instance,
        ];
        if explicit {
            args.push("--local-unbound");
        }
        args.push("--json");
        let (envelope, code) = harness.cairn_json(&args, Some(repository.root())).await;
        assert_eq!(code, 1);
        assert_eq!(
            envelope["error"]["code"],
            serde_json::json!("PROJECT_SCOPE_REQUIRED")
        );
    }

    let agent = uuid::Uuid::now_v7().to_string();
    let bound_args = [
        "session",
        "start",
        "--agent",
        "bound-cli",
        "--agent-instance",
        &agent,
        "--project-id",
        &project_id,
        "--task-revision-id",
        &revision_id,
        "--json",
    ];
    let (bound, code) = harness
        .cairn_json(&bound_args, Some(repository.root()))
        .await;
    assert_eq!(code, 0);
    let result: SessionStartResult = serde_json::from_value(bound["data"].clone()).unwrap();
    let session_id = result.session.session_id.to_string();
    assert!(matches!(
        result.session.scope,
        cairn_protocol::SessionScopeDto::ProjectBound { .. }
    ));
    let (same, code) = harness
        .cairn_json(&bound_args, Some(repository.root()))
        .await;
    assert_eq!(code, 0);
    let same: SessionStartResult = serde_json::from_value(same["data"].clone()).unwrap();
    assert_eq!(same.outcome, cairn_protocol::StartOutcome::Existing);
    assert!(same.resume_token.is_none());

    let (conflict, code) = harness
        .cairn_json(
            &[
                "session",
                "start",
                "--agent",
                "bound-cli",
                "--agent-instance",
                &agent,
                "--local-unbound",
                "--json",
            ],
            Some(repository.root()),
        )
        .await;
    assert_eq!(code, 1);
    assert_eq!(
        conflict["error"]["code"],
        serde_json::json!("SESSION_SCOPE_CONFLICT")
    );

    let (shown, code) = harness
        .cairn_json(
            &["session", "show", "--session", &session_id, "--json"],
            Some(repository.root()),
        )
        .await;
    assert_eq!(code, 0);
    let shown: SessionGetResult = serde_json::from_value(shown["data"].clone()).unwrap();
    assert_eq!(shown.session.unwrap().scope, result.session.scope);
    let (listed, code) = harness
        .cairn_json(
            &[
                "session",
                "list",
                "--repository-id",
                &repository_id,
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
    let listed: SessionListResult = serde_json::from_value(listed["data"].clone()).unwrap();
    assert_eq!(listed.sessions.len(), 1);

    let human = harness
        .cairn(
            &["session", "show", "--session", &session_id],
            Some(repository.root()),
        )
        .await;
    assert_eq!(human.status.code(), Some(0));
    let output = String::from_utf8_lossy(&human.stdout);
    assert!(output.contains("project_bound"));
    assert!(output.contains(&project_id));
    assert!(output.contains(&revision_id));
    assert!(!output.contains("resume_token"));
    harness.stop();
}

#[tokio::test(flavor = "multi_thread")]
async fn paired_and_mutually_exclusive_flags_return_one_json_usage_envelope() {
    let harness = CliHarness::start().await;
    let project_id = uuid::Uuid::now_v7().to_string();
    let revision_id = uuid::Uuid::now_v7().to_string();
    for args in [
        vec![
            "session",
            "start",
            "--agent",
            "invalid",
            "--project-id",
            &project_id,
            "--json",
        ],
        vec![
            "session",
            "start",
            "--agent",
            "invalid",
            "--local-unbound",
            "--project-id",
            &project_id,
            "--task-revision-id",
            &revision_id,
            "--json",
        ],
    ] {
        let output = harness.cairn(&args, None).await;
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stderr.is_empty());
        let stdout = String::from_utf8(output.stdout).unwrap();
        assert_eq!(stdout.lines().count(), 1);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(stdout.trim()).unwrap(),
            golden("session-scope-usage.json")
        );
    }
    harness.stop();
}

#[test]
fn checked_in_session_scope_cli_goldens_are_typed_private_and_exit_codes_are_stable() {
    for file in [
        "session-scope-bound.json",
        "session-scope-project-required.json",
        "session-scope-conflict.json",
        "session-scope-project-not-found.json",
        "session-scope-project-archived.json",
        "session-scope-repository-mismatch.json",
        "session-scope-revision-not-found.json",
        "session-scope-task-mismatch.json",
        "session-scope-usage.json",
        "watcher-start-failed-install.json",
        "watcher-start-failed-reconcile.json",
    ] {
        let value = golden(file);
        let envelope: CliEnvelope = serde_json::from_value(value.clone()).unwrap();
        if let Some(error) = envelope.error {
            assert_eq!(
                error.code.exit_code(),
                if error.code == ErrorCode::Usage { 2 } else { 1 },
                "{file}"
            );
        }
        let rendered = serde_json::to_string(&value).unwrap();
        for forbidden in ["resume_token", "/private/", "SELECT ", "goal_contract"] {
            assert!(!rendered.contains(forbidden), "{file} leaked {forbidden}");
        }
    }
    assert_eq!(ErrorCode::ProjectScopeRequired.exit_code(), 1);
    assert_eq!(ErrorCode::SessionScopeConflict.exit_code(), 1);
    assert_eq!(ErrorCode::Usage.exit_code(), 2);
    let manifest = include_str!("../Cargo.toml");
    assert!(!manifest
        .lines()
        .any(|line| line.trim_start().starts_with("sqlx")));
}
