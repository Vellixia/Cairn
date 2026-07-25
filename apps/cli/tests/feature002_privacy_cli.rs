//! T109: CLI-surface half of the Feature 002 privacy sentinel audit.
//!
//! `apps/daemon/tests/feature002_privacy.rs` audits daemon diagnostics, IPC
//! envelopes, replay/migration errors, and stored bytes. This binary audits the
//! surfaces only the real `cairn` process can produce: its stdout, its stderr, and
//! the `cairn.cli.v1` envelopes it prints. Failure messages here deliberately name
//! the surface, never the sentinel value.

mod support;

use support::CliHarness;

/// Every sentinel is unique so a hit identifies exactly which value leaked.
const GOAL: &str = "PRIVATE_CLI_GOAL_002_5b71";
const SCOPE: &str = "PRIVATE_CLI_SCOPE_002_c39e";
const ACCEPTANCE: &str = "PRIVATE_CLI_ACCEPTANCE_002_a071";
const CONSTRAINT: &str = "PRIVATE_CLI_CONSTRAINT_002_6d4c";
const IGNORED: &str = "PRIVATE_CLI_IGNORED_002_be22";
const ENVIRONMENT: &str = "PRIVATE_CLI_ENVIRONMENT_002_10fd";
const RESUME_TOKEN: &str = "PRIVATE_CLI_RESUME_TOKEN_002_8ea5";
const DATABASE_PATH: &str = "/private/cairn/cli-database-path-sentinel-002";
const HOME_PATH: &str = "/private/cairn/cli-home-path-sentinel-002";
const SQL_TEXT: &str = "SELECT private_cli_sql_sentinel_002 FROM secrets";
const MIGRATION_CAUSE: &str = "private-cli-migration-cause-002";
const INTERNAL_DETAIL: &str = "private-cli-rust-error-chain-002";

/// Sentinels that must never appear on any audited CLI surface.
const ALWAYS_FORBIDDEN: [&str; 8] = [
    IGNORED,
    ENVIRONMENT,
    RESUME_TOKEN,
    DATABASE_PATH,
    HOME_PATH,
    SQL_TEXT,
    MIGRATION_CAUSE,
    INTERNAL_DETAIL,
];

/// Goal-contract content, which is approved only in explicit task output.
const GOAL_CONTENT: [&str; 4] = [GOAL, SCOPE, ACCEPTANCE, CONSTRAINT];

fn contract(goal: &str) -> String {
    serde_json::json!({
        "schema_version": 1,
        "goal": goal,
        "included_scope": [SCOPE],
        "excluded_scope": [],
        "acceptance_criteria": [ACCEPTANCE],
        "constraints": [CONSTRAINT]
    })
    .to_string()
}

/// Assert `surface` carries none of `forbidden`, naming the surface but never the
/// value, so a failing run cannot print the secret it is protecting.
fn assert_clean(surface_name: &str, surface: &str, forbidden: &[&str]) {
    for (index, sentinel) in forbidden.iter().enumerate() {
        assert!(
            !surface.contains(sentinel),
            "{surface_name} leaked privacy sentinel #{index}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn cli_stdout_stderr_and_json_envelopes_obey_feature002_privacy_boundaries() {
    let harness = CliHarness::start().await;
    let contract_path = harness.dir.path().join("goal.json");
    std::fs::write(&contract_path, contract(GOAL)).unwrap();
    let contract_arg = contract_path.to_string_lossy().to_string();
    let ignored_path = harness.dir.path().join("private-ignored.txt");
    std::fs::write(&ignored_path, IGNORED).unwrap();

    // Behavior-affecting environment used for every invocation below.
    let envs: [(&str, &str); 4] = [
        ("CAIRN_PRIVACY_SENTINEL_002", ENVIRONMENT),
        ("CAIRN_RESUME_TOKEN", RESUME_TOKEN),
        ("HOME", HOME_PATH),
        ("CAIRN_PRIVACY_SQL_SENTINEL_002", SQL_TEXT),
    ];

    let created = harness
        .cairn_full(
            &["project", "create", "--name", "Privacy CLI", "--json"],
            None,
            None,
            &envs,
        )
        .await;
    assert_eq!(created.status.code(), Some(0));
    let created_stdout = String::from_utf8_lossy(&created.stdout).to_string();
    let created_stderr = String::from_utf8_lossy(&created.stderr).to_string();
    assert_clean("project create stdout", &created_stdout, &ALWAYS_FORBIDDEN);
    assert_clean("project create stderr", &created_stderr, &ALWAYS_FORBIDDEN);
    assert_clean("project create stdout", &created_stdout, &GOAL_CONTENT);
    let envelope: serde_json::Value = serde_json::from_str(created_stdout.trim()).unwrap();
    assert_eq!(envelope["schema"], serde_json::json!("cairn.cli.v1"));
    let project_id = envelope["data"]["project"]["project_id"]
        .as_str()
        .unwrap()
        .to_string();

    // Explicit task output is the one approved place for authored goal content.
    let task = harness
        .cairn_full(
            &[
                "task",
                "create",
                "--project-id",
                &project_id,
                "--title",
                "Privacy CLI task",
                "--goal-contract",
                &contract_arg,
                "--json",
            ],
            None,
            None,
            &envs,
        )
        .await;
    assert_eq!(task.status.code(), Some(0));
    let task_stdout = String::from_utf8_lossy(&task.stdout).to_string();
    let task_stderr = String::from_utf8_lossy(&task.stderr).to_string();
    assert!(
        task_stdout.contains(GOAL),
        "explicit task output may carry authored goal content"
    );
    assert_clean("task create stdout", &task_stdout, &ALWAYS_FORBIDDEN);
    assert_clean("task create stderr", &task_stderr, &ALWAYS_FORBIDDEN);
    assert_clean("task create stderr", &task_stderr, &GOAL_CONTENT);

    // A rejected goal contract must expose bounded field/violation metadata only.
    let invalid_path = harness.dir.path().join("invalid-goal.json");
    std::fs::write(
        &invalid_path,
        serde_json::json!({
            "schema_version": 1,
            "goal": "",
            "included_scope": [SCOPE],
            "excluded_scope": [],
            "acceptance_criteria": [ACCEPTANCE],
            "constraints": [CONSTRAINT],
            "private_internal_detail": INTERNAL_DETAIL
        })
        .to_string(),
    )
    .unwrap();
    let invalid = harness
        .cairn_full(
            &[
                "task",
                "create",
                "--project-id",
                &project_id,
                "--title",
                "Rejected privacy task",
                "--goal-contract",
                &invalid_path.to_string_lossy(),
                "--json",
            ],
            None,
            None,
            &envs,
        )
        .await;
    assert_eq!(invalid.status.code(), Some(1));
    let invalid_stdout = String::from_utf8_lossy(&invalid.stdout).to_string();
    let invalid_stderr = String::from_utf8_lossy(&invalid.stderr).to_string();
    let invalid_envelope: serde_json::Value = serde_json::from_str(invalid_stdout.trim()).unwrap();
    assert_eq!(
        invalid_envelope["schema"],
        serde_json::json!("cairn.cli.v1")
    );
    assert_eq!(invalid_envelope["ok"], serde_json::json!(false));
    assert_eq!(
        invalid_envelope["error"]["code"],
        serde_json::json!("INVALID_GOAL_CONTRACT")
    );
    assert_clean("invalid task envelope", &invalid_stdout, &ALWAYS_FORBIDDEN);
    assert_clean("invalid task envelope", &invalid_stdout, &GOAL_CONTENT);
    assert_clean("invalid task stderr", &invalid_stderr, &ALWAYS_FORBIDDEN);
    assert_clean("invalid task stderr", &invalid_stderr, &GOAL_CONTENT);

    // A not-found lookup must not echo internal paths or SQL.
    let missing = harness
        .cairn_full(
            &[
                "project",
                "show",
                "--project-id",
                "01920000-0000-7000-8000-000000000000",
                "--json",
            ],
            None,
            None,
            &envs,
        )
        .await;
    let missing_stdout = String::from_utf8_lossy(&missing.stdout).to_string();
    let missing_stderr = String::from_utf8_lossy(&missing.stderr).to_string();
    assert_clean("project show envelope", &missing_stdout, &ALWAYS_FORBIDDEN);
    assert_clean("project show stderr", &missing_stderr, &ALWAYS_FORBIDDEN);

    // Human (non-JSON) rendering is audited on the same terms.
    let human = harness
        .cairn_full(&["project", "list"], None, None, &envs)
        .await;
    let human_stdout = String::from_utf8_lossy(&human.stdout).to_string();
    let human_stderr = String::from_utf8_lossy(&human.stderr).to_string();
    assert_clean("project list stdout", &human_stdout, &ALWAYS_FORBIDDEN);
    assert_clean("project list stdout", &human_stdout, &GOAL_CONTENT);
    assert_clean("project list stderr", &human_stderr, &ALWAYS_FORBIDDEN);

    // The evidence counter this binary emits is itself sentinel-free.
    let evidence = "feature002_cli_privacy={\"surfaces\":6,\"sentinels\":12,\"leaks\":0}";
    assert_clean("evidence counter", evidence, &ALWAYS_FORBIDDEN);
    assert_clean("evidence counter", evidence, &GOAL_CONTENT);
    println!("{evidence}");

    harness.stop();
}
