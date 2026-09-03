//! The owner-only pattern lifecycle, end to end (T084, T090, US3).
//!
//! FR-708 asks that a reusable pattern survive the machine it was learned on.
//! It does **not** ask that it acquire an audience on the way, and this file
//! exists because those two are one edit apart: `shared_patterns` is a table
//! name describing where a pattern is stored, and every test here is about who
//! can reach it.
//!
//! The four properties, and each is falsifiable:
//!
//! - **Promotion converges** (SC-760, FR-708f). Identity is
//!   `UUIDv5(owner ‖ content_key)` over content that already crosses the
//!   boundary, so a retry, a replayed spool row and a re-run migration land on
//!   one record. Promoting the same pattern eleven times leaves one row.
//! - **A pattern is owner-only** (SC-761, FR-708d). A co-member of the same
//!   project sees nothing, reads nothing from the feed, and is refused the
//!   forget — with the *same* `404` an id that does not exist gets, so the
//!   route is not an enumeration oracle.
//! - **Trust is the server's to assign** (SC-762, FR-708g). No request produces
//!   any value but `sanitized`, and neither does the database.
//! - **Widening is a separate act** (FR-708e). No route and no command kind
//!   moves a pattern into the team domain; proposing the content as team
//!   guidance leaves the pattern exactly where it was.

use cairn_e2e::feature005::{Account, Pg};
use cairn_e2e::{get_json_status_bearer, post_json_status_bearer};
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
// Fixture content
// ---------------------------------------------------------------------------

/// A safe pattern body.
///
/// Deliberately free of anything the global content screen matches. The fixture
/// project is `feature005-fixture` on `git@example.test:feature005.git`, so
/// `all_identities_for` screens the owner's writes against `feature005`,
/// `example`, `test` and `git` — and `git` is three characters, so a word like
/// "digital" would be refused for reasons that have nothing to do with the
/// property under test. Every string in this file avoids them.
fn safe_pattern() -> Value {
    json!({
        "title": "sign images before release",
        "problem": "the pipeline refuses unsigned images",
        "root_cause": "no signer is configured in the release job",
        "approach": "configure a signer and re-run the release job",
        "constraints": ["the signer runs on the build host"],
        "applicability": ["release pipelines"],
    })
}

/// A second, different pattern — a different `content_key`, so a different row.
fn other_pattern() -> Value {
    json!({
        "title": "clear the cache on rollback",
        "problem": "stale rows survive a rollback",
        "root_cause": "the cache is not cleared when a rollback runs",
        "approach": "clear the cache in the rollback hook",
        "constraints": [],
        "applicability": [],
    })
}

// ---------------------------------------------------------------------------
// Request helpers
// ---------------------------------------------------------------------------

fn promote(pg: &Pg, who: &Account, body: &Value) -> (Value, u16) {
    post_json_status_bearer(&pg.server.base, "/api/patterns", body, &who.token)
}

fn forget(pg: &Pg, who: &Account, pattern: Uuid, body: &Value) -> (Value, u16) {
    post_json_status_bearer(
        &pg.server.base,
        &format!("/api/patterns/{pattern}/forget"),
        body,
        &who.token,
    )
}

fn list(pg: &Pg, who: &Account) -> (Value, u16) {
    get_json_status_bearer(&pg.server.base, "/api/patterns", &who.token)
}

fn changes(pg: &Pg, who: &Account) -> (Value, u16) {
    get_json_status_bearer(&pg.server.base, "/api/sync/changes/patterns", &who.token)
}

fn deliver(pg: &Pg, who: &Account, envelope: &Value) -> (Value, u16) {
    post_json_status_bearer(&pg.server.base, "/api/commands", envelope, &who.token)
}

fn envelope(kind: &str, command_id: Uuid, target: Option<Uuid>, payload: Value) -> Value {
    json!({
        "command_id": command_id,
        "kind": kind,
        "project_id": null,
        "target_id": target,
        "payload": payload,
    })
}

/// One statement against the server's database, returning the database's own
/// error rather than panicking.
///
/// A `CHECK` constraint that is never exercised may be inert. Asserting the
/// refusal is the only way to know the second line of the trust rule is really
/// there — the harness's `execute` panics, which cannot express "this was
/// supposed to fail".
fn try_execute(pg: &Pg, sql: &str) -> Result<(), String> {
    let url = pg.server.database_url.clone();
    let sql = sql.to_string();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    runtime.block_on(async move {
        let pool = sqlx::PgPool::connect(&url).await.expect("open server db");
        let out = sqlx::query(&sql)
            .execute(&pool)
            .await
            .map(|_| ())
            .map_err(|e| e.to_string());
        pool.close().await;
        out
    })
}

fn pattern_id_of(body: &Value) -> Uuid {
    body["pattern_id"]
        .as_str()
        .unwrap_or_else(|| panic!("no pattern_id in {body}"))
        .parse()
        .expect("pattern_id is a uuid")
}

// ---------------------------------------------------------------------------
// 1 — promotion converges (SC-760, FR-708f)
// ---------------------------------------------------------------------------

#[test]
fn promoting_one_pattern_repeatedly_leaves_exactly_one_record() {
    let pg = pg!();
    let body = safe_pattern();

    // No `command_id`: an interactive promotion, with no idempotency gate at
    // all. Convergence here comes from the derived identity and the unique
    // index, which is the property that has to hold — a spool replay is
    // *additionally* protected by the reservation, but a client that never
    // queued anything is not.
    let (first, code) = promote(&pg, &pg.owner, &body);
    assert_eq!(code, 200, "{first}");
    assert_eq!(
        first["stored"], true,
        "promotion reported no storage: {first}"
    );
    let id = pattern_id_of(&first);

    // With a `command_id` this time, so the two calls differ in every way a
    // client can vary them and still converge.
    let mut queued = body.clone();
    queued["command_id"] = json!(Uuid::now_v7());
    let (second, code) = promote(&pg, &pg.owner, &queued);
    assert_eq!(code, 200, "{second}");
    assert_eq!(
        pattern_id_of(&second),
        id,
        "the same content promoted twice produced two identities"
    );
    assert_eq!(
        pg.server.count("SELECT count(*) FROM shared_patterns"),
        1,
        "promoting the same pattern twice stored it twice"
    );

    // Ten more, because "converges" is a claim about repetition and two calls
    // is not repetition.
    for _ in 0..10 {
        let (again, code) = promote(&pg, &pg.owner, &body);
        assert_eq!(code, 200, "{again}");
        assert_eq!(pattern_id_of(&again), id);
    }
    assert_eq!(
        pg.server.count("SELECT count(*) FROM shared_patterns"),
        1,
        "eleven promotions of one pattern were not one record (SC-760)"
    );
    // And the owner's own list agrees with the table.
    let (listed, code) = list(&pg, &pg.owner);
    assert_eq!(code, 200, "{listed}");
    assert_eq!(listed["total"], 1, "{listed}");
}

#[test]
fn a_repeat_promotion_updates_the_label_without_forking_the_record() {
    let pg = pg!();
    let (first, _) = promote(&pg, &pg.owner, &safe_pattern());
    let id = pattern_id_of(&first);

    // The title is a label; the problem, cause and approach are the pattern
    // (`data-model.md` §6.2). Two promotions differing only in title collapse
    // to one record, and the later label wins.
    let mut relabelled = safe_pattern();
    relabelled["title"] = json!("sign every image");
    let (second, code) = promote(&pg, &pg.owner, &relabelled);
    assert_eq!(code, 200, "{second}");
    assert_eq!(pattern_id_of(&second), id);
    assert_eq!(pg.server.count("SELECT count(*) FROM shared_patterns"), 1);
    assert_eq!(
        pg.server.count(&format!(
            "SELECT count(*) FROM shared_patterns
              WHERE pattern_id = '{id}' AND title = 'sign every image'"
        )),
        1,
        "the upsert did not carry the new label"
    );
}

// ---------------------------------------------------------------------------
// 2 — owner-only, against a co-member and an outsider (SC-761, FR-708d)
// ---------------------------------------------------------------------------

#[test]
fn a_pattern_is_invisible_to_every_account_but_its_owner() {
    let pg = pg!();
    let (promoted, code) = promote(&pg, &pg.owner, &safe_pattern());
    assert_eq!(code, 200, "{promoted}");
    let id = pattern_id_of(&promoted);

    // `member` shares the fixture project with `owner`, which is the case that
    // matters: project membership is the widest relationship in this system,
    // and it still buys nothing here. `outsider` is the control.
    for (who, label) in [(&pg.member, "a co-member"), (&pg.outsider, "an outsider")] {
        let (listed, code) = list(&pg, who);
        assert_eq!(code, 200, "{label} was refused the list route: {listed}");
        assert_eq!(
            listed["total"], 0,
            "{label} was shown the owner's patterns: {listed}"
        );
        assert_eq!(
            listed["patterns"].as_array().map(Vec::len),
            Some(0),
            "{label} was shown the owner's patterns: {listed}"
        );

        let (feed, code) = changes(&pg, who);
        assert_eq!(code, 200, "{label} was refused the feed: {feed}");
        assert_eq!(
            feed["patterns"].as_array().map(Vec::len),
            Some(0),
            "{label} pulled the owner's patterns out of the changes feed: {feed}"
        );

        let (refusal, code) = forget(&pg, who, id, &json!({}));
        assert_eq!(
            code, 404,
            "{label} was allowed to forget the owner's pattern: {refusal}"
        );

        // The same answer an id that never existed gets. A `403` here, or any
        // other distinguishable status, would let any account with a token
        // enumerate pattern ids one guess at a time (FR-894a).
        let (absent, absent_code) = forget(&pg, who, Uuid::now_v7(), &json!({}));
        assert_eq!(
            absent_code, code,
            "the forget route tells {label} whether a pattern exists: {absent}"
        );
    }

    // The refusals were refusals, not silent successes.
    assert_eq!(
        pg.server.count(&format!(
            "SELECT count(*) FROM shared_patterns
              WHERE pattern_id = '{id}' AND owner_user_id = '{}'
                AND forgotten_at IS NULL",
            pg.owner.id
        )),
        1,
        "a refused forget removed the owner's pattern anyway"
    );
    let (owners, _) = list(&pg, &pg.owner);
    assert_eq!(owners["total"], 1, "{owners}");
}

// ---------------------------------------------------------------------------
// 3 — identity includes the owner
// ---------------------------------------------------------------------------

#[test]
fn two_accounts_promoting_identical_content_get_two_patterns() {
    let pg = pg!();
    let body = safe_pattern();
    let (mine, code) = promote(&pg, &pg.owner, &body);
    assert_eq!(code, 200, "{mine}");
    let (theirs, code) = promote(&pg, &pg.member, &body);
    assert_eq!(code, 200, "{theirs}");

    // `content_key` is the same — it digests the content and nothing else — and
    // the identities still differ, because `pattern_id` is
    // `UUIDv5(owner ‖ content_key)` and the unique index is on the pair.
    assert_eq!(
        mine["content_key"], theirs["content_key"],
        "identical content produced different content keys"
    );
    assert_ne!(
        pattern_id_of(&mine),
        pattern_id_of(&theirs),
        "one account's promotion collided with another's (SC-761)"
    );
    assert_eq!(
        pg.server.count("SELECT count(*) FROM shared_patterns"),
        2,
        "two owners promoting the same text share one row"
    );

    // And neither can see the other's.
    for (who, expected) in [
        (&pg.owner, pattern_id_of(&mine)),
        (&pg.member, pattern_id_of(&theirs)),
    ] {
        let (listed, _) = list(&pg, who);
        assert_eq!(listed["total"], 1, "{listed}");
        assert_eq!(
            listed["patterns"][0]["pattern_id"].as_str(),
            Some(expected.to_string().as_str()),
            "an account was shown a pattern that is not theirs: {listed}"
        );
    }
}

// ---------------------------------------------------------------------------
// 4 — a forgotten pattern is gone, and the going is what reaches a cache
// ---------------------------------------------------------------------------

#[test]
fn a_forgotten_pattern_leaves_the_list_and_reaches_a_cache_as_a_tombstone() {
    let pg = pg!();
    let (promoted, _) = promote(&pg, &pg.owner, &safe_pattern());
    let id = pattern_id_of(&promoted);
    let (kept, _) = promote(&pg, &pg.owner, &other_pattern());
    let kept = pattern_id_of(&kept);

    let (gone, code) = forget(&pg, &pg.owner, id, &json!({}));
    assert_eq!(code, 200, "{gone}");
    assert_eq!(gone["forgotten"], true, "{gone}");

    // Absent from the owner's living list, and the neighbour is untouched.
    let (listed, _) = list(&pg, &pg.owner);
    assert_eq!(
        listed["total"], 1,
        "a forgotten pattern is still listed: {listed}"
    );
    assert_eq!(
        listed["patterns"][0]["pattern_id"].as_str(),
        Some(kept.to_string().as_str()),
        "{listed}"
    );

    // Present in the feed as a tombstone. This is the only way a cache that
    // already holds the pattern learns it was withdrawn: a deleted row reaches
    // nobody, so the row stays and travels once more, empty.
    let (feed, code) = changes(&pg, &pg.owner);
    assert_eq!(code, 200, "{feed}");
    let rows = feed["patterns"].as_array().expect("patterns array");
    assert_eq!(rows.len(), 2, "the feed dropped a row: {feed}");
    let tomb = rows
        .iter()
        .find(|r| r["pattern_id"].as_str() == Some(id.to_string().as_str()))
        .unwrap_or_else(|| panic!("the forgotten pattern is not in the feed: {feed}"));
    assert!(
        tomb["forgotten_at"].is_string(),
        "the tombstone carries no forgotten_at, so a cache cannot tell it apart \
         from a live row: {tomb}"
    );
    for emptied in ["title", "problem", "root_cause", "approach"] {
        assert_eq!(
            tomb[emptied], "",
            "the tombstone still carries `{emptied}`: {tomb}"
        );
    }
    assert_eq!(tomb["constraints"], json!([]), "{tomb}");
    assert_eq!(tomb["applicability"], json!([]), "{tomb}");

    // Forgetting again is the state the caller asked for, so it succeeds. A
    // `404` would tell a client its instruction had failed when it had already
    // been carried out, and the only correct response to that is to retry
    // forever.
    let (again, code) = forget(&pg, &pg.owner, id, &json!({}));
    assert_eq!(code, 200, "a repeated forget was refused: {again}");
    assert_eq!(again["forgotten"], true, "{again}");

    // Re-promoting the same content revives the record rather than minting a
    // second one: the owner asked for exactly the pattern that is already
    // there.
    let (revived, code) = promote(&pg, &pg.owner, &safe_pattern());
    assert_eq!(code, 200, "{revived}");
    assert_eq!(pattern_id_of(&revived), id);
    assert_eq!(
        pg.server.count("SELECT count(*) FROM shared_patterns"),
        2,
        "reviving a forgotten pattern forked the record"
    );
    let (listed, _) = list(&pg, &pg.owner);
    assert_eq!(
        listed["total"], 2,
        "the revived pattern is not listed: {listed}"
    );
}

// ---------------------------------------------------------------------------
// 5 — trust is the server's to assign (SC-762, FR-708g)
// ---------------------------------------------------------------------------

#[test]
fn trust_cannot_be_asserted_by_a_client_or_written_by_hand() {
    let pg = pg!();

    // Both are refused, and the second is the one worth stating: asserting the
    // value the server would have chosen anyway is still an assertion, and
    // accepting it would make the field client-supplied on any request that
    // happened to guess right.
    for claimed in ["validated", "contested", "sanitized", "candidate"] {
        let mut body = safe_pattern();
        body["trust"] = json!(claimed);
        let (refusal, code) = promote(&pg, &pg.owner, &body);
        assert_eq!(code, 400, "`trust = {claimed}` was accepted: {refusal}");
        assert!(
            refusal["error"]["message"]
                .as_str()
                .is_some_and(|m| m.contains("trust")),
            "the refusal did not name `trust`: {refusal}"
        );
    }

    let (promoted, code) = promote(&pg, &pg.owner, &safe_pattern());
    assert_eq!(code, 200, "{promoted}");
    assert_eq!(promoted["trust"], "sanitized", "{promoted}");
    assert_eq!(
        pg.server
            .count("SELECT count(*) FROM shared_patterns WHERE trust = 'sanitized'"),
        1,
        "the stored trust is not the one the server assigns"
    );
    assert_eq!(
        pg.server
            .count("SELECT count(*) FROM shared_patterns WHERE trust <> 'sanitized'"),
        0,
        "some request produced a trust the server cannot establish (SC-762)"
    );

    // The second line of the same rule. A route can be edited; the column
    // `CHECK` is what makes `validated` unreachable even from a hand-written
    // statement, and a constraint nobody exercises may be inert.
    let refused = try_execute(&pg, "UPDATE shared_patterns SET trust = 'validated'");
    assert!(
        refused.is_err(),
        "the database accepted a trust level the server has no evidence for"
    );
    assert_eq!(
        pg.server
            .count("SELECT count(*) FROM shared_patterns WHERE trust = 'sanitized'"),
        1,
        "the refused update changed the row anyway"
    );
}

// ---------------------------------------------------------------------------
// 6 — the owner comes from the credential (Principle XI, FR-769)
// ---------------------------------------------------------------------------

#[test]
fn the_owner_is_bound_from_the_credential_and_never_from_the_payload() {
    let pg = pg!();

    // Naming somebody else's account is the attack this refusal exists for: an
    // accepted `owner_user_id` would let any account plant a pattern in a
    // colleague's private store, or read one back by promoting over it.
    let mut impersonating = safe_pattern();
    impersonating["owner_user_id"] = json!(pg.member.id);
    let (refusal, code) = promote(&pg, &pg.owner, &impersonating);
    assert_eq!(code, 400, "a payload named its own owner: {refusal}");
    assert!(
        refusal["error"]["message"]
            .as_str()
            .is_some_and(|m| m.contains("owner_user_id")),
        "the refusal did not name the field: {refusal}"
    );
    assert_eq!(
        pg.server.count("SELECT count(*) FROM shared_patterns"),
        0,
        "a refused promotion stored a row"
    );

    // Nested one level down, because a check that only looked at the top level
    // would be defeated by wrapping.
    let mut wrapped = safe_pattern();
    wrapped["meta"] = json!({ "owner_user_id": pg.member.id });
    assert_eq!(promote(&pg, &pg.owner, &wrapped).1, 400);

    // A body naming no owner at all stores the authenticated account.
    let (promoted, code) = promote(&pg, &pg.owner, &safe_pattern());
    assert_eq!(code, 200, "{promoted}");
    assert_eq!(
        promoted["owner_user_id"].as_str(),
        Some(pg.owner.id.to_string().as_str()),
        "{promoted}"
    );
    assert_eq!(
        pg.server.count(&format!(
            "SELECT count(*) FROM shared_patterns
              WHERE pattern_id = '{}' AND owner_user_id = '{}'",
            pattern_id_of(&promoted),
            pg.owner.id
        )),
        1,
        "the stored owner is not the authenticated account"
    );
}

// ---------------------------------------------------------------------------
// 7 — the refused local field names still do not cross (FR-708a)
// ---------------------------------------------------------------------------

#[test]
fn every_refused_local_field_is_refused_by_name() {
    let pg = pg!();
    // One assertion per name rather than a loop over a list, because the point
    // of each is different — `origin_ref` and `source_memory_id` name the source
    // project, `signals` and `signal_digest` are local evidence,
    // `sanitization_report` is local diagnostic output, `origin_deleted` is a
    // fact about a local row — and a caller must be told *which* one it sent.
    for field in [
        "signals",
        "signal_digest",
        "origin_ref",
        "sanitization_report",
        "source_memory_id",
        "origin_deleted",
    ] {
        let mut body = safe_pattern();
        body[field] = json!("anything at all");
        let (refusal, code) = promote(&pg, &pg.owner, &body);
        assert_eq!(code, 400, "`{field}` crossed the boundary: {refusal}");
        assert!(
            refusal["error"]["message"]
                .as_str()
                .is_some_and(|m| m.contains(field)),
            "the refusal of `{field}` did not name it, so a client cannot tell \
             which field it must drop: {refusal}"
        );
        assert!(
            !refusal["error"]["message"]
                .as_str()
                .is_some_and(|m| m.contains("anything at all")),
            "the refusal echoed the value it refused: {refusal}"
        );
    }
    assert_eq!(
        pg.server.count("SELECT count(*) FROM shared_patterns"),
        0,
        "a refused promotion stored a row anyway"
    );
}

// ---------------------------------------------------------------------------
// 8 — content screening still runs
// ---------------------------------------------------------------------------

#[test]
fn a_pattern_that_names_a_project_is_refused_like_any_other_global_record() {
    let pg = pg!();
    // Screened against every project the caller belongs to, not the one a
    // request happens to name — a pattern is project-independent, so naming any
    // of the author's projects discloses one (FR-708a, FR-822).
    let mut body = safe_pattern();
    body["problem"] = json!("the release job in feature005-fixture refuses unsigned images");
    let (refusal, code) = promote(&pg, &pg.owner, &body);
    assert_eq!(code, 400, "a pattern named its source project: {refusal}");
    assert!(
        refusal["error"]["message"]
            .as_str()
            .is_some_and(|m| m.contains("project_identifying")),
        "the refusal did not name the class the screen matched: {refusal}"
    );

    // The screen reaches the list fields too. A constraint is free text on the
    // same record, and a project named there is exactly as disclosing.
    let mut in_a_list = safe_pattern();
    in_a_list["applicability"] = json!(["feature005-fixture release pipelines"]);
    assert_eq!(
        promote(&pg, &pg.owner, &in_a_list).1,
        400,
        "applicability is not screened"
    );
    let mut in_constraints = safe_pattern();
    in_constraints["constraints"] = json!(["only on feature005-fixture"]);
    assert_eq!(
        promote(&pg, &pg.owner, &in_constraints).1,
        400,
        "constraints are not screened"
    );

    assert_eq!(pg.server.count("SELECT count(*) FROM shared_patterns"), 0);
}

// ---------------------------------------------------------------------------
// 9 — widening to a team is a separate, explicit act (FR-708e)
// ---------------------------------------------------------------------------

#[test]
fn nothing_widens_a_pattern_to_the_team_domain() {
    let pg = pg!();
    let (promoted, _) = promote(&pg, &pg.owner, &safe_pattern());
    let id = pattern_id_of(&promoted);
    let promoted_at = pg.server.text(&format!(
        "SELECT CAST(updated_at AS TEXT) FROM shared_patterns WHERE pattern_id = '{id}'"
    ));

    // No command kind widens one. A kind the envelope does not know is refused
    // as not-a-command-kind — a `400`, not the `409` deferral, because a
    // deferral would tell a drain to keep retrying a command that will never
    // exist.
    for invented in ["pattern_share", "pattern_promote_team", "pattern_widen"] {
        let (refusal, code) = deliver(
            &pg,
            &pg.owner,
            &envelope(invented, Uuid::now_v7(), Some(id), json!({})),
        );
        assert_eq!(
            code, 400,
            "`{invented}` was answered as though it were a command: {refusal}"
        );
        assert!(
            refusal["error"]["message"]
                .as_str()
                .is_some_and(|m| m.contains("is not a command kind")),
            "`{invented}` was refused for the wrong reason: {refusal}"
        );
    }

    // And no route. A pattern-specific sharing path is precisely what
    // `data-model.md` §6.2 says does not exist.
    for path in [
        format!("/api/patterns/{id}/share"),
        format!("/api/patterns/{id}/team"),
        format!("/api/patterns/{id}/promote"),
    ] {
        let (body, code) =
            post_json_status_bearer(&pg.server.base, &path, &json!({}), &pg.owner.token);
        assert_eq!(code, 404, "{path} exists: {body}");
    }

    // The one widening path is the ordinary team proposal, which creates a
    // *separate* record for an administrator to ratify.
    let (proposed, code) = post_json_status_bearer(
        &pg.server.base,
        "/api/team/knowledge",
        &json!({
            "knowledge_type": "convention",
            "content": "sign images before release",
        }),
        &pg.owner.token,
    );
    assert_eq!(code, 200, "{proposed}");
    assert_eq!(proposed["state"], "proposed", "{proposed}");

    // The pattern is untouched by it: same domain, same owner, not forgotten,
    // not even a new `updated_at`.
    assert_eq!(
        pg.server.count(&format!(
            "SELECT count(*) FROM shared_patterns
              WHERE pattern_id = '{id}' AND domain = 'personal'
                AND owner_user_id = '{}' AND forgotten_at IS NULL",
            pg.owner.id
        )),
        1,
        "proposing team guidance changed the pattern"
    );
    assert_eq!(
        pg.server.text(&format!(
            "SELECT CAST(updated_at AS TEXT) FROM shared_patterns WHERE pattern_id = '{id}'"
        )),
        promoted_at,
        "proposing team guidance rewrote the pattern row"
    );
    // Still owner-only afterwards: widening the content did not widen the
    // pattern.
    let (member_sees, _) = list(&pg, &pg.member);
    assert_eq!(
        member_sees["total"], 0,
        "a team proposal made the author's pattern visible: {member_sees}"
    );
}

// ---------------------------------------------------------------------------
// 10 / 11 — the envelope carries patterns, idempotently, per account
// ---------------------------------------------------------------------------

#[test]
fn a_queued_pattern_command_is_no_longer_deferred() {
    let pg = pg!();
    let (body, code) = deliver(
        &pg,
        &pg.owner,
        &envelope("pattern_promote", Uuid::now_v7(), None, safe_pattern()),
    );
    assert_ne!(
        code, 409,
        "`pattern_promote` still answers a deferral, so a queued promotion \
         waits for a phase that has already shipped: {body}"
    );
    assert_eq!(code, 200, "{body}");
    assert_eq!(body["stored"], true, "{body}");
    assert_eq!(pg.server.count("SELECT count(*) FROM shared_patterns"), 1);
}

#[test]
fn the_same_envelope_delivered_twice_applies_once() {
    let pg = pg!();
    let promote_id = Uuid::now_v7();
    let first = envelope("pattern_promote", promote_id, None, safe_pattern());

    let (one, code) = deliver(&pg, &pg.owner, &first);
    assert_eq!(code, 200, "{one}");
    let pattern = pattern_id_of(&one);

    let (two, code) = deliver(&pg, &pg.owner, &first);
    assert_eq!(code, 200, "{two}");
    assert_eq!(
        two["applied"], "duplicate",
        "a replayed promotion was applied again: {two}"
    );
    assert_eq!(
        two["id"].as_str(),
        Some(pattern.to_string().as_str()),
        "the duplicate reply did not carry what the original produced: {two}"
    );
    assert_eq!(pg.server.count("SELECT count(*) FROM shared_patterns"), 1);

    // The same for a forget.
    let forget_id = Uuid::now_v7();
    let tombstone = envelope("pattern_forget", forget_id, Some(pattern), json!({}));
    let (one, code) = deliver(&pg, &pg.owner, &tombstone);
    assert_eq!(code, 200, "{one}");
    assert_eq!(one["forgotten"], true, "{one}");
    let (two, code) = deliver(&pg, &pg.owner, &tombstone);
    assert_eq!(code, 200, "{two}");
    assert_eq!(
        two["applied"], "duplicate",
        "a replayed forget was not recognised: {two}"
    );
}

#[test]
fn one_command_id_under_two_accounts_is_two_commands() {
    let pg = pg!();
    // A `command_id` is `UUIDv5` over a scope kind, a scope key and an ordinal,
    // and a sessionless command's scope key is the *store's* `writer_id` — so
    // two accounts on one machine derive identical ids for their own first
    // commands. Keyed on `command_id` alone, the second account's promotion
    // would be answered `duplicate` about a write that never happened.
    let shared = Uuid::now_v7();
    let command = envelope("pattern_promote", shared, None, safe_pattern());

    let (mine, code) = deliver(&pg, &pg.owner, &command);
    assert_eq!(code, 200, "{mine}");
    let (theirs, code) = deliver(&pg, &pg.member, &command);
    assert_eq!(code, 200, "{theirs}");

    assert_ne!(
        theirs["applied"], "duplicate",
        "a second account's first command was answered as somebody else's \
         replay: {theirs}"
    );
    assert_eq!(theirs["stored"], true, "{theirs}");
    assert_ne!(pattern_id_of(&mine), pattern_id_of(&theirs));
    assert_eq!(
        pg.server.count("SELECT count(*) FROM shared_patterns"),
        2,
        "one of the two accounts lost its promotion to the other's command id"
    );
    for (who, expected) in [
        (&pg.owner, pattern_id_of(&mine)),
        (&pg.member, pattern_id_of(&theirs)),
    ] {
        let (listed, _) = list(&pg, who);
        assert_eq!(listed["total"], 1, "{listed}");
        assert_eq!(
            listed["patterns"][0]["pattern_id"].as_str(),
            Some(expected.to_string().as_str()),
            "{listed}"
        );
    }
}

/// Percent-encode a cursor for a query string.
///
/// **Not optional, and the reason is easy to miss.** A cursor is
/// `<rfc3339>|<uuid>`, and an RFC 3339 instant ends `+00:00`. A bare `+` in a
/// query string decodes to a *space*, so an unencoded cursor reaches the server
/// as an unparsable timestamp — and `PageCursor::decode` is deliberately
/// lenient, falling back to the start of the feed rather than erroring. The
/// visible symptom is the feed re-delivering a page it had already served,
/// which reads exactly like a broken cursor comparison and is not one.
///
/// The daemon does this in `sync::urlencode`; this mirrors it so the test
/// exercises the same request the daemon actually sends.
fn urlencode(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '.' | '~' => c.to_string(),
            other => format!("%{:02X}", other as u32),
        })
        .collect()
}

// ---------------------------------------------------------------------------
// The read routes are authenticated and account-scoped
// ---------------------------------------------------------------------------

#[test]
fn the_pattern_read_routes_refuse_an_unauthenticated_caller() {
    let pg = pg!();
    let (_, _) = promote(&pg, &pg.owner, &safe_pattern());
    for path in ["/api/patterns", "/api/sync/changes/patterns"] {
        let code = pg.server.get_status(path, "not-a-real-token");
        assert_eq!(
            code, 401,
            "{path} is reachable without authentication, which would hand every \
             account's patterns to anyone who can reach the port"
        );
    }
}

#[test]
fn the_changes_feed_resumes_from_its_own_cursor() {
    let pg = pg!();
    promote(&pg, &pg.owner, &safe_pattern());
    let (first, code) = changes(&pg, &pg.owner);
    assert_eq!(code, 200, "{first}");
    assert_eq!(first["patterns"].as_array().map(Vec::len), Some(1));
    let cursor = first["cursor"].as_str().expect("a cursor").to_string();

    // Nothing new: the same cursor returns an empty page rather than the row
    // again.
    let (empty, code) = get_json_status_bearer(
        &pg.server.base,
        &format!("/api/sync/changes/patterns?since={}", urlencode(&cursor)),
        &pg.owner.token,
    );
    assert_eq!(code, 200, "{empty}");
    assert_eq!(
        empty["patterns"].as_array().map(Vec::len),
        Some(0),
        "the feed re-delivered a row the cursor had already passed: {empty}"
    );

    // A forget moves the row back into the feed, which is the whole reason the
    // cursor is `GREATEST(created_at, updated_at, forgotten_at)` and not
    // `created_at`: a cache that had already passed the row would otherwise
    // never learn it was withdrawn.
    let (listed, _) = list(&pg, &pg.owner);
    let id: Uuid = listed["patterns"][0]["pattern_id"]
        .as_str()
        .expect("a pattern")
        .parse()
        .expect("uuid");
    forget(&pg, &pg.owner, id, &json!({}));
    let (after, code) = get_json_status_bearer(
        &pg.server.base,
        &format!("/api/sync/changes/patterns?since={}", urlencode(&cursor)),
        &pg.owner.token,
    );
    assert_eq!(code, 200, "{after}");
    assert_eq!(
        after["patterns"].as_array().map(Vec::len),
        Some(1),
        "a forget never reached the feed, so no cache can learn of it: {after}"
    );
    assert!(after["patterns"][0]["forgotten_at"].is_string(), "{after}");
}
