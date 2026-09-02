//! The post-cutover command boundary, as a contract (T024,
//! `contracts/knowledge-commands.md` §3).
//!
//! ## Why this boundary exists at all
//!
//! Not a design preference — an audit finding. On `main`, `POST /api/sync/batch`
//! upserts a `memory` row with `ON CONFLICT (id) DO UPDATE … WHERE
//! memories.project_id = $2`. Scoped to the *project*, not to the author: any
//! member of a project can overwrite any other member's memory content, state
//! and verification by naming its id. And `reinforcement_count`,
//! `verification`, `verification_authority` and the rest are bound straight from
//! the payload, so a client asserts derived state the server never computed.
//!
//! A command replaces the upsert with an **intent**. The server computes the
//! consequences, which is what makes the two problems disappear together: there
//! is no command that replaces another member's content, and there is no field
//! in which a client can assert a derived value.
//!
//! ## What is asserted here
//!
//! - **Intent only.** Every field in §3.1 is refused when present, one at a
//!   time, so a field that stopped being refused shows up as itself.
//! - **Identity from the credential.** Attribution is never read from the body
//!   (Principle XI).
//! - **No cross-member overwrite.** Correcting a colleague's knowledge is
//!   `supersede` — a new record, linked, attributed and reversible.
//! - **Retries are idempotent.** A command carries its own identity, so a
//!   client that could not record its own success does not write twice.
//! - **Personal is owner-only**, and no other owner is nameable.
//! - **Team transitions are atomic**, reusing the existing compare-and-swap.

use cairn_e2e::feature005::{Account, Pg};
use cairn_e2e::{get_json_status_bearer, post_json_status_bearer, post_status_bearer};
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

fn post(pg: &Pg, who: &Account, path: &str, body: &Value) -> (Value, u16) {
    post_json_status_bearer(&pg.server.base, path, body, &who.token)
}

fn status(pg: &Pg, who: &Account, path: &str, body: &Value) -> u16 {
    post_status_bearer(&pg.server.base, path, body, &who.token)
}

/// Seed a project memory the way the server will once commands exist, so a
/// test about superseding does not depend on the create command it is not
/// testing.
fn seed_memory(pg: &Pg, who: &Account, content: &str) -> Uuid {
    let id = Uuid::now_v7();
    let session = pg.session_for(who);
    pg.server.execute(&format!(
        "INSERT INTO memories (id, project_id, type, scope, scope_key, content,
                               origin_session_id, origin_kind)
         VALUES ('{id}', '{}', 'decision', 'project', '{}', '{content}', '{session}',
                 'explicit')",
        pg.project, pg.project
    ));
    id
}

/// Every field §3.1 says the server computes, with a plausible value.
///
/// One at a time, because a body carrying all of them would pass if any single
/// one were refused, and this needs each of them to be.
fn derived_fields() -> Vec<(&'static str, Value)> {
    vec![
        ("state", json!("superseded")),
        ("superseded_by_id", json!(Uuid::now_v7())),
        ("superseded_at", json!("2026-09-02T10:00:00Z")),
        ("stale_at", json!("2026-09-02T10:00:00Z")),
        ("reinforcement_count", json!(99)),
        ("distinct_origin_count", json!(99)),
        ("evidence_count", json!(99)),
        ("evidence_fact_count", json!(99)),
        ("verification", json!("verified")),
        ("verification_authority", json!("cairn")),
        ("last_verified_at", json!("2026-09-02T10:00:00Z")),
        ("verification_basis", json!(["command"])),
        ("created_at", json!("2020-01-01T00:00:00Z")),
        ("updated_at", json!("2020-01-01T00:00:00Z")),
    ]
}

// ---------------------------------------------------------------------------
// Intent only (§3.1)
// ---------------------------------------------------------------------------

#[test]
fn creating_a_project_memory_states_an_intent_and_nothing_derived() {
    let pg = pg!();
    let path = format!("/api/projects/{}/memories", pg.project);
    let ok = json!({
        "type": "decision",
        "scope": "project",
        "content": "storage authority moves to the server",
        "topic_key": "storage.authority",
        "value_key": "server",
    });
    let (body, code) = post(&pg, &pg.owner, &path, &ok);
    assert_eq!(code, 200, "an intent-only command was refused: {body}");

    // The server computed what the client did not send, rather than leaving it
    // absent or trusting a value.
    let id: Uuid = body["id"].as_str().expect("id").parse().expect("uuid");
    assert_eq!(
        pg.server
            .text(&format!("SELECT state FROM memories WHERE id = '{id}'")),
        "active"
    );
    assert_eq!(
        pg.server.text(&format!(
            "SELECT origin_kind FROM memories WHERE id = '{id}'"
        )),
        "explicit",
        "an explicitly created memory was not marked explicit (FR-816)"
    );
}

#[test]
fn every_derived_field_is_refused_when_a_client_sends_it() {
    let pg = pg!();
    let path = format!("/api/projects/{}/memories", pg.project);
    for (field, value) in derived_fields() {
        let mut body = json!({
            "type": "decision",
            "scope": "project",
            "content": "a claim",
        });
        body[field] = value;
        let code = status(&pg, &pg.owner, &path, &body);
        assert_eq!(
            code, 400,
            "a command carrying `{field}` was accepted; a client that can send \
             it can assert a value the server never computed"
        );
    }
    // And nothing was written by any of them.
    assert_eq!(
        pg.server.count(&format!(
            "SELECT count(*) FROM memories WHERE project_id = '{}'",
            pg.project
        )),
        0
    );
}

#[test]
fn a_command_cannot_name_its_own_author() {
    let pg = pg!();
    // Attribution is bound from the credential (Principle XI). A body that
    // could name it could attribute one member's work to another — the same
    // class of defect as a falsified session on an event.
    let path = format!("/api/projects/{}/memories", pg.project);
    for field in ["origin_session_id", "owner_user_id", "proposed_by_user_id"] {
        let mut body = json!({ "type": "fact", "scope": "project", "content": "a claim" });
        body[field] = json!(Uuid::now_v7());
        assert_eq!(
            status(&pg, &pg.owner, &path, &body),
            400,
            "a command was allowed to name `{field}`"
        );
    }
}

// ---------------------------------------------------------------------------
// No cross-member overwrite (§3.2)
// ---------------------------------------------------------------------------

#[test]
fn there_is_no_command_that_replaces_another_members_memory() {
    let pg = pg!();
    let theirs = seed_memory(&pg, &pg.owner, "the original claim");

    // The upsert's shape, aimed at a colleague's row. There is deliberately no
    // route for it: correcting someone else's knowledge is `supersede`, which
    // creates a new record and links it — visible, attributed and reversible.
    let overwrite = json!({ "content": "a silent rewrite" });
    for path in [
        format!("/api/memories/{theirs}"),
        format!("/api/projects/{}/memories/{theirs}", pg.project),
    ] {
        let code = status(&pg, &pg.member, &path, &overwrite);
        assert!(
            code == 404 || code == 405,
            "{path} answered {code}; a route that overwrites a colleague's \
             memory content must not exist"
        );
    }
    assert_eq!(
        pg.server.text(&format!(
            "SELECT content FROM memories WHERE id = '{theirs}'"
        )),
        "the original claim"
    );
}

#[test]
fn superseding_a_colleagues_memory_records_a_new_record_rather_than_editing_theirs() {
    let pg = pg!();
    let theirs = seed_memory(&pg, &pg.owner, "the original claim");

    // The correcting member names a session of their own, so the replacement
    // is attributed to the account that actually made it. The session is
    // verified against the credential exactly as an event's is (FR-769a).
    let mine = pg.session_for(&pg.member);
    let (body, code) = post(
        &pg,
        &pg.member,
        &format!("/api/memories/{theirs}/supersede"),
        &json!({
            "content": "the corrected claim",
            "type": "decision",
            "scope": "project",
            "session_id": mine,
        }),
    );
    assert_eq!(code, 200, "{body}");

    // The old record keeps saying what it said, and is marked superseded by the
    // new one rather than rewritten.
    assert_eq!(
        pg.server.text(&format!(
            "SELECT content FROM memories WHERE id = '{theirs}'"
        )),
        "the original claim",
        "supersede rewrote the record it superseded"
    );
    assert_eq!(
        pg.server
            .text(&format!("SELECT state FROM memories WHERE id = '{theirs}'")),
        "superseded"
    );
    let replacement: Uuid = body["id"].as_str().expect("id").parse().expect("uuid");
    assert_eq!(
        pg.server.text(&format!(
            "SELECT superseded_by_id::text FROM memories WHERE id = '{theirs}'"
        )),
        replacement.to_string()
    );
    // Attributed to the member who did it, not to the original author.
    assert_eq!(
        pg.server.count(&format!(
            "SELECT count(*) FROM memories m JOIN sessions s ON s.id = m.origin_session_id
              WHERE m.id = '{replacement}' AND s.user_id = '{}'",
            pg.member.id
        )),
        1,
        "the replacement was not attributed to the account that made it"
    );
}

#[test]
fn a_non_member_cannot_issue_any_project_command() {
    let pg = pg!();
    let existing = seed_memory(&pg, &pg.owner, "a claim");
    let cases: Vec<(String, Value)> = vec![
        (
            format!("/api/projects/{}/memories", pg.project),
            json!({ "type": "fact", "scope": "project", "content": "x" }),
        ),
        (
            format!("/api/memories/{existing}/supersede"),
            json!({ "content": "x", "type": "fact", "scope": "project" }),
        ),
        (format!("/api/memories/{existing}/reinforce"), json!({})),
        (
            format!("/api/memories/{existing}/pin"),
            json!({ "pinned": true }),
        ),
        (
            format!("/api/projects/{}/memory-relations", pg.project),
            json!({ "from_memory_id": existing, "to_memory_id": existing, "kind": "reinforces" }),
        ),
    ];
    for (path, body) in cases {
        assert_eq!(
            status(&pg, &pg.outsider, &path, &body),
            403,
            "{path} was reachable by a non-member"
        );
    }
}

// ---------------------------------------------------------------------------
// Idempotent retries
// ---------------------------------------------------------------------------

#[test]
fn retrying_a_command_applies_it_once() {
    let pg = pg!();
    let path = format!("/api/projects/{}/memories", pg.project);
    let command_id = Uuid::now_v7();
    let body = json!({
        "command_id": command_id,
        "type": "decision",
        "scope": "project",
        "content": "storage authority moves to the server",
    });

    let (first, code) = post(&pg, &pg.owner, &path, &body);
    assert_eq!(code, 200, "{first}");

    // A client that could not record its own success sends it again. The
    // command carries its own identity, so this is the same command rather than
    // a second one.
    for _ in 0..3 {
        let (again, code) = post(&pg, &pg.owner, &path, &body);
        assert_eq!(code, 200, "{again}");
        assert_eq!(
            again["id"], first["id"],
            "a retry created a second record instead of returning the first"
        );
    }
    assert_eq!(
        pg.server.count(&format!(
            "SELECT count(*) FROM memories WHERE project_id = '{}'",
            pg.project
        )),
        1
    );
}

#[test]
fn reinforcing_twice_under_one_command_id_counts_once() {
    let pg = pg!();
    let id = seed_memory(&pg, &pg.owner, "a claim");
    let command_id = Uuid::now_v7();
    let body = json!({ "command_id": command_id });
    for _ in 0..3 {
        assert_eq!(
            status(
                &pg,
                &pg.owner,
                &format!("/api/memories/{id}/reinforce"),
                &body
            ),
            200
        );
    }
    // The count is the server's to compute, and a replayed command must not
    // inflate it.
    assert_eq!(
        pg.server.count(&format!(
            "SELECT reinforcement_count::bigint FROM memories WHERE id = '{id}'"
        )),
        1,
        "a retried reinforcement was counted more than once"
    );
}

// ---------------------------------------------------------------------------
// Personal knowledge is owner-only
// ---------------------------------------------------------------------------

#[test]
fn personal_knowledge_belongs_to_the_caller_and_no_other_owner_is_nameable() {
    let pg = pg!();
    let (body, code) = post(
        &pg,
        &pg.owner,
        "/api/personal/knowledge",
        &json!({ "knowledge_type": "convention", "content": "always sign release images" }),
    );
    assert_eq!(code, 200, "{body}");
    let id: Uuid = body["id"].as_str().expect("id").parse().expect("uuid");
    assert_eq!(
        pg.server.text(&format!(
            "SELECT owner_user_id::text FROM personal_knowledge WHERE id = '{id}'"
        )),
        pg.owner.id.to_string()
    );

    // There is no owner to name. A body that could name one would let an
    // account write into a colleague's private knowledge.
    let code = status(
        &pg,
        &pg.owner,
        "/api/personal/knowledge",
        &json!({
            "knowledge_type": "fact",
            "content": "x",
            "owner_user_id": pg.member.id,
        }),
    );
    assert_eq!(code, 400, "a personal command was allowed to name an owner");
}

#[test]
fn only_the_owner_may_forget_their_own_personal_knowledge() {
    let pg = pg!();
    let (body, _) = post(
        &pg,
        &pg.owner,
        "/api/personal/knowledge",
        &json!({ "knowledge_type": "fact", "content": "mine alone" }),
    );
    let id: Uuid = body["id"].as_str().expect("id").parse().expect("uuid");

    // A co-member of every project the owner is in still has no standing here.
    // An administrator's standing is over team guidance, not a colleague's
    // private notes — so there is deliberately no admin exemption to test for.
    assert_eq!(
        status(
            &pg,
            &pg.member,
            &format!("/api/personal/knowledge/{id}/forget"),
            &json!({})
        ),
        404,
        "a colleague could forget someone else's personal knowledge"
    );
    assert_eq!(
        pg.server.count(&format!(
            "SELECT count(*) FROM personal_knowledge
              WHERE id = '{id}' AND forgotten_at IS NULL"
        )),
        1
    );

    assert_eq!(
        status(
            &pg,
            &pg.owner,
            &format!("/api/personal/knowledge/{id}/forget"),
            &json!({})
        ),
        200
    );
    assert_eq!(
        pg.server.count(&format!(
            "SELECT count(*) FROM personal_knowledge
              WHERE id = '{id}' AND forgotten_at IS NOT NULL"
        )),
        1
    );
}

// ---------------------------------------------------------------------------
// Pattern command shape, and its owner guard
// ---------------------------------------------------------------------------

#[test]
fn a_pattern_command_carries_the_safe_shape_and_refuses_the_local_one() {
    let pg = pg!();
    let safe = json!({
        "title": "sign images before deploying",
        "problem": "the pipeline rejects unsigned images",
        "root_cause": "no signer configured",
        "approach": "configure a signer in the release job",
        "constraints": [],
        "applicability": [],
    });
    // The route exists and accepts the safe shape. Whether it stores anything
    // is US3's business (T083+); T025 supplies the shape and the owner guard.
    let code = status(&pg, &pg.owner, "/api/patterns", &safe);
    assert!(
        code != 404,
        "the pattern promote command has no route at all"
    );

    // The five fields the safe shape drops are refused rather than ignored.
    // Each is a refused field name or names the source project, and a route
    // that ignored them would let a client believe they had travelled.
    for field in [
        "signals",
        "signal_digest",
        "origin_ref",
        "sanitization_report",
        "source_memory_id",
        "origin_deleted",
        "trust",
        "owner_user_id",
        "domain",
    ] {
        let mut body = safe.clone();
        body[field] = json!("anything");
        assert_eq!(
            status(&pg, &pg.owner, "/api/patterns", &body),
            400,
            "the pattern command accepted `{field}`"
        );
    }
}

#[test]
fn a_pattern_that_names_its_source_project_is_refused() {
    let pg = pg!();
    // The most likely way promotion fails, and it goes through the same
    // global-content validator personal and team knowledge use.
    let code = status(
        &pg,
        &pg.owner,
        "/api/patterns",
        &json!({
            "title": "a pattern",
            "problem": "the deploy at /home/andres/acme keeps failing",
            "root_cause": "a cause",
            "approach": "an approach",
            "constraints": [],
            "applicability": [],
        }),
    );
    assert_eq!(
        code, 400,
        "a pattern carrying an absolute path was accepted"
    );
}

// ---------------------------------------------------------------------------
// Team transitions stay atomic and admin-only
// ---------------------------------------------------------------------------

#[test]
fn a_team_proposal_is_attributed_from_the_credential_and_starts_proposed() {
    let pg = pg!();
    let (body, code) = post(
        &pg,
        &pg.member,
        "/api/team/knowledge",
        &json!({ "knowledge_type": "convention", "content": "release images are signed" }),
    );
    assert_eq!(code, 200, "{body}");
    let id: Uuid = body["id"].as_str().expect("id").parse().expect("uuid");
    assert_eq!(
        pg.server.text(&format!(
            "SELECT state FROM team_knowledge WHERE id = '{id}'"
        )),
        "proposed",
        "a proposal arrived already authoritative"
    );
    assert_eq!(
        pg.server.text(&format!(
            "SELECT proposed_by_user_id::text FROM team_knowledge WHERE id = '{id}'"
        )),
        pg.member.id.to_string()
    );
}

#[test]
fn a_member_cannot_ratify_and_the_existing_compare_and_swap_still_decides() {
    let pg = pg!();
    let (body, _) = post(
        &pg,
        &pg.member,
        "/api/team/knowledge",
        &json!({ "knowledge_type": "convention", "content": "release images are signed" }),
    );
    let id: Uuid = body["id"].as_str().expect("id").parse().expect("uuid");

    // Only a human administrator may make team guidance authoritative. The
    // proposer is not one, however sure they are.
    assert_eq!(
        status(
            &pg,
            &pg.member,
            &format!("/api/team/{id}/ratify"),
            &json!({})
        ),
        403
    );
    assert_eq!(
        pg.server.text(&format!(
            "SELECT state FROM team_knowledge WHERE id = '{id}'"
        )),
        "proposed"
    );

    // And a proposal cannot be retired: `retire` refuses anything that is not
    // authoritative, which is the same compare-and-swap that refuses a second
    // ratification. The transition is decided by one statement, not by a read
    // followed by a write.
    assert_ne!(
        status(
            &pg,
            &pg.member,
            &format!("/api/team/{id}/retire"),
            &json!({})
        ),
        200
    );
}

#[test]
fn a_read_back_of_a_created_memory_shows_what_the_server_computed() {
    let pg = pg!();
    let (created, code) = post(
        &pg,
        &pg.owner,
        &format!("/api/projects/{}/memories", pg.project),
        &json!({ "type": "fact", "scope": "project", "content": "a claim" }),
    );
    assert_eq!(code, 200, "{created}");
    let id = created["id"].as_str().expect("id");

    let (read, code) = get_json_status_bearer(
        &pg.server.base,
        &format!("/api/memories/{id}"),
        &pg.owner.token,
    );
    assert_eq!(code, 200, "{read}");
    // The derived fields the client was forbidden to send come back with the
    // server's own values, which is the point of forbidding them. The read
    // route nests the record under `memory`.
    assert_eq!(read["memory"]["state"], "active", "{read}");
    // `reinforcement_count` is not in this read route's projection — that is
    // the existing read API's shape and not a command's business. The
    // assertion that matters is that the server computed a value the client
    // was forbidden to send, so it is made where that value lives.
    assert_eq!(
        pg.server.count(&format!(
            "SELECT reinforcement_count::bigint FROM memories WHERE id = '{id}'"
        )),
        0
    );
}
