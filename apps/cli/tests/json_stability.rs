//! T057 (SC-008): a scripted consumer parses every provided command's
//! machine-readable output — all eight commands, success AND failure paths,
//! across repeated runs — with zero format ambiguity.

mod support;

use fixtures_repositories::FixtureRepo;
use support::CliHarness;

fn assert_envelope(envelope: &serde_json::Value) {
    assert_eq!(envelope["schema"], serde_json::json!("cairn.cli.v1"));
    let ok = envelope["ok"].as_bool().expect("ok is boolean");
    assert!(envelope["command"].is_string());
    if ok {
        assert!(envelope.get("data").is_some(), "ok envelope carries data");
        assert!(envelope.get("error").is_none());
    } else {
        let err = envelope
            .get("error")
            .expect("failure envelope carries error");
        assert!(err["code"].is_string());
        assert!(err["message"].is_string());
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn all_eight_commands_parse_stably_across_repeated_runs() {
    let h = CliHarness::start().await;
    let repo = FixtureRepo::new().unwrap();
    let non_git = tempfile::TempDir::new().unwrap();
    let inst = uuid::Uuid::new_v4().to_string();

    for round in 0..3 {
        // 1. init — success (idempotent) and failure (non-git).
        let (env1, _) = h.cairn_json(&["init", "--json"], Some(repo.root())).await;
        assert_envelope(&env1);
        let (env1f, code) = h
            .cairn_json(&["init", "--json"], Some(non_git.path()))
            .await;
        assert_envelope(&env1f);
        assert_eq!(code, 3);

        // 2. status — success and failure.
        let (env2, _) = h.cairn_json(&["status", "--json"], Some(repo.root())).await;
        assert_envelope(&env2);
        let (env2f, _) = h
            .cairn_json(&["status", "--json"], Some(non_git.path()))
            .await;
        assert_envelope(&env2f);

        // 3. session start — success.
        let (env3, _) = h
            .cairn_json(
                &[
                    "session",
                    "start",
                    "--agent",
                    "stab",
                    "--agent-instance",
                    &inst,
                    "--json",
                ],
                Some(repo.root()),
            )
            .await;
        assert_envelope(&env3);
        let sid = env3["data"]["session"]["session_id"]
            .as_str()
            .unwrap()
            .to_string();
        let token = env3["data"]["resume_token"]
            .as_str()
            .map(str::to_string)
            .unwrap_or_default();

        // 4. session show — success; failure (unknown session).
        let (env4, _) = h
            .cairn_json(
                &["session", "show", "--agent-instance", &inst, "--json"],
                Some(repo.root()),
            )
            .await;
        assert_envelope(&env4);
        let bogus = uuid::Uuid::new_v4().to_string();
        let (env4f, _) = h
            .cairn_json(
                &["session", "show", "--session", &bogus, "--json"],
                Some(repo.root()),
            )
            .await;
        assert_envelope(&env4f);

        // 5. session heartbeat — success (round 0 issues the token) + failure.
        if !token.is_empty() {
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
            let env5: serde_json::Value =
                serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap();
            assert_envelope(&env5);
        }
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
                Some("wrong"),
                &[],
            )
            .await;
        let env5f: serde_json::Value =
            serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap();
        assert_envelope(&env5f);

        // 6. session reattach — failure path (session is active, not recovering).
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
                Some("irrelevant"),
                &[],
            )
            .await;
        let env6f: serde_json::Value =
            serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap();
        assert_envelope(&env6f);

        // 7. session stop — success; failure (already stopped).
        let (env7, _) = h
            .cairn_json(
                &["session", "stop", "--agent-instance", &inst, "--json"],
                Some(repo.root()),
            )
            .await;
        assert_envelope(&env7);
        let (env7f, _) = h
            .cairn_json(
                &["session", "stop", "--session", &sid, "--json"],
                Some(repo.root()),
            )
            .await;
        assert_envelope(&env7f);

        // 8. daemon status — success.
        let (env8, _) = h.cairn_json(&["daemon", "status", "--json"], None).await;
        assert_envelope(&env8);

        let _ = round;
    }
    h.stop();
}
