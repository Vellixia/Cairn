//! Queued commands, delivered through the real HTTP boundary (T024–T027,
//! T039).
//!
//! ## Why this file exists
//!
//! The first drain mapped command kinds to `/api/commands/*` paths the server
//! does not serve, and posted the intent payload alone — so the deterministic
//! `command_id` never crossed the wire. Two defects with one cause: the wire
//! form did not carry the command. A compile-time enum-to-string mapping test
//! passed throughout, which is why there is no such test here.
//!
//! So every assertion below puts a command into `command_spool`, drains it over
//! HTTP against a real router, and looks at what the server actually did.
//!
//! ## What the harness has to do that is worth saying once
//!
//! `cairnd`'s drain is not linkable from here — it is a binary — so these tests
//! reproduce the wire form the daemon sends and post it themselves. The
//! envelope shape is asserted against the daemon's own builder in a unit test
//! in `sync.rs`, so the two cannot drift silently; what this file proves is
//! that the *server* honours it.

use cairn_e2e::feature005::{Account, Pg};
use cairn_e2e::{post_json_status_bearer, post_status_bearer};
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

/// The envelope the daemon posts, built the same way it builds it.
fn envelope(
    kind: &str,
    command_id: Uuid,
    project_id: Option<Uuid>,
    target_id: Option<Uuid>,
    payload: Value,
) -> Value {
    json!({
        "command_id": command_id,
        "kind": kind,
        "project_id": project_id,
        "target_id": target_id,
        "payload": payload,
    })
}

fn deliver(pg: &Pg, who: &Account, body: &Value) -> (Value, u16) {
    post_json_status_bearer(&pg.server.base, "/api/commands", body, &who.token)
}

fn seed_memory(pg: &Pg, content: &str) -> Uuid {
    let id = Uuid::now_v7();
    pg.server.execute(&format!(
        "INSERT INTO memories (id, project_id, type, scope, scope_key, content,
                               origin_session_id, origin_kind)
         VALUES ('{id}', '{}', 'fact', 'project', '{}', '{content}',
                 '00000000-0000-0000-0000-000000000000', 'explicit')",
        pg.project, pg.project
    ));
    id
}

fn seed_personal(pg: &Pg, who: &Account) -> Uuid {
    let id = Uuid::now_v7();
    pg.server.execute(&format!(
        "INSERT INTO personal_knowledge
             (id, owner_user_id, knowledge_type, content, writer_id, writer_seq)
         VALUES ('{id}', '{}', 'fact', 'mine', 'w-{id}', 1)",
        who.id
    ));
    id
}

// ---------------------------------------------------------------------------
// Every Foundation-owned kind actually lands
// ---------------------------------------------------------------------------

#[test]
fn every_foundation_command_kind_is_delivered_and_has_an_observable_effect() {
    let pg = pg!();
    let target = seed_memory(&pg, "the original claim");
    let other = seed_memory(&pg, "another claim");
    let personal = seed_personal(&pg, &pg.owner);

    struct Case {
        kind: &'static str,
        project: bool,
        target: Option<Uuid>,
        payload: Value,
    }
    let cases = vec![
        Case {
            kind: "remember",
            project: true,
            target: None,
            payload: json!({ "type": "decision", "scope": "project", "content": "queued claim" }),
        },
        Case {
            kind: "supersede",
            project: false,
            target: Some(target),
            payload: json!({ "type": "decision", "scope": "project", "content": "corrected" }),
        },
        Case {
            kind: "reinforce",
            project: false,
            target: Some(other),
            payload: json!({}),
        },
        Case {
            kind: "pin",
            project: false,
            target: Some(other),
            payload: json!({ "pinned": true }),
        },
        Case {
            kind: "relate",
            project: true,
            target: None,
            payload: json!({
                "from_memory_id": target, "to_memory_id": other, "kind": "narrows"
            }),
        },
        Case {
            kind: "personal_create",
            project: false,
            target: None,
            payload: json!({ "knowledge_type": "convention", "content": "sign images" }),
        },
        Case {
            kind: "personal_forget",
            project: false,
            target: Some(personal),
            payload: json!({}),
        },
        Case {
            kind: "team_propose",
            project: false,
            target: None,
            payload: json!({ "knowledge_type": "convention", "content": "images are signed" }),
        },
        // Last, because it tombstones a memory the cases above reference.
        Case {
            kind: "forget",
            project: false,
            target: Some(other),
            payload: json!({}),
        },
    ];

    for case in &cases {
        let (body, code) = deliver(
            &pg,
            &pg.owner,
            &envelope(
                case.kind,
                Uuid::now_v7(),
                case.project.then_some(pg.project),
                case.target,
                case.payload.clone(),
            ),
        );
        assert_eq!(code, 200, "`{}` was not delivered: {body}", case.kind);
    }

    // The effects, observed rather than inferred from a 200.
    assert!(
        pg.server.count(&format!(
            "SELECT count(*) FROM memories
              WHERE project_id = '{}' AND content = 'queued claim'",
            pg.project
        )) == 1,
        "remember produced no memory"
    );
    assert_eq!(
        pg.server
            .text(&format!("SELECT state FROM memories WHERE id = '{target}'")),
        "superseded"
    );
    assert_eq!(
        pg.server.count(&format!(
            "SELECT reinforcement_count::bigint FROM memories WHERE id = '{other}'"
        )),
        1
    );
    assert_eq!(
        pg.server.count(&format!(
            "SELECT count(*) FROM memories WHERE id = '{other}' AND pinned"
        )),
        1
    );
    assert_eq!(
        pg.server.count(&format!(
            "SELECT count(*) FROM memory_relations
              WHERE project_id = '{}' AND kind = 'narrows'",
            pg.project
        )),
        1
    );
    assert_eq!(
        pg.server.count(&format!(
            "SELECT count(*) FROM personal_knowledge
              WHERE owner_user_id = '{}' AND content = 'sign images'",
            pg.owner.id
        )),
        1
    );
    assert_eq!(
        pg.server.count(&format!(
            "SELECT count(*) FROM personal_knowledge
              WHERE id = '{personal}' AND forgotten_at IS NOT NULL"
        )),
        1
    );
    assert_eq!(
        pg.server.count(&format!(
            "SELECT count(*) FROM team_knowledge
              WHERE proposed_by_user_id = '{}' AND state = 'proposed'",
            pg.owner.id
        )),
        1
    );
    assert_eq!(
        pg.server.count(&format!(
            "SELECT count(*) FROM memories WHERE id = '{other}' AND deleted_at IS NOT NULL"
        )),
        1
    );
}

#[test]
fn a_deferred_kind_answers_a_recognisable_deferral_and_not_a_refusal() {
    let pg = pg!();
    // Verification is US5's. What matters is that the answer is a **deferral**:
    // the drain leaves the row durable and retries after an upgrade. A `404`
    // would be read as permanent, and the user's queued instruction would be
    // marked terminal and lost.
    //
    // The pattern kinds were here too until US3 shipped their repository
    // (T085). They are asserted below instead — a deferral that outlives the
    // phase it was waiting for is a queued instruction that never lands, so the
    // list shrinking is the mechanism working rather than coverage being lost.
    for kind in ["verification_run", "verification_attestation"] {
        let (body, code) = deliver(
            &pg,
            &pg.owner,
            &envelope(kind, Uuid::now_v7(), None, Some(Uuid::now_v7()), json!({})),
        );
        assert_eq!(code, 409, "`{kind}` answered {code}: {body}");
        assert_eq!(
            body["error"]["code"], "unsupported_kind",
            "`{kind}` did not answer with the code the drain reads as a deferral"
        );
    }
}

#[test]
fn a_deferral_ends_when_the_phase_that_owned_it_ships() {
    let pg = pg!();
    // The other half of the deferral contract, and the half that is easy to
    // leave untested: a kind that stays deferred after its repository exists is
    // a spool row that waits forever. `pattern_promote` and `pattern_forget`
    // answered `409 unsupported_kind` until T085; they must not any more.
    for (kind, payload) in [
        (
            "pattern_promote",
            json!({
                "title": "sign images before release",
                "problem": "the pipeline refuses unsigned images",
                "root_cause": "no signer is configured in the release job",
                "approach": "configure a signer and re-run the release job",
            }),
        ),
        ("pattern_forget", json!({})),
    ] {
        let (body, code) = deliver(
            &pg,
            &pg.owner,
            &envelope(kind, Uuid::now_v7(), None, Some(Uuid::now_v7()), payload),
        );
        assert_ne!(
            code, 409,
            "`{kind}` is still deferred after US3 shipped its repository, so a \
             queued row would wait for an upgrade that already happened: {body}"
        );
        assert_ne!(
            body["error"]["code"], "unsupported_kind",
            "`{kind}` still answers the drain's deferral code: {body}"
        );
    }
}

#[test]
fn a_pattern_promotion_reports_the_durability_it_actually_has() {
    let pg = pg!();
    // **This test used to assert the opposite, and the reversal is the point.**
    // Until T085 the route validated the shape, derived the identity and stored
    // nothing, so it answered `"stored": false` — reporting a durability it did
    // not have would have been the defect. US3 supplied the repository, so the
    // honest answer changed with it.
    //
    // What is unchanged is the rule: the flag reports what happened, and the row
    // count is what decides whether it is telling the truth.
    let (body, code) = post_json_status_bearer(
        &pg.server.base,
        "/api/patterns",
        &json!({
            "title": "sign images", "problem": "unsigned images are rejected",
            "root_cause": "no signer", "approach": "configure one",
            "constraints": [], "applicability": [],
        }),
        &pg.owner.token,
    );
    assert_eq!(code, 200, "{body}");
    assert_eq!(
        body["stored"], true,
        "the pattern route disclaimed durability it now has, which would tell a \
         caller to retry a promotion that already landed"
    );
    assert_eq!(
        pg.server.count(&format!(
            "SELECT count(*) FROM shared_patterns WHERE owner_user_id = '{}'",
            pg.owner.id
        )),
        1,
        "the route claimed to store a pattern and stored nothing"
    );
}

#[test]
fn an_envelope_without_a_command_id_is_refused() {
    let pg = pg!();
    // A queued command always has one; an envelope missing it is a client that
    // has lost the idempotency the spool derived for it.
    let mut body = envelope(
        "remember",
        Uuid::now_v7(),
        Some(pg.project),
        None,
        json!({ "type": "fact", "scope": "project", "content": "x" }),
    );
    body.as_object_mut().unwrap().remove("command_id");
    let (_, code) = deliver(&pg, &pg.owner, &body);
    assert_eq!(code, 400);
}

#[test]
fn an_envelope_cannot_name_an_account_or_an_authority() {
    let pg = pg!();
    // The daemon must not be able to invent authorization information, so
    // there is no field for it — the payload is screened exactly as a direct
    // call's body is.
    for field in [
        "account_id",
        "owner_user_id",
        "proposed_by_user_id",
        "verification_authority",
        "verification",
    ] {
        let mut payload = json!({ "type": "fact", "scope": "project", "content": "x" });
        payload[field] = json!(pg.member.id);
        let (_, code) = deliver(
            &pg,
            &pg.owner,
            &envelope("remember", Uuid::now_v7(), Some(pg.project), None, payload),
        );
        assert_eq!(
            code, 400,
            "an envelope was allowed to name `{field}` in its payload"
        );

        // At envelope level too. The dispatch folds only `payload` into the
        // handler's body, so a field named out here would be ignored rather
        // than refused if the envelope were not screened whole.
        let mut outer = envelope(
            "remember",
            Uuid::now_v7(),
            Some(pg.project),
            None,
            json!({ "type": "fact", "scope": "project", "content": "x" }),
        );
        outer[field] = json!(pg.member.id);
        let (_, code) = deliver(&pg, &pg.owner, &outer);
        assert_eq!(
            code, 400,
            "an envelope was allowed to name `{field}` at envelope level"
        );
    }

    // A deferred kind is screened too. It never reaches a handler, so without
    // screening at the envelope its payload would cross unchecked and be
    // answered with a deferral as though it were well formed.
    let mut payload = json!({});
    payload["verification_authority"] = json!("cairn");
    let (_, code) = deliver(
        &pg,
        &pg.owner,
        &envelope("verification_run", Uuid::now_v7(), None, None, payload),
    );
    assert_eq!(code, 400, "a deferred kind's payload was never screened");
}

#[test]
fn an_envelope_is_authorized_like_any_other_command() {
    let pg = pg!();
    let target = seed_memory(&pg, "a claim");
    assert_eq!(
        post_status_bearer(
            &pg.server.base,
            "/api/commands",
            &envelope(
                "remember",
                Uuid::now_v7(),
                Some(pg.project),
                None,
                json!({ "type": "fact", "scope": "project", "content": "x" })
            ),
            &pg.outsider.token
        ),
        403
    );
    assert_eq!(
        post_status_bearer(
            &pg.server.base,
            "/api/commands",
            &envelope("reinforce", Uuid::now_v7(), None, Some(target), json!({})),
            &pg.outsider.token
        ),
        404,
        "a record-addressed envelope leaked whether the record exists"
    );
    assert_eq!(
        post_status_bearer(
            &pg.server.base,
            "/api/commands",
            &envelope("personal_create", Uuid::now_v7(), None, None, json!({})),
            "not-a-real-token"
        ),
        401
    );
}

// ---------------------------------------------------------------------------
// Idempotency, atomically
// ---------------------------------------------------------------------------

#[test]
fn a_replayed_forget_is_a_duplicate_success_and_not_a_missing_record() {
    let pg = pg!();
    let personal = seed_personal(&pg, &pg.owner);
    let command_id = Uuid::now_v7();
    let body = envelope(
        "personal_forget",
        command_id,
        None,
        Some(personal),
        json!({}),
    );

    let (first, code) = deliver(&pg, &pg.owner, &body);
    assert_eq!(code, 200, "{first}");

    // The client acts as though the response was lost and sends it again.
    // Without the gate the replay finds `forgotten_at` already set, affects
    // zero rows, and answers 404 — telling the client its instruction failed
    // when it had already been carried out. The client's only correct response
    // to that is to retry forever.
    for _ in 0..3 {
        let (again, code) = deliver(&pg, &pg.owner, &body);
        assert_eq!(code, 200, "a replayed forget was refused: {again}");
        assert_eq!(again["applied"], "duplicate");
    }
}

#[test]
fn concurrent_deliveries_of_one_reinforce_count_once() {
    let pg = pg!();
    let target = seed_memory(&pg, "a claim");
    let command_id = Uuid::now_v7();
    let base = pg.server.base.clone();
    let token = pg.owner.token.clone();
    let body = envelope("reinforce", command_id, None, Some(target), json!({}));

    // Eight threads, one command id. The pre-read version had both readers see
    // "not applied", both increment, and one lose the receipt insert — so the
    // count went up twice and one caller was told `duplicate` about a write
    // that had happened anyway.
    let mut handles = Vec::new();
    for _ in 0..8 {
        let (base, token, body) = (base.clone(), token.clone(), body.clone());
        handles.push(std::thread::spawn(move || {
            post_status_bearer(&base, "/api/commands", &body, &token)
        }));
    }
    let codes: Vec<u16> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    assert!(
        codes.iter().all(|c| *c == 200),
        "a concurrent delivery failed: {codes:?}"
    );
    assert_eq!(
        pg.server.count(&format!(
            "SELECT reinforcement_count::bigint FROM memories WHERE id = '{target}'"
        )),
        1,
        "eight concurrent deliveries of one command incremented more than once"
    );
    assert_eq!(
        pg.server.count(&format!(
            "SELECT count(*) FROM applied_commands WHERE command_id = '{command_id}'"
        )),
        1
    );
}

#[test]
fn two_accounts_may_share_a_command_id_without_shadowing_each_other() {
    let pg = pg!();
    // A `command_id` is UUIDv5 over a scope kind, a scope key and an ordinal,
    // and a sessionless command's scope key is the *store's* writer id — so two
    // accounts on one machine derive the same ids for their own first commands.
    // Keyed on `command_id` alone, the second account's write would be answered
    // `duplicate` and silently never happen.
    let shared = Uuid::now_v7();
    for who in [&pg.owner, &pg.member] {
        let (body, code) = deliver(
            &pg,
            who,
            &envelope(
                "personal_create",
                shared,
                None,
                None,
                json!({ "knowledge_type": "fact", "content": "each account's own note" }),
            ),
        );
        assert_eq!(code, 200, "{body}");
        assert_ne!(
            body["applied"], "duplicate",
            "one account's command was suppressed by another's identical id"
        );
    }
    assert_eq!(
        pg.server.count(
            "SELECT count(*) FROM personal_knowledge
                          WHERE content = 'each account''s own note'"
        ),
        2,
        "two accounts' independent commands produced one record"
    );
    // Two reservations, one per account, and neither can read the other's.
    assert_eq!(
        pg.server.count(&format!(
            "SELECT count(*) FROM applied_commands WHERE command_id = '{shared}'"
        )),
        2
    );
}

#[test]
fn a_rolled_back_effect_leaves_the_command_replayable() {
    let pg = pg!();
    let theirs = seed_personal(&pg, &pg.member);
    let command_id = Uuid::now_v7();
    let body = envelope("personal_forget", command_id, None, Some(theirs), json!({}));

    // The owner is somebody else, so the effect fails and the transaction rolls
    // back — taking the reservation with it. A reservation that survived would
    // burn this command id, and the caller's own later, legitimate use of it
    // would be answered `duplicate` about a write that never happened.
    let (_, code) = deliver(&pg, &pg.owner, &body);
    assert_eq!(code, 404);
    assert_eq!(
        pg.server.count(&format!(
            "SELECT count(*) FROM applied_commands
              WHERE account_id = '{}' AND command_id = '{command_id}'",
            pg.owner.id
        )),
        0,
        "a failed command left a reservation behind, so its id is spent"
    );

    // The same id, now used for something the caller may actually do.
    let mine = seed_personal(&pg, &pg.owner);
    let (body, code) = deliver(
        &pg,
        &pg.owner,
        &envelope("personal_forget", command_id, None, Some(mine), json!({})),
    );
    assert_eq!(code, 200, "the command id was not replayable: {body}");
    assert_ne!(body["applied"], "duplicate");
}

#[test]
fn a_replayed_create_returns_the_original_record_rather_than_a_second_one() {
    let pg = pg!();
    let command_id = Uuid::now_v7();
    let body = envelope(
        "remember",
        command_id,
        Some(pg.project),
        None,
        json!({ "type": "decision", "scope": "project", "content": "one claim" }),
    );
    let (first, _) = deliver(&pg, &pg.owner, &body);
    for _ in 0..3 {
        let (again, code) = deliver(&pg, &pg.owner, &body);
        assert_eq!(code, 200);
        assert_eq!(
            again["id"], first["id"],
            "a replay produced a second record instead of returning the first"
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
fn a_replayed_relation_and_pin_are_duplicate_successes_too() {
    let pg = pg!();
    let a = seed_memory(&pg, "a");
    let b = seed_memory(&pg, "b");
    // State-idempotent shapes get protocol idempotency as well, so their replay
    // behaviour is the same as every other command's rather than being the one
    // exception a client has to know about.
    for (kind, project, target, payload) in [
        (
            "relate",
            Some(pg.project),
            None,
            json!({ "from_memory_id": a, "to_memory_id": b, "kind": "narrows" }),
        ),
        ("pin", None, Some(a), json!({ "pinned": true })),
    ] {
        let command_id = Uuid::now_v7();
        let body = envelope(kind, command_id, project, target, payload);
        let (_, code) = deliver(&pg, &pg.owner, &body);
        assert_eq!(code, 200);
        let (again, code) = deliver(&pg, &pg.owner, &body);
        assert_eq!(code, 200);
        assert_eq!(again["applied"], "duplicate", "`{kind}` replay: {again}");
    }
}
