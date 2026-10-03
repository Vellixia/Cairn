use cairn_e2e::{get_json_status_bearer, post_file_status_bearer, post_json_status_bearer, Server};
use serde_json::json;
use uuid::Uuid;

const MIB: usize = 1024 * 1024;
const MAX_EXPORT_BYTES: usize = 32 * MIB;
// Import has one extra MiB for the `import_id`/`bundle` envelope around a
// maximum-sized export. Keep this independent from the server constant: this
// test is the public HTTP contract that detects an accidental route change.
const MAX_IMPORT_REQUEST_BYTES: usize = MAX_EXPORT_BYTES + MIB;

#[test]
fn logical_export_accepts_near_limit_and_refuses_a_record_larger_than_32_mib() {
    let Some(source) = Server::start_with_admin("oversized-export@example.test", "hunter2hunter2")
    else {
        eprintln!("NOT RUN: CAIRN_TEST_DATABASE_URL is required");
        return;
    };
    let token = source.token_for("oversized-export@example.test", "hunter2hunter2");
    let project = Uuid::now_v7();
    let session = Uuid::now_v7();
    let account = source.get_json("/api/auth/me", &token)["id"]
        .as_str()
        .unwrap()
        .to_owned();
    source.execute(&format!(
        "INSERT INTO projects (id, name, repository_remote) VALUES ('{project}', 'transfer-limit', 'github.com/example/transfer-limit')"
    ));
    source.execute(&format!(
        "INSERT INTO sessions (id, project_id, user_id, agent, branch, status, started_at) VALUES ('{session}', '{project}', '{account}', 'codex', 'main', 'active', now())"
    ));
    // Generate content in PostgreSQL so the HTTP response exercises a nearly
    // full bundle without putting a 32 MiB SQL argument on curl.
    let memory = Uuid::now_v7();
    source.execute(&format!(
        "INSERT INTO memories (id, project_id, type, scope, scope_key, content, origin_session_id) VALUES ('{memory}', '{project}', 'fact', 'project', 'transfer-limit', repeat('x', {}), '{session}')",
        MAX_EXPORT_BYTES - 4096,
    ));

    let (bundle, status) =
        get_json_status_bearer(&source.base, "/api/admin/logical-export", &token);
    assert_eq!(status, 200, "near-limit export status");
    let bytes = serde_json::to_vec(&bundle).unwrap().len();
    assert!(
        bytes <= MAX_EXPORT_BYTES,
        "near-limit export exceeded cap: {bytes}"
    );
    assert!(
        bytes >= MAX_EXPORT_BYTES - 8192,
        "fixture is not near cap: {bytes}"
    );

    let destination = Server::start_with_admin("near-import@example.test", "hunter2hunter2")
        .expect("destination database");
    let destination_token = destination.token_for("near-import@example.test", "hunter2hunter2");
    let request = serde_json::to_vec(&json!({
        "import_id": bundle["bundle_id"],
        "bundle": bundle,
    }))
    .unwrap();
    assert!(request.len() <= MAX_IMPORT_REQUEST_BYTES);
    for _ in 0..2 {
        assert_eq!(
            post_file_status_bearer(
                &destination.base,
                "/api/admin/logical-import",
                &request,
                &destination_token,
            ),
            200,
            "near-limit import or idempotent retry failed"
        );
    }
    assert_eq!(destination.count("SELECT count(*) FROM memories"), 1);
    assert_eq!(
        destination.count("SELECT length(content)::bigint FROM memories"),
        (MAX_EXPORT_BYTES - 4096) as i64,
        "near-limit import did not conserve the memory content"
    );

    source.execute(&format!(
        "UPDATE memories SET content = repeat('x', {MAX_EXPORT_BYTES}) WHERE id = '{memory}'"
    ));

    let (body, status) = get_json_status_bearer(&source.base, "/api/admin/logical-export", &token);
    assert_eq!(status, 413, "oversized export: {body}");
    assert_eq!(body["error"]["code"], "logical_export_too_large");
}

#[test]
fn logical_import_accepts_33_mib_request_and_refuses_one_byte_more() {
    let Some(server) = Server::start_with_admin("oversized-import@example.test", "hunter2hunter2")
    else {
        eprintln!("NOT RUN: CAIRN_TEST_DATABASE_URL is required");
        return;
    };
    let token = server.token_for("oversized-import@example.test", "hunter2hunter2");
    let bundle_id = Uuid::now_v7();
    let mut body = serde_json::to_vec(&json!({
        "import_id": bundle_id,
        "bundle": {
            "format": "cairn-logical",
            "version": 1,
            "bundle_id": bundle_id,
            "exported_at": "2026-10-03T00:00:00Z",
            "records": [],
        },
    }))
    .unwrap();
    // JSON permits trailing whitespace, so these requests differ only by one
    // byte at the route's documented boundary and both retain a valid bundle.
    body.resize(MAX_IMPORT_REQUEST_BYTES, b' ');
    assert_eq!(body.len(), MAX_IMPORT_REQUEST_BYTES);
    assert_eq!(
        post_file_status_bearer(&server.base, "/api/admin/logical-import", &body, &token,),
        200,
        "a request at the documented 33 MiB limit must reach import"
    );
    assert_eq!(server.count("SELECT count(*) FROM logical_imports"), 1);

    body.push(b' ');
    assert_eq!(body.len(), MAX_IMPORT_REQUEST_BYTES + 1);
    assert_eq!(
        post_file_status_bearer(&server.base, "/api/admin/logical-import", &body, &token,),
        413,
        "a request one byte beyond the documented limit must be refused"
    );
    assert_eq!(
        server.count("SELECT count(*) FROM logical_imports"),
        1,
        "an oversized request must not reserve an import"
    );
}

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
