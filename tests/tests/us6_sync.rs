//! T065 — opt-in sharing, idempotent sync, membership, and the observation
//! boundary (SC-009, SC-010, FR-055, FR-056, FR-064).

use cairn_e2e::{attach_server, post_json_bearer, Sandbox, Server};
use serde_json::json;

fn server() -> Option<Server> {
    match Server::start() {
        Some(s) => Some(s),
        None => {
            eprintln!(
                "SKIPPED: set CAIRN_TEST_DATABASE_URL (e.g. `docker compose up -d postgres`) \
                 to run the server suite"
            );
            None
        }
    }
}

fn seed_local_work(s: &Sandbox) {
    let task = s.json(&[
        "task",
        "new",
        "--title",
        "Share me",
        "--goal",
        "shared goal",
    ]);
    let task_id = task["task"]["id"].as_str().unwrap().to_string();
    s.hook(
        "SessionStart",
        json!({ "session_id": "sync", "source": "startup" }),
    );
    s.must(&["session", "start", "--key", "sync", "--task", &task_id]);
    s.write_file("shared.rs", "pub fn shared() {}\n");
    s.hook(
        "PostToolUse",
        json!({ "session_id": "sync", "tool_name": "Edit",
                "tool_input": { "file_path": "secret-local-path.rs" } }),
    );
    s.must(&[
        "memory",
        "add",
        "--type",
        "convention",
        "--scope",
        "project",
        "shared convention worth keeping",
    ]);
    s.hook(
        "SessionEnd",
        json!({ "session_id": "sync", "reason": "clear" }),
    );
    // The close is sealed and its handoff is produced just after the hook
    // returns (FR-240, D22). A fixture that queued the sync before the handoff
    // existed would be testing the race, not the sync.
    s.settle("the closed session's handoff", |s| {
        s.cairn(&["--json", "status"])
            .stdout
            .contains("\"sessions_awaiting_handoff\": 0")
    });

    // The fixture is only a fixture once its state exists. Asserting it here
    // means a seed that failed says so at the seed, instead of reappearing
    // later as "the memory did not reach the server" — a claim about syncing
    // that would in fact be about a memory nobody ever wrote.
    let seeded =
        s.query_column("SELECT id FROM memories WHERE content = 'shared convention worth keeping'");
    assert_eq!(
        seeded.len(),
        1,
        "seed_local_work did not leave exactly one seeded memory: {seeded:?}"
    );
}

#[test]
fn linking_syncs_history_and_replaying_a_batch_changes_nothing() {
    let Some(server) = server() else { return };
    let s = Sandbox::new();
    let token = server.new_user_token("owner");
    attach_server(&s, &server, &token);
    seed_local_work(&s);

    let linked = s.json(&["link", "--create"]);
    assert_eq!(linked["linked"], true);
    let project_id = linked["server_project_id"].as_str().unwrap().to_string();

    // Either the manual call or the background worker may do the delivering;
    // what matters is that the queue empties and nothing is rejected.
    let first = s.json(&["sync", "now"]);
    assert_eq!(first["rejected"], 0);
    s.settle("the queue to drain", |s| {
        s.json(&["sync", "status"])["pending"].as_i64() == Some(0)
    });

    // SC-009: replaying changes nothing on the server and rejects nothing.
    // Asserted on server state rather than on the count a particular call
    // reports, because the background worker shares the queue.
    let before = server.get_json(&format!("/api/projects/{project_id}/memories"), &token);
    let second = s.json(&["sync", "now"]);
    assert_eq!(second["rejected"], 0);
    let after = server.get_json(&format!("/api/projects/{project_id}/memories"), &token);
    assert_eq!(
        before["memories"], after["memories"],
        "a replay must leave server state identical"
    );

    let memories = server.get_json(&format!("/api/projects/{project_id}/memories"), &token);
    let found = memories["memories"].as_array().unwrap();
    assert!(
        found.iter().any(|m| m["content"]
            .as_str()
            .unwrap_or_default()
            .contains("shared convention")),
        "the memory did not reach the server"
    );
}

#[test]
fn the_server_holds_provenance_references_but_no_observation_content() {
    // SC-010's second clause, and the whole point of FR-055.
    let Some(server) = server() else { return };
    let s = Sandbox::new();
    let token = server.new_user_token("privacy");
    attach_server(&s, &server, &token);
    seed_local_work(&s);
    let project_id = s.json(&["link", "--create"])["server_project_id"]
        .as_str()
        .unwrap()
        .to_string();
    s.json(&["sync", "now"]);

    let memories = server.get_json(&format!("/api/projects/{project_id}/memories"), &token);
    let text = serde_json::to_string(&memories).unwrap();
    assert!(
        !text.contains("secret-local-path.rs"),
        "an observation path reached the server: {text}"
    );

    let first = &memories["memories"][0];
    assert!(
        first["provenance"]["session_id"].is_string(),
        "provenance must survive"
    );
    assert!(first["provenance"]["evidence_count"].is_number());

    let sessions = server.get_json(&format!("/api/projects/{project_id}/sessions"), &token);
    let sessions_text = serde_json::to_string(&sessions).unwrap();
    for local_only in ["worktree_path", "agent_session_key", "daemon_run_id"] {
        assert!(
            !sessions_text.contains(local_only),
            "{local_only} reached the server"
        );
    }
}

#[test]
fn an_item_carrying_observation_content_is_rejected() {
    let Some(server) = server() else { return };
    let s = Sandbox::new();
    let token = server.new_user_token("reject");
    attach_server(&s, &server, &token);
    let project_id = s.json(&["link", "--create"])["server_project_id"]
        .as_str()
        .unwrap()
        .to_string();

    let body = json!({
        "project_id": project_id,
        "items": [{
            "idempotency_key": "handmade-1",
            "entity_type": "memory",
            "entity_id": uuid::Uuid::now_v7(),
            "operation": "upsert",
            "payload": { "content": "x", "summary": "raw observation text" }
        }]
    });
    let out = server.post_json("/api/sync/batch", &body, None);
    // Unauthenticated: the daemon uses a bearer token, so this is a 401 shape.
    assert!(out.get("error").is_some() || out.get("results").is_some());

    let authed = std::process::Command::new("curl")
        .args([
            "-s",
            "-X",
            "POST",
            "-H",
            "content-type: application/json",
            "-H",
            &format!("authorization: Bearer {token}"),
            "-d",
            &body.to_string(),
            &format!("{}/api/sync/batch", server.base),
        ])
        .output()
        .expect("curl");
    let parsed: serde_json::Value = serde_json::from_slice(&authed.stdout).expect("json");
    assert_eq!(parsed["results"][0]["status"], "rejected");
    assert!(parsed["results"][0]["error"]["message"]
        .as_str()
        .unwrap()
        .contains("summary"));
}

#[test]
fn a_non_member_is_refused_rather_than_shown_nothing() {
    let Some(server) = server() else { return };
    let s = Sandbox::new();
    let owner = server.new_user_token("member");
    attach_server(&s, &server, &owner);
    seed_local_work(&s);
    let project_id = s.json(&["link", "--create"])["server_project_id"]
        .as_str()
        .unwrap()
        .to_string();
    s.json(&["sync", "now"]);

    let stranger = server.new_user_token("stranger");
    let status = server.get_status(&format!("/api/projects/{project_id}/memories"), &stranger);
    assert_eq!(
        status, 403,
        "a non-member must be refused, not shown an empty list (FR-057)"
    );
}

#[test]
fn an_unlinked_project_queues_nothing_and_refuses_to_sync() {
    let Some(server) = server() else { return };
    let s = Sandbox::new();
    let token = server.new_user_token("unlinked");
    attach_server(&s, &server, &token);
    seed_local_work(&s);

    let status = s.json(&["sync", "status"]);
    assert_eq!(status["linked"], false);
    assert_eq!(
        status["pending"], 0,
        "an unlinked project must never queue (SC-010)"
    );

    let err = s.json_err(&["sync", "now"]);
    assert_eq!(err["code"], "not_linked");
}

#[test]
fn two_clones_at_different_paths_link_to_one_shared_project() {
    // FR-064: identity is server-assigned, never derived from a path.
    let Some(server) = server() else { return };
    let token = server.new_user_token("clone");

    let a = Sandbox::new();
    attach_server(&a, &server, &token);
    seed_local_work(&a);
    let project_id = a.json(&["link", "--create"])["server_project_id"]
        .as_str()
        .unwrap()
        .to_string();
    a.json(&["sync", "now"]);

    // A second checkout at a different path, standing in for another machine.
    let b = Sandbox::new();
    attach_server(&b, &server, &token);
    let joined = b.json(&["link", "--project", &project_id]);
    assert_eq!(joined["linked"], true);
    assert_eq!(joined["server_project_id"].as_str().unwrap(), project_id);

    b.json(&["sync", "now"]);
    let projects = server.get_json("/api/projects", &token);
    let matching: Vec<_> = projects["projects"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|p| p["id"].as_str() == Some(project_id.as_str()))
        .collect();
    assert_eq!(
        matching.len(),
        1,
        "two clones must not create two shared projects"
    );
}

#[test]
fn a_teammates_memory_becomes_locally_searchable() {
    // FR-056: the pull half of sync.
    let Some(server) = server() else { return };
    let token = server.new_user_token("team");

    let a = Sandbox::new();
    attach_server(&a, &server, &token);
    seed_local_work(&a);
    let project_id = a.json(&["link", "--create"])["server_project_id"]
        .as_str()
        .unwrap()
        .to_string();
    a.json(&["sync", "now"]);

    let b = Sandbox::new();
    attach_server(&b, &server, &token);
    b.json(&["link", "--project", &project_id]);
    b.json(&["sync", "now"]);

    let found = b.json(&["memory", "search", "convention"]);
    assert!(
        !found["results"].as_array().unwrap().is_empty(),
        "a shared memory should be locally searchable after a pull"
    );
}

#[test]
fn offline_work_accumulates_and_drains_without_duplicates() {
    let Some(server) = server() else { return };
    let s = Sandbox::new();
    let token = server.new_user_token("offline");
    attach_server(&s, &server, &token);
    let project_id = s.json(&["link", "--create"])["server_project_id"]
        .as_str()
        .unwrap()
        .to_string();
    s.json(&["sync", "now"]);

    // Point the daemon at a dead server and keep working.
    s.must(&[
        "auth",
        "token",
        "set",
        &token,
        "--server",
        "http://127.0.0.1:1",
    ]);
    seed_local_work(&s);
    s.must(&[
        "memory",
        "add",
        "--type",
        "fact",
        "--scope",
        "project",
        "written while offline",
    ]);

    assert!(
        s.cairn(&["status"]).ok(),
        "local operations must keep working offline"
    );
    let pending = s.json(&["sync", "status"])["pending"].as_i64().unwrap();
    assert!(
        pending > 0,
        "offline changes should accumulate in the queue"
    );

    // Reconnect and drain.
    s.must(&["auth", "token", "set", &token, "--server", &server.base]);
    let drained = s.json(&["sync", "now"]);
    assert_eq!(drained["rejected"], 0);
    assert_eq!(s.json(&["sync", "status"])["pending"], 0);

    let memories = server.get_json(&format!("/api/projects/{project_id}/memories"), &token);
    let offline: Vec<_> = memories["memories"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|m| m["content"].as_str() == Some("written while offline"))
        .collect();
    assert_eq!(offline.len(), 1, "draining must not duplicate");
}

#[test]
fn the_background_worker_delivers_queued_work_with_no_manual_sync() {
    // C1 / FR-056: offline work reaches the server once it comes back, without
    // anyone typing `cairn sync now`.
    let Some(server) = server() else { return };
    let s = Sandbox::new();
    let token = server.new_user_token("auto");
    attach_server(&s, &server, &token);

    let project_id = s.json(&["link", "--create"])["server_project_id"]
        .as_str()
        .unwrap()
        .to_string();

    // 1. Take the server away.
    s.must(&[
        "auth",
        "token",
        "set",
        &token,
        "--server",
        "http://127.0.0.1:1",
    ]);

    // 2. Produce syncable local changes.
    s.must(&[
        "memory",
        "add",
        "--type",
        "fact",
        "--scope",
        "project",
        "delivered by the background worker",
    ]);
    s.must(&[
        "task",
        "new",
        "--title",
        "queued task",
        "--goal",
        "arrive automatically",
    ]);
    let pending = s.json(&["sync", "status"])["pending"].as_i64().unwrap();
    assert!(pending > 0, "offline changes must queue");

    // 3. Restore the server.
    s.must(&["auth", "token", "set", &token, "--server", &server.base]);

    // 4. No manual `sync now` from here on — the worker must do it.
    s.settle("the queue to drain by itself", |s| {
        s.json(&["sync", "status"])["pending"].as_i64() == Some(0)
    });

    // 5. Exactly once on the server.
    let memories = server.get_json(&format!("/api/projects/{project_id}/memories"), &token);
    let delivered: Vec<_> = memories["memories"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|m| m["content"].as_str() == Some("delivered by the background worker"))
        .collect();
    assert_eq!(delivered.len(), 1, "the worker must deliver exactly once");

    let tasks = server.get_json(&format!("/api/projects/{project_id}/tasks"), &token);
    let queued: Vec<_> = tasks["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|t| t["title"].as_str() == Some("queued task"))
        .collect();
    assert_eq!(
        queued.len(),
        1,
        "every queued entity is delivered, not just memory"
    );

    // And it stays exactly once: the worker keeps running.
    std::thread::sleep(std::time::Duration::from_millis(1500));
    let again = server.get_json(&format!("/api/projects/{project_id}/memories"), &token);
    let still: Vec<_> = again["memories"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|m| m["content"].as_str() == Some("delivered by the background worker"))
        .collect();
    assert_eq!(still.len(), 1, "a running worker must not duplicate");
}

#[test]
fn a_permanently_rejected_item_becomes_failed_and_is_surfaced() {
    // M3 / FR-058: permanent failures are shown with the affected item rather
    // than retried forever in silence.
    let Some(server) = server() else { return };
    let s = Sandbox::new();
    let token = server.new_user_token("reject-surface");
    attach_server(&s, &server, &token);
    s.json(&["link", "--create"]);

    // Queue and corrupt the item with the server away. The background worker
    // shares this queue, and an item it delivers before the corruption lands is
    // simply a valid item — this test would then be asserting nothing.
    s.must(&[
        "auth",
        "token",
        "set",
        &token,
        "--server",
        "http://127.0.0.1:1",
    ]);
    s.must(&[
        "memory",
        "add",
        "--type",
        "fact",
        "--scope",
        "project",
        "will be rejected",
    ]);
    s.settle("the item to queue", |s| {
        s.json(&["sync", "status"])["pending"].as_i64().unwrap_or(0) > 0
    });

    // Corrupt the queued payload so the server refuses it permanently: an
    // observation field is exactly what the allowlist rejects (FR-055).
    // Undelivered means `pending` or claimed by a drainer that could not reach
    // the server, and both are still waiting to be sent.
    s.execute_sql(
        "UPDATE outbox SET payload = json_set(payload, '$.summary', 'raw observation text') \
         WHERE state IN ('pending', 'in_flight')",
    );

    // Give the server back and ask for the drain. Whether this call or the
    // worker does the delivering is not the point — the outcome is.
    s.must(&["auth", "token", "set", &token, "--server", &server.base]);
    s.json(&["sync", "now"]);

    s.settle("the refusal to be recorded", |s| {
        s.json(&["sync", "status"])["failed"].as_i64().unwrap_or(0) >= 1
    });
    let status = s.json(&["sync", "status"]);
    assert!(
        status["failed"].as_i64().unwrap() >= 1,
        "the item must be recorded failed"
    );
    let failures = status["failures"].as_array().unwrap();
    assert!(!failures.is_empty(), "the failure must be surfaced");
    // Every corrupted item is refused, and each is surfaced with its identity
    // and the reason.
    for f in failures {
        assert!(
            f["entity_id"].is_string(),
            "a failure must identify its item"
        );
        assert!(
            f["error"].as_str().unwrap_or_default().contains("summary"),
            "the reason must name the offending field: {:?}",
            f["error"]
        );
    }
    assert!(
        failures.iter().any(|f| f["entity_type"] == "memory"),
        "the memory we corrupted must be among them: {failures:?}"
    );

    // A failed item is not retried forever: the queue no longer counts it pending.
    assert_eq!(s.json(&["sync", "status"])["pending"], 0);
}

#[test]
fn a_handoff_reaches_the_server_with_the_fields_the_spec_allows() {
    // M2: FR-033 requires changed files, decisions, failures and tests on a
    // handoff, and FR-055 allowlists handoffs wholesale — so those fields do
    // travel. What must never travel is an observation row or its content.
    let Some(server) = server() else { return };
    let s = Sandbox::new();
    let token = server.new_user_token("handoff-fields");
    attach_server(&s, &server, &token);
    seed_local_work(&s);
    let project_id = s.json(&["link", "--create"])["server_project_id"]
        .as_str()
        .unwrap()
        .to_string();
    s.json(&["sync", "now"]);

    let sessions = server.get_json(&format!("/api/projects/{project_id}/sessions"), &token);
    let session_id = sessions["sessions"][0]["id"]
        .as_str()
        .expect("a session")
        .to_string();
    let handoff =
        server.get_json(&format!("/api/sessions/{session_id}/handoff"), &token)["handoff"].clone();

    // Allowed by FR-033, and present.
    assert!(handoff["changed_files"].is_array());
    assert!(handoff["next_step"].is_string());
    assert!(handoff["repository_state"].is_object());

    // Evidence is references and a count — never content.
    assert!(handoff["evidence"]["observation_ids"].is_array());
    assert!(handoff["evidence"]["evidence_count"].is_number());
    let text = serde_json::to_string(&handoff).unwrap();
    for forbidden in ["\"summary\"", "\"details\"", "\"exit_code\"", "\"path\""] {
        assert!(
            !text.contains(forbidden),
            "{forbidden} appeared on a synced handoff"
        );
    }
}

// ---------------------------------------------------------------------------
// H-A: two drainers, one queue
//
// The background worker and `cairn sync now` share the outbox. Before rows were
// claimed they could be sent twice at once, and the server's read-then-insert
// idempotency check turned the loser of that race into a duplicate-key error,
// reported as `rejected` and recorded locally as a permanent failure. Nothing
// was wrong with the item (FR-056, SC-009).
// ---------------------------------------------------------------------------

#[test]
fn concurrent_drains_deliver_every_record_exactly_once_and_fail_none() {
    let Some(server) = server() else { return };
    let s = Sandbox::new();
    let token = server.new_user_token("concurrent");
    attach_server(&s, &server, &token);
    let project_id = s.json(&["link", "--create"])["server_project_id"]
        .as_str()
        .unwrap()
        .to_string();

    // Build a real backlog with the server away, so the drainers have something
    // substantial to race over.
    s.must(&[
        "auth",
        "token",
        "set",
        &token,
        "--server",
        "http://127.0.0.1:1",
    ]);
    const RECORDS: usize = 32;
    for i in 0..RECORDS {
        s.must(&[
            "memory",
            "add",
            "--type",
            "fact",
            "--scope",
            "project",
            &format!("concurrent record {i}"),
        ]);
    }
    let queued = s.json(&["sync", "status"])["pending"].as_i64().unwrap();
    assert!(
        queued >= RECORDS as i64,
        "expected a backlog of at least {RECORDS}, got {queued}"
    );

    // The server comes back. The background worker is already running; several
    // manual drains pile on top, all reaching for the same rows.
    s.must(&["auth", "token", "set", &token, "--server", &server.base]);
    let outcomes: Vec<serde_json::Value> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..4)
            .map(|_| {
                scope.spawn(|| {
                    let result = s.cairn(&["--json", "sync", "now"]);
                    serde_json::from_str::<serde_json::Value>(&result.stdout)
                        .unwrap_or(serde_json::Value::Null)
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("a concurrent sync finished"))
            .collect()
    });

    for out in &outcomes {
        assert_eq!(out["ok"], true, "a concurrent sync failed: {out}");
        assert_eq!(
            out["data"]["rejected"], 0,
            "a concurrent delivery was rejected: {out}"
        );
    }

    s.settle_within(
        "the queue to drain",
        std::time::Duration::from_secs(60),
        |s| s.json(&["sync", "status"])["pending"].as_i64() == Some(0),
    );
    let status = s.json(&["sync", "status"]);
    assert_eq!(status["pending"], 0);
    assert_eq!(
        status["failed"], 0,
        "a concurrent drain must not fail anything permanently: {:?}",
        status["failures"]
    );

    // Every record reached the server once, logically: a redelivery of the same
    // idempotency key is a `duplicate` that changes nothing.
    let memories = server.get_json(
        &format!("/api/projects/{project_id}/memories?limit=100"),
        &token,
    );
    let found = memories["memories"].as_array().unwrap();
    for i in 0..RECORDS {
        let wanted = format!("concurrent record {i}");
        let arrivals = found
            .iter()
            .filter(|m| m["content"].as_str() == Some(wanted.as_str()))
            .count();
        assert_eq!(arrivals, 1, "`{wanted}` arrived {arrivals} times");
    }
}

#[test]
fn concurrent_deliveries_of_one_key_apply_once_and_duplicate_the_rest() {
    // The server half of the same race, driven directly: identical batches
    // arriving at the same instant must not produce an internal error or a
    // rejection, which the daemon would record as a permanent failure.
    let Some(server) = server() else { return };
    let s = Sandbox::new();
    let token = server.new_user_token("key-race");
    attach_server(&s, &server, &token);
    let project_id = s.json(&["link", "--create"])["server_project_id"]
        .as_str()
        .unwrap()
        .to_string();

    let entity = uuid::Uuid::now_v7();
    let body = json!({
        "project_id": project_id,
        "items": [{
            "idempotency_key": format!("race-{entity}"),
            "entity_type": "memory",
            "entity_id": entity,
            "operation": "upsert",
            "payload": {
                "id": entity,
                "type": "fact",
                "scope": "project",
                "scope_key": project_id,
                "content": "delivered by several drainers at once",
                "state": "active",
                "provenance": {
                    "session_id": uuid::Uuid::now_v7(),
                    "observation_ids": [],
                    "evidence_count": 0
                }
            }
        }]
    });

    let responses: Vec<serde_json::Value> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let base = server.base.clone();
                let token = token.clone();
                let body = body.clone();
                scope.spawn(move || post_json_bearer(&base, "/api/sync/batch", &body, &token))
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("a delivery finished"))
            .collect()
    });

    let (mut applied, mut duplicate) = (0, 0);
    for response in &responses {
        let status = response["results"][0]["status"]
            .as_str()
            .unwrap_or_else(|| panic!("no per-item result: {response}"));
        match status {
            "applied" => applied += 1,
            "duplicate" => duplicate += 1,
            other => panic!("a concurrent redelivery reported `{other}`: {response}"),
        }
    }
    assert_eq!(applied, 1, "exactly one delivery may apply");
    assert_eq!(
        duplicate,
        responses.len() - 1,
        "every other delivery must be a duplicate, never a rejection"
    );

    // And one row, not eight.
    let memories = server.get_json(
        &format!("/api/projects/{project_id}/memories?limit=100"),
        &token,
    );
    let arrivals = memories["memories"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|m| m["content"].as_str() == Some("delivered by several drainers at once"))
        .count();
    assert_eq!(arrivals, 1);
}

#[test]
fn a_claim_abandoned_mid_send_is_taken_back_and_delivered() {
    // A drainer killed between claiming a row and acknowledging it must not
    // strand that row. The claim goes stale and the next drain takes it back.
    let Some(server) = server() else { return };
    let s = Sandbox::new();
    let token = server.new_user_token("abandoned");
    attach_server(&s, &server, &token);
    let project_id = s.json(&["link", "--create"])["server_project_id"]
        .as_str()
        .unwrap()
        .to_string();
    s.settle("the initial queue to drain", |s| {
        s.json(&["sync", "status"])["pending"].as_i64() == Some(0)
    });

    // Queue with the server away, so nothing drains the row before we strand it.
    s.must(&[
        "auth",
        "token",
        "set",
        &token,
        "--server",
        "http://127.0.0.1:1",
    ]);
    s.must(&[
        "memory",
        "add",
        "--type",
        "fact",
        "--scope",
        "project",
        "stranded by a dead drainer",
    ]);
    s.settle("the row to queue", |s| {
        s.json(&["sync", "status"])["pending"].as_i64().unwrap_or(0) > 0
    });

    // Exactly what a process killed mid-send leaves behind: a claim with an
    // owner that is never coming back to acknowledge it.
    s.execute_sql(
        "UPDATE outbox SET state = 'in_flight', claimed_at = '1970-01-01T00:00:00+00:00' \
         WHERE state = 'pending'",
    );
    assert!(
        s.json(&["sync", "status"])["pending"].as_i64().unwrap() > 0,
        "a claimed row is still work that has not arrived"
    );

    // Give the server back and type nothing further.
    s.must(&["auth", "token", "set", &token, "--server", &server.base]);
    s.settle_within(
        "the abandoned claim to drain by itself",
        std::time::Duration::from_secs(60),
        |s| s.json(&["sync", "status"])["pending"].as_i64() == Some(0),
    );

    let status = s.json(&["sync", "status"]);
    assert_eq!(status["pending"], 0);
    assert_eq!(
        status["failed"], 0,
        "an abandoned claim is not a permanent failure: {:?}",
        status["failures"]
    );

    let memories = server.get_json(
        &format!("/api/projects/{project_id}/memories?limit=100"),
        &token,
    );
    let arrivals = memories["memories"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|m| m["content"].as_str() == Some("stranded by a dead drainer"))
        .count();
    assert_eq!(arrivals, 1, "an abandoned claim must still arrive, once");
}

#[test]
fn a_delivery_racing_an_uncommitted_key_reports_duplicate_not_rejected() {
    // The deterministic form of the same race. Eight parallel requests only
    // *might* overlap; holding the idempotency key in an uncommitted
    // transaction guarantees it. The server's claim on the key blocks on that
    // row, and once it is released the server must read the outcome as
    // "someone else has this key" — a duplicate.
    //
    // Reading the key first would instead have seen nothing (the holder had not
    // committed), applied the item, and then failed on the primary key: a valid
    // item surfaced as a permanent rejection (FR-056, SC-009).
    let Some(server) = server() else { return };
    let Ok(database_url) = std::env::var("CAIRN_TEST_DATABASE_URL") else {
        return;
    };
    let s = Sandbox::new();
    let token = server.new_user_token("held-key");
    attach_server(&s, &server, &token);
    let project_id = s.json(&["link", "--create"])["server_project_id"]
        .as_str()
        .unwrap()
        .to_string();

    let entity = uuid::Uuid::now_v7();
    let key = format!("held-{entity}");
    let body = json!({
        "project_id": project_id,
        "items": [{
            "idempotency_key": key,
            "entity_type": "memory",
            "entity_id": entity,
            "operation": "upsert",
            "payload": {
                "id": entity,
                "type": "fact",
                "scope": "project",
                "scope_key": project_id,
                "content": "racing an uncommitted key",
                "state": "active",
                "provenance": {
                    "session_id": uuid::Uuid::now_v7(),
                    "observation_ids": [],
                    "evidence_count": 0
                }
            }
        }]
    });

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    let pool = runtime
        .block_on(sqlx::PgPool::connect(&database_url))
        .expect("postgres");

    // Another delivery of this key is already in flight and has claimed it,
    // but has not committed yet.
    let mut holder = runtime.block_on(pool.begin()).expect("begin");
    runtime
        .block_on(
            sqlx::query(
                "INSERT INTO sync_state (idempotency_key, project_id, entity_type, entity_id)
                 VALUES ($1, $2, $3, $4)",
            )
            .bind(&key)
            .bind(uuid::Uuid::parse_str(&project_id).unwrap())
            .bind("memory")
            .bind(entity)
            .execute(&mut *holder),
        )
        .expect("hold the key");

    let response = std::thread::scope(|scope| {
        let base = server.base.clone();
        let bearer = token.clone();
        let payload = body.clone();
        let delivery =
            scope.spawn(move || post_json_bearer(&base, "/api/sync/batch", &payload, &bearer));
        // Long enough for the request to reach the claim and block on it.
        std::thread::sleep(std::time::Duration::from_millis(500));
        runtime.block_on(holder.commit()).expect("release the key");
        delivery.join().expect("the delivery finished")
    });

    assert_eq!(
        response["results"][0]["status"], "duplicate",
        "a delivery that lost the race for a key is a duplicate, never rejected \
         and never an internal error: {response}"
    );
    runtime.block_on(pool.close());
}
