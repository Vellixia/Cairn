mod support;

use std::process::Command;

use cairn_protocol::{ProjectCreateResult, TaskCreateResult};
use support::CliHarness;

#[tokio::test(flavor = "multi_thread")]
async fn project_and_task_ambiguity_is_sorted_capped_and_truncated() {
    let harness = CliHarness::start().await;
    let mut project_ids = Vec::new();
    for _ in 0..21 {
        let (value, code) = harness
            .cairn_json(
                &["project", "create", "--name", "Same project", "--json"],
                None,
            )
            .await;
        assert_eq!(code, 0);
        project_ids.push(
            serde_json::from_value::<ProjectCreateResult>(value["data"].clone())
                .unwrap()
                .project
                .project_id
                .to_string(),
        );
    }
    project_ids.sort();
    let ambiguous = harness
        .cairn(&["project", "show", "--project", "Same project"], None)
        .await;
    assert_ambiguity(&ambiguous, &project_ids[..20], true);

    let (project, _) = harness
        .cairn_json(
            &["project", "create", "--name", "Task project", "--json"],
            None,
        )
        .await;
    let project_id = project["data"]["project"]["project_id"].as_str().unwrap();
    let goal = harness.dir.path().join("goal.json");
    std::fs::write(&goal, r#"{"schema_version":1,"goal":"bounded ambiguity","included_scope":[],"excluded_scope":[],"acceptance_criteria":[],"constraints":[]}"#).unwrap();
    let goal = goal.to_string_lossy().to_string();
    let mut task_ids = Vec::new();
    for _ in 0..21 {
        let (value, code) = harness
            .cairn_json(
                &[
                    "task",
                    "create",
                    "--project-id",
                    project_id,
                    "--title",
                    "Same task",
                    "--goal-contract",
                    &goal,
                    "--json",
                ],
                None,
            )
            .await;
        assert_eq!(code, 0);
        task_ids.push(
            serde_json::from_value::<TaskCreateResult>(value["data"].clone())
                .unwrap()
                .task
                .task_id
                .to_string(),
        );
    }
    task_ids.sort();
    let ambiguous = harness
        .cairn(
            &[
                "task",
                "show",
                "--task",
                "Same task",
                "--project-id",
                project_id,
            ],
            None,
        )
        .await;
    assert_ambiguity(&ambiguous, &task_ids[..20], true);

    let (unique, _) = harness
        .cairn_json(
            &["project", "create", "--name", "Unique project", "--json"],
            None,
        )
        .await;
    let unique_id = unique["data"]["project"]["project_id"].as_str().unwrap();
    let unique = harness
        .cairn(&["project", "show", "--project", "Unique project"], None)
        .await;
    assert_eq!(unique.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&unique.stdout).contains(unique_id));
    let missing = harness
        .cairn(&["project", "show", "--project", "No such project"], None)
        .await;
    assert_eq!(missing.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&missing.stderr).contains("PROJECT_NOT_FOUND"));
    harness.stop();
}

#[test]
fn every_json_name_selector_is_rejected_before_ipc() {
    let dir = tempfile::TempDir::new().unwrap();
    let goal = dir.path().join("goal.json");
    std::fs::write(&goal, r#"{"schema_version":1,"goal":"unused","included_scope":[],"excluded_scope":[],"acceptance_criteria":[],"constraints":[]}"#).unwrap();
    let goal = goal.to_string_lossy().to_string();
    let id = "018f4e6e-5f2b-7c3e-9a4d-2f6e8b1c9d0a";
    let cases: Vec<Vec<&str>> = vec![
        vec!["project", "show", "--project", "Name", "--json"],
        vec![
            "project",
            "update",
            "--project",
            "Name",
            "--name",
            "Next",
            "--json",
        ],
        vec![
            "project",
            "repository",
            "add",
            "--project",
            "Name",
            "--repository-id",
            "repo",
            "--json",
        ],
        vec![
            "task",
            "show",
            "--task",
            "Name",
            "--project-id",
            id,
            "--json",
        ],
        vec![
            "task",
            "revise",
            "--task",
            "Name",
            "--project-id",
            id,
            "--goal-contract",
            &goal,
            "--json",
        ],
    ];
    for args in cases {
        let output = Command::new(env!("CARGO_BIN_EXE_cairn"))
            .args(args)
            .env("CAIRN_NO_SPAWN", "1")
            .env("CAIRN_SOCKET_PATH", dir.path().join("missing.sock"))
            .env("CAIRN_PIPE_NAME", "cairn-missing-ambiguity-pipe")
            .env("CAIRN_DATA_DIR", dir.path().join("data"))
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2));
        let stdout = String::from_utf8(output.stdout).unwrap();
        assert_eq!(stdout.lines().count(), 1);
        let value: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
        assert_eq!(value["error"]["code"], "USAGE");
    }
}

fn assert_ambiguity(output: &std::process::Output, expected: &[String], truncated: bool) {
    assert_eq!(output.status.code(), Some(4));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    let ids: Vec<_> = stderr
        .lines()
        .map(str::trim)
        .filter(|line| line.len() == 36 && line.chars().filter(|value| *value == '-').count() == 4)
        .collect();
    assert_eq!(ids.len(), 20);
    assert!(ids.windows(2).all(|pair| pair[0] < pair[1]));
    assert_eq!(ids, expected.iter().map(String::as_str).collect::<Vec<_>>());
    assert_eq!(stderr.contains("additional matches omitted"), truncated);
}
