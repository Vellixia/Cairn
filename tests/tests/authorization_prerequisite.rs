//! Regression tests for the authorization prerequisite hotfix.
//!
//! Five defects shipped in v0.1.0-alpha.5, and together they were a complete
//! compromise chain: register an account, look a project up by its public git
//! remote, join it, then read and write everything in it — including
//! everything in *other* projects, because the sync ingest path never re-checked
//! which project a client-supplied id belonged to.
//!
//! Every test here is written so that reverting its fix makes it fail. That is
//! the only property that makes a security regression test worth having, and it
//! is stated per test rather than assumed. Feature 003's `scope_audit.rs` is the
//! cautionary case: it split on a function name that did not exist, compared
//! against `""`, and passed for its entire life.

use cairn_e2e::{post_json_bearer, post_status_anon, post_status_bearer, Server};
use serde_json::json;
use uuid::Uuid;

fn server() -> Option<Server> {
    match Server::start() {
        Some(s) => Some(s),
        None => {
            eprintln!("SKIPPED: set CAIRN_TEST_DATABASE_URL to run the server suite");
            None
        }
    }
}

/// A user, their token, and a project they own.
struct Owner {
    token: String,
    project: Uuid,
}

fn owner(server: &Server, label: &str, remote: &str) -> Owner {
    let token = server.new_user_token(label);
    let created = post_json_bearer(
        &server.base,
        "/api/projects",
        &json!({ "name": label, "repository_remote": remote }),
        &token,
    );
    let project = created["id"]
        .as_str()
        .unwrap_or_else(|| panic!("no project id in {created}"))
        .parse()
        .expect("project uuid");
    Owner { token, project }
}

/// Seed a memory, and the session its `origin_session_id` requires.
///
/// `memories.origin_session_id` is `NOT NULL`, so a memory cannot be seeded
/// alone. The session belongs to the same project, which is what makes the row
/// a realistic victim rather than an orphan the guard might reject for an
/// unrelated reason.
fn seed_memory(server: &Server, project: Uuid, id: Uuid, content: &str) {
    let session = Uuid::now_v7();
    server.execute(&format!(
        "INSERT INTO sessions (id, project_id, agent, branch, status, started_at)
         VALUES ('{session}', '{project}', 'test', 'main', 'active', now())"
    ));
    server.execute(&format!(
        "INSERT INTO memories
             (id, project_id, type, scope, scope_key, content, state, origin_session_id)
         VALUES ('{id}', '{project}', 'convention', 'project', '', '{}', 'active', '{session}')",
        content.replace('\'', "''")
    ));
}

/// One sync item, addressed to `project`.
fn batch(
    project: Uuid,
    entity_type: &str,
    entity_id: Uuid,
    op: &str,
    payload: serde_json::Value,
) -> serde_json::Value {
    json!({
        "project_id": project,
        "items": [{
            "idempotency_key": format!("k-{}", Uuid::now_v7()),
            "entity_type": entity_type,
            "entity_id": entity_id,
            "operation": op,
            "payload": payload,
        }],
    })
}

fn statuses(response: &serde_json::Value) -> Vec<String> {
    response["results"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .map(|r| r["status"].as_str().unwrap_or("?").to_string())
                .collect()
        })
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Defect 1 — unauthenticated public self-registration
// ---------------------------------------------------------------------------

/// Restoring `POST /api/auth/register` makes this fail: the route would answer
/// 200 (or 400 on a short password) instead of 404.
#[test]
fn there_is_no_route_that_creates_an_account() {
    let Some(server) = server() else { return };
    let status = post_status_anon(
        &server.base,
        "/api/auth/register",
        &json!({ "email": "intruder@example.test", "display_name": "Intruder",
                 "password": "hunter2hunter2" }),
    );
    assert_eq!(status, 404, "self-registration answered {status}");
    assert_eq!(
        server.count("SELECT count(*) FROM users WHERE email = 'intruder@example.test'"),
        0,
        "an account was created by an unauthenticated request"
    );
}

// ---------------------------------------------------------------------------
// Defect 2 — any authenticated user could join any project by UUID
// ---------------------------------------------------------------------------

/// Restoring `POST /api/projects/{id}/join` makes this fail twice: the status
/// assertion, and the membership count that proves nothing was granted.
#[test]
fn naming_a_project_uuid_does_not_grant_membership() {
    let Some(server) = server() else { return };
    let alice = owner(&server, "join-alice", "git@example.test:alice/one.git");
    let bob = server.new_user_token("join-bob");

    let status = post_status_bearer(
        &server.base,
        &format!("/api/projects/{}/join", alice.project),
        &json!({}),
        &bob,
    );
    assert_eq!(status, 404, "the join route answered {status}");
    assert_eq!(
        server.count(&format!(
            "SELECT count(*) FROM project_members WHERE project_id = '{}'",
            alice.project
        )),
        1,
        "membership changed; only Alice should be a member"
    );
}

// ---------------------------------------------------------------------------
// Defect 3 — discovery leaked project UUIDs for any git remote
// ---------------------------------------------------------------------------

/// Dropping the membership join from `lookup_projects` makes this fail: the
/// non-member would see Alice's project, which is the input the join route
/// needed. A git remote is not a secret — it is in every clone.
#[test]
fn discovery_returns_nothing_to_a_non_member() {
    let Some(server) = server() else { return };
    let remote = format!("git@example.test:alice/{}.git", Uuid::now_v7());
    let alice = owner(&server, "lookup-alice", &remote);
    let bob = server.new_user_token("lookup-bob");

    let mine = server.get_json(
        &format!("/api/projects/lookup?remote={remote}"),
        &alice.token,
    );
    assert_eq!(
        mine["projects"].as_array().map(|a| a.len()),
        Some(1),
        "a member cannot see their own project: {mine}"
    );

    let theirs = server.get_json(&format!("/api/projects/lookup?remote={remote}"), &bob);
    assert_eq!(
        theirs["projects"].as_array().map(|a| a.len()),
        Some(0),
        "discovery leaked a project to a non-member: {theirs}"
    );
}

// ---------------------------------------------------------------------------
// Defect 4 — tombstone had no project predicate
// ---------------------------------------------------------------------------

/// Removing `AND project_id = $2` from `tombstone` makes this fail: Bob's
/// delete would blank Alice's memory content and set `deleted_at`.
///
/// This was the most destructive of the five, because a tombstone clears the
/// content as well as marking the row deleted — the data does not come back.
#[test]
fn a_tombstone_cannot_reach_another_projects_memory() {
    let Some(server) = server() else { return };
    let alice = owner(&server, "tomb-alice", "git@example.test:alice/two.git");
    let bob = owner(&server, "tomb-bob", "git@example.test:bob/two.git");

    let victim = Uuid::now_v7();
    seed_memory(&server, alice.project, victim, "alice's content");

    let response = post_json_bearer(
        &server.base,
        "/api/sync/batch",
        &batch(bob.project, "memory", victim, "delete", json!({})),
        &bob.token,
    );
    // The item is accepted as a no-op rather than refused: a tombstone is
    // idempotent by contract, and answering "forbidden" would confirm that the
    // id exists in a project the caller cannot see.
    assert!(
        !statuses(&response).is_empty(),
        "no result for the delete: {response}"
    );
    assert_eq!(
        server.text(&format!(
            "SELECT content FROM memories WHERE id = '{victim}'"
        )),
        "alice's content",
        "another project's delete cleared the content"
    );
    assert_eq!(
        server.count(&format!(
            "SELECT count(*) FROM memories WHERE id = '{victim}' AND deleted_at IS NULL"
        )),
        1,
        "another project's delete tombstoned the row"
    );
}

// ---------------------------------------------------------------------------
// Defect 5 — `ON CONFLICT (id) DO UPDATE` never re-checked the project
// ---------------------------------------------------------------------------

/// Removing `WHERE tasks.project_id = $2` from the `DO UPDATE` makes this fail:
/// Bob's upsert would rewrite the title of Alice's task, because the insert
/// conflicts on `id` and the update fired unconditionally. `project_id` is not
/// in the `SET` list, so the row stayed in Alice's project wearing Bob's data.
#[test]
fn an_upsert_cannot_overwrite_another_projects_task() {
    let Some(server) = server() else { return };
    let alice = owner(&server, "task-alice", "git@example.test:alice/three.git");
    let bob = owner(&server, "task-bob", "git@example.test:bob/three.git");

    let victim = Uuid::now_v7();
    server.execute(&format!(
        "INSERT INTO tasks (id, project_id, title, goal, status)
         VALUES ('{victim}', '{}', 'alice''s task', 'alice''s goal', 'todo')",
        alice.project
    ));

    let response = post_json_bearer(
        &server.base,
        "/api/sync/batch",
        &batch(
            bob.project,
            "task",
            victim,
            "upsert",
            json!({ "title": "bob's rewrite", "goal": "bob's goal",
                    "acceptance_criteria": [], "status": "todo" }),
        ),
        &bob.token,
    );
    assert_eq!(
        statuses(&response),
        vec!["rejected"],
        "a cross-project upsert was not rejected: {response}"
    );
    assert_eq!(
        server.text(&format!("SELECT title FROM tasks WHERE id = '{victim}'")),
        "alice's task",
        "another project's upsert rewrote the task"
    );
}

/// The same hole in `upsert_memory`, which has two SQL branches — one per
/// schema version. Both carry the predicate; this exercises the one this
/// server's schema uses.
#[test]
fn an_upsert_cannot_overwrite_another_projects_memory() {
    let Some(server) = server() else { return };
    let alice = owner(&server, "mem-alice", "git@example.test:alice/four.git");
    let bob = owner(&server, "mem-bob", "git@example.test:bob/four.git");

    let victim = Uuid::now_v7();
    seed_memory(&server, alice.project, victim, "alice's memory");

    let response = post_json_bearer(
        &server.base,
        "/api/sync/batch",
        &batch(
            bob.project,
            "memory",
            victim,
            "upsert",
            json!({ "type": "convention", "scope": "project", "scope_key": "",
                    "content": "bob's rewrite", "state": "active",
                    "provenance": { "session_id": Uuid::now_v7() } }),
        ),
        &bob.token,
    );
    assert_eq!(
        statuses(&response),
        vec!["rejected"],
        "a cross-project memory upsert was not rejected: {response}"
    );
    assert_eq!(
        server.text(&format!(
            "SELECT content FROM memories WHERE id = '{victim}'"
        )),
        "alice's memory",
        "another project's upsert rewrote the memory"
    );
}

/// Removing the `all_in_project` check on `task_id` makes this fail: a criterion
/// row would be created against Alice's task from Bob's project. The `DO UPDATE`
/// predicate cannot catch this one, because there is no existing row to conflict
/// with — it is an insert, not an overwrite.
#[test]
fn a_criterion_cannot_attach_to_another_projects_task() {
    let Some(server) = server() else { return };
    let alice = owner(&server, "crit-alice", "git@example.test:alice/five.git");
    let bob = owner(&server, "crit-bob", "git@example.test:bob/five.git");

    let victim_task = Uuid::now_v7();
    server.execute(&format!(
        "INSERT INTO tasks (id, project_id, title, goal, status)
         VALUES ('{victim_task}', '{}', 'alice''s task', 'g', 'todo')",
        alice.project
    ));

    let response = post_json_bearer(
        &server.base,
        "/api/sync/batch",
        &batch(
            bob.project,
            "task_criterion",
            Uuid::now_v7(),
            "upsert",
            json!({ "task_id": victim_task, "ordinal": 1, "label": "c1",
                    "text": "bob's criterion", "state": "pending" }),
        ),
        &bob.token,
    );
    assert_eq!(
        statuses(&response),
        vec!["rejected"],
        "a criterion attached to another project's task: {response}"
    );
    assert_eq!(
        server.count(&format!(
            "SELECT count(*) FROM task_criteria WHERE task_id = '{victim_task}'"
        )),
        0,
        "a criterion row was created against another project's task"
    );
}

/// Removing the `all_in_project` check on a relation's endpoints makes this
/// fail. A relation conflicts `DO NOTHING`, so `rows_affected` cannot
/// distinguish a refused write from a legitimate duplicate — the endpoints have
/// to be checked directly, which also forbids a relation spanning two projects
/// rather than only one that overwrites.
#[test]
fn a_relation_cannot_name_another_projects_memory() {
    let Some(server) = server() else { return };
    let alice = owner(&server, "rel-alice", "git@example.test:alice/six.git");
    let bob = owner(&server, "rel-bob", "git@example.test:bob/six.git");

    let alice_memory = Uuid::now_v7();
    let bob_memory = Uuid::now_v7();
    seed_memory(&server, alice.project, alice_memory, "a");
    seed_memory(&server, bob.project, bob_memory, "b");

    let response = post_json_bearer(
        &server.base,
        "/api/sync/batch",
        &batch(
            bob.project,
            "memory_relation",
            Uuid::now_v7(),
            "upsert",
            json!({ "from_memory_id": bob_memory, "to_memory_id": alice_memory,
                    "kind": "duplicates", "decided_by_session": Uuid::now_v7(),
                    "basis": "b" }),
        ),
        &bob.token,
    );
    assert_eq!(
        statuses(&response),
        vec!["rejected"],
        "a relation spanned two projects: {response}"
    );
    assert_eq!(
        server.count(&format!(
            "SELECT count(*) FROM memory_relations WHERE to_memory_id = '{alice_memory}'"
        )),
        0,
        "a cross-project relation row was created"
    );
}

// ---------------------------------------------------------------------------
// The legitimate paths still work
// ---------------------------------------------------------------------------

/// The guards refuse the right thing only if they still admit the right thing.
/// A test suite that proved the refusals without this one would pass on a
/// server that refused every write.
#[test]
fn a_projects_own_upserts_and_deletes_still_apply() {
    let Some(server) = server() else { return };
    let alice = owner(&server, "own-alice", "git@example.test:alice/seven.git");

    let own = Uuid::now_v7();
    let first = post_json_bearer(
        &server.base,
        "/api/sync/batch",
        &batch(
            alice.project,
            "task",
            own,
            "upsert",
            json!({ "title": "mine", "goal": "g", "acceptance_criteria": [],
                    "status": "todo" }),
        ),
        &alice.token,
    );
    assert_eq!(statuses(&first), vec!["applied"], "own insert: {first}");

    let again = post_json_bearer(
        &server.base,
        "/api/sync/batch",
        &batch(
            alice.project,
            "task",
            own,
            "upsert",
            json!({ "title": "mine, edited", "goal": "g", "acceptance_criteria": [],
                    "status": "todo" }),
        ),
        &alice.token,
    );
    assert_eq!(statuses(&again), vec!["applied"], "own re-upsert: {again}");
    assert_eq!(
        server.text(&format!("SELECT title FROM tasks WHERE id = '{own}'")),
        "mine, edited",
        "a project could not edit its own task"
    );

    let deleted = post_json_bearer(
        &server.base,
        "/api/sync/batch",
        &batch(alice.project, "task", own, "delete", json!({})),
        &alice.token,
    );
    assert_eq!(statuses(&deleted), vec!["applied"], "own delete: {deleted}");
    assert_eq!(
        server.count(&format!(
            "SELECT count(*) FROM tasks WHERE id = '{own}' AND deleted_at IS NOT NULL"
        )),
        1,
        "a project could not delete its own task"
    );
}
