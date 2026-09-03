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
