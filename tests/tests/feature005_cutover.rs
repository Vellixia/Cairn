//! The cutover switch itself, and what flipping it does and does not do
//! (T146, User Story 7, `contracts/migration-cutover.md` §1-§3, §11.9, §12.1;
//! SC-746, SC-747, FR-876, FR-876b1, FR-876c, FR-877).
//!
//! `POST /api/admin/cutover` is one administrator action that turns a whole
//! deployment's dual-authority write path off, forever, with no route back
//! (§2). Everything in this file is a consequence of that switch being real
//! but narrow:
//!
//! - **It refuses a shape, not a client version.** `personal_knowledge`,
//!   `team_knowledge`, `memory` and `memory_relation` — upsert or delete — are
//!   refused after cutover, and nothing else is, because the server cannot
//!   reliably learn which binary is calling and does not try (§3.1).
//! - **The refusal has a code an upgraded client can act on**, and that code
//!   is not the one a pre-005 capability gate already uses for a different
//!   reason. `unknown_entity_type` says "wait"; `upgrade_required` says "stop
//!   retrying, migrate" (§3.2, FR-876b1). Collapsing them would make an
//!   upgraded client indistinguishable from a deployment that simply is not
//!   there yet.
//! - **Producing the refusal is read-only** (§3.3, SC-747). A refused request
//!   is not almost a no-op — it is a no-op, verified by comparing the whole
//!   of every table it could have touched, before and after.
//! - **The switch is one compare-and-swap**, exactly like `ratify_team` and
//!   `retire_team` before it (§2). A second call is not an error; it is the
//!   same answer the first call already gave.
//! - **The drain route survives the refusal it is the escape from** (§12.1).
//!   Without an exemption, the refusal would be self-perpetuating: a client
//!   migrating *after* cutover would be refused its own migration traffic and
//!   could never stop being a pre-005 caller.
//! - **Reads are never refused** (§11.9). A demoted personal or team replica
//!   is a cache, and a cache with no read path can never refill.
//!
//! What would falsify this file as a whole: any knowledge-bearing write
//! silently accepted after cutover, any row changed by a refusal that did
//! nothing else, or a second `/api/admin/cutover` call that re-demotes or
//! re-audits anything.

use cairn_e2e::feature005::{Account, Pg};
use cairn_e2e::{get_json_status_bearer, post_json_status_bearer, Server};
use serde_json::{json, Value};
use uuid::Uuid;

macro_rules! pg {
    () => {
        match Pg::start() {
            Some(pg) => pg,
            None => {
                eprintln!("skipped: CAIRN_TEST_DATABASE_URL is not set");
                return;
            }
        }
    };
}

// ---------------------------------------------------------------------------
// Admin, sync batch and its per-item envelope
// ---------------------------------------------------------------------------

/// Promote one account to administrator, directly.
///
/// Routing this through an admin route would make every admin assertion
/// depend on another admin already existing, and `CurrentUser::role` is read
/// fresh from `users` on every request (`auth.rs`), so a plain `UPDATE` is
/// exactly as good a credential as one minted through the API.
fn make_admin(pg: &Pg, who: &Account) {
    pg.server.execute(&format!(
        "UPDATE users SET role = 'admin' WHERE id = '{}'",
        who.id
    ));
}

/// Cut this deployment over, as the given (already-admin) account.
fn cutover_as(pg: &Pg, who: &Account) -> (Value, u16) {
    post_json_status_bearer(
        &pg.server.base,
        "/api/admin/cutover",
        &json!({}),
        &who.token,
    )
}

/// One `sync/batch` item, in the wire shape `sync::SyncItem` deserializes.
fn item(entity_type: &str, entity_id: Uuid, operation: &str, payload: Value) -> Value {
    json!({
        "idempotency_key": format!("k-{}", Uuid::now_v7()),
        "entity_type": entity_type,
        "entity_id": entity_id,
        "operation": operation,
        "payload": payload,
    })
}

fn batch_of(project: Uuid, items: Vec<Value>) -> Value {
    json!({ "project_id": project, "items": items })
}

/// Post a batch of items to the fixture project, as `who`.
fn post_batch(pg: &Pg, who: &Account, items: Vec<Value>) -> (Value, u16) {
    post_json_status_bearer(
        &pg.server.base,
        "/api/sync/batch",
        &batch_of(pg.project, items),
        &who.token,
    )
}

/// The `results` array of a `sync/batch` reply, one entry per item posted, in
/// order.
///
/// `sync/batch` never answers with a top-level error status for a single bad
/// item (`authorization_prerequisite.rs`'s own `statuses` helper makes the same
/// point): a batch of ten items where one is refused must not fail the other
/// nine, so the per-item verdict lives in this array and the HTTP status of
/// the call itself stays `200`.
fn results(body: &Value) -> Vec<Value> {
    body["results"].as_array().cloned().unwrap_or_default()
}

fn status_of(result: &Value) -> &str {
    result["status"].as_str().unwrap_or("?")
}

fn error_code_of(result: &Value) -> &str {
    result["error"]["code"].as_str().unwrap_or("")
}

// ---------------------------------------------------------------------------
// Legal payloads, one builder per entity type this file exercises
// ---------------------------------------------------------------------------

/// A `personal_knowledge` payload with a writer identity fresh enough that
/// `UNIQUE (writer_id, writer_seq)` never collides between calls, whatever
/// `writer_seq` is — so every caller of this can just say `1`.
fn personal_payload(content: &str) -> Value {
    json!({
        "knowledge_type": "fact",
        "content": content,
        "writer_id": format!("cutover-fixture-{}", Uuid::now_v7()),
        "writer_seq": 1,
    })
}

fn team_payload(content: &str) -> Value {
    json!({
        "knowledge_type": "convention",
        "content": content,
        "writer_id": format!("cutover-fixture-{}", Uuid::now_v7()),
        "writer_seq": 1,
    })
}

fn memory_payload(session: Uuid, content: &str) -> Value {
    json!({
        "type": "fact",
        "scope": "project",
        "scope_key": "",
        "content": content,
        "state": "active",
        "provenance": { "session_id": session },
    })
}

fn relation_payload(from: Uuid, to: Uuid, session: Uuid) -> Value {
    json!({
        "from_memory_id": from,
        "to_memory_id": to,
        "kind": "supersedes",
        "decided_by_session": session,
        "basis": "deterministic_rule",
    })
}

fn task_payload(title: &str) -> Value {
    json!({ "title": title, "goal": "a goal", "acceptance_criteria": [], "status": "todo" })
}

fn session_payload() -> Value {
    json!({ "agent": "claude-code", "branch": "main", "status": "active" })
}

fn handoff_payload(session: Uuid) -> Value {
    json!({
        "session_id": session,
        "trigger": "session_end",
        "goal": "a goal",
        "progress": "some progress",
        "next_step": "the next step",
    })
}

fn criterion_payload(task_id: Uuid) -> Value {
    json!({
        "task_id": task_id,
        "ordinal": 1,
        "label": "a label",
        "text": "a criterion",
        "state": "pending",
        "verification": "unverified",
    })
}

fn blocker_payload(task_id: Uuid, session: Uuid) -> Value {
    json!({
        "task_id": task_id,
        "description": "a blocker",
        "state": "open",
        "opened_by_session": session,
    })
}

/// A fresh project memory, seeded directly so a relation has two real
/// endpoints without going through `sync/batch` first.
fn seed_memory(pg: &Pg, session: Uuid, content: &str) -> Uuid {
    let id = Uuid::now_v7();
    pg.server.execute(&format!(
        "INSERT INTO memories (id, project_id, type, scope, scope_key, content,
                               origin_session_id)
         VALUES ('{id}', '{}', 'fact', 'project', '{}', '{content}', '{session}')",
        pg.project, pg.project
    ));
    id
}

fn seed_task(pg: &Pg, title: &str) -> Uuid {
    let id = Uuid::now_v7();
    pg.server.execute(&format!(
        "INSERT INTO tasks (id, project_id, title, goal, status)
         VALUES ('{id}', '{}', '{title}', 'a goal', 'todo')",
        pg.project
    ));
    id
}

// ---------------------------------------------------------------------------
// 1. Before cutover, the baseline: knowledge-bearing sync works as it does
//    today (the control that makes the post-cutover assertion mean something)
// ---------------------------------------------------------------------------

/// Every knowledge-bearing shape is accepted before cutover.
///
/// Without this passing, a later test asserting "refused after cutover" could
/// be vacuous — refused for some unrelated reason (a malformed payload, a
/// missing fixture row) that has nothing to do with `server_authority.mode`.
#[test]
fn personal_and_team_knowledge_sync_as_usual_before_cutover() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    let a = seed_memory(&pg, session, "the release job signs images");
    let b = seed_memory(&pg, session, "the signer is cosign");

    let (body, status) = post_batch(
        &pg,
        &pg.owner,
        vec![
            item(
                "memory",
                Uuid::now_v7(),
                "upsert",
                memory_payload(session, "a fresh project memory"),
            ),
            item(
                "memory_relation",
                Uuid::now_v7(),
                "upsert",
                relation_payload(a, b, session),
            ),
            item(
                "personal_knowledge",
                Uuid::now_v7(),
                "upsert",
                personal_payload("the owner prefers one signer"),
            ),
            item(
                "team_knowledge",
                Uuid::now_v7(),
                "upsert",
                team_payload("the team signs every release image"),
            ),
        ],
    );
    assert_eq!(status, 200, "the batch call itself: {body}");
    let outcomes = results(&body);
    let statuses: Vec<&str> = outcomes.iter().map(status_of).collect();
    assert_eq!(
        statuses,
        vec!["applied", "applied", "applied", "applied"],
        "before cutover, every knowledge-bearing shape is accepted exactly as it \
         is today: {body}"
    );
}

// ---------------------------------------------------------------------------
// 2. After cutover, every knowledge-bearing shape is refused, upsert or
//    delete, with no sample left untested (SC-746)
// ---------------------------------------------------------------------------

#[test]
fn every_knowledge_bearing_shape_is_refused_with_upgrade_required_after_cutover() {
    let pg = pg!();
    make_admin(&pg, &pg.owner);
    let session = pg.session_for(&pg.owner);
    let a = seed_memory(&pg, session, "the release job signs images");
    let b = seed_memory(&pg, session, "the signer is cosign");
    let (cutover_body, cutover_status) = cutover_as(&pg, &pg.owner);
    assert_eq!(cutover_status, 200, "cutover itself: {cutover_body}");

    // Every knowledge-bearing entity type, as an upsert and as a delete naming
    // it — the full cross-product §3.1 names, not a sample of it.
    let items = vec![
        (
            "memory upsert",
            item(
                "memory",
                Uuid::now_v7(),
                "upsert",
                memory_payload(session, "a post-cutover memory"),
            ),
        ),
        ("memory delete", item("memory", a, "delete", json!({}))),
        (
            "memory_relation upsert",
            item(
                "memory_relation",
                Uuid::now_v7(),
                "upsert",
                relation_payload(a, b, session),
            ),
        ),
        (
            "memory_relation delete",
            item("memory_relation", a, "delete", json!({})),
        ),
        (
            "personal_knowledge upsert",
            item(
                "personal_knowledge",
                Uuid::now_v7(),
                "upsert",
                personal_payload("a post-cutover personal note"),
            ),
        ),
        (
            "personal_knowledge delete",
            item("personal_knowledge", Uuid::now_v7(), "delete", json!({})),
        ),
        (
            "team_knowledge upsert",
            item(
                "team_knowledge",
                Uuid::now_v7(),
                "upsert",
                team_payload("a post-cutover team claim"),
            ),
        ),
        (
            "team_knowledge delete",
            item("team_knowledge", Uuid::now_v7(), "delete", json!({})),
        ),
    ];
    let labels: Vec<&str> = items.iter().map(|(label, _)| *label).collect();
    let (body, status) = post_batch(&pg, &pg.owner, items.into_iter().map(|(_, i)| i).collect());
    assert_eq!(status, 200, "the batch call itself: {body}");

    let outcomes = results(&body);
    assert_eq!(
        outcomes.len(),
        labels.len(),
        "one result per item, or the assertions below are not about what they \
         claim: {body}"
    );
    for (label, outcome) in labels.iter().zip(outcomes.iter()) {
        assert_eq!(
            status_of(outcome),
            "rejected",
            "{label}: every knowledge-bearing shape must be refused after \
             cutover, not silently accepted (SC-746): {outcome}"
        );
        assert_eq!(
            error_code_of(outcome),
            "upgrade_required",
            "{label}: the refusal must carry `upgrade_required`, not any other \
             code (§3.2): {outcome}"
        );
    }
}

/// `upgrade_required` and `unknown_entity_type` are the same HTTP status and
/// different codes, and an upgraded client's whole retry strategy depends on
/// telling them apart (FR-876b1): one means "hold this and retry after the
/// migration runs", the other means "stop retrying against this route, this
/// store itself must migrate."
///
/// Demonstrated against two real servers rather than by comparing string
/// constants: a schema-2 deployment refusing `personal_knowledge` because it
/// has nowhere to put it yet (the capability gate, `sync.rs`'s
/// `held_until_migrated`), and a cut-over schema-4 deployment refusing the
/// very same shape because dual-authority sync has been retired for it
/// (the cutover refusal). Same entity type, same operation, two different
/// reasons, and the reasons must not collapse into one code.
#[test]
fn upgrade_required_is_distinct_from_unknown_entity_type() {
    // The capability-deferral side: a real schema-2 server, which cannot hold
    // `personal_knowledge` at all yet.
    let Some(old) = Server::start_at_schema(2) else {
        eprintln!("skipped: CAIRN_TEST_DATABASE_URL is not set");
        return;
    };
    let (owner_id, owner_token) = old.new_user("old-owner");
    let old_project = Uuid::now_v7();
    old.execute(&format!(
        "INSERT INTO projects (id, name) VALUES ('{old_project}', 'old-schema-fixture')"
    ));
    old.execute(&format!(
        "INSERT INTO project_members (project_id, user_id) VALUES ('{old_project}', '{owner_id}')"
    ));
    let (old_body, old_status) = post_json_status_bearer(
        &old.base,
        "/api/sync/batch",
        &batch_of(
            old_project,
            vec![item(
                "personal_knowledge",
                Uuid::now_v7(),
                "upsert",
                personal_payload("a note pushed at schema 2"),
            )],
        ),
        &owner_token,
    );
    assert_eq!(old_status, 200, "{old_body}");
    let old_outcome = &results(&old_body)[0];
    assert_eq!(
        status_of(old_outcome),
        "rejected",
        "a schema-2 server accepted personal_knowledge, which it has no table \
         for: {old_body}"
    );
    assert_eq!(
        error_code_of(old_outcome),
        "unknown_entity_type",
        "the capability-deferral refusal must carry its own code: {old_body}"
    );

    // The cutover side: the same shape, refused for the other reason.
    let pg = pg!();
    make_admin(&pg, &pg.owner);
    assert_eq!(cutover_as(&pg, &pg.owner).1, 200);
    let (new_body, new_status) = post_batch(
        &pg,
        &pg.owner,
        vec![item(
            "personal_knowledge",
            Uuid::now_v7(),
            "upsert",
            personal_payload("a note pushed after cutover"),
        )],
    );
    assert_eq!(new_status, 200, "{new_body}");
    let new_outcome = &results(&new_body)[0];
    assert_eq!(status_of(new_outcome), "rejected", "{new_body}");
    let new_code = error_code_of(new_outcome);
    assert_eq!(new_code, "upgrade_required", "{new_body}");

    assert_ne!(
        error_code_of(old_outcome),
        new_code,
        "a capability-deferral refusal and a cutover refusal produced the same \
         code; an upgraded client cannot tell 'wait' from 'stop retrying' \
         (FR-876b1)"
    );
}

// ---------------------------------------------------------------------------
// 3. The refusal touches no row anywhere (§3.3, SC-747)
// ---------------------------------------------------------------------------

/// A full, row-by-row snapshot of every table a refused item could have
/// touched, comparable byte for byte.
///
/// `to_jsonb(t)::text` rather than naming columns, so a column this file does
/// not know about is still covered — the guarantee under test is "nothing in
/// this table changed", and a snapshot that only checked the columns this test
/// happened to name could not tell "nothing changed" from "nothing this test
/// looked at changed."
fn snapshot(pg: &Pg, table: &str) -> Vec<String> {
    pg.server.query_column(&format!(
        "SELECT to_jsonb(t)::text FROM {table} t ORDER BY to_jsonb(t)::text"
    ))
}

#[test]
fn a_refused_batch_leaves_every_row_of_every_table_it_could_have_touched_untouched() {
    let pg = pg!();
    make_admin(&pg, &pg.owner);
    let session = pg.session_for(&pg.owner);

    // Pre-existing rows in all four tables the refusal could reach, seeded
    // *before* cutover so their presence is not itself part of what is under
    // test — only their being left alone is.
    let a = seed_memory(&pg, session, "the release job signs images");
    let b = seed_memory(&pg, session, "the signer is cosign");
    pg.server.execute(&format!(
        "INSERT INTO memory_relations (from_memory_id, to_memory_id, kind, project_id,
                                       decided_by_session, basis)
         VALUES ('{a}', '{b}', 'supersedes', '{}', '{session}', 'deterministic_rule')",
        pg.project
    ));
    pg.server.execute(&format!(
        "INSERT INTO personal_knowledge (id, owner_user_id, knowledge_type, content,
                                         writer_id, writer_seq)
         VALUES ('{}', '{}', 'fact', 'a pre-existing personal note',
                 'snapshot-fixture-personal', 1)",
        Uuid::now_v7(),
        pg.owner.id
    ));
    pg.server.execute(&format!(
        "INSERT INTO team_knowledge (id, knowledge_type, content, proposed_by_user_id,
                                     writer_id, writer_seq)
         VALUES ('{}', 'convention', 'a pre-existing team claim', '{}',
                 'snapshot-fixture-team', 1)",
        Uuid::now_v7(),
        pg.owner.id
    ));

    assert_eq!(cutover_as(&pg, &pg.owner).1, 200);

    const TABLES: &[&str] = &[
        "personal_knowledge",
        "team_knowledge",
        "memories",
        "memory_relations",
    ];
    let before: Vec<Vec<String>> = TABLES.iter().map(|t| snapshot(&pg, t)).collect();

    // The refused batch: one of every knowledge-bearing shape, all brand new
    // ids, so an implementation that silently wrote them would show up as an
    // extra row rather than a changed one.
    let (body, status) = post_batch(
        &pg,
        &pg.owner,
        vec![
            item(
                "memory",
                Uuid::now_v7(),
                "upsert",
                memory_payload(session, "must never be written"),
            ),
            item(
                "memory_relation",
                Uuid::now_v7(),
                "upsert",
                relation_payload(a, b, session),
            ),
            item(
                "personal_knowledge",
                Uuid::now_v7(),
                "upsert",
                personal_payload("must never be written"),
            ),
            item(
                "team_knowledge",
                Uuid::now_v7(),
                "upsert",
                team_payload("must never be written"),
            ),
            item("memory", a, "delete", json!({})),
            item("personal_knowledge", Uuid::now_v7(), "delete", json!({})),
        ],
    );
    assert_eq!(status, 200, "{body}");
    for outcome in results(&body) {
        assert_eq!(
            status_of(&outcome),
            "rejected",
            "an item in the refusal-touches-nothing batch was not refused, so \
             this test cannot tell a real no-op from one that got lucky: {outcome}"
        );
    }

    let after: Vec<Vec<String>> = TABLES.iter().map(|t| snapshot(&pg, t)).collect();
    for (table, (b, a)) in TABLES.iter().zip(before.iter().zip(after.iter())) {
        assert_eq!(
            b, a,
            "`{table}` changed as a side effect of a refused sync batch — the \
             refusal must be the entire action (§3.3, SC-747)"
        );
    }
}

// ---------------------------------------------------------------------------
// 4. Non-knowledge sync keeps working after cutover (FR-877)
// ---------------------------------------------------------------------------

/// `project`, `task`, `session`, `handoff`, `task_criterion` and
/// `task_blocker` upserts, all in the SAME batch, all succeed after cutover —
/// they are work tracking and continuity, not durable knowledge, and cutover
/// does not touch their sync path at all (§3.1).
#[test]
fn every_non_knowledge_upsert_succeeds_together_in_one_batch_after_cutover() {
    let pg = pg!();
    make_admin(&pg, &pg.owner);
    assert_eq!(cutover_as(&pg, &pg.owner).1, 200);

    let task_id = Uuid::now_v7();
    let session_id = Uuid::now_v7();
    let handoff_id = Uuid::now_v7();
    let criterion_id = Uuid::now_v7();
    let blocker_id = Uuid::now_v7();

    let items = vec![
        (
            "project",
            item(
                "project",
                pg.project,
                "upsert",
                json!({ "name": "renamed-after-cutover" }),
            ),
        ),
        (
            "task",
            item(
                "task",
                task_id,
                "upsert",
                task_payload("a post-cutover task"),
            ),
        ),
        (
            "session",
            item("session", session_id, "upsert", session_payload()),
        ),
        (
            "handoff",
            item("handoff", handoff_id, "upsert", handoff_payload(session_id)),
        ),
        (
            "task_criterion",
            item(
                "task_criterion",
                criterion_id,
                "upsert",
                criterion_payload(task_id),
            ),
        ),
        (
            "task_blocker",
            item(
                "task_blocker",
                blocker_id,
                "upsert",
                blocker_payload(task_id, session_id),
            ),
        ),
    ];
    let labels: Vec<&str> = items.iter().map(|(l, _)| *l).collect();
    let (body, status) = post_batch(&pg, &pg.owner, items.into_iter().map(|(_, i)| i).collect());
    assert_eq!(status, 200, "{body}");

    let outcomes = results(&body);
    assert_eq!(outcomes.len(), labels.len(), "{body}");
    for (label, outcome) in labels.iter().zip(outcomes.iter()) {
        assert_eq!(
            status_of(outcome),
            "applied",
            "{label}: non-knowledge sync must keep working after cutover \
             (FR-877): {outcome}"
        );
    }

    // Handoff, criterion and blocker are genuinely there, not merely reported
    // `applied` by a route that ignored them.
    assert_eq!(
        pg.server.count(&format!(
            "SELECT count(*) FROM handoffs WHERE id = '{handoff_id}'"
        )),
        1
    );
    assert_eq!(
        pg.server.count(&format!(
            "SELECT count(*) FROM task_criteria WHERE id = '{criterion_id}'"
        )),
        1
    );
    assert_eq!(
        pg.server.count(&format!(
            "SELECT count(*) FROM task_blockers WHERE id = '{blocker_id}'"
        )),
        1
    );
}

/// Deletion of the same six non-knowledge types, after cutover.
///
/// `project`, `task`, `session` and `handoff` delete through the shared
/// `tombstone` path with `operation: "delete"`. `task_criterion` and
/// `task_blocker` have no such path in this server — `sync.rs`'s `tombstone`
/// answers only `memory | handoff | session | task | project` for a genuine
/// `"delete"` operation, and a criterion or blocker is retired the way this
/// server has always retired one: an upsert whose payload sets `"deleted":
/// true`, read by `sync.rs`'s `deleted_at` helper. Both are exercised here in
/// the form this server actually accepts, which is what "still succeeds after
/// cutover" has to mean for a type that was never deleted the other way.
#[test]
fn every_non_knowledge_deletion_still_succeeds_after_cutover() {
    let pg = pg!();
    make_admin(&pg, &pg.owner);

    // A disposable project, so tombstoning it does not take the fixture's own
    // project down with it and break every later assertion in this file.
    let throwaway = pg.extra_project("cutover-disposable", &[&pg.owner]);

    let task_id = seed_task(&pg, "a task to delete");
    let session_id = Uuid::now_v7();
    pg.server.execute(&format!(
        "INSERT INTO sessions (id, project_id, agent, branch, status, started_at)
         VALUES ('{session_id}', '{}', 'claude-code', 'main', 'active', now())",
        pg.project
    ));
    let handoff_id = Uuid::now_v7();
    pg.server.execute(&format!(
        "INSERT INTO handoffs (id, project_id, session_id, trigger, goal, progress, next_step)
         VALUES ('{handoff_id}', '{}', '{session_id}', 'session_end', 'g', 'p', 'n')",
        pg.project
    ));
    let criterion_id = Uuid::now_v7();
    pg.server.execute(&format!(
        "INSERT INTO task_criteria (id, task_id, project_id, ordinal, label, text)
         VALUES ('{criterion_id}', '{task_id}', '{}', 1, 'l', 't')",
        pg.project
    ));
    let blocker_id = Uuid::now_v7();
    pg.server.execute(&format!(
        "INSERT INTO task_blockers (id, task_id, project_id, description, opened_by_session)
         VALUES ('{blocker_id}', '{task_id}', '{}', 'd', '{session_id}')",
        pg.project
    ));

    assert_eq!(cutover_as(&pg, &pg.owner).1, 200);

    // `project`'s own delete, alone, against the throwaway project.
    let (body, status) = post_json_status_bearer(
        &pg.server.base,
        "/api/sync/batch",
        &batch_of(
            throwaway,
            vec![item("project", throwaway, "delete", json!({}))],
        ),
        &pg.owner.token,
    );
    assert_eq!(status, 200, "{body}");
    assert_eq!(status_of(&results(&body)[0]), "applied", "{body}");
    assert_eq!(
        pg.server.count(&format!(
            "SELECT count(*) FROM projects WHERE id = '{throwaway}' AND deleted_at IS NOT NULL"
        )),
        1,
        "the project delete did not take effect"
    );

    // `task`, `session`, `handoff` — true deletes — plus `task_criterion` and
    // `task_blocker` retired the way this server actually retires them.
    let items = vec![
        ("task", item("task", task_id, "delete", json!({}))),
        ("session", item("session", session_id, "delete", json!({}))),
        ("handoff", item("handoff", handoff_id, "delete", json!({}))),
        (
            "task_criterion",
            item(
                "task_criterion",
                criterion_id,
                "upsert",
                json!({ "task_id": task_id, "ordinal": 1, "label": "l", "text": "t",
                        "state": "pending", "verification": "unverified", "deleted": true }),
            ),
        ),
        (
            "task_blocker",
            item(
                "task_blocker",
                blocker_id,
                "upsert",
                json!({ "task_id": task_id, "description": "d", "state": "open",
                        "opened_by_session": session_id, "deleted": true }),
            ),
        ),
    ];
    let labels: Vec<&str> = items.iter().map(|(l, _)| *l).collect();
    let (body, status) = post_batch(&pg, &pg.owner, items.into_iter().map(|(_, i)| i).collect());
    assert_eq!(status, 200, "{body}");
    let outcomes = results(&body);
    assert_eq!(outcomes.len(), labels.len(), "{body}");
    for (label, outcome) in labels.iter().zip(outcomes.iter()) {
        assert_eq!(
            status_of(outcome),
            "applied",
            "{label}: deleting/retiring this type must still succeed after \
             cutover (FR-877): {outcome}"
        );
    }
    for (table, id) in [
        ("tasks", task_id),
        ("sessions", session_id),
        ("handoffs", handoff_id),
        ("task_criteria", criterion_id),
        ("task_blockers", blocker_id),
    ] {
        assert_eq!(
            pg.server.count(&format!(
                "SELECT count(*) FROM {table} WHERE id = '{id}' AND deleted_at IS NOT NULL"
            )),
            1,
            "`{table}` row {id} was not marked deleted"
        );
    }
}

// ---------------------------------------------------------------------------
// 5. Reads are never refused (§11.9)
// ---------------------------------------------------------------------------

#[test]
fn all_three_changes_feeds_still_answer_200_after_cutover() {
    let pg = pg!();
    make_admin(&pg, &pg.owner);
    assert_eq!(cutover_as(&pg, &pg.owner).1, 200);

    let (body, status) = get_json_status_bearer(
        &pg.server.base,
        &format!("/api/sync/changes?project_id={}", pg.project),
        &pg.owner.token,
    );
    assert_eq!(
        status, 200,
        "`GET /api/sync/changes` was refused after cutover; a demoted replica \
         with no read path can never refill (§11.9): {body}"
    );

    let (body, status) = get_json_status_bearer(
        &pg.server.base,
        "/api/sync/changes/personal",
        &pg.owner.token,
    );
    assert_eq!(status, 200, "`GET /api/sync/changes/personal`: {body}");

    let (body, status) =
        get_json_status_bearer(&pg.server.base, "/api/sync/changes/team", &pg.owner.token);
    assert_eq!(status, 200, "`GET /api/sync/changes/team`: {body}");
}

// ---------------------------------------------------------------------------
// 6. Cutover is one compare-and-swap
// ---------------------------------------------------------------------------

#[test]
fn a_second_cutover_returns_the_same_answer_and_does_nothing_again() {
    let pg = pg!();
    make_admin(&pg, &pg.owner);

    let (first, first_status) = cutover_as(&pg, &pg.owner);
    assert_eq!(first_status, 200, "{first}");
    assert_eq!(first["already"], json!(false), "{first}");
    let cutover_at = first["cutover_at"].clone();
    assert!(cutover_at.is_string(), "{first}");

    let (second, second_status) = cutover_as(&pg, &pg.owner);
    assert_eq!(second_status, 200, "{second}");
    assert_eq!(
        second["already"],
        json!(true),
        "a second cutover call must report `already: true`: {second}"
    );
    assert_eq!(
        second["cutover_at"], cutover_at,
        "a second cutover call must return the SAME `cutover_at`, not a fresh \
         one (§2): {second}"
    );
    assert_eq!(second["demoted"], json!(0), "{second}");
    assert_eq!(second["audited"], json!(0), "{second}");
}

/// A memory whose asserted verification the new authority model cannot
/// substantiate is demoted and audited exactly once — not once per call.
#[test]
fn a_second_cutover_does_not_re_demote_or_re_audit() {
    let pg = pg!();
    make_admin(&pg, &pg.owner);
    let session = pg.session_for(&pg.owner);
    let memory = Uuid::now_v7();
    pg.server.execute(&format!(
        "INSERT INTO memories (id, project_id, type, scope, scope_key, content,
                               origin_session_id, verification, verification_authority)
         VALUES ('{memory}', '{}', 'fact', 'project', '{}',
                 'a claim nobody verified through the server', '{session}',
                 'verified', 'cairn')",
        pg.project, pg.project
    ));

    let (first, first_status) = cutover_as(&pg, &pg.owner);
    assert_eq!(first_status, 200, "{first}");
    let demoted = first["demoted"].as_i64().unwrap_or(-1);
    let audited = first["audited"].as_i64().unwrap_or(-1);
    assert!(
        demoted >= 1,
        "the unsubstantiated memory was not demoted: {first}"
    );
    assert!(
        audited >= 1,
        "the unsubstantiated memory was not audited: {first}"
    );
    let audit_rows_after_first = pg.server.count(&format!(
        "SELECT count(*) FROM legacy_verification_audit WHERE knowledge_id = '{memory}'"
    ));
    assert_eq!(audit_rows_after_first, 1, "{first}");

    let (second, second_status) = cutover_as(&pg, &pg.owner);
    assert_eq!(second_status, 200, "{second}");
    assert_eq!(second["demoted"], json!(0), "{second}");
    assert_eq!(second["audited"], json!(0), "{second}");
    assert_eq!(
        pg.server.count(&format!(
            "SELECT count(*) FROM legacy_verification_audit WHERE knowledge_id = '{memory}'"
        )),
        1,
        "a second cutover call wrote a second audit row for the same record"
    );
}

// ---------------------------------------------------------------------------
// 7. Only an admin may cut over
// ---------------------------------------------------------------------------

#[test]
fn only_an_admin_may_cut_over() {
    let pg = pg!();

    for who in [&pg.member, &pg.outsider] {
        let (body, status) = cutover_as(&pg, who);
        assert_eq!(
            status, 403,
            "a non-admin was allowed to cut this deployment over: {body}"
        );
        assert_eq!(
            pg.server
                .text("SELECT mode FROM server_authority WHERE id = 1"),
            "pre_cutover",
            "a refused cutover attempt changed `server_authority.mode` anyway"
        );
    }

    // The refusal is a real capability check, not a fixed answer — the same
    // account succeeds once actually made an administrator.
    make_admin(&pg, &pg.member);
    let (body, status) = cutover_as(&pg, &pg.member);
    assert_eq!(
        status, 200,
        "an account promoted to admin was still refused: {body}"
    );
}

// ---------------------------------------------------------------------------
// 8. The migration drain route survives its own refusal (§12.1)
// ---------------------------------------------------------------------------

/// Register a migration for `who`, returning its token.
fn register_migration(pg: &Pg, who: &Account, writer_id: &str) -> String {
    let (body, status) = post_json_status_bearer(
        &pg.server.base,
        "/api/migration/register",
        &json!({ "writer_id": writer_id }),
        &who.token,
    );
    assert_eq!(status, 200, "registering a migration: {body}");
    body["migration_token"]
        .as_str()
        .unwrap_or_else(|| panic!("no migration_token in {body}"))
        .to_string()
}

#[test]
fn migration_drain_accepts_what_sync_batch_now_refuses() {
    let pg = pg!();
    make_admin(&pg, &pg.owner);
    let token = register_migration(&pg, &pg.owner, "cutover-drain-fixture");
    assert_eq!(cutover_as(&pg, &pg.owner).1, 200);

    let entity_id = Uuid::now_v7();
    let payload = personal_payload("drained after this server's own cutover");

    let (drain_body, drain_status) = post_json_status_bearer(
        &pg.server.base,
        "/api/migration/drain",
        &json!({
            "migration_token": token,
            "items": [{
                "entity_type": "personal_knowledge",
                "entity_id": entity_id,
                "operation": "upsert",
                "payload": payload,
            }],
        }),
        &pg.owner.token,
    );
    assert_eq!(drain_status, 200, "{drain_body}");
    assert_eq!(
        drain_body["results"][0]["accepted"],
        json!(true),
        "a registered store's own drain was refused after its server's cutover, \
         which would make the refusal self-perpetuating (§12.1): {drain_body}"
    );
    assert_eq!(
        pg.server.count(&format!(
            "SELECT count(*) FROM personal_knowledge WHERE id = '{entity_id}'"
        )),
        1,
        "the drain reported accepted but wrote nothing"
    );

    // The ordinary sync route, asked to do the exact same thing, still refuses
    // it — the exemption belongs to `/api/migration/drain` alone.
    let (sync_body, sync_status) = post_batch(
        &pg,
        &pg.owner,
        vec![item(
            "personal_knowledge",
            Uuid::now_v7(),
            "upsert",
            personal_payload("the same shape, over the refused route"),
        )],
    );
    assert_eq!(sync_status, 200, "{sync_body}");
    let outcome = &results(&sync_body)[0];
    assert_eq!(status_of(outcome), "rejected", "{sync_body}");
    assert_eq!(
        error_code_of(outcome),
        "upgrade_required",
        "the drain route's exemption leaked into the ordinary sync route, which \
         would make it a general bypass of `upgrade_required` rather than the \
         narrow migration-scoped exception §12.1 describes: {sync_body}"
    );
}

// ---------------------------------------------------------------------------
// 9. `/api/version` advertises the mode
// ---------------------------------------------------------------------------

#[test]
fn version_advertises_pre_cutover_then_server_authoritative() {
    let pg = pg!();
    make_admin(&pg, &pg.owner);

    let (before, status) = get_json_status_bearer(&pg.server.base, "/api/version", &pg.owner.token);
    assert_eq!(status, 200, "{before}");
    assert_eq!(
        before["authority"]["mode"],
        json!("pre_cutover"),
        "a fresh deployment must advertise `pre_cutover`: {before}"
    );
    assert!(
        before["authority"]["cutover_at"].is_null(),
        "a deployment that has not cut over must not carry a `cutover_at`: {before}"
    );

    assert_eq!(cutover_as(&pg, &pg.owner).1, 200);

    let (after, status) = get_json_status_bearer(&pg.server.base, "/api/version", &pg.owner.token);
    assert_eq!(status, 200, "{after}");
    assert_eq!(
        after["authority"]["mode"],
        json!("server_authoritative"),
        "`/api/version` did not advertise the cutover: {after}"
    );
    assert!(
        after["authority"]["cutover_at"].as_str().is_some(),
        "a cut-over deployment must carry a `cutover_at`: {after}"
    );
}
