mod support;

use std::collections::BTreeSet;

use cairn_protocol::*;
use fixtures_repositories::FixtureRepo;
use support::CliHarness;

#[tokio::test(flavor = "multi_thread")]
async fn every_feature002_command_emits_one_typed_stable_json_envelope() {
    let harness = CliHarness::start().await;
    let repository = FixtureRepo::new().unwrap();
    let mut seen = BTreeSet::new();

    let registered = run(
        &harness,
        &["init", "--json"],
        Some(repository.root()),
        "init",
        &mut seen,
    )
    .await;
    let repository_id = registered["data"]["repository"]["repository_id"]
        .as_str()
        .unwrap()
        .to_string();
    let agent = AgentInstanceId::new_v7().to_string();
    let started = run(
        &harness,
        &[
            "session",
            "start",
            "--agent",
            "json-stability",
            "--agent-instance",
            &agent,
            "--local-unbound",
            "--json",
        ],
        Some(repository.root()),
        "session.start",
        &mut seen,
    )
    .await;
    let session_id = started["data"]["session"]["session_id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(started["data"]["session"]["scope"]["mode"], "local_unbound");

    let created = run(
        &harness,
        &["project", "create", "--name", "JSON surface", "--json"],
        None,
        "project.create",
        &mut seen,
    )
    .await;
    let project_id = created["data"]["project"]["project_id"]
        .as_str()
        .unwrap()
        .to_string();
    run(
        &harness,
        &["project", "list", "--json"],
        None,
        "project.list",
        &mut seen,
    )
    .await;
    run(
        &harness,
        &["project", "show", "--project-id", &project_id, "--json"],
        None,
        "project.show",
        &mut seen,
    )
    .await;
    run(
        &harness,
        &[
            "project",
            "update",
            "--project-id",
            &project_id,
            "--description",
            "typed",
            "--json",
        ],
        None,
        "project.update",
        &mut seen,
    )
    .await;
    run(
        &harness,
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
        "project.repository.add",
        &mut seen,
    )
    .await;

    let goal1 = harness.dir.path().join("goal-1.json");
    let goal2 = harness.dir.path().join("goal-2.json");
    std::fs::write(&goal1, goal("revision one")).unwrap();
    std::fs::write(&goal2, goal("revision two")).unwrap();
    let goal1 = goal1.to_string_lossy().to_string();
    let goal2 = goal2.to_string_lossy().to_string();
    let task = run(
        &harness,
        &[
            "task",
            "create",
            "--project-id",
            &project_id,
            "--title",
            "JSON task",
            "--goal-contract",
            &goal1,
            "--json",
        ],
        None,
        "task.create",
        &mut seen,
    )
    .await;
    let task_id = task["data"]["task"]["task_id"]
        .as_str()
        .unwrap()
        .to_string();
    let revision_id = task["data"]["revision"]["revision_id"]
        .as_str()
        .unwrap()
        .to_string();
    run(
        &harness,
        &["task", "list", "--project-id", &project_id, "--json"],
        None,
        "task.list",
        &mut seen,
    )
    .await;
    run(
        &harness,
        &[
            "task",
            "show",
            "--task-id",
            &task_id,
            "--revision-id",
            &revision_id,
            "--json",
        ],
        None,
        "task.show",
        &mut seen,
    )
    .await;
    run(
        &harness,
        &[
            "task",
            "revise",
            "--task-id",
            &task_id,
            "--parent-revision-id",
            &revision_id,
            "--goal-contract",
            &goal2,
            "--json",
        ],
        None,
        "task.revise",
        &mut seen,
    )
    .await;

    let bound = run(
        &harness,
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
        "session.bind",
        &mut seen,
    )
    .await;
    assert_eq!(bound["data"]["scope"]["mode"], "project_bound");
    run(
        &harness,
        &["session", "show", "--session", &session_id, "--json"],
        Some(repository.root()),
        "session.show",
        &mut seen,
    )
    .await;
    run(
        &harness,
        &[
            "session",
            "list",
            "--project-id",
            &project_id,
            "--task-revision-id",
            &revision_id,
            "--json",
        ],
        Some(repository.root()),
        "session.list",
        &mut seen,
    )
    .await;

    let approved: BTreeSet<_> = [
        "project.create",
        "project.list",
        "project.show",
        "project.update",
        "project.repository.add",
        "task.create",
        "task.revise",
        "task.list",
        "task.show",
        "session.bind",
        "session.start",
        "session.show",
        "session.list",
        "init",
    ]
    .into_iter()
    .collect();
    assert_eq!(seen, approved);

    let missing = ProjectId::new_v7().to_string();
    let (domain, code) = harness
        .cairn_json(
            &["project", "show", "--project-id", &missing, "--json"],
            None,
        )
        .await;
    assert_eq!(code, 1);
    assert_failure(&domain, "project.show", ErrorCode::ProjectNotFound);

    let (usage, code) = harness
        .cairn_json(
            &[
                "session",
                "start",
                "--agent",
                "bad",
                "--project-id",
                &project_id,
                "--json",
            ],
            Some(repository.root()),
        )
        .await;
    assert_eq!(code, 2);
    assert_failure(&usage, "session.start", ErrorCode::Usage);

    let outside = tempfile::TempDir::new().unwrap();
    let (not_repo, code) = harness
        .cairn_json(&["status", "--json"], Some(outside.path()))
        .await;
    assert_eq!(code, 3);
    assert_eq!(not_repo["ok"], false);

    assert_eq!(ErrorCode::AmbiguousName.exit_code(), 4);
    assert_eq!(ErrorCode::DaemonUnavailable.exit_code(), 5);
    assert_eq!(ErrorCode::MigrationFailed.exit_code(), 6);
    harness.stop();
}

async fn run(
    harness: &CliHarness,
    args: &[&str],
    cwd: Option<&std::path::Path>,
    command: &'static str,
    seen: &mut BTreeSet<&'static str>,
) -> serde_json::Value {
    let output = harness.cairn(args, cwd).await;
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(stdout.lines().count(), 1, "{command}: {stdout:?}");
    let value: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    let envelope: CliEnvelope = serde_json::from_value(value.clone()).unwrap();
    assert!(envelope.ok);
    assert_eq!(envelope.command, command);
    assert!(envelope.error.is_none());
    seen.insert(command);
    value
}

fn assert_failure(value: &serde_json::Value, command: &str, code: ErrorCode) {
    let envelope: CliEnvelope = serde_json::from_value(value.clone()).unwrap();
    assert!(!envelope.ok);
    assert_eq!(envelope.command, command);
    assert!(envelope.data.is_none());
    assert_eq!(envelope.error.unwrap().code, code);
}

fn goal(value: &str) -> String {
    serde_json::json!({
        "schema_version": 1,
        "goal": value,
        "included_scope": [],
        "excluded_scope": [],
        "acceptance_criteria": [],
        "constraints": []
    })
    .to_string()
}
