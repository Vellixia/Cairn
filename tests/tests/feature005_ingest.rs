//! Safe-event ingest, as a contract (T028, `contracts/safe-events.md` §7).
//!
//! The assertions that matter here are the ones about what the server refuses
//! to be told:
//!
//! - **Identity is re-derived, not accepted.** A client that could choose its
//!   own `event_id` could submit a colliding one, be answered `duplicate`, and
//!   suppress a genuine event — or pre-claim ids it guessed. So the server
//!   recomputes it and refuses a mismatch.
//! - **A session is verified, not believed.** An event names its session, and a
//!   session identifier is body data. A project member submitting events
//!   against a colleague's session would have consolidation attribute a
//!   colleague's authorship to work they never did (FR-769a).
//! - **Non-membership is a request-level refusal.** A per-item answer would
//!   confirm the session's existence to someone with no business knowing.
//! - **`duplicate` is a success.** A retry that gets it has achieved exactly
//!   what it was for.

use cairn_e2e::feature005::{Account, Pg};
use cairn_e2e::{
    post_file_status_bearer, post_json_bearer, post_json_status_bearer, post_status_bearer,
};
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

/// The UUIDv5 the server will re-derive, computed the same way the daemon does.
fn event_id(session: Uuid, seq: u64) -> Uuid {
    cairn_core::eventid::event_id(session, seq)
}

fn event(session: Uuid, seq: u64, kind: &str, content: Value) -> Value {
    let mut event = json!({
        "event_id": event_id(session, seq),
        "contract_version": 1,
        "kind": kind,
        "agent": "claude_code",
        "vendor_event": "PostToolUse",
        "session_id": session,
        "session_seq": seq,
        "occurred_at": "2026-09-02T10:00:00Z",
    });
    if !content.is_null() {
        event["content"] = content;
    }
    event
}

fn file_event(session: Uuid, seq: u64, path: &str) -> Value {
    event(
        session,
        seq,
        "file_changed",
        json!({ "File": {
            "repo_file": path,
            "repo_file_from": null,
            "change_kind": "modified",
            "file_identity": "present"
        }}),
    )
}

fn batch(events: Vec<Value>) -> Value {
    json!({ "contract_version": 1, "events": events })
}

fn post(pg: &Pg, who: &Account, body: &Value) -> (Value, u16) {
    post_json_status_bearer(&pg.server.base, "/api/events/batch", body, &who.token)
}

fn statuses(response: &Value) -> Vec<String> {
    response["results"]
        .as_array()
        .map(|r| {
            r.iter()
                .map(|o| o["status"].as_str().unwrap_or("?").to_string())
                .collect()
        })
        .unwrap_or_default()
}

fn reasons(response: &Value) -> Vec<String> {
    response["results"]
        .as_array()
        .map(|r| {
            r.iter()
                .map(|o| o["reason"].as_str().unwrap_or("").to_string())
                .collect()
        })
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// The happy path, and what it establishes
// ---------------------------------------------------------------------------

#[test]
fn an_accepted_event_is_persisted_and_enqueued_in_one_transaction() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    let (body, status) = post(
        &pg,
        &pg.owner,
        &batch(vec![file_event(session, 1, "crates/cairnd/src/sync.rs")]),
    );
    assert_eq!(status, 200, "{body}");
    assert_eq!(statuses(&body), vec!["accepted"]);

    let id = event_id(session, 1);
    assert_eq!(
        pg.server.count(&format!(
            "SELECT count(*) FROM safe_events WHERE event_id = '{id}'"
        )),
        1
    );
    // Enqueued in the same transaction, so an accepted event always eventually
    // becomes knowledge. The lease row must exist too — `consolidation_work`
    // has a foreign key to it, and consolidation locks the lease rather than
    // the group.
    assert_eq!(
        pg.server.count(&format!(
            "SELECT count(*) FROM consolidation_work WHERE event_id = '{id}'"
        )),
        1
    );
    assert_eq!(
        pg.server.count(&format!(
            "SELECT count(*) FROM consolidation_session WHERE session_id = '{session}'"
        )),
        1
    );

    // The project and the account are bound from the session and the
    // credential, and the client never named either.
    assert_eq!(
        pg.server.text(&format!(
            "SELECT project_id::text FROM safe_events WHERE event_id = '{id}'"
        )),
        pg.project.to_string()
    );
    assert_eq!(
        pg.server.text(&format!(
            "SELECT account_id::text FROM safe_events WHERE event_id = '{id}'"
        )),
        pg.owner.id.to_string()
    );
}

#[test]
fn redelivering_an_event_is_a_duplicate_and_a_duplicate_is_a_success() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    let one = batch(vec![file_event(session, 1, "a.rs")]);

    let (first, _) = post(&pg, &pg.owner, &one);
    assert_eq!(statuses(&first), vec!["accepted"]);

    // Five redeliveries — a spool that could not record its own success.
    for _ in 0..5 {
        let (again, status) = post(&pg, &pg.owner, &one);
        assert_eq!(status, 200);
        assert_eq!(
            statuses(&again),
            vec!["duplicate"],
            "a retry must report duplicate, not an error"
        );
    }
    let id = event_id(session, 1);
    assert_eq!(
        pg.server.count(&format!(
            "SELECT count(*) FROM safe_events WHERE event_id = '{id}'"
        )),
        1,
        "at most one canonical event exists"
    );
    // And exactly one piece of work: a duplicate must not re-enqueue, or a
    // retrying client would grow the backlog without adding an event.
    assert_eq!(
        pg.server.count(&format!(
            "SELECT count(*) FROM consolidation_work WHERE event_id = '{id}'"
        )),
        1
    );
}

#[test]
fn a_repeated_act_is_a_distinct_event_rather_than_a_suppressed_duplicate() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    // The same file read twice. Two ordinals, two identities, two events.
    let (body, _) = post(
        &pg,
        &pg.owner,
        &batch(vec![
            file_event(session, 1, "a.rs"),
            file_event(session, 2, "a.rs"),
        ]),
    );
    assert_eq!(statuses(&body), vec!["accepted", "accepted"]);
    assert_eq!(
        pg.server.count(&format!(
            "SELECT count(*) FROM safe_events WHERE session_id = '{session}'"
        )),
        2
    );
}

// ---------------------------------------------------------------------------
// Identity
// ---------------------------------------------------------------------------

#[test]
fn an_event_id_the_server_cannot_re_derive_is_refused() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    let mut forged = file_event(session, 1, "a.rs");
    forged["event_id"] = json!(Uuid::now_v7());

    let (body, status) = post(&pg, &pg.owner, &batch(vec![forged]));
    assert_eq!(status, 200);
    assert_eq!(statuses(&body), vec!["rejected"]);
    assert_eq!(reasons(&body), vec!["event_id_mismatch"]);
    assert_eq!(
        pg.server.count(&format!(
            "SELECT count(*) FROM safe_events WHERE session_id = '{session}'"
        )),
        0
    );
}

#[test]
fn a_client_cannot_pre_claim_an_id_to_suppress_a_later_genuine_event() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);

    // The attack: submit ordinal 1's *identity* attached to different content,
    // so the genuine event at ordinal 1 is later answered `duplicate` and
    // silently discarded. Re-derivation is what closes it — the id and the
    // ordinal have to agree, so a squatted id is a mismatch.
    let mut squat = file_event(session, 1, "decoy.rs");
    squat["session_seq"] = json!(9);
    let (body, _) = post(&pg, &pg.owner, &batch(vec![squat]));
    assert_eq!(reasons(&body), vec!["event_id_mismatch"]);

    let (genuine, _) = post(
        &pg,
        &pg.owner,
        &batch(vec![file_event(session, 1, "real.rs")]),
    );
    assert_eq!(
        statuses(&genuine),
        vec!["accepted"],
        "a genuine event was suppressed by a pre-claimed id"
    );
}

// ---------------------------------------------------------------------------
// Session binding (FR-769, FR-769a, FR-894a)
// ---------------------------------------------------------------------------

#[test]
fn a_member_may_not_submit_events_against_a_colleagues_session() {
    let pg = pg!();
    // Both accounts are members of the same project, so membership alone does
    // not close this. Without the ownership check, consolidation would produce
    // durable knowledge attributing the owner's authorship to work the member
    // never did.
    let theirs = pg.session_for(&pg.owner);
    let (body, status) = post(&pg, &pg.member, &batch(vec![file_event(theirs, 1, "a.rs")]));
    assert_eq!(
        status, 200,
        "a co-member gets a per-item refusal, not a 403"
    );
    assert_eq!(statuses(&body), vec!["rejected"]);
    assert_eq!(reasons(&body), vec!["session_not_found"]);
    assert_eq!(
        pg.server.count(&format!(
            "SELECT count(*) FROM safe_events WHERE session_id = '{theirs}'"
        )),
        0
    );
}

#[test]
fn a_non_member_cannot_tell_an_existing_session_from_one_that_does_not_exist() {
    let pg = pg!();
    let real = pg.session_for(&pg.owner);
    let imaginary = Uuid::now_v7();

    // The probe oracle this closes: if non-membership were a request-level 403
    // and an unknown id were a per-item `session_not_found`, the difference
    // would let anyone with an account enumerate which session ids exist across
    // the whole server, one guess at a time (FR-894a).
    let probe = |session: Uuid| {
        post_status_bearer(
            &pg.server.base,
            "/api/events/batch",
            &batch(vec![file_event(session, 1, "a.rs")]),
            &pg.outsider.token,
        )
    };
    assert_eq!(probe(real), 403);
    assert_eq!(
        probe(imaginary),
        probe(real),
        "an outsider can tell an existing session from an imaginary one"
    );
    assert_eq!(
        pg.server.count(&format!(
            "SELECT count(*) FROM safe_events WHERE session_id = '{real}'"
        )),
        0
    );
}

#[test]
fn an_unauthenticated_caller_cannot_ingest_anything() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    let status = post_status_bearer(
        &pg.server.base,
        "/api/events/batch",
        &batch(vec![file_event(session, 1, "a.rs")]),
        "not-a-real-token",
    );
    assert_eq!(status, 401);
}

#[test]
fn a_member_naming_a_session_that_does_not_exist_gets_the_same_blunt_refusal() {
    let pg = pg!();
    // The cost of closing the oracle: a member who mistypes a session id is
    // told only that no session it can write to was named. A narrower message
    // would be the probe oracle again, wearing a friendlier face.
    let status = post_status_bearer(
        &pg.server.base,
        "/api/events/batch",
        &batch(vec![file_event(Uuid::now_v7(), 1, "a.rs")]),
        &pg.owner.token,
    );
    assert_eq!(status, 403);
}

// ---------------------------------------------------------------------------
// Strict fields, refused names, bounds
// ---------------------------------------------------------------------------

#[test]
fn an_unknown_field_is_refused_because_the_schema_is_closed() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    let mut extra = file_event(session, 1, "a.rs");
    extra["extra_field"] = json!("smuggled");
    let (body, _) = post(&pg, &pg.owner, &batch(vec![extra]));
    assert_eq!(statuses(&body), vec!["rejected"]);
    assert_eq!(reasons(&body), vec!["unknown_field"]);
}

#[test]
fn a_name_the_sync_boundary_refuses_is_refused_here_too_at_any_depth() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    // Two boundaries on one server disagreeing about one name is exactly the
    // drift FR-777a1 forbids. `path` is the canonical case: it is the natural
    // name for what `repo_file` carries.
    for (label, mutate) in [
        ("top level", json!({ "summary": "what happened" })),
        ("inside content", json!({ "content": { "path": "a.rs" } })),
        ("session field", json!({ "agent_session_key": "abc" })),
    ] {
        let mut e = file_event(session, 1, "a.rs");
        for (k, v) in mutate.as_object().unwrap() {
            e[k] = v.clone();
        }
        let (body, _) = post(&pg, &pg.owner, &batch(vec![e]));
        assert_eq!(
            reasons(&body),
            vec!["forbidden_field_name"],
            "a refused name at {label} was not caught"
        );
    }
}

#[test]
fn an_absolute_or_traversing_repo_file_is_refused_by_its_own_reason() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    for (path, expected) in [
        ("/etc/passwd", "repo_file_absolute"),
        ("../../etc/passwd", "repo_file_traversal"),
        ("C:\\Users\\andres", "repo_file_malformed"),
        ("crates//src", "repo_file_malformed"),
    ] {
        let (body, _) = post(&pg, &pg.owner, &batch(vec![file_event(session, 1, path)]));
        assert_eq!(
            reasons(&body),
            vec![expected],
            "{path:?} was not refused as {expected}"
        );
    }
    assert_eq!(
        pg.server.count(&format!(
            "SELECT count(*) FROM safe_events WHERE session_id = '{session}'"
        )),
        0,
        "a refused path was persisted anyway"
    );
}

#[test]
fn an_over_bound_value_is_refused_rather_than_stored_truncated() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    let long = "x".repeat(600);
    let e = event(
        session,
        1,
        "command_executed",
        json!({ "Command": { "command_line": long, "exit_status": 0 }}),
    );
    let (body, _) = post(&pg, &pg.owner, &batch(vec![e]));
    assert_eq!(reasons(&body), vec!["bound_exceeded"]);
}

#[test]
fn a_batch_over_its_bound_is_refused_as_a_request() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    let events: Vec<Value> = (1..=257)
        .map(|seq| file_event(session, seq, "a.rs"))
        .collect();
    let status = post_status_bearer(
        &pg.server.base,
        "/api/events/batch",
        &batch(events),
        &pg.owner.token,
    );
    assert_eq!(status, 400);
}

#[test]
fn an_unsupported_contract_version_is_refused_recognisably() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    // Per batch.
    let mut body = batch(vec![file_event(session, 1, "a.rs")]);
    body["contract_version"] = json!(99);
    let status = post_status_bearer(&pg.server.base, "/api/events/batch", &body, &pg.owner.token);
    assert_eq!(status, 400);

    // And per event, so a client can defer exactly the events a server cannot
    // hold rather than the whole batch.
    let mut e = file_event(session, 1, "a.rs");
    e["contract_version"] = json!(99);
    let (body, _) = post(&pg, &pg.owner, &batch(vec![e]));
    assert_eq!(reasons(&body), vec!["contract_version_unsupported"]);
}

#[test]
fn a_kind_outside_the_union_is_refused() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    let mut e = file_event(session, 1, "a.rs");
    e["kind"] = json!("prompt_submitted");
    let (body, _) = post(&pg, &pg.owner, &batch(vec![e]));
    assert_eq!(statuses(&body), vec!["rejected"]);
}

// ---------------------------------------------------------------------------
// Content screening, enforced independently of the client
// ---------------------------------------------------------------------------

#[test]
fn a_credential_inside_an_approved_field_is_refused_by_the_server_itself() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    // Client-side redaction is where secrets are removed; this is where the
    // boundary is enforced. A hostile or broken client skipping redaction must
    // not be able to store one by asking (FR-777, SC-741).
    let e = event(
        session,
        1,
        "command_executed",
        json!({ "Command": {
            "command_line": "deploy --url https://user:hunter2@example.test/repo",
            "exit_status": 0
        }}),
    );
    let (body, _) = post(&pg, &pg.owner, &batch(vec![e]));
    assert_eq!(reasons(&body), vec!["content_screening_failed"]);

    // The refusal names the check and never the content that failed it.
    let rendered = body.to_string();
    assert!(
        !rendered.contains("hunter2"),
        "the response echoed a secret"
    );
}

#[test]
fn an_ordinary_command_is_not_refused_for_looking_like_a_command() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    let e = event(
        session,
        1,
        "command_executed",
        json!({ "Command": { "command_line": "cargo test -p cairn-core", "exit_status": 0 }}),
    );
    let (body, _) = post(&pg, &pg.owner, &batch(vec![e]));
    assert_eq!(
        statuses(&body),
        vec!["accepted"],
        "refusing this would refuse every command_executed event there is"
    );
}

// ---------------------------------------------------------------------------
// Vocabulary justification
// ---------------------------------------------------------------------------

#[test]
fn a_decision_token_must_be_justified_by_an_earlier_event_in_the_session() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);

    // Unjustified: nothing in this session has mentioned `deploy` or `images`.
    let signal = |seq: u64, subject: &str, object: &str| {
        event(
            session,
            seq,
            "decision_signal",
            json!({ "Decision": {
                "decision_kind": "adopt",
                "subject_token": subject,
                "object_token": object,
                "justified_by_seq": null,
                "lexicon_version": 1
            }}),
        )
    };
    let (body, _) = post(&pg, &pg.owner, &batch(vec![signal(1, "deploy", "images")]));
    assert_eq!(reasons(&body), vec!["token_not_in_vocabulary"]);

    // Now establish the vocabulary with a real event, then cite it.
    let (body, _) = post(
        &pg,
        &pg.owner,
        &batch(vec![file_event(session, 2, "deploy/images.rs")]),
    );
    assert_eq!(statuses(&body), vec!["accepted"]);

    let (body, _) = post(&pg, &pg.owner, &batch(vec![signal(3, "deploy", "images")]));
    assert_eq!(
        statuses(&body),
        vec!["accepted"],
        "a token justified by an earlier event was refused"
    );
}

#[test]
fn a_prompt_fragment_cannot_be_smuggled_through_a_decision_token() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    post(
        &pg,
        &pg.owner,
        &batch(vec![file_event(session, 1, "deploy/images.rs")]),
    );
    // A sentence's words are not files, commands, tests or established keys, so
    // no amount of charset-legal shaping justifies them.
    let e = event(
        session,
        2,
        "decision_signal",
        json!({ "Decision": {
            "decision_kind": "adopt",
            "subject_token": "the_api_key_is_sk_abc123",
            "object_token": "images",
            "justified_by_seq": 1,
            "lexicon_version": 1
        }}),
    );
    let (body, _) = post(&pg, &pg.owner, &batch(vec![e]));
    assert_eq!(reasons(&body), vec!["token_not_in_vocabulary"]);
}

#[test]
fn a_token_justified_only_by_a_later_event_is_refused() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    // The signal is ordinal 1; the event that would justify it is ordinal 2.
    // The machine that built the signal knew the earlier events too, so it
    // could not legitimately have cited a later one.
    post(
        &pg,
        &pg.owner,
        &batch(vec![file_event(session, 2, "deploy/images.rs")]),
    );
    let e = event(
        session,
        1,
        "decision_signal",
        json!({ "Decision": {
            "decision_kind": "adopt",
            "subject_token": "deploy",
            "object_token": "images",
            "justified_by_seq": 2,
            "lexicon_version": 1
        }}),
    );
    let (body, _) = post(&pg, &pg.owner, &batch(vec![e]));
    assert_eq!(reasons(&body), vec!["token_not_in_vocabulary"]);
}

#[test]
fn an_established_project_key_justifies_a_token_no_session_event_mentions() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    // A server that checked only session events would refuse this, permanently,
    // destroying a decision the client legitimately justified.
    pg.server.execute(&format!(
        "INSERT INTO memories (id, project_id, type, scope, scope_key, content,
                               origin_session_id, topic_key, value_key)
         VALUES ('{}', '{}', 'decision', 'project', '{}', 'storage is server-authoritative',
                 '{session}', 'storage.authority', 'server')",
        Uuid::now_v7(),
        pg.project,
        pg.project
    ));
    let e = event(
        session,
        1,
        "decision_signal",
        json!({ "Decision": {
            "decision_kind": "adopt",
            "subject_token": "storage.authority",
            "object_token": "server",
            "justified_by_seq": null,
            "lexicon_version": 1
        }}),
    );
    let (body, _) = post(&pg, &pg.owner, &batch(vec![e]));
    assert_eq!(statuses(&body), vec!["accepted"], "{body}");
}

// ---------------------------------------------------------------------------
// Per-event outcomes
// ---------------------------------------------------------------------------

#[test]
fn one_bad_event_does_not_refuse_the_good_ones_beside_it() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    let mut bad = file_event(session, 2, "a.rs");
    bad["event_id"] = json!(Uuid::now_v7());

    let (body, status) = post(
        &pg,
        &pg.owner,
        &batch(vec![
            file_event(session, 1, "good.rs"),
            bad,
            file_event(session, 3, "also-good.rs"),
        ]),
    );
    assert_eq!(status, 200);
    // Per-event outcomes exist so a client can retry precisely what needs
    // retrying (FR-771). Refusing the batch would make one malformed event cost
    // two good ones.
    assert_eq!(
        statuses(&body),
        vec!["accepted", "rejected", "accepted"],
        "{body}"
    );
    assert_eq!(
        pg.server.count(&format!(
            "SELECT count(*) FROM safe_events WHERE session_id = '{session}'"
        )),
        2
    );
}

#[test]
fn a_rejection_names_a_reason_from_the_fixed_vocabulary_and_nothing_else() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    let permitted = [
        "unknown_field",
        "forbidden_field_name",
        "bound_exceeded",
        "repo_file_absolute",
        "repo_file_traversal",
        "repo_file_malformed",
        "event_id_mismatch",
        "session_not_found",
        "content_screening_failed",
        "token_not_in_vocabulary",
        "unsupported_kind",
        "contract_version_unsupported",
    ];
    let mut forged = file_event(session, 1, "a.rs");
    forged["event_id"] = json!(Uuid::now_v7());
    let (body, _) = post(&pg, &pg.owner, &batch(vec![forged]));
    for reason in reasons(&body) {
        assert!(
            permitted.contains(&reason.as_str()),
            "{reason} is not in the declared rejection vocabulary"
        );
    }
    // An accepted outcome carries no reason at all, rather than an empty one.
    let (ok, _) = post(&pg, &pg.owner, &batch(vec![file_event(session, 5, "b.rs")]));
    assert!(ok["results"][0].get("reason").is_none(), "{ok}");
}

#[test]
fn an_empty_batch_is_accepted_and_changes_nothing() {
    let pg = pg!();
    let (body, status) = post(&pg, &pg.owner, &batch(vec![]));
    assert_eq!(status, 200, "{body}");
    assert_eq!(statuses(&body), Vec::<String>::new());
    let _ = post_json_bearer;
}

// ---------------------------------------------------------------------------
// The request body limit (T030, FR-773, SC-743)
// ---------------------------------------------------------------------------

/// One legal event in a request body padded past `target` bytes.
///
/// Padded with **whitespace between JSON tokens**, which is where the first
/// attempt at this test went wrong and is worth recording. Padding by adding
/// events instead needs roughly eleven hundred of them to reach a megabyte,
/// which trips the 256-event batch cap first — so the request was refused, and
/// zero events were created, and every assertion passed with the body limit
/// removed entirely. The test measured the wrong bound and would have shipped
/// green.
///
/// Whitespace is the discriminator because nothing else objects to it: the
/// batch holds one event, serde ignores the padding, and the *only* property
/// that can refuse this request is its size in bytes.
fn padded_body(session: Uuid, target: usize) -> Vec<u8> {
    let inner = serde_json::to_string(&file_event(session, 1, "a.rs")).expect("serializes");
    let mut body = String::from("{\"contract_version\": 1, \"events\": [");
    body.push_str(&inner);
    body.push(']');
    // Pad inside the object, after the last member, where JSON allows
    // insignificant whitespace.
    while body.len() < target {
        body.push(' ');
    }
    body.push('}');
    body.into_bytes()
}

#[test]
fn a_request_inside_the_body_limit_reaches_ingest() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    // Comfortably under 1 MiB, and a real event: the point is that the limit
    // does not refuse ordinary traffic.
    // Padded to just under the limit, so this is the same shape of request as
    // the oversized one and differs only in being inside the bound.
    let body = padded_body(session, 1024 * 1024 - 4096);
    assert!(body.len() < 1024 * 1024);

    let status =
        post_file_status_bearer(&pg.server.base, "/api/events/batch", &body, &pg.owner.token);
    assert_eq!(status, 200);
    assert_eq!(
        pg.server.count(&format!(
            "SELECT count(*) FROM safe_events WHERE session_id = '{session}'"
        )),
        1,
        "a request within the limit did not reach ingest"
    );
}

#[test]
fn a_request_over_one_mebibyte_is_refused_by_the_body_boundary() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    let body = padded_body(session, 1024 * 1024 + 4096);
    assert!(
        body.len() > 1024 * 1024,
        "the fixture must exceed the limit"
    );
    // Under Axum's own 2 MB default, so removing the route limit lets this
    // through rather than being caught by the framework anyway. That is what
    // makes the assertions below able to fail.
    assert!(body.len() < 2 * 1024 * 1024);

    let status =
        post_file_status_bearer(&pg.server.base, "/api/events/batch", &body, &pg.owner.token);
    // Refused by the boundary, not by the handler. 413 is what the body limit
    // produces; 400 is what a rejected extraction produces. Either is a refusal
    // and neither is a success, and the assertions below are what actually
    // matter: nothing was persisted.
    assert!(
        status == 413 || status == 400,
        "an oversized request was answered {status}, which is not a refusal"
    );

    // Zero events, and zero work. The second is the one worth checking
    // separately: ingest enqueues consolidation in the same transaction as the
    // insert, so a partial acceptance would show up here as work with no event.
    assert_eq!(
        pg.server.count(&format!(
            "SELECT count(*) FROM safe_events WHERE session_id = '{session}'"
        )),
        0,
        "an oversized request created events"
    );
    assert_eq!(
        pg.server.count(&format!(
            "SELECT count(*) FROM consolidation_work WHERE session_id = '{session}'"
        )),
        0,
        "an oversized request created consolidation work"
    );
    assert_eq!(
        pg.server.count(&format!(
            "SELECT count(*) FROM consolidation_session WHERE session_id = '{session}'"
        )),
        0,
        "an oversized request created a consolidation lease"
    );
}

#[test]
fn the_body_limit_is_the_one_the_contract_states() {
    // Stated as a number so SC-743 has something to fail against, and asserted
    // here so the route and the contract cannot drift apart silently.
    assert_eq!(cairn_core::event::BODY_MAX_BYTES, 1024 * 1024);
}

#[test]
fn the_body_limit_belongs_to_this_route_and_not_to_the_server() {
    let pg = pg!();
    // Axum's `DefaultBodyLimit` is a layer, so putting it on the main router
    // would silently retighten every other endpoint from the 2 MB default to
    // 1 MiB — including `/api/sync/batch`, a different boundary with its own
    // bounds and no requirement asking for this one.
    let big = {
        let mut body = String::from("{\"items\": []");
        while body.len() < 1024 * 1024 + 4096 {
            body.push(' ');
        }
        body.push('}');
        body.into_bytes()
    };
    assert!(big.len() > 1024 * 1024 && big.len() < 2 * 1024 * 1024);

    let ingest =
        post_file_status_bearer(&pg.server.base, "/api/events/batch", &big, &pg.owner.token);
    let sync = post_file_status_bearer(&pg.server.base, "/api/sync/batch", &big, &pg.owner.token);
    assert_eq!(ingest, 413, "the ingest route did not apply its own limit");
    assert_ne!(
        sync, 413,
        "the ingest route's body limit leaked onto /api/sync/batch"
    );
}
