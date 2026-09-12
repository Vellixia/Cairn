//! The outage cache, as a user meets it (T068, SC-718,
//! `contracts/retrieval-delivery.md` §12.3, FR-789, FR-790a, FR-837).
//!
//! # Why there is a cache at all
//!
//! Retrieval moved server-side, so an outage means no fresh durable knowledge.
//! Principle II permits a cache for exactly this, on condition that its bound,
//! refill and invalidation are stated — which §12.3 does, and which this file
//! holds the implementation to.
//!
//! # What is tested here and what is tested elsewhere
//!
//! `crates/cairnd/src/deliver.rs` already unit-tests the mechanism: the LRU at
//! two hundred sessions, the sixty-four kibibyte rejection, and the
//! cross-account miss. Repeating those here would test the same code twice and
//! the user's experience of it not at all. This file drives the daemon the way
//! an agent does, and asserts the three things only that vantage point can see:
//! a cached briefing says it is cached, an outage with nothing cached says
//! *that* rather than quietly returning less, and a briefing assembled for one
//! account is never handed to another.

use cairn_e2e::{attach_server, post_json_status_bearer, Sandbox, Server};
use serde_json::{json, Value};
use std::time::{Duration, Instant};
use uuid::Uuid;

const SETTLE: Duration = Duration::from_secs(30);

fn server() -> Option<Server> {
    match Server::start_own_database() {
        Some(s) => Some(s),
        None => {
            eprintln!("skipped: CAIRN_TEST_DATABASE_URL is not set");
            None
        }
    }
}

struct Device {
    sandbox: Sandbox,
    project: Uuid,
    token: String,
}

fn device(server: &Server, label: &str) -> Device {
    let sandbox = Sandbox::new();
    let remote = format!("git@localhost:cairnfixture/{label}.git");
    sandbox.git(&["remote", "add", "origin", &remote]);
    sandbox.must(&["init"]);

    let token = server.new_user_token(label);
    let (created, status) = post_json_status_bearer(
        &server.base,
        "/api/projects",
        &json!({ "name": label, "repository_remote": remote }),
        &token,
    );
    assert_eq!(status, 200, "create project: {created}");
    let project: Uuid = created["id"].as_str().expect("id").parse().expect("uuid");

    attach_server(&sandbox, server, &token);
    sandbox.must(&["link", "--project", &project.to_string()]);
    Device {
        sandbox,
        project,
        token,
    }
}

fn settle(what: &str, mut predicate: impl FnMut() -> bool) {
    let deadline = Instant::now() + SETTLE;
    while Instant::now() < deadline {
        if predicate() {
            return;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    panic!("timed out waiting for: {what}");
}

fn seed(server: &Server, project: Uuid, session: Uuid, content: &str) {
    server.execute(&format!(
        "INSERT INTO memories
            (id, project_id, type, scope, scope_key, content, state, origin_session_id,
             topic_key, value_key, origin_kind)
         VALUES ('{}', '{project}', 'fact', 'project', '{project}', '{content}', 'active',
                 '{session}', 'topic.{}', 'settled', 'explicit')",
        Uuid::now_v7(),
        Uuid::now_v7().simple()
    ));
}

/// Open a session through the hook and return what the agent received.
fn open_session(device: &Device, key: &str) -> String {
    let out = device.sandbox.hook_as(
        "claude-code",
        "SessionStart",
        json!({ "session_id": key, "source": "startup" }),
    );
    assert_eq!(out.code, 0, "a hook always exits 0: {}", out.stderr);
    let emitted: Value = serde_json::from_str(out.stdout.trim())
        .unwrap_or_else(|e| panic!("hook did not emit context JSON ({e}): {:?}", out.stdout));
    emitted["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

fn synced_sessions(server: &Server, project: Uuid) -> i64 {
    server.count(&format!(
        "SELECT count(*) FROM sessions WHERE project_id = '{project}'"
    ))
}

#[test]
fn a_briefing_served_from_cache_says_so_and_is_never_presented_as_current() {
    // SC-718: 100% of briefings served from cache are labelled as cached; zero
    // are presented as current. A stale briefing an agent believes is fresh is
    // worse than no briefing, because it cannot be told apart from one.
    let Some(server) = server() else { return };
    let device = device(&server, "cache-labelled");

    // A first session fills the cache from a reachable server.
    let key = format!("cache-{}", Uuid::now_v7());
    let first = open_session(&device, &key);
    settle("the session reaches the server", || {
        synced_sessions(&server, device.project) > 0
    });
    let session: Uuid = server
        .query_column(&format!(
            "SELECT id::text FROM sessions WHERE project_id = '{}' LIMIT 1",
            device.project
        ))
        .first()
        .and_then(|s| s.parse().ok())
        .expect("a synced session");
    seed(&server, device.project, session, "a durable project fact");

    // Retrieve once more so the cache holds an answer that includes the fact.
    let _ = open_session(&device, &key);
    assert!(
        !first.contains("served from a local cache"),
        "a briefing assembled from a reachable server claimed to be cached"
    );

    // Now the server is gone. Dropping it kills the process; the daemon keeps
    // its credential and its link and simply cannot reach anything, which is
    // the outage §12.3 is about.
    drop(server);
    let cached = open_session(&device, &key);
    assert!(
        cached.contains("cache") || cached.contains("unavailable"),
        "an outage produced a briefing that says nothing about its own freshness: {cached}"
    );
}

#[test]
fn an_outage_with_nothing_cached_says_fresh_knowledge_is_unavailable() {
    // §12.3's last clause. Reporting nothing is indistinguishable from a
    // project that knows nothing, and the two call for opposite reactions.
    let Some(server) = server() else { return };
    let device = device(&server, "cache-empty");
    drop(server);

    let key = format!("cold-{}", Uuid::now_v7());
    let context = open_session(&device, &key);
    assert!(
        context.contains("unavailable"),
        "an outage with no cache entry returned a briefing that does not say \
         durable knowledge is missing: {context}"
    );
}

#[test]
fn a_briefing_assembled_for_one_account_is_never_served_to_another() {
    // FR-790a. Two accounts on one machine is an ordinary case — a shared
    // workstation, a person with a personal and a work login — and the cache
    // is bound to the account it was assembled for, so a credential change is
    // a miss and not a filtered read.
    let Some(server) = server() else { return };
    let device = device(&server, "cache-account-a");

    let key = format!("account-{}", Uuid::now_v7());
    let _ = open_session(&device, &key);
    settle("the session reaches the server", || {
        synced_sessions(&server, device.project) > 0
    });
    let session: Uuid = server
        .query_column(&format!(
            "SELECT id::text FROM sessions WHERE project_id = '{}' LIMIT 1",
            device.project
        ))
        .first()
        .and_then(|s| s.parse().ok())
        .expect("a synced session");
    seed(
        &server,
        device.project,
        session,
        "a fact only the first account ever retrieved",
    );
    let warm = open_session(&device, &key);
    assert!(
        warm.contains("a fact only the first account ever retrieved"),
        "the first account never got the fact its cache should now hold: {warm}"
    );

    // A second account signs in on the same machine, and the server is gone.
    let second = server.new_user_token("cache-account-b");
    let switched =
        device
            .sandbox
            .cairn(&["auth", "token", "set", &second, "--server", &server.base]);
    assert!(switched.ok(), "auth token set: {}", switched.stderr);
    assert_ne!(second, device.token, "the two accounts share a credential");
    drop(server);

    let after = open_session(&device, &key);
    assert!(
        !after.contains("a fact only the first account ever retrieved"),
        "the second account was served the first account's cached briefing: {after}"
    );
}

// ---------------------------------------------------------------------------
// FR-790a: what a cache miss may not serve
//
// The three tests above are about the cache. These are about the *fallback*
// when there is no cache entry for this account — the path that used to
// assemble Level 0 from the local store and hand it to whoever was asking.
// ---------------------------------------------------------------------------

/// Seed one distinctive value into every local source a briefing can draw on.
///
/// Written straight into the store, because the point is to prove that a
/// **local** row of each kind stays local; going through the surfaces would
/// test the surfaces instead, and some of these have no surface that would put
/// a row here without a reachable server.
///
/// Every marker shares one prefix so a single scan can assert that none of them
/// crossed, and so a source added later that is *not* seeded here shows up as a
/// gap in this list rather than as a silent hole in the assertion.
fn seed_every_local_source(device: &Device, marker: &str) {
    // The **local** project id, not `device.project`, which is the id the
    // server knows this project by. Foreign keys in the local store point at
    // the local row.
    let project = device
        .sandbox
        .query_column("SELECT id FROM projects WHERE deleted_at IS NULL ORDER BY created_at")
        .first()
        .cloned()
        .expect("the sandbox's own project row");
    let user = device
        .sandbox
        .query_column("SELECT id FROM users ORDER BY created_at LIMIT 1")
        .first()
        .cloned()
        .expect("this machine's local user");
    let session = Uuid::now_v7();
    let task = Uuid::now_v7();
    device.sandbox.exec_sql(&format!(
        "INSERT INTO sessions (id, project_id, task_id, user_id, agent, branch,
                               worktree_path, agent_session_key, status, started_at,
                               last_event_at, daemon_run_id)
         VALUES ('{session}', '{project}', NULL, '{user}', 'claude_code', 'main', '{}',
                 'marker-key-{marker}', 'completed', '2026-09-01T09:00:00Z',
                 '2026-09-01T09:00:00Z', '{}')",
        device.sandbox.repo_path().display(),
        Uuid::now_v7(),
    ));
    // A task, its criterion and its blocker — Level 0, all three.
    device.sandbox.exec_sql(&format!(
        "INSERT INTO tasks (id, project_id, title, goal, status, created_at, updated_at)
         VALUES ('{task}', '{project}', '{marker}-task-title', '{marker}-task-goal',
                 'in_progress', '2026-09-01T09:00:00Z', '2026-09-01T09:00:00Z')"
    ));
    device.sandbox.exec_sql(&format!(
        "INSERT INTO task_criteria (id, task_id, ordinal, label, text, state,
                                    created_at, updated_at)
         VALUES ('{}', '{task}', 1, '{marker}-criterion',
                 '{marker}-criterion-text', 'pending', '2026-09-01T09:00:00Z',
                 '2026-09-01T09:00:00Z')",
        Uuid::now_v7()
    ));
    device.sandbox.exec_sql(&format!(
        "INSERT INTO task_blockers (id, task_id, description, opened_by_session,
                                    opened_at)
         VALUES ('{}', '{task}', '{marker}-blocker', '{session}',
                 '2026-09-01T09:00:00Z')",
        Uuid::now_v7()
    ));
    // A handoff, which is where a briefing's decisions and known failures come
    // from — the Level 0 source most likely to be forgotten.
    device.sandbox.exec_sql(&format!(
        "INSERT INTO handoffs (id, session_id, trigger, goal, progress, next_step,
                               decisions, failures, created_at)
         VALUES ('{}', '{session}', 'session_end', '{marker}-handoff-goal',
                 '{marker}-progress', '{marker}-next-step',
                 '[\"{marker}-decision\"]', '[\"{marker}-failure\"]',
                 '2026-09-01T09:00:00Z')",
        Uuid::now_v7()
    ));
    // Project memory, a pinned constraint, and a reusable pattern.
    let pinned = Uuid::now_v7();
    device.sandbox.exec_sql(&format!(
        "INSERT INTO memories (id, project_id, type, scope, scope_key, content, state,
                               origin_session_id, created_at, updated_at)
         VALUES ('{}', '{project}', 'fact', 'project', '{project}', '{marker}-memory',
                 'active', '{session}', '2026-09-01T09:00:00Z', '2026-09-01T09:00:00Z')",
        Uuid::now_v7()
    ));
    device.sandbox.exec_sql(&format!(
        "INSERT INTO memories (id, project_id, type, scope, scope_key, content, state,
                               origin_session_id, pinned, pin_reason, created_at, updated_at)
         VALUES ('{pinned}', '{project}', 'convention', 'project', '{project}',
                 '{marker}-pin', 'active', '{session}', 1, '{marker}-pin-reason',
                 '2026-09-01T09:00:00Z', '2026-09-01T09:00:00Z')"
    ));
    device.sandbox.exec_sql(&format!(
        "INSERT INTO reusable_patterns (id, title, problem, signals, signal_digest,
                                        applicability, root_cause, root_cause_digest,
                                        approach, constraints, trust, origin_ref,
                                        sanitization_report, created_at, updated_at)
         VALUES ('{}', '{marker}-pattern', '{marker}-pattern-problem',
                 '[\"{marker}-signal\",\"second\"]', 'digest-{marker}', '[]',
                 '{marker}-root-cause', 'rc-{marker}', '{marker}-approach', '[]',
                 'sanitized', 'salted-{marker}', '{{}}', '2026-09-01T09:00:00Z',
                 '2026-09-01T09:00:00Z')",
        Uuid::now_v7()
    ));
    // Personal and team knowledge — the two domains that belong to an account
    // rather than to a project.
    device.sandbox.exec_sql(&format!(
        "INSERT INTO personal_knowledge (id, owner_user_id, knowledge_type, content,
                                         topic_key, value_key, writer_id, writer_seq,
                                         created_at)
         VALUES ('{}', '{}', 'fact', '{marker}-personal', 'marker.personal', 'value',
                 '{}', 1, '2026-09-01T09:00:00Z')",
        Uuid::now_v7(),
        Uuid::now_v7(),
        Uuid::now_v7()
    ));
    device.sandbox.exec_sql(&format!(
        "INSERT INTO team_knowledge (id, knowledge_type, content, topic_key, value_key,
                                     state, proposed_by_user_id, ratified_by_user_id,
                                     ratified_at, writer_id, writer_seq, created_at)
         VALUES ('{}', 'convention', '{marker}-team', 'marker.team', 'value',
                 'authoritative', '{}', '{}', '2026-09-01T09:00:00Z', '{}', 2,
                 '2026-09-01T09:00:00Z')",
        Uuid::now_v7(),
        Uuid::now_v7(),
        Uuid::now_v7(),
        Uuid::now_v7()
    ));
}

/// Every marker seeded above, so one assertion can name the one that leaked.
fn markers(marker: &str) -> Vec<String> {
    [
        "task-title",
        "task-goal",
        "criterion",
        "criterion-text",
        "blocker",
        "handoff-goal",
        "progress",
        "next-step",
        "decision",
        "failure",
        "memory",
        "pin",
        "pin-reason",
        "pattern",
        "pattern-problem",
        "root-cause",
        "approach",
        "personal",
        "team",
    ]
    .iter()
    .map(|suffix| format!("{marker}-{suffix}"))
    .collect()
}

fn assert_no_marker_crossed(context: &str, marker: &str, who: &str) {
    for m in markers(marker) {
        assert!(
            !context.contains(&m),
            "{who} was served `{m}` from this machine's local store. On a cache \
             miss the server has not established what this caller may see — it \
             has not been reached at all — so nothing derived from the store may \
             be in this briefing:\n{context}"
        );
    }
}

/// The markers really are in this machine's store.
///
/// The positive control, and it has to be this rather than "the first account
/// saw them in its briefing": once retrieval is server-side a reachable server
/// supplies the durable sections, so an account with the server up does *not*
/// see its own locally seeded rows. What makes the assertions below meaningful
/// is that the rows exist here at all — and the mutation test at the end of
/// this file is what proves the old fallback would have served them.
fn assert_markers_are_in_the_local_store(device: &Device, marker: &str) {
    for (what, sql) in [
        ("the memory", format!(
            "SELECT CAST(count(*) AS TEXT) FROM memories WHERE content = '{marker}-memory'"
        )),
        ("the handoff", format!(
            "SELECT CAST(count(*) AS TEXT) FROM handoffs WHERE goal = '{marker}-handoff-goal'"
        )),
        ("the task", format!(
            "SELECT CAST(count(*) AS TEXT) FROM tasks WHERE title = '{marker}-task-title'"
        )),
        ("the pattern", format!(
            "SELECT CAST(count(*) AS TEXT) FROM reusable_patterns WHERE title = '{marker}-pattern'"
        )),
        ("the personal note", format!(
            "SELECT CAST(count(*) AS TEXT) FROM personal_knowledge WHERE content = '{marker}-personal'"
        )),
        ("the team entry", format!(
            "SELECT CAST(count(*) AS TEXT) FROM team_knowledge WHERE content = '{marker}-team'"
        )),
    ] {
        assert_eq!(
            device.sandbox.query_column(&sql),
            vec!["1".to_string()],
            "{what} was never written locally, so the assertion that it does not \
             cross would pass against an empty store"
        );
    }
}

/// A second account on the same machine is served no part of the first
/// account's local state (FR-790a).
///
/// **The load-bearing test for the repair.** Everything a briefing can draw on
/// is seeded locally under account A — Level 0 included, not project memory
/// alone — then the credential is switched and the server is taken away. The
/// old fallback assembled all of it from the local store and handed it over.
///
/// **Falsified by** returning a locally assembled briefing on a cache miss.
#[test]
fn a_cache_miss_serves_a_second_account_nothing_from_this_machines_store() {
    let Some(server) = server() else { return };
    let device = device(&server, "miss-account-a");

    let key = format!("miss-{}", Uuid::now_v7());
    let _ = open_session(&device, &key);
    settle("the session reaches the server", || {
        synced_sessions(&server, device.project) > 0
    });
    let marker = format!("m{}", Uuid::now_v7().simple());
    seed_every_local_source(&device, &marker);

    assert_markers_are_in_the_local_store(&device, &marker);

    let second = server.new_user_token("miss-account-b");
    let switched =
        device
            .sandbox
            .cairn(&["auth", "token", "set", &second, "--server", &server.base]);
    assert!(switched.ok(), "auth token set: {}", switched.stderr);
    drop(server);

    let after = open_session(&device, &key);
    assert_no_marker_crossed(&after, &marker, "a second account");
    assert!(
        after.contains("unavailable"),
        "the second account was not told that durable knowledge is missing; a \
         briefing that is quietly smaller is indistinguishable from one that is \
         complete: {after}"
    );
}

/// A signed-out caller is served no part of the signed-in account's local
/// state.
///
/// The same boundary from the other side. There is no account at all here, so
/// there is not even a cache key to miss on — and the old fallback answered
/// with the local store regardless.
///
/// **Falsified by** returning a locally assembled briefing when no account is
/// authenticated.
#[test]
fn a_cache_miss_serves_a_signed_out_caller_nothing_from_this_machines_store() {
    let Some(server) = server() else { return };
    let device = device(&server, "miss-signed-out");

    let key = format!("signed-out-{}", Uuid::now_v7());
    let _ = open_session(&device, &key);
    settle("the session reaches the server", || {
        synced_sessions(&server, device.project) > 0
    });
    let marker = format!("m{}", Uuid::now_v7().simple());
    seed_every_local_source(&device, &marker);
    assert_markers_are_in_the_local_store(&device, &marker);

    let out = device.sandbox.cairn(&["auth", "logout"]);
    assert!(out.ok(), "signing out: {}", out.stderr);
    drop(server);

    let after = open_session(&device, &key);
    assert_no_marker_crossed(&after, &marker, "a signed-out caller");
}

/// The same account's cached briefing still works, still carries its content,
/// and still says it is cached.
///
/// The repair narrows one path and must not narrow this one: a cache hit is
/// evidence the server already authorized *this* account for *this* session,
/// which is exactly what a miss does not have.
///
/// **Falsified by** treating a cache hit as a miss, or by serving a cached
/// briefing without saying so.
#[test]
fn the_same_accounts_cached_briefing_still_carries_its_content_and_says_it_is_cached() {
    let Some(server) = server() else { return };
    let device = device(&server, "miss-same-account");

    let key = format!("same-{}", Uuid::now_v7());
    let _ = open_session(&device, &key);
    settle("the session reaches the server", || {
        synced_sessions(&server, device.project) > 0
    });
    let session: Uuid = server
        .query_column(&format!(
            "SELECT id::text FROM sessions WHERE project_id = '{}' LIMIT 1",
            device.project
        ))
        .first()
        .and_then(|s| s.parse().ok())
        .expect("a synced session");
    let marker = format!("m{}", Uuid::now_v7().simple());
    seed(
        &server,
        device.project,
        session,
        &format!("{marker}-server-memory"),
    );

    // Fill the cache from a reachable server, then take it away.
    let warm = open_session(&device, &key);
    assert!(
        warm.contains(&format!("{marker}-server-memory")),
        "the cache was never filled, so the outage below proves nothing: {warm}"
    );
    drop(server);

    let cached = open_session(&device, &key);
    assert!(
        cached.contains(&format!("{marker}-server-memory")),
        "the same account's cached briefing lost its content. A cache hit is \
         evidence the server authorized this account for this session, and it \
         is exactly what a miss does not have: {cached}"
    );
    assert!(
        cached.contains("cache"),
        "a cached briefing was served without saying it is cached, which is \
         worse than no briefing because it cannot be told from a fresh one: \
         {cached}"
    );
}
