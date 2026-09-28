//! Alpha 9's installed edge journey: setup links a canonical remote, two
//! callers remain distinct, and an accepted explicit memory is retrievable.

use cairn_e2e::feature005::Pg;
use cairn_e2e::{post_json_status_bearer, Mcp, Sandbox};
use serde_json::json;
use uuid::Uuid;

#[test]
fn installed_setup_remembers_and_recalls_across_callers() {
    let Some(pg) = Pg::start() else {
        eprintln!("skipped: CAIRN_TEST_DATABASE_URL is not set");
        return;
    };
    let remote = format!(
        "https://github.com/example/alpha9-journey-{}.git",
        Uuid::now_v7()
    );
    let (project, status) = post_json_status_bearer(
        &pg.server.base,
        "/api/projects",
        &json!({
            "name": "alpha9 installed journey",
            "repository_remote": remote,
        }),
        &pg.owner.token,
    );
    assert_eq!(status, 200, "creating the linked project: {project}");

    let sandbox = Sandbox::new();
    sandbox.git(&["remote", "set-url", "origin", &remote]);
    let setup = sandbox.cairn_with_env(
        &["--json", "setup"],
        &[
            ("CAIRN_SERVER_URL", &pg.server.base),
            ("CAIRN_SERVER_TOKEN", &pg.owner.token),
        ],
    );
    assert!(setup.ok(), "setup failed: {}", setup.stderr);
    let setup_json: serde_json::Value = serde_json::from_str(&setup.stdout).expect("setup JSON");
    assert_eq!(setup_json["data"]["project"]["linked"], true);

    for key in ["alpha9-caller-a", "alpha9-caller-b"] {
        let hook = sandbox.hook_as(
            "codex",
            "SessionStart",
            json!({"session_id": key, "source": "startup"}),
        );
        assert_eq!(hook.code, 0, "native hook for {key}: {}", hook.stderr);
    }

    let mut mcp = Mcp::start(&sandbox);
    let accepted = mcp.tool_result(
        "cairn_remember",
        json!({
            "action": "create",
            "agent_session_key": "alpha9-caller-a",
            "type": "fact",
            "topic_key": "alpha9.journey",
            "value_key": "remembered",
            "content": "the alpha9 journey remembers this durable fact",
        }),
        &sandbox.repo_dir().display().to_string(),
    );
    assert_eq!(accepted["isError"], false, "{accepted}");
    assert_eq!(
        accepted["content"][0]["text"]["accepted_for_delivery"],
        true
    );

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while pg.server.count("SELECT count(*) FROM memories") != 1 {
        assert!(
            std::time::Instant::now() < deadline,
            "the accepted command never reached the server"
        );
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    let recalled = mcp.tool(
        "cairn_search",
        json!({
            "action": "search",
            "agent_session_key": "alpha9-caller-b",
            "query": "alpha9 journey remembered durable fact",
        }),
        &sandbox.repo_dir().display().to_string(),
    );
    assert!(recalled.contains("the alpha9 journey remembers this durable fact"));
    assert_eq!(pg.server.count("SELECT count(*) FROM memories"), 1);
}
