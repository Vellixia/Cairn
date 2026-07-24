//! T065/T066/T068: Feature 002 task CLI behavior and stable JSON envelopes.

mod support;

use cairn_protocol::{
    CliEnvelope, ErrorCode, TaskCreateResult, TaskGetResult, TaskListResult, TaskReviseResult,
};
use support::CliHarness;

fn contract(goal: &str) -> String {
    serde_json::json!({
        "schema_version": 1,
        "goal": goal,
        "included_scope": ["task model"],
        "excluded_scope": ["session binding"],
        "acceptance_criteria": ["immutable revisions"],
        "constraints": ["offline"]
    })
    .to_string()
}

async fn create_project(harness: &CliHarness) -> String {
    let (created, code) = harness
        .cairn_json(&["project", "create", "--name", "Tasks", "--json"], None)
        .await;
    assert_eq!(code, 0);
    created["data"]["project"]["project_id"]
        .as_str()
        .unwrap()
        .to_string()
}

fn parse_full_json(output: &std::process::Output) -> serde_json::Value {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let value: serde_json::Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|error| panic!("expected one JSON envelope: {error}: {stdout}"));
    assert_eq!(value["schema"], serde_json::json!("cairn.cli.v1"));
    value
}

#[tokio::test(flavor = "multi_thread")]
async fn task_commands_use_daemon_ipc_and_preserve_historical_revisions() {
    let harness = CliHarness::start().await;
    let project_id = create_project(&harness).await;
    let contract_path = harness.dir.path().join("goal.json");
    std::fs::write(&contract_path, contract("revision one")).unwrap();
    let path = contract_path.to_string_lossy().to_string();

    let (created, create_code) = harness
        .cairn_json(
            &[
                "task",
                "create",
                "--project-id",
                &project_id,
                "--title",
                "Duplicate",
                "--goal-contract",
                &path,
                "--json",
            ],
            None,
        )
        .await;
    assert_eq!(create_code, 0);
    let first: TaskCreateResult = serde_json::from_value(created["data"].clone()).unwrap();
    assert_eq!(first.revision.revision_number, 1);
    let original_contract = serde_json::to_vec(&first.revision.goal_contract).unwrap();
    let original_fingerprint = first.revision.goal_contract_fingerprint.clone();

    let (duplicate, duplicate_code) = harness
        .cairn_json(
            &[
                "task",
                "create",
                "--project-id",
                &project_id,
                "--title",
                "Duplicate",
                "--goal-contract",
                &path,
                "--json",
            ],
            None,
        )
        .await;
    assert_eq!(duplicate_code, 0);
    let duplicate: TaskCreateResult = serde_json::from_value(duplicate["data"].clone()).unwrap();
    assert_ne!(first.task.task_id, duplicate.task.task_id);

    let task_id = first.task.task_id.to_string();
    let revised_output = harness
        .cairn_full(
            &[
                "task",
                "revise",
                "--task-id",
                &task_id,
                "--goal-contract",
                "-",
                "--json",
            ],
            None,
            Some(&contract("revision two")),
            &[],
        )
        .await;
    assert_eq!(revised_output.status.code(), Some(0));
    let revised_json = parse_full_json(&revised_output);
    let revised: TaskReviseResult = serde_json::from_value(revised_json["data"].clone()).unwrap();
    assert_eq!(revised.revision.revision_number, 2);
    assert_eq!(
        revised.revision.parent_revision_id,
        Some(first.revision.revision_id)
    );

    let revision_one_id = first.revision.revision_id.to_string();
    let (historical, historical_code) = harness
        .cairn_json(
            &[
                "task",
                "show",
                "--task-id",
                &task_id,
                "--revision-id",
                &revision_one_id,
                "--json",
            ],
            None,
        )
        .await;
    assert_eq!(historical_code, 0);
    let historical: TaskGetResult = serde_json::from_value(historical["data"].clone()).unwrap();
    assert_eq!(historical.task.latest_revision_number, 2);
    assert_eq!(historical.revision.revision_number, 1);
    assert_eq!(
        serde_json::to_vec(&historical.revision.goal_contract).unwrap(),
        original_contract
    );
    assert_eq!(
        historical.revision.goal_contract_fingerprint,
        original_fingerprint
    );

    let (listed, list_code) = harness
        .cairn_json(
            &["task", "list", "--project-id", &project_id, "--json"],
            None,
        )
        .await;
    assert_eq!(list_code, 0);
    let listed: TaskListResult = serde_json::from_value(listed["data"].clone()).unwrap();
    assert_eq!(listed.tasks.len(), 2);
    assert!(listed
        .tasks
        .windows(2)
        .all(|tasks| tasks[0].task_id < tasks[1].task_id));
    harness.stop();
}

#[tokio::test(flavor = "multi_thread")]
async fn task_title_selection_and_global_idempotency_are_stable() {
    let harness = CliHarness::start().await;
    let project_id = create_project(&harness).await;
    let contract_path = harness.dir.path().join("goal.json");
    std::fs::write(&contract_path, contract("same request")).unwrap();
    let path = contract_path.to_string_lossy().to_string();
    let key = uuid::Uuid::now_v7().to_string();
    let base = [
        "task",
        "create",
        "--project-id",
        &project_id,
        "--title",
        "Ambiguous",
        "--goal-contract",
        &path,
        "--idempotency-key",
        &key,
        "--json",
    ];
    let (created, code) = harness.cairn_json(&base, None).await;
    assert_eq!(code, 0);
    let first: TaskCreateResult = serde_json::from_value(created["data"].clone()).unwrap();
    let (retried, retry_code) = harness.cairn_json(&base, None).await;
    assert_eq!(retry_code, 0);
    let retry: TaskCreateResult = serde_json::from_value(retried["data"].clone()).unwrap();
    assert_eq!(first, retry);

    let (conflict, conflict_code) = harness
        .cairn_json(
            &[
                "task",
                "create",
                "--project-id",
                &project_id,
                "--title",
                "Different",
                "--goal-contract",
                &path,
                "--idempotency-key",
                &key,
                "--json",
            ],
            None,
        )
        .await;
    assert_eq!(conflict_code, 1);
    assert_eq!(
        conflict["error"]["code"],
        serde_json::json!("IDEMPOTENCY_CONFLICT")
    );

    harness
        .cairn_json(
            &[
                "task",
                "create",
                "--project-id",
                &project_id,
                "--title",
                "Ambiguous",
                "--goal-contract",
                &path,
                "--json",
            ],
            None,
        )
        .await;
    let json_name = harness
        .cairn_full(
            &[
                "task",
                "show",
                "--task",
                "Ambiguous",
                "--project-id",
                &project_id,
                "--json",
            ],
            None,
            None,
            &[("CAIRN_SOCKET_PATH", "/definitely-not-an-ipc-endpoint")],
        )
        .await;
    assert_eq!(json_name.status.code(), Some(2));
    let json_name = parse_full_json(&json_name);
    assert_eq!(json_name["error"]["code"], serde_json::json!("USAGE"));

    let ambiguous = harness
        .cairn(
            &[
                "task",
                "show",
                "--task",
                "Ambiguous",
                "--project-id",
                &project_id,
            ],
            None,
        )
        .await;
    assert_eq!(ambiguous.status.code(), Some(4));
    assert!(ambiguous.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&ambiguous.stderr);
    let ids: Vec<_> = stderr
        .lines()
        .filter(|line| line.trim().len() == 36)
        .map(str::trim)
        .collect();
    assert_eq!(ids.len(), 2);
    assert!(ids.windows(2).all(|pair| pair[0] < pair[1]));
    harness.stop();
}

#[tokio::test(flavor = "multi_thread")]
async fn every_goal_contract_violation_exits_one_without_content_leakage() {
    let harness = CliHarness::start().await;
    let project_id = create_project(&harness).await;
    let base = serde_json::json!({
        "schema_version":1,
        "goal":"private-goal-sentinel",
        "included_scope":[],
        "excluded_scope":[],
        "acceptance_criteria":[],
        "constraints":[]
    });
    let required = [
        "schema_version",
        "goal",
        "included_scope",
        "excluded_scope",
        "acceptance_criteria",
        "constraints",
    ];
    let mut invalid = Vec::new();
    for field in required {
        let mut value = base.clone();
        value.as_object_mut().unwrap().remove(field);
        invalid.push(value.to_string());
    }
    invalid.push("[\"private-goal-sentinel\"]".into());
    let mut empty_goal = base.clone();
    empty_goal["goal"] = serde_json::json!(" \t ");
    invalid.push(empty_goal.to_string());
    for field in [
        "included_scope",
        "excluded_scope",
        "acceptance_criteria",
        "constraints",
    ] {
        let mut value = base.clone();
        value[field] = serde_json::json!([" \r\n "]);
        invalid.push(value.to_string());
    }
    let mut unsupported = base;
    unsupported["schema_version"] = serde_json::json!(2);
    invalid.push(unsupported.to_string());
    assert_eq!(invalid.len(), 13);

    for contract in invalid {
        let output = harness
            .cairn_full(
                &[
                    "task",
                    "create",
                    "--project-id",
                    &project_id,
                    "--title",
                    "Invalid",
                    "--goal-contract",
                    "-",
                    "--json",
                ],
                None,
                Some(&contract),
                &[],
            )
            .await;
        assert_eq!(output.status.code(), Some(1));
        assert!(output.stderr.is_empty());
        let envelope = parse_full_json(&output);
        assert_eq!(
            envelope["error"]["code"],
            serde_json::json!("INVALID_GOAL_CONTRACT")
        );
        let rendered = serde_json::to_string(&envelope).unwrap();
        assert!(!rendered.contains("private-goal-sentinel"));
        assert!(!rendered.contains("/private/"));
        assert!(!rendered.contains("resume-token"));
    }
    harness.stop();
}

#[test]
fn checked_in_cli_task_goldens_are_valid_and_cli_has_no_sqlx_dependency() {
    for file in [
        "task-create.json",
        "task-revise.json",
        "task-historical.json",
        "task-idempotency-error.json",
    ] {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/goldens")
            .join(file);
        let value: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let envelope: CliEnvelope = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(serde_json::to_value(envelope).unwrap(), value);
    }
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/goldens/task-goal-contract-errors.json");
    let values: Vec<serde_json::Value> =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(values.len(), 13);
    for value in values {
        let envelope: CliEnvelope = serde_json::from_value(value).unwrap();
        assert_eq!(envelope.error.unwrap().code, ErrorCode::InvalidGoalContract);
    }
    let manifest = include_str!("../Cargo.toml");
    assert!(!manifest
        .lines()
        .any(|line| line.trim_start().starts_with("sqlx")));
}
