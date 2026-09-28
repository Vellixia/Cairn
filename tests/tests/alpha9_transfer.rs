use cairn_e2e::{get_json_status_bearer, post_json_status_bearer, Server};
use serde_json::json;
use uuid::Uuid;

#[test]
fn snapshot_preserves_pending_work_and_import_is_idempotent() {
    let Some(source) = Server::start_with_admin("source-transfer@example.test", "hunter2hunter2")
    else {
        eprintln!("NOT RUN: CAIRN_TEST_DATABASE_URL is required");
        return;
    };
    let source_token = source.token_for("source-transfer@example.test", "hunter2hunter2");
    let project = Uuid::now_v7();
    let session = Uuid::now_v7();
    let event = cairn_core::eventid::event_id(session, 1);
    let actor = source.count("SELECT count(*) FROM users");
    assert_eq!(actor, 1);
    let account = source.get_json("/api/auth/me", &source_token)["id"]
        .as_str()
        .unwrap()
        .to_owned();
    source.execute(&format!("INSERT INTO projects (id, name, repository_remote) VALUES ('{project}', 'transfer', 'github.com/example/transfer')"));
    source.execute(&format!(
        "INSERT INTO project_members (project_id, user_id) VALUES ('{project}', '{account}')"
    ));
    source.execute(&format!("INSERT INTO sessions (id, project_id, user_id, agent, branch, status, started_at) VALUES ('{session}', '{project}', '{account}', 'codex', 'main', 'active', now())"));
    source.execute(&format!("INSERT INTO safe_events (event_id, project_id, session_id, account_id, agent, kind, session_seq, contract_version, content, occurred_at) VALUES ('{event}', '{project}', '{session}', '{account}', 'codex', 'file_changed', 1, 1, '{{\"File\":{{\"repo_file\":\"src/main.rs\",\"repo_file_from\":null,\"change_kind\":\"modified\",\"file_identity\":\"present\"}}}}', now())"));
    source.execute(&format!("INSERT INTO consolidation_session (project_id, session_id, state, oldest_enqueued_at) VALUES ('{project}', '{session}', 'claimed', now())"));
    source.execute(&format!("INSERT INTO consolidation_work (event_id, project_id, session_id, session_seq, state, attempts) VALUES ('{event}', '{project}', '{session}', 1, 'pending', 4)"));
    let (bundle, status) =
        get_json_status_bearer(&source.base, "/api/admin/logical-export", &source_token);
    assert_eq!(status, 200, "export: {bundle}");
    assert!(bundle["bundle_id"]
        .as_str()
        .unwrap()
        .parse::<Uuid>()
        .is_ok());
    assert!(serde_json::to_vec(&bundle).unwrap().len() <= 32 * 1024 * 1024);
    let Some(destination) =
        Server::start_with_admin("destination-transfer@example.test", "hunter2hunter2")
    else {
        panic!("destination database unavailable")
    };
    let destination_token =
        destination.token_for("destination-transfer@example.test", "hunter2hunter2");
    let (readiness, ready_status) =
        get_json_status_bearer(&destination.base, "/api/readiness", &destination_token);
    assert_eq!(
        ready_status, 503,
        "a required worker is disabled: {readiness}"
    );
    assert_eq!(readiness["worker"]["available"], false);
    assert_eq!(readiness["worker"]["required"], true);
    let body = json!({"import_id": bundle["bundle_id"], "bundle": bundle});
    let (receipt, status) = post_json_status_bearer(
        &destination.base,
        "/api/admin/logical-import",
        &body,
        &destination_token,
    );
    assert_eq!(status, 200, "import: {receipt}");
    assert_eq!(receipt["rejected"], 0, "{receipt}");
    assert_eq!(destination.count("SELECT count(*) FROM safe_events"), 1);
    assert_eq!(
        destination
            .count("SELECT count(*) FROM consolidation_work WHERE state='pending' AND attempts=0"),
        1
    );
    assert_eq!(destination.count("SELECT count(*) FROM consolidation_session WHERE state='pending' AND claimed_by IS NULL"), 1);
    let (retried, status) = post_json_status_bearer(
        &destination.base,
        "/api/admin/logical-import",
        &body,
        &destination_token,
    );
    assert_eq!(status, 200, "retry: {retried}");
    assert_eq!(retried, receipt);
    assert_eq!(destination.count("SELECT count(*) FROM safe_events"), 1);
    // The shared test harness uses a four-connection pool, below the worker's
    // five-connection minimum. The imported row must stay claimable, not run.
    assert_eq!(
        destination
            .count("SELECT count(*) FROM consolidation_work WHERE state='pending' AND attempts=0"),
        1
    );
}
