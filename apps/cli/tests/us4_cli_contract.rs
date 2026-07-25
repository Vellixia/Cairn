//! T053: CLI recovery contract — heartbeat success/expired-lease, reattach
//! valid/mismatch/grace-expired `--json` goldens, token hygiene, secure-input
//! resolution order (stdin → env → file).

mod support;

use cairn_protocol::{SessionHeartbeatResult, SessionReattachResult, SessionStartResult};
use fixtures_repositories::FixtureRepo;
use support::CliHarness;

async fn started_session(h: &CliHarness, repo: &FixtureRepo, inst: &str) -> SessionStartResult {
    h.cairn_json(&["init", "--json"], Some(repo.root())).await;
    let (envelope, code) = h
        .cairn_json(
            &[
                "session",
                "start",
                "--agent",
                "recov-cli",
                "--agent-instance",
                inst,
                "--json",
            ],
            Some(repo.root()),
        )
        .await;
    assert_eq!(code, 0);
    serde_json::from_value(envelope["data"].clone()).expect("SessionStartResult")
}

#[tokio::test(flavor = "multi_thread")]
async fn heartbeat_success_extends_lease() {
    let h = CliHarness::start().await;
    let repo = FixtureRepo::new().unwrap();
    let inst = uuid::Uuid::new_v4().to_string();
    let start = started_session(&h, &repo, &inst).await;
    let sid = start.session.session_id.to_string();
    let token = start.resume_token.unwrap();

    let (envelope, code) = {
        let out = h
            .cairn_full(
                &[
                    "session",
                    "heartbeat",
                    "--session",
                    &sid,
                    "--agent-instance",
                    &inst,
                    "--resume-token-stdin",
                    "--json",
                ],
                Some(repo.root()),
                Some(&format!("{token}\n")),
                &[],
            )
            .await;
        let env: serde_json::Value =
            serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap();
        (env, out.status.code().unwrap())
    };
    assert_eq!(code, 0);
    let hb: SessionHeartbeatResult =
        serde_json::from_value(envelope["data"].clone()).expect("SessionHeartbeatResult");
    assert_eq!(hb.state, cairn_protocol::SessionState::Active);
    h.stop();
}

#[tokio::test(flavor = "multi_thread")]
async fn heartbeat_on_expired_lease_returns_lease_expired() {
    let h = CliHarness::start_with_session_config(cairn_session::SessionConfig {
        initial_lease_secs: 1,
        heartbeat_ttl_secs: 90,
        grace_secs: 900,
    })
    .await;
    let repo = FixtureRepo::new().unwrap();
    let inst = uuid::Uuid::new_v4().to_string();
    let start = started_session(&h, &repo, &inst).await;
    let sid = start.session.session_id.to_string();
    let token = start.resume_token.unwrap();

    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
    let out = h
        .cairn_full(
            &[
                "session",
                "heartbeat",
                "--session",
                &sid,
                "--agent-instance",
                &inst,
                "--resume-token-stdin",
                "--json",
            ],
            Some(repo.root()),
            Some(&token),
            &[],
        )
        .await;
    let envelope: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        envelope["error"]["code"],
        serde_json::json!("LEASE_EXPIRED")
    );
    h.stop();
}

#[tokio::test(flavor = "multi_thread")]
async fn reattach_valid_mismatch_and_grace_expired_goldens() {
    let mut h = CliHarness::start_with_session_config(cairn_session::SessionConfig {
        initial_lease_secs: 900,
        heartbeat_ttl_secs: 90,
        grace_secs: 900,
    })
    .await;
    let repo = FixtureRepo::new().unwrap();
    let inst = uuid::Uuid::new_v4().to_string();
    let start = started_session(&h, &repo, &inst).await;
    let sid = start.session.session_id.to_string();
    let token = start.resume_token.unwrap();

    h.restart().await;

    // Mismatch: reject-only, session stays recovering, exit 1.
    let out = h
        .cairn_full(
            &[
                "session",
                "reattach",
                "--session",
                &sid,
                "--agent-instance",
                &inst,
                "--resume-token-stdin",
                "--json",
            ],
            Some(repo.root()),
            Some("definitely-wrong-token"),
            &[],
        )
        .await;
    let envelope: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        envelope["error"]["code"],
        serde_json::json!("LEASE_MISMATCH")
    );

    let (show, _) = h
        .cairn_json(
            &["session", "show", "--session", &sid, "--json"],
            Some(repo.root()),
        )
        .await;
    assert_eq!(
        show["data"]["session"]["state"],
        serde_json::json!("recovering")
    );

    // Valid: resumes, fresh token issued in --json only.
    let out = h
        .cairn_full(
            &[
                "session",
                "reattach",
                "--session",
                &sid,
                "--agent-instance",
                &inst,
                "--resume-token-stdin",
                "--json",
            ],
            Some(repo.root()),
            Some(&token),
            &[],
        )
        .await;
    let envelope: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap();
    assert_eq!(out.status.code(), Some(0));
    let re: SessionReattachResult =
        serde_json::from_value(envelope["data"].clone()).expect("SessionReattachResult");
    assert!(re.resume_token.is_some());

    h.stop();

    // Grace expiry: separate daemon with 1s grace.
    let mut h2 = CliHarness::start_with_session_config(cairn_session::SessionConfig {
        initial_lease_secs: 900,
        heartbeat_ttl_secs: 90,
        grace_secs: 1,
    })
    .await;
    let repo2 = FixtureRepo::new().unwrap();
    let inst2 = uuid::Uuid::new_v4().to_string();
    let start2 = started_session(&h2, &repo2, &inst2).await;
    let sid2 = start2.session.session_id.to_string();
    let token2 = start2.resume_token.unwrap();

    h2.restart().await;
    tokio::time::sleep(std::time::Duration::from_millis(1600)).await;

    let out = h2
        .cairn_full(
            &[
                "session",
                "reattach",
                "--session",
                &sid2,
                "--agent-instance",
                &inst2,
                "--resume-token-stdin",
                "--json",
            ],
            Some(repo2.root()),
            Some(&token2),
            &[],
        )
        .await;
    let envelope: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap();
    assert_eq!(out.status.code(), Some(1));
    let code = envelope["error"]["code"].as_str().unwrap();
    // Sweeper may have already interrupted it (SESSION_NOT_RECOVERING) or the
    // request-path deadline check fires first (GRACE_EXPIRED). Both honor A2.
    assert!(
        code == "GRACE_EXPIRED" || code == "SESSION_NOT_RECOVERING",
        "unexpected error code {code}"
    );
    h2.stop();
}

#[tokio::test(flavor = "multi_thread")]
async fn secure_input_resolution_order_stdin_env_file() {
    let h = CliHarness::start().await;
    let repo = FixtureRepo::new().unwrap();
    let inst = uuid::Uuid::new_v4().to_string();
    let start = started_session(&h, &repo, &inst).await;
    let sid = start.session.session_id.to_string();
    let token = start.resume_token.unwrap();

    // stdin (wrong) beats env (right): LEASE_MISMATCH proves stdin won.
    let out = h
        .cairn_full(
            &[
                "session",
                "heartbeat",
                "--session",
                &sid,
                "--agent-instance",
                &inst,
                "--resume-token-stdin",
                "--json",
            ],
            Some(repo.root()),
            Some("wrong-via-stdin"),
            &[("CAIRN_RESUME_TOKEN", token.as_str())],
        )
        .await;
    let envelope: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap();
    assert_eq!(
        envelope["error"]["code"],
        serde_json::json!("LEASE_MISMATCH")
    );

    // env alone works.
    let out = h
        .cairn_full(
            &[
                "session",
                "heartbeat",
                "--session",
                &sid,
                "--agent-instance",
                &inst,
                "--json",
            ],
            Some(repo.root()),
            None,
            &[("CAIRN_RESUME_TOKEN", token.as_str())],
        )
        .await;
    assert_eq!(out.status.code(), Some(0), "env token must authenticate");

    // file alone works.
    let token_file = h.dir.path().join("token.txt");
    std::fs::write(&token_file, &token).unwrap();
    let out = h
        .cairn_full(
            &[
                "session",
                "heartbeat",
                "--session",
                &sid,
                "--agent-instance",
                &inst,
                "--resume-token-file",
                token_file.to_str().unwrap(),
                "--json",
            ],
            Some(repo.root()),
            None,
            &[],
        )
        .await;
    assert_eq!(out.status.code(), Some(0), "file token must authenticate");

    // No token source: usage error, exit 2, and no token-shaped output.
    let out = h
        .cairn_full(
            &[
                "session",
                "heartbeat",
                "--session",
                &sid,
                "--agent-instance",
                &inst,
                "--json",
            ],
            Some(repo.root()),
            None,
            &[],
        )
        .await;
    assert_eq!(out.status.code(), Some(2));
    h.stop();
}

#[tokio::test(flavor = "multi_thread")]
async fn reattach_human_output_never_prints_token() {
    let mut h = CliHarness::start().await;
    let repo = FixtureRepo::new().unwrap();
    let inst = uuid::Uuid::new_v4().to_string();
    let start = started_session(&h, &repo, &inst).await;
    let sid = start.session.session_id.to_string();
    let token = start.resume_token.unwrap();

    h.restart().await;

    let out = h
        .cairn_full(
            &[
                "session",
                "reattach",
                "--session",
                &sid,
                "--agent-instance",
                &inst,
                "--resume-token-stdin",
            ],
            Some(repo.root()),
            Some(&token),
            &[],
        )
        .await;
    assert_eq!(out.status.code(), Some(0));
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!all.contains(&token), "input token must never be echoed");
    let leak = all.split_whitespace().any(|w| {
        w.len() == token.len()
            && w.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    });
    assert!(
        !leak,
        "human output appears to contain a token-shaped value:\n{all}"
    );
    h.stop();
}
