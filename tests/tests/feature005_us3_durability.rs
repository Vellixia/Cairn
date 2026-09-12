//! User Story 3, end to end: the laptop dies and the knowledge does not (T092,
//! SC-713, SC-714, SC-738, SC-761).
//!
//! # What makes this the *story* test
//!
//! `feature005_local_loss.rs` asserts the mechanism — the inventory names the
//! right categories, each merge accepts a correction, each lane refills. This
//! asserts the promise a user was made: they used Cairn, their machine was
//! replaced, and their knowledge was there.
//!
//! So it does the destructive thing literally. The store is deleted from disk,
//! not truncated and not emptied through an API, and nothing between the
//! deletion and the assertions repairs anything. There is no `cairn restore`,
//! and its absence is the point of FR-704: restoration is what synchronization
//! already does, or it is a manual step the requirement forbids.
//!
//! # A second account is in the story on purpose
//!
//! Durability and privacy are pulled in opposite directions by the same change.
//! Making a pattern survive means storing it centrally, and the easy way to
//! restore it to its owner is to serve it to everyone. So the colleague here is
//! a genuine member of the same project — they see the project's memories, and
//! they must still never see the owner's patterns or personal knowledge
//! (FR-708d, SC-761). A test that only proved restoration would pass with that
//! hole wide open.

use cairn_e2e::{attach_server, get_json_status_bearer, post_json_status_bearer, Sandbox, Server};
use serde_json::{json, Value};
use uuid::Uuid;

fn server() -> Option<Server> {
    match Server::start_own_database() {
        Some(s) => Some(s),
        None => {
            eprintln!("skipped: CAIRN_TEST_DATABASE_URL is not set");
            None
        }
    }
}

struct Person {
    sandbox: Sandbox,
    account: Uuid,
    token: String,
}

/// One person, one machine, linked to `project`.
fn person(server: &Server, label: &str, project: Option<Uuid>, remote: &str) -> (Person, Uuid) {
    let sandbox = Sandbox::new();
    sandbox.git(&["remote", "add", "origin", remote]);
    sandbox.must(&["init"]);

    let (account, token) = server.new_user(label);
    let project = match project {
        Some(existing) => {
            // Joining is an act of the server's, not the client's: the project
            // exists and this account is added to it.
            server.execute(&format!(
                "INSERT INTO project_members (project_id, user_id) VALUES ('{existing}', '{account}')"
            ));
            existing
        }
        None => {
            let (created, status) = post_json_status_bearer(
                &server.base,
                "/api/projects",
                &json!({ "name": label, "repository_remote": remote }),
                &token,
            );
            assert_eq!(status, 200, "create project: {created}");
            created["id"].as_str().expect("id").parse().expect("uuid")
        }
    };

    attach_server(&sandbox, server, &token);
    sandbox.must(&["link", "--project", &project.to_string()]);
    sandbox.must(&["sync", "now"]);
    (
        Person {
            sandbox,
            account,
            token,
        },
        project,
    )
}

/// What a new machine has to be told, and what it must not have to repair.
///
/// The project link lives in the store that was just deleted: `projects.server_project_id`
/// is a local row, so a store with no rows is a store that does not know which
/// server project this checkout is. Naming it again is **setup**, the same setup
/// a genuinely new machine needs, and it is deliberately not automatic —
/// `cairn link` with no argument *offers* remote-derived candidates and never
/// applies one silently (FR-064, D14), because linking a checkout to a project
/// nobody chose is worse than asking.
///
/// What FR-704 forbids is a *repair* step: an import, a reconciliation, a merge
/// the user has to resolve. None of that happens here. Everything after this
/// line is synchronization doing what it already does.
///
/// The credential is untouched on purpose: it lives in a token file rather than
/// in the store (`cairn_core::paths::token_path`), so the account-scoped lanes —
/// personal, team and patterns — re-establish with no setup at all.
fn name_the_project_again(sandbox: &cairn_e2e::Sandbox, project: Uuid) {
    sandbox.must(&["init"]);
    sandbox.must(&["link", "--project", &project.to_string()]);
}

const SETTLE: std::time::Duration = std::time::Duration::from_secs(60);

/// Wait for a condition, re-driving synchronization while waiting.
///
/// `Sandbox::settle` allows five seconds and only observes; both are wrong here.
/// A refill after local loss has to establish its lanes first, and the lane a
/// pull needs may not exist on the first attempt — so this drives `sync now`
/// each round rather than watching a background worker's backoff, and gives the
/// whole sequence a minute. A test that waited passively would be measuring the
/// worker's schedule instead of whether the knowledge comes back.
fn settle_syncing(sandbox: &Sandbox, what: &str, mut predicate: impl FnMut() -> bool) {
    let deadline = std::time::Instant::now() + SETTLE;
    while std::time::Instant::now() < deadline {
        if predicate() {
            return;
        }
        let _ = sandbox.cairn(&["sync", "now"]);
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    panic!("timed out waiting for: {what}");
}

fn local_count(p: &Person, sql: &str) -> i64 {
    p.sandbox
        .query_column(sql)
        .first()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

/// Delete the store the way a failed disk deletes it.
fn destroy(p: &Person) {
    p.sandbox.stop_daemon();
    let db = p.sandbox.db_path();
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", db.display()));
    }
    assert!(!db.exists(), "the store survived the deletion");
}

/// Everything, in order: work, loss, recovery, and the privacy that must hold
/// throughout.
///
/// **Falsified by** any of: dropping the patterns lane, reverting a merge to
/// insert-once, serving `GET /api/patterns` without the owner filter, or making
/// restoration wait on a command the user has to type.
#[test]
fn a_destroyed_store_comes_back_with_everything_the_server_accepted() {
    let Some(server) = server() else { return };
    let remote = "git@localhost:cairnfixture/us3.git";
    let (owner, project) = person(&server, "us3-owner", None, remote);
    let (colleague, _) = person(&server, "us3-colleague", Some(project), remote);

    // ---------------------------------------------------------------------
    // 1. Work happens. Four kinds of knowledge reach the server.
    // ---------------------------------------------------------------------
    let session = Uuid::now_v7();
    server.execute(&format!(
        "INSERT INTO sessions (id, project_id, agent, branch, status, started_at)
         VALUES ('{session}', '{project}', 'claude_code', 'main', 'active', now())"
    ));
    server.execute(&format!(
        "INSERT INTO memories
            (id, project_id, type, scope, scope_key, content, state, origin_session_id,
             topic_key, value_key, origin_kind)
         VALUES ('{}', '{project}', 'decision', 'project', '{project}',
                 'the ingest boundary owns rejection, not the parser',
                 'active', '{session}', 'decision.ingest', 'boundary-owns', 'explicit')",
        Uuid::now_v7()
    ));
    server.execute(&format!(
        "INSERT INTO personal_knowledge
            (id, owner_user_id, knowledge_type, content, topic_key, value_key,
             writer_id, writer_seq, created_at)
         VALUES ('{}', '{}', 'convention', 'I read the failing assertion before the stack',
                 'convention.debugging', 'assertion-first', '{}', 1, now())",
        Uuid::now_v7(),
        owner.account,
        Uuid::now_v7()
    ));
    server.execute(&format!(
        "INSERT INTO team_knowledge
            (id, knowledge_type, content, topic_key, value_key, state,
             proposed_by_user_id, ratified_by_user_id, ratified_at,
             writer_id, writer_seq, created_at)
         VALUES ('{}', 'convention', 'a migration ships with the code that needs it',
                 'convention.migrations', 'ship-together', 'authoritative',
                 '{}', '{}', now(), '{}', 1, now())",
        Uuid::now_v7(),
        owner.account,
        owner.account,
        Uuid::now_v7()
    ));

    let (promoted, status) = post_json_status_bearer(
        &server.base,
        "/api/patterns",
        &json!({
            "title": "a test that only fails on the slower machine",
            "problem": "an assertion that holds locally and fails under load",
            "root_cause": "the wait is a sleep rather than a condition",
            "approach": "wait on the condition and give it a deadline",
            "constraints": ["the condition has to be observable"],
        }),
        &owner.token,
    );
    assert_eq!(status, 200, "promote: {promoted}");
    assert_eq!(
        promoted["stored"],
        json!(true),
        "promotion answered without storing anything, so nothing was made \
         durable and the rest of this test would prove nothing: {promoted}"
    );
    let pattern_id: Uuid = promoted["pattern_id"]
        .as_str()
        .expect("pattern_id")
        .parse()
        .expect("uuid");

    // Machine-local records too, so the loss half of the story is real rather
    // than a matter of counting zero rows twice.
    owner.sandbox.must(&[
        "memory",
        "add",
        "the staging password reset trick, which stays on this laptop",
        "--local-only",
    ]);
    let key = format!("us3-{}", Uuid::now_v7());
    let hook = owner.sandbox.hook(
        "SessionStart",
        json!({ "session_id": key, "source": "startup" }),
    );
    assert_eq!(hook.code, 0, "a hook always exits 0: {}", hook.stderr);
    let hook = owner.sandbox.hook(
        "PostToolUse",
        json!({
            "session_id": key,
            "tool_name": "Edit",
            "tool_input": { "file_path": "src/release.rs" }
        }),
    );
    assert_eq!(hook.code, 0, "a hook always exits 0: {}", hook.stderr);
    owner.sandbox.settle_observations(1);

    owner.sandbox.must(&["sync", "now"]);
    settle_syncing(
        &owner.sandbox,
        "everything the server holds reaches this machine",
        || {
            local_count(
                &owner,
                "SELECT CAST(COUNT(*) AS TEXT) FROM memories WHERE local_only = 0",
            ) > 0
                && local_count(
                    &owner,
                    "SELECT CAST(COUNT(*) AS TEXT) FROM personal_knowledge",
                ) > 0
                && local_count(&owner, "SELECT CAST(COUNT(*) AS TEXT) FROM team_knowledge") > 0
                && local_count(&owner, "SELECT CAST(COUNT(*) AS TEXT) FROM cached_patterns") > 0
        },
    );

    let observations = local_count(&owner, "SELECT CAST(COUNT(*) AS TEXT) FROM observations");
    assert!(observations > 0, "no local-only evidence was recorded");

    // ---------------------------------------------------------------------
    // 2. The colleague sees the project and nothing personal — before the loss,
    //    so a later empty result cannot be mistaken for privacy working.
    // ---------------------------------------------------------------------
    colleague.sandbox.must(&["sync", "now"]);
    settle_syncing(
        &colleague.sandbox,
        "the colleague pulls the shared project",
        || local_count(&colleague, "SELECT CAST(COUNT(*) AS TEXT) FROM memories") > 0,
    );
    assert_eq!(
        local_count(
            &colleague,
            "SELECT CAST(COUNT(*) AS TEXT) FROM cached_patterns"
        ),
        0,
        "a project member's store cached the owner's pattern; storing a pattern \
         centrally is durability, not publication (FR-708d)"
    );
    assert_eq!(
        local_count(
            &colleague,
            "SELECT CAST(COUNT(*) AS TEXT) FROM personal_knowledge"
        ),
        0,
        "a project member pulled the owner's personal knowledge"
    );
    let (theirs, status) = get_json_status_bearer(&server.base, "/api/patterns", &colleague.token);
    assert_eq!(status, 200, "the colleague may ask: {theirs}");
    assert_eq!(
        theirs["patterns"].as_array().map(Vec::len),
        Some(0),
        "the owner's pattern was served to another account: {theirs}"
    );

    // ---------------------------------------------------------------------
    // 3. The machine is lost.
    // ---------------------------------------------------------------------
    destroy(&owner);

    // ---------------------------------------------------------------------
    // 4. Recovery, with no repair step. A sync is all.
    // ---------------------------------------------------------------------
    name_the_project_again(&owner.sandbox, project);
    owner.sandbox.must(&["sync", "now"]);
    settle_syncing(&owner.sandbox, "every domain comes back", || {
        local_count(
            &owner,
            "SELECT CAST(COUNT(*) AS TEXT) FROM memories WHERE local_only = 0",
        ) > 0
            && local_count(
                &owner,
                "SELECT CAST(COUNT(*) AS TEXT) FROM personal_knowledge",
            ) > 0
            && local_count(&owner, "SELECT CAST(COUNT(*) AS TEXT) FROM team_knowledge") > 0
            && local_count(&owner, "SELECT CAST(COUNT(*) AS TEXT) FROM cached_patterns") > 0
    });

    assert_eq!(
        local_count(
            &owner,
            &format!("SELECT CAST(COUNT(*) AS TEXT) FROM cached_patterns WHERE pattern_id = '{pattern_id}'")
        ),
        1,
        "the pattern did not survive, which is the gap Feature 004 deferred and \
         FR-708 exists to close (SC-738)"
    );

    // Reachable, not merely present: the content came back, not an empty row.
    let restored: Vec<String> = owner.sandbox.query_column(&format!(
        "SELECT problem FROM cached_patterns WHERE pattern_id = '{pattern_id}'"
    ));
    assert_eq!(
        restored.first().map(String::as_str),
        Some("an assertion that holds locally and fails under load"),
        "the pattern row came back without its content"
    );

    // ---------------------------------------------------------------------
    // 5. What did not survive, and the report says which.
    // ---------------------------------------------------------------------
    assert_eq!(
        local_count(&owner, "SELECT CAST(COUNT(*) AS TEXT) FROM observations"),
        0,
        "an observation survived a deletion it has no server table to survive"
    );
    assert_eq!(
        local_count(
            &owner,
            "SELECT CAST(COUNT(*) AS TEXT) FROM memories WHERE local_only = 1"
        ),
        0,
        "the local-only memory came back, so it had left the machine after all"
    );

    let report: Value = owner.sandbox.json(&["doctor", "--durability"]);
    let category_names = |key: &str| -> Vec<String> {
        report[key]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|e| e["category"].as_str().unwrap_or("").to_string())
                    .collect()
            })
            .unwrap_or_default()
    };
    assert!(
        category_names("lost_on_deletion").contains(&"observations".to_string()),
        "the report does not name observations as lost: {report}"
    );
    assert!(
        category_names("restorable_from_server").contains(&"cached patterns".to_string()),
        "the report does not name the pattern cache as restorable: {report}"
    );

    // ---------------------------------------------------------------------
    // 6. And the colleague still cannot see any of it, after the recovery.
    // ---------------------------------------------------------------------
    let (theirs, status) = get_json_status_bearer(&server.base, "/api/patterns", &colleague.token);
    assert_eq!(status, 200);
    assert_eq!(
        theirs["patterns"].as_array().map(Vec::len),
        Some(0),
        "recovery widened the pattern's audience: {theirs}"
    );
    assert_eq!(
        server.count(&format!(
            "SELECT COUNT(*) FROM shared_patterns
              WHERE pattern_id = '{pattern_id}' AND owner_user_id = '{}'",
            owner.account
        )),
        1,
        "the canonical record is not the owner's, or there is more than one of it"
    );
    assert_eq!(
        server.count(&format!(
            "SELECT COUNT(*) FROM shared_patterns WHERE pattern_id = '{pattern_id}'
               AND domain = 'personal' AND trust = 'sanitized'"
        )),
        1,
        "the surviving pattern is not the personal-domain, server-sanitized \
         record FR-708c and FR-708g describe"
    );
}
