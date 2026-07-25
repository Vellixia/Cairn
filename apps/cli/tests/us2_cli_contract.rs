//! T041: US2 CLI contract — session start/show/stop `--json` validate against
//! DTOs; exit 4 on ambiguity; tokens absent from human output.

mod support;

use cairn_protocol::{
    CliEnvelope, ErrorCode, ErrorData, SessionGetResult, SessionStartResult, SessionStopResult,
};
use fixtures_repositories::FixtureRepo;
use support::CliHarness;

fn watcher_golden(name: &str) -> serde_json::Value {
    let text = match name {
        "watcher-start-failed-install.json" => {
            include_str!("goldens/watcher-start-failed-install.json")
        }
        "watcher-start-failed-reconcile.json" => {
            include_str!("goldens/watcher-start-failed-reconcile.json")
        }
        _ => panic!("unknown watcher golden"),
    };
    serde_json::from_str(text).expect("valid CLI golden")
}

#[tokio::test(flavor = "multi_thread")]
async fn session_start_show_stop_json_contract() {
    let h = CliHarness::start().await;
    let repo = FixtureRepo::new().unwrap();
    h.cairn_json(&["init", "--json"], Some(repo.root())).await;

    let inst = uuid::Uuid::new_v4().to_string();
    let (start_env, code) = h
        .cairn_json(
            &[
                "session",
                "start",
                "--agent",
                "cli-test",
                "--agent-instance",
                &inst,
                "--json",
            ],
            Some(repo.root()),
        )
        .await;
    assert_eq!(code, 0);
    let start: SessionStartResult =
        serde_json::from_value(start_env["data"].clone()).expect("SessionStartResult DTO");
    assert!(start.resume_token.is_some());

    let (show_env, code) = h
        .cairn_json(
            &["session", "show", "--agent-instance", &inst, "--json"],
            Some(repo.root()),
        )
        .await;
    assert_eq!(code, 0);
    let show: SessionGetResult =
        serde_json::from_value(show_env["data"].clone()).expect("SessionGetResult DTO");
    assert_eq!(show.session.unwrap().session_id, start.session.session_id);

    let (stop_env, code) = h
        .cairn_json(
            &["session", "stop", "--agent-instance", &inst, "--json"],
            Some(repo.root()),
        )
        .await;
    assert_eq!(code, 0);
    let stop: SessionStopResult =
        serde_json::from_value(stop_env["data"].clone()).expect("SessionStopResult DTO");
    assert_eq!(stop.session.session_id, start.session.session_id);
    h.stop();
}

#[tokio::test(flavor = "multi_thread")]
async fn ambiguous_show_exits_4_with_candidates() {
    let h = CliHarness::start().await;
    let repo = FixtureRepo::new().unwrap();
    h.cairn_json(&["init", "--json"], Some(repo.root())).await;

    let a = uuid::Uuid::new_v4().to_string();
    let b = uuid::Uuid::new_v4().to_string();
    h.cairn_json(
        &[
            "session",
            "start",
            "--agent",
            "x",
            "--agent-instance",
            &a,
            "--json",
        ],
        Some(repo.root()),
    )
    .await;
    h.cairn_json(
        &[
            "session",
            "start",
            "--agent",
            "y",
            "--agent-instance",
            &b,
            "--json",
        ],
        Some(repo.root()),
    )
    .await;

    let (envelope, code) = h
        .cairn_json(&["session", "show", "--json"], Some(repo.root()))
        .await;
    assert_eq!(code, 4, "ambiguous resolution must exit 4");
    let data: SessionGetResult = serde_json::from_value(envelope["data"].clone()).unwrap();
    assert_eq!(data.candidates.unwrap().len(), 2);
    h.stop();
}

#[tokio::test(flavor = "multi_thread")]
async fn human_output_never_contains_resume_token() {
    let h = CliHarness::start().await;
    let repo = FixtureRepo::new().unwrap();
    h.cairn_json(&["init", "--json"], Some(repo.root())).await;

    let inst = uuid::Uuid::new_v4().to_string();
    // Capture the real token via JSON first (separate instance).
    let (json_env, _) = h
        .cairn_json(
            &[
                "session",
                "start",
                "--agent",
                "tok",
                "--agent-instance",
                &inst,
                "--json",
            ],
            Some(repo.root()),
        )
        .await;
    let token = json_env["data"]["resume_token"]
        .as_str()
        .unwrap()
        .to_string();
    h.cairn_json(
        &["session", "stop", "--agent-instance", &inst, "--json"],
        Some(repo.root()),
    )
    .await;

    // Human-mode start for a fresh instance: stdout+stderr must not leak the
    // (new) token — assert no base64url blob of token length appears at all.
    let inst2 = uuid::Uuid::new_v4().to_string();
    let out = h
        .cairn(
            &[
                "session",
                "start",
                "--agent",
                "tok",
                "--agent-instance",
                &inst2,
            ],
            Some(repo.root()),
        )
        .await;
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!all.contains(&token), "old token must never appear");
    let leak = all.split_whitespace().any(|w| {
        w.len() == token.len()
            && w.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    });
    assert!(
        !leak,
        "human output appears to contain a token-shaped value:\n{all}"
    );
    assert!(all.contains("resume token issued"), "human hint expected");
    h.stop();
}

async fn assert_watcher_failure_cli_golden(stage: &str) {
    let h = CliHarness::start().await;
    let repo = FixtureRepo::new().unwrap();
    h.cairn_json(&["init", "--json"], Some(repo.root())).await;
    let controls = h
        .config
        .watcher_test_controls
        .as_ref()
        .expect("watcher test controls");
    match stage {
        "install" => controls.force_install_failure(),
        "reconcile" => controls.force_reconcile_failure(),
        _ => panic!("unsupported watcher stage"),
    }

    let instance = uuid::Uuid::new_v4().to_string();
    let (envelope, code) = h
        .cairn_json(
            &[
                "session",
                "start",
                "--agent",
                "watcher-contract",
                "--agent-instance",
                &instance,
                "--json",
            ],
            Some(repo.root()),
        )
        .await;
    assert_eq!(code, 1, "both watcher failure stages map to exit 1");
    let typed: CliEnvelope = serde_json::from_value(envelope.clone()).unwrap();
    let error = typed.error.expect("typed CLI error envelope");
    assert_eq!(error.code, ErrorCode::WatcherStartFailed);
    assert_eq!(
        error.data.map(ErrorData::watcher_stage).map(|s| s.as_str()),
        Some(stage)
    );
    assert_eq!(
        envelope,
        watcher_golden(&format!("watcher-start-failed-{stage}.json"))
    );

    let serialized = serde_json::to_string(&envelope).unwrap();
    assert!(!serialized.contains(&repo.root().to_string_lossy().to_string()));
    assert!(!serialized.contains("deterministic test injection"));
    assert!(!serialized.contains("resume_token"));
    h.stop();
}

#[tokio::test(flavor = "multi_thread")]
async fn watcher_install_failure_cli_json_golden_exits_one() {
    assert_watcher_failure_cli_golden("install").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn watcher_reconcile_failure_cli_json_golden_exits_one() {
    assert_watcher_failure_cli_golden("reconcile").await;
}
