//! T053: Feature 002 project CLI JSON/human contracts and exit codes.

mod support;

use cairn_protocol::{
    CliEnvelope, ErrorCode, ProjectCreateResult, ProjectGetResult, ProjectListResult,
    ProjectRepositoryAssociateResult, ProjectUpdateResult,
};
use fixtures_repositories::FixtureRepo;
use support::CliHarness;

#[tokio::test(flavor = "multi_thread")]
async fn project_commands_use_daemon_ipc_and_emit_typed_json() {
    let harness = CliHarness::start().await;
    let (first, first_code) = harness
        .cairn_json(
            &["project", "create", "--name", "Duplicate", "--json"],
            None,
        )
        .await;
    assert_eq!(first_code, 0);
    let first_result: ProjectCreateResult = serde_json::from_value(first["data"].clone()).unwrap();
    let (second, second_code) = harness
        .cairn_json(
            &["project", "create", "--name", "Duplicate", "--json"],
            None,
        )
        .await;
    assert_eq!(second_code, 0);
    let second_result: ProjectCreateResult =
        serde_json::from_value(second["data"].clone()).unwrap();
    assert_ne!(
        first_result.project.project_id,
        second_result.project.project_id
    );

    let (listed, list_code) = harness
        .cairn_json(&["project", "list", "--json"], None)
        .await;
    assert_eq!(list_code, 0);
    let list: ProjectListResult = serde_json::from_value(listed["data"].clone()).unwrap();
    assert_eq!(list.projects.len(), 2);
    assert!(list
        .projects
        .windows(2)
        .all(|values| values[0].project_id < values[1].project_id));

    let id = first_result.project.project_id.to_string();
    let (shown, show_code) = harness
        .cairn_json(&["project", "show", "--project-id", &id, "--json"], None)
        .await;
    assert_eq!(show_code, 0);
    let _: ProjectGetResult = serde_json::from_value(shown["data"].clone()).unwrap();

    let (updated, update_code) = harness
        .cairn_json(
            &[
                "project",
                "update",
                "--project-id",
                &id,
                "--description",
                "Local",
                "--json",
            ],
            None,
        )
        .await;
    assert_eq!(update_code, 0);
    let updated: ProjectUpdateResult = serde_json::from_value(updated["data"].clone()).unwrap();
    assert_eq!(updated.project.description.as_deref(), Some("Local"));

    let repository = FixtureRepo::new().unwrap();
    let (registered, init_code) = harness
        .cairn_json(&["init", "--json"], Some(repository.root()))
        .await;
    assert_eq!(init_code, 0);
    let repository_id = registered["data"]["repository"]["repository_id"]
        .as_str()
        .unwrap();
    let (association, association_code) = harness
        .cairn_json(
            &[
                "project",
                "repository",
                "add",
                "--project-id",
                &id,
                "--repository-id",
                repository_id,
                "--json",
            ],
            None,
        )
        .await;
    assert_eq!(association_code, 0);
    let association: ProjectRepositoryAssociateResult =
        serde_json::from_value(association["data"].clone()).unwrap();
    assert!(association.created);
    harness.stop();
}

#[tokio::test(flavor = "multi_thread")]
async fn name_selection_is_human_only_bounded_and_unambiguous() {
    let harness = CliHarness::start().await;
    harness
        .cairn_json(&["project", "create", "--name", "Same", "--json"], None)
        .await;
    harness
        .cairn_json(&["project", "create", "--name", "Same", "--json"], None)
        .await;
    harness
        .cairn_json(&["project", "create", "--name", "Unique", "--json"], None)
        .await;

    let (usage, usage_code) = harness
        .cairn_json(&["project", "show", "--project", "Unique", "--json"], None)
        .await;
    assert_eq!(usage_code, 2);
    assert_eq!(usage["error"]["code"], serde_json::json!("USAGE"));

    let ambiguous = harness
        .cairn(&["project", "show", "--project", "Same"], None)
        .await;
    assert_eq!(ambiguous.status.code(), Some(4));
    assert!(ambiguous.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&ambiguous.stderr);
    let candidate_lines: Vec<_> = stderr
        .lines()
        .filter(|line| line.trim().len() == 36)
        .map(str::trim)
        .collect();
    assert_eq!(candidate_lines.len(), 2);
    assert!(candidate_lines.windows(2).all(|ids| ids[0] < ids[1]));

    let unique = harness
        .cairn(&["project", "show", "--project", "Unique"], None)
        .await;
    assert_eq!(unique.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&unique.stdout);
    assert!(stdout.contains("Unique"));
    assert!(stdout.contains("project:"));
    harness.stop();
}

#[tokio::test(flavor = "multi_thread")]
async fn project_cli_error_exit_codes_are_stable() {
    let harness = CliHarness::start().await;
    let idempotency_key = uuid::Uuid::now_v7().to_string();
    harness
        .cairn_json(
            &[
                "project",
                "create",
                "--name",
                "Key owner",
                "--idempotency-key",
                &idempotency_key,
                "--json",
            ],
            None,
        )
        .await;
    let (key_conflict, key_conflict_code) = harness
        .cairn_json(
            &[
                "project",
                "create",
                "--name",
                "Different request",
                "--idempotency-key",
                &idempotency_key,
                "--json",
            ],
            None,
        )
        .await;
    assert_eq!(key_conflict_code, 1);
    assert_eq!(
        key_conflict["error"]["code"],
        serde_json::json!("IDEMPOTENCY_CONFLICT")
    );

    let (created, _) = harness
        .cairn_json(&["project", "create", "--name", "Archive", "--json"], None)
        .await;
    let project_id = created["data"]["project"]["project_id"].as_str().unwrap();
    let (archived, archive_code) = harness
        .cairn_json(
            &[
                "project",
                "update",
                "--project-id",
                project_id,
                "--status",
                "archived",
                "--json",
            ],
            None,
        )
        .await;
    assert_eq!(archive_code, 0);
    let _: ProjectUpdateResult = serde_json::from_value(archived["data"].clone()).unwrap();

    let repository = FixtureRepo::new().unwrap();
    let (registered, _) = harness
        .cairn_json(&["init", "--json"], Some(repository.root()))
        .await;
    let repository_id = registered["data"]["repository"]["repository_id"]
        .as_str()
        .unwrap();
    let (rejected, rejected_code) = harness
        .cairn_json(
            &[
                "project",
                "repository",
                "add",
                "--project-id",
                project_id,
                "--repository-id",
                repository_id,
                "--json",
            ],
            None,
        )
        .await;
    assert_eq!(rejected_code, 1);
    assert_eq!(
        rejected["error"]["code"],
        serde_json::json!("PROJECT_ARCHIVED")
    );

    let missing_id = uuid::Uuid::now_v7().to_string();
    let (missing, missing_code) = harness
        .cairn_json(
            &["project", "show", "--project-id", &missing_id, "--json"],
            None,
        )
        .await;
    assert_eq!(missing_code, ErrorCode::ProjectNotFound.exit_code());
    assert_eq!(
        missing["error"]["code"],
        serde_json::json!("PROJECT_NOT_FOUND")
    );
    harness.stop();
}

#[test]
fn checked_in_cli_project_goldens_are_valid_envelopes() {
    for file in [
        "project-create.json",
        "project-association-retry.json",
        "project-archived-error.json",
        "project-idempotency-error.json",
    ] {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/goldens")
            .join(file);
        let text = std::fs::read_to_string(path).unwrap();
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        let envelope: CliEnvelope = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(serde_json::to_value(envelope).unwrap(), value);
    }
}
