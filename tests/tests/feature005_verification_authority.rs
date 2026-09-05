//! Who may say a check ran, and what the server is willing to believe about it
//! (T125, US6, SC-765, SC-767, FR-811b, FR-811h, FR-811i).
//!
//! One thesis, stated once: **route names, payload fields, discriminator values
//! and optimistic interpretation must never manufacture verification.** A
//! bearer token proves which account is talking. An HTTP path proves which URL
//! was requested. Neither proves that a deterministic Cairn verifier executed
//! anywhere, so neither may produce an authority that says one did
//! (`contracts/verification-summary.md` §4).
//!
//! Everything below is a consequence:
//!
//! - **Both routes assign `remote_attested`.** `/api/verification/runs` reads
//!   like "Cairn ran a check" and is not: an authenticated client said it did.
//!   `/api/verification/attestations` is the same claim with a different shape.
//!   Choosing between them is choosing a discriminator, and a discriminator is
//!   caller input.
//! - **`cairn` is unreachable over HTTP.** It is reserved for a check the
//!   server itself executed, in process. No request, on either route, with any
//!   body this file can construct, may produce it.
//! - **`remote_cairn` has no producer in baseline Feature 005.** The enum value
//!   survives; nothing writes it until a future specification names an evidence
//!   mechanism that establishes client-side Cairn execution.
//! - **Identity is server-assigned and complete.** The caller does not choose
//!   the `report_id`, and naming one is refused rather than ignored — a client
//!   whose id was silently dropped believes it can address the report later.
//!   The natural key is the whole logical reference plus the authenticated
//!   account, the verifier kind and `run_at`, so one account's retry is one
//!   report and two accounts' reports are two.
//! - **State is derived, in the transaction that accepted the report.** A
//!   refused report moves nothing, and a duplicate moves nothing — without
//!   which §6's rule that leaving `conflicted` costs a *second, subsequent*
//!   passed run is satisfied by sending one run twice.
//!
//! What would falsify the file as a whole: a single row in
//! `verification_reports` whose `authority` is anything but `remote_attested`
//! after every request below has been made.

use cairn_e2e::feature005::{Account, Pg};
use cairn_e2e::post_json_status_bearer;
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
// The two routes, and the request shape they share
// ---------------------------------------------------------------------------

/// A client report shaped as a deterministic run (§4, row 2).
const RUNS: &str = "/api/verification/runs";
/// An agent attestation relayed by a client (§4, row 3).
const ATTESTATIONS: &str = "/api/verification/attestations";

/// Both, always iterated together.
///
/// Every authority assertion in this file is made about *both* routes, because
/// the property under test is precisely that they are not distinguishable in
/// their result. A test that exercised one would pass on a server that treated
/// the other as a stronger boundary, which is the bug (SC-765).
const ROUTES: [&str; 2] = [RUNS, ATTESTATIONS];

/// The extra field `/attestations` requires beyond `/runs` (§4).
///
/// §4 permits the two routes to differ in payload validation — `/runs` takes
/// the declared `VerifierKind` vocabulary, `/attestations` additionally
/// requires the named attesting agent — and says in the same breath that the
/// difference describes the *reported check shape*, not established execution
/// provenance. So the field exists, and changes nothing about authority.
const ATTESTING_AGENT: &str = "claude-code";

/// A `KnowledgeRef` as it travels: the domain is part of the reference, never
/// a hint (§3, §5).
fn knowledge_ref(domain: &str, id: Uuid) -> Value {
    json!({ "domain": domain, "knowledge_id": id })
}

/// A `PatternRef`: `pattern_id` and no domain slot (`data-model.md` §6.1).
fn pattern_ref(id: Uuid) -> Value {
    json!({ "pattern_id": id })
}

/// The whole legal payload, per §3, plus the one field `/attestations` adds.
///
/// Deliberately nothing else. §3 is explicit that this *is* the payload: no
/// observed value, no locator, no digest, no command output, no local path,
/// and no `authority` or `report_id`.
fn report_body(route: &str, reference: Value, verdict: &str, kind: &str, run_at: &str) -> Value {
    let mut body = json!({
        "memory_ref": reference,
        "verdict": verdict,
        "verifier_kind": kind,
        "run_at": run_at,
    });
    if route == ATTESTATIONS {
        body["attesting_agent"] = json!(ATTESTING_AGENT);
    }
    body
}

fn post(pg: &Pg, who: &Account, route: &str, body: &Value) -> (Value, u16) {
    post_json_status_bearer(&pg.server.base, route, body, &who.token)
}

/// A run timestamp, distinct per index and stable across a run of the suite.
///
/// Stable rather than `now()`, because `run_at` is a quarter of the natural key
/// (§5) and a fixture that minted a fresh timestamp per attempt would make
/// every retry a new report — which would let the idempotency tests pass
/// against a server that had no idempotency at all.
fn run_at(n: u32) -> String {
    // Carried into the hour once `n` reaches sixty. The first spelling formatted
    // `n` straight into the minute field, so `run_at(60)` produced
    // `09:60:00Z` — not a time, refused as malformed, and the refusal then read
    // as the *server* rejecting a legal report.
    format!("2026-08-30T{:02}:{:02}:00Z", 9 + n / 60, n % 60)
}

// ---------------------------------------------------------------------------
// Reading the server's own record back
// ---------------------------------------------------------------------------

fn reports_total(pg: &Pg) -> i64 {
    pg.server.count("SELECT count(*) FROM verification_reports")
}

fn reports_where(pg: &Pg, predicate: &str) -> i64 {
    pg.server.count(&format!(
        "SELECT count(*) FROM verification_reports WHERE {predicate}"
    ))
}

/// Every distinct `authority` the server has actually stored.
///
/// The set rather than a count, so a failure names the value that appeared
/// instead of merely reporting that one did.
fn stored_authorities(pg: &Pg) -> Vec<String> {
    pg.server
        .query_column("SELECT DISTINCT authority FROM verification_reports ORDER BY authority")
}

/// One project memory's derived summary, as the four values §6 derives.
#[derive(Debug, PartialEq, Eq)]
struct Summary {
    state: String,
    authority: String,
    last_verified_at: String,
    basis: Vec<String>,
    evidence_fact_count: i64,
}

fn project_summary(pg: &Pg, memory: Uuid) -> Summary {
    // `COALESCE` throughout: `memories.verification` is nullable and a record
    // nobody has reported on has never been written, so NULL is the honest
    // starting value and the fixture must be able to read it without the
    // decoder refusing it.
    let row = pg.server.query_column(&format!(
        "SELECT COALESCE(verification, '')
             || '|' || COALESCE(verification_authority, '')
             || '|' || COALESCE(to_char(last_verified_at AT TIME ZONE 'UTC',
                                        'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"'), '')
             || '|' || COALESCE(verification_basis::text, '[]')
             || '|' || evidence_fact_count::text
           FROM memories WHERE id = '{memory}'"
    ));
    let raw = row
        .first()
        .unwrap_or_else(|| panic!("no memories row for {memory}"));
    let parts: Vec<&str> = raw.splitn(5, '|').collect();
    let basis: Value = serde_json::from_str(parts[3]).unwrap_or(json!([]));
    Summary {
        state: parts[0].to_string(),
        authority: parts[1].to_string(),
        last_verified_at: parts[2].to_string(),
        basis: basis
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|v| v.as_str().unwrap_or_default().to_string())
                    .collect()
            })
            .unwrap_or_default(),
        evidence_fact_count: parts[4].parse().unwrap_or(-1),
    }
}

/// The derived state alone, which is what the transition table is about.
fn state_of(pg: &Pg, memory: Uuid) -> String {
    let state = project_summary(pg, memory).state;
    // The table's `unverified` row is the starting row, and a record no report
    // has reached has a NULL column rather than the string — the column is
    // nullable and Feature 005 adds no default. Both mean the same thing, and
    // §6's point is that this is the *truthful* value, so they are read as one.
    if state.is_empty() {
        "unverified".to_string()
    } else {
        state
    }
}

// ---------------------------------------------------------------------------
// Fixture records, one per domain
// ---------------------------------------------------------------------------

/// One reportable record in each of the four references, with **different**
/// UUIDs.
///
/// Different here on purpose: the identical-UUID case is its own test below and
/// in `feature005_verification_summaries.rs`, and mixing the two would make a
/// failure ambiguous between "the authority rule broke" and "the reference key
/// collapsed".
struct Refs {
    project: Uuid,
    personal: Uuid,
    team: Uuid,
    pattern: Uuid,
}

fn seed_refs(pg: &Pg) -> Refs {
    let session = pg.session_for(&pg.owner);
    let project = Uuid::now_v7();
    pg.server.execute(&format!(
        "INSERT INTO memories (id, project_id, type, scope, scope_key, content,
                               origin_session_id)
         VALUES ('{project}', '{}', 'fact', 'project', '{}',
                 'the release job signs images', '{session}')",
        pg.project, pg.project
    ));

    let personal = Uuid::now_v7();
    pg.server.execute(&format!(
        "INSERT INTO personal_knowledge
             (id, owner_user_id, knowledge_type, content, writer_id, writer_seq)
         VALUES ('{personal}', '{}', 'fact', 'the owner prefers one signer',
                 'authority-fixture-{personal}', 1)",
        pg.owner.id
    ));

    // Ratified rather than proposed, so the team row is reportable by *any*
    // authenticated account (§7). The proposed case has its own authorization
    // test in `feature005_verification_summaries.rs`; here the point is that
    // two accounts reporting one team record produce two reports, and a
    // `proposed` row would refuse the second for an unrelated reason.
    let team = Uuid::now_v7();
    pg.server.execute(&format!(
        "INSERT INTO team_knowledge
             (id, knowledge_type, content, state, proposed_by_user_id,
              ratified_by_user_id, ratified_at, writer_id, writer_seq)
         VALUES ('{team}', 'convention', 'the team signs every release image',
                 'authoritative', '{}', '{}', now(),
                 'authority-fixture-{team}', 2)",
        pg.owner.id, pg.owner.id
    ));

    let pattern = Uuid::now_v7();
    assert!(
        pg.seed_pattern_with_id(&pg.owner, pattern, "sign images before release"),
        "the pattern fixture needs `shared_patterns`, which server schema v4 adds"
    );

    Refs {
        project,
        personal,
        team,
        pattern,
    }
}

// ---------------------------------------------------------------------------
// Assertion helpers
// ---------------------------------------------------------------------------

/// A refusal, with the code the contract names.
///
/// Both the status and the code, because §10.5 is explicit that a payload
/// naming authority is *refused rather than ignored*, and a server that
/// answered `200` with the field dropped would satisfy an assertion about the
/// code alone if it happened to emit one.
fn assert_refused(what: &str, outcome: (Value, u16), code: &str) {
    let (body, status) = outcome;
    assert_eq!(
        status, 400,
        "{what}: expected a 400 refusal, got {status} with body {body}"
    );
    assert_eq!(
        body["error"]["code"],
        json!(code),
        "{what}: expected error code `{code}`, got body {body}"
    );
}

fn assert_accepted(what: &str, outcome: &(Value, u16)) -> Uuid {
    let (body, status) = outcome;
    assert_eq!(
        *status, 200,
        "{what}: expected the report to be accepted with 200, got {status} \
         with body {body}"
    );
    let id = body["report_id"]
        .as_str()
        .unwrap_or_else(|| panic!("{what}: no `report_id` in the reply {body}"));
    Uuid::parse_str(id).unwrap_or_else(|_| panic!("{what}: `report_id` is not a UUID: {id}"))
}

// ---------------------------------------------------------------------------
// 1. Both routes assign `remote_attested`, and nothing else does
// ---------------------------------------------------------------------------

#[test]
fn both_routes_assign_remote_attested_to_every_accepted_report() {
    let pg = pg!();
    let refs = seed_refs(&pg);

    let mut n = 0;
    for (i, route) in ROUTES.iter().enumerate() {
        // Every domain and every verdict, on both routes. The matrix is the
        // assertion: SC-765 says "in 100% of trials", and a single happy-path
        // request would pass on a server that special-cased one shape.
        for (j, reference) in [
            knowledge_ref("project", refs.project),
            knowledge_ref("personal", refs.personal),
            knowledge_ref("team", refs.team),
            pattern_ref(refs.pattern),
        ]
        .into_iter()
        .enumerate()
        {
            for (k, verdict) in ["passed", "failed", "inconclusive"].iter().enumerate() {
                let at = run_at((i * 12 + j * 3 + k) as u32);
                let body = report_body(route, reference.clone(), verdict, "test_outcome", &at);
                let outcome = post(&pg, &pg.owner, route, &body);
                let what = format!("{route} {verdict} on {reference}");
                let report_id = assert_accepted(&what, &outcome);
                n += 1;

                // The reply says it too. A stored value the caller cannot see
                // is a value the caller keeps guessing about, and the guess a
                // client makes about `/runs` is exactly the wrong one.
                assert_eq!(
                    outcome.0["authority"],
                    json!("remote_attested"),
                    "{what}: the reply must state the authority the server assigned, \
                     got {}",
                    outcome.0
                );
                let stored = pg.server.text(&format!(
                    "SELECT authority FROM verification_reports WHERE report_id = '{report_id}'"
                ));
                assert_eq!(
                    stored, "remote_attested",
                    "{what}: stored authority must be `remote_attested`; a route name \
                     establishes no verifier (§4, FR-811h)"
                );
            }
        }
    }

    assert_eq!(reports_total(&pg), n, "every accepted report is one row");
    assert_eq!(
        stored_authorities(&pg),
        vec!["remote_attested".to_string()],
        "after {n} accepted reports across both routes, `remote_attested` must be \
         the only authority the database holds"
    );
}

// ---------------------------------------------------------------------------
// 2. Authority cannot be selected — by name, or by smuggling
// ---------------------------------------------------------------------------

#[test]
fn an_authority_named_in_the_body_is_refused_on_both_routes() {
    let pg = pg!();
    let refs = seed_refs(&pg);

    // `remote_attested` is on the list deliberately. Naming the value the
    // server would have assigned anyway is still an assertion of authority, and
    // accepting it teaches a client that the field is honoured — after which
    // sending `cairn` is a one-word edit.
    for (i, claimed) in [
        "cairn",
        "remote_cairn",
        "local_cairn",
        "attested",
        "remote_attested",
    ]
    .iter()
    .enumerate()
    {
        for route in ROUTES {
            let mut body = report_body(
                route,
                knowledge_ref("project", refs.project),
                "passed",
                "test_outcome",
                &run_at(i as u32),
            );
            body["authority"] = json!(claimed);
            assert_refused(
                &format!("{route} carrying authority={claimed}"),
                post(&pg, &pg.owner, route, &body),
                "authority_not_assertable",
            );
        }
    }

    assert_eq!(
        reports_total(&pg),
        0,
        "a refused payload stores nothing: the refusal and the absence of a row \
         are the same guarantee (§10.5, §10.7)"
    );
    assert_eq!(
        state_of(&pg, refs.project),
        "unverified",
        "a refused report establishes no summary, so the record stays unverified \
         (FR-811d)"
    );
}

#[test]
fn a_nested_authority_is_refused_too() {
    let pg = pg!();
    let refs = seed_refs(&pg);

    // Wrapping is the obvious defeat for a top-level-only check, and the
    // existing boundaries on this server already screen recursively
    // (`commands::reject_server_owned`, `events::carries_refused_name`). A
    // third boundary that did not would be the drift FR-777a1 forbids.
    for route in ROUTES {
        let mut body = report_body(
            route,
            knowledge_ref("project", refs.project),
            "passed",
            "test_outcome",
            &run_at(1),
        );
        body["memory_ref"]["authority"] = json!("cairn");
        assert_refused(
            &format!("{route} carrying a nested authority"),
            post(&pg, &pg.owner, route, &body),
            "authority_not_assertable",
        );
    }
    assert_eq!(reports_total(&pg), 0);
}

#[test]
fn the_server_owned_verification_columns_may_not_be_sent_either() {
    let pg = pg!();
    let refs = seed_refs(&pg);

    // The same five values `/api/sync/batch` used to take verbatim (§2). They
    // are already on `commands::COMPUTED_FIELDS`, and this route must not be
    // the one place a client can still name them — the bypass §2 describes was
    // exactly one boundary forgetting.
    for (i, field) in [
        "verification",
        "verification_authority",
        "verification_basis",
        "evidence_fact_count",
        "last_verified_at",
    ]
    .iter()
    .enumerate()
    {
        for route in ROUTES {
            let mut body = report_body(
                route,
                knowledge_ref("project", refs.project),
                "passed",
                "test_outcome",
                &run_at(20 + i as u32),
            );
            body[*field] = json!("verified");
            let (reply, status) = post(&pg, &pg.owner, route, &body);
            assert_eq!(
                status, 400,
                "{route} carrying `{field}`: a computed value a client names must be \
                 refused, not ignored; got {status} with {reply}"
            );
        }
    }
    assert_eq!(reports_total(&pg), 0);
    assert_eq!(state_of(&pg, refs.project), "unverified");
}

#[test]
fn a_verifier_kind_of_cairn_does_not_become_an_authority() {
    let pg = pg!();
    let refs = seed_refs(&pg);

    // The discriminator smuggle. `verifier_kind` is a closed vocabulary (§10.4,
    // `VerifierKind` in `cairn-core::domain`), and `cairn` is not in it — but
    // the assertion that matters is the second one: even if a build widened the
    // vocabulary tomorrow, a *kind* is a description of the check that ran and
    // can never be promoted into a statement about who ran it.
    for (i, kind) in ["cairn", "remote_cairn", "attested", "server", "trusted"]
        .iter()
        .enumerate()
    {
        for route in ROUTES {
            let body = report_body(
                route,
                knowledge_ref("project", refs.project),
                "passed",
                kind,
                &run_at(30 + i as u32),
            );
            let (reply, status) = post(&pg, &pg.owner, route, &body);
            assert_eq!(
                status, 400,
                "{route} with verifier_kind={kind}: the vocabulary is closed (§10.4); \
                 got {status} with {reply}"
            );
        }
    }

    assert_eq!(
        reports_where(&pg, "authority <> 'remote_attested'"),
        0,
        "no verifier-kind value produced an authority other than `remote_attested`"
    );
    assert_eq!(reports_total(&pg), 0, "and none of them was stored at all");
}

// ---------------------------------------------------------------------------
// 3. The server assigns the report id
// ---------------------------------------------------------------------------

#[test]
fn the_server_assigns_the_report_id_and_two_reports_get_two_ids() {
    let pg = pg!();
    let refs = seed_refs(&pg);

    let first = assert_accepted(
        "the first report",
        &post(
            &pg,
            &pg.owner,
            RUNS,
            &report_body(
                RUNS,
                knowledge_ref("project", refs.project),
                "passed",
                "test_outcome",
                &run_at(1),
            ),
        ),
    );
    let second = assert_accepted(
        "a second, genuinely different report",
        &post(
            &pg,
            &pg.owner,
            ATTESTATIONS,
            &report_body(
                ATTESTATIONS,
                knowledge_ref("project", refs.project),
                "passed",
                "file_exists",
                &run_at(2),
            ),
        ),
    );

    assert_ne!(
        first, second,
        "two distinct reports are two identities; a shared id would make the \
         second unaddressable"
    );
    for id in [first, second] {
        assert_eq!(
            reports_where(&pg, &format!("report_id = '{id}'")),
            1,
            "the id the reply named must be the id the row carries — a reply id \
             the database does not hold is a promise the client cannot redeem"
        );
    }
}

#[test]
fn a_caller_supplied_report_id_is_refused_rather_than_ignored() {
    let pg = pg!();
    let refs = seed_refs(&pg);
    let chosen = Uuid::now_v7();

    for route in ROUTES {
        let mut body = report_body(
            route,
            knowledge_ref("project", refs.project),
            "passed",
            "test_outcome",
            &run_at(3),
        );
        body["report_id"] = json!(chosen);
        // Ignored would be worse than refused and quieter: the client believes
        // it can address the report by the id it chose, and every later
        // reference to it silently names nothing (§5, §10.5).
        assert_refused(
            &format!("{route} carrying a caller-chosen report_id"),
            post(&pg, &pg.owner, route, &body),
            "authority_not_assertable",
        );
    }

    assert_eq!(
        reports_where(&pg, &format!("report_id = '{chosen}'")),
        0,
        "the caller's id must not have been stored"
    );
    assert_eq!(
        reports_total(&pg),
        0,
        "and the report must not have been stored under a different id either — \
         'refused' means the request did not happen"
    );
}

// ---------------------------------------------------------------------------
// 4. Baseline Feature 005 produces no `remote_cairn`, and no `cairn`
// ---------------------------------------------------------------------------

#[test]
fn no_request_on_either_route_produces_cairn_or_remote_cairn() {
    let pg = pg!();
    let refs = seed_refs(&pg);

    // Everything this file knows how to ask for, in one pass: legal reports on
    // both routes for all four references, and every attempt to name a stronger
    // authority. The accepted ones must all be `remote_attested`; the refused
    // ones must be absent. Either way the two reserved values must not appear.
    let mut accepted = 0;
    for (i, route) in ROUTES.iter().enumerate() {
        for (j, reference) in [
            knowledge_ref("project", refs.project),
            knowledge_ref("personal", refs.personal),
            knowledge_ref("team", refs.team),
            pattern_ref(refs.pattern),
        ]
        .into_iter()
        .enumerate()
        {
            let at = run_at((i * 4 + j) as u32);
            let ok = report_body(route, reference.clone(), "passed", "git_commit", &at);
            if post(&pg, &pg.owner, route, &ok).1 == 200 {
                accepted += 1;
            }

            for claimed in ["cairn", "remote_cairn"] {
                let mut hostile = ok.clone();
                hostile["authority"] = json!(claimed);
                let _ = post(&pg, &pg.owner, route, &hostile);

                let mut hostile_kind = ok.clone();
                hostile_kind["verifier_kind"] = json!(claimed);
                let _ = post(&pg, &pg.owner, route, &hostile_kind);
            }
        }
    }

    assert_eq!(
        accepted, 8,
        "the eight legal reports (two routes, four references) must all have been \
         accepted, or the negative assertion below is vacuous"
    );
    assert_eq!(
        reports_where(&pg, "authority = 'cairn'"),
        0,
        "`cairn` is assigned only by the server's own verifier, in process; no \
         HTTP request may reach it (§4)"
    );
    assert_eq!(
        reports_where(&pg, "authority = 'remote_cairn'"),
        0,
        "baseline Feature 005 has no producer for `remote_cairn` at all — the enum \
         value survives and nothing writes it (§4, FR-811h)"
    );
    assert_eq!(
        stored_authorities(&pg),
        vec!["remote_attested".to_string()],
        "and no third value appeared by another path"
    );
    assert_eq!(
        pg.server.count(
            "SELECT count(*) FROM memories WHERE verification_authority IS NOT NULL
               AND verification_authority <> 'remote_attested'"
        ),
        0,
        "the derived summary inherits the report's authority and may not exceed it"
    );
}

// ---------------------------------------------------------------------------
// 5. Duplicate identity is the complete natural key
// ---------------------------------------------------------------------------

#[test]
fn one_account_retrying_one_logical_report_is_one_row() {
    let pg = pg!();
    let refs = seed_refs(&pg);
    let body = report_body(
        RUNS,
        knowledge_ref("project", refs.project),
        "passed",
        "test_outcome",
        &run_at(5),
    );

    let first = assert_accepted("the first delivery", &post(&pg, &pg.owner, RUNS, &body));
    let (reply, status) = post(&pg, &pg.owner, RUNS, &body);

    // A success, and it says which kind. A retry answered `409` would tell a
    // spool its instruction failed when it had already been carried out, and
    // the spool's only correct response to that is to retry forever — the same
    // reasoning `commands::duplicate_reply` records.
    assert_eq!(
        status, 200,
        "a retry of an accepted report is a success, not an error: got {status} \
         with {reply}"
    );
    assert_eq!(
        reply["applied"],
        json!("duplicate"),
        "the reply must say the report was already held, so a client can tell a \
         second acceptance from a replay: got {reply}"
    );
    assert_eq!(
        reply["report_id"],
        json!(first.to_string()),
        "and it must name the original report, not mint a second identity"
    );
    assert_eq!(reports_total(&pg), 1, "one logical report is one row (§5)");
}

#[test]
fn each_component_of_the_natural_key_makes_a_different_report() {
    let pg = pg!();
    let refs = seed_refs(&pg);
    let base = report_body(
        RUNS,
        knowledge_ref("project", refs.project),
        "passed",
        "test_outcome",
        &run_at(6),
    );
    assert_accepted("the baseline report", &post(&pg, &pg.owner, RUNS, &base));

    // Each variant changes exactly one component of
    // `(reference_key, account_id, verifier_kind, run_at)` and must therefore
    // be a second report. A key missing any one of these would collapse the
    // corresponding pair, and §5 explains what each collapse costs.
    let mut different_verifier = base.clone();
    different_verifier["verifier_kind"] = json!("file_digest");
    let mut different_run_at = base.clone();
    different_run_at["run_at"] = json!(run_at(7));
    let mut different_reference = base.clone();
    different_reference["memory_ref"] = knowledge_ref("personal", refs.personal);

    for (what, body) in [
        ("a different verifier kind", different_verifier),
        ("a different run_at", different_run_at),
        ("a different reference", different_reference),
    ] {
        let (reply, status) = post(&pg, &pg.owner, RUNS, &body);
        assert_eq!(status, 200, "{what}: {reply}");
        assert_eq!(
            reply["applied"],
            json!("accepted"),
            "{what} is a distinct report, not a duplicate: {reply}"
        );
    }
    assert_eq!(reports_total(&pg), 4);

    // The route is *not* part of the key. The same logical report relayed on
    // the other path is the same report — otherwise a client could double its
    // evidence count by alternating URLs, which is the route-as-authority bug
    // wearing a different hat.
    let (reply, status) = post(&pg, &pg.owner, ATTESTATIONS, &{
        let mut b = base.clone();
        b["attesting_agent"] = json!(ATTESTING_AGENT);
        b
    });
    assert_eq!(status, 200, "{reply}");
    assert_eq!(
        reply["applied"],
        json!("duplicate"),
        "the route is not part of report identity (§5 names four components, and \
         the path is not one of them): {reply}"
    );
    assert_eq!(reports_total(&pg), 4);
}

#[test]
fn two_accounts_reporting_one_project_reference_are_two_reports() {
    let pg = pg!();
    let refs = seed_refs(&pg);

    // Identical in every component except the credential. Two accounts are two
    // pieces of evidence, and collapsing them would silently discard the second
    // machine's run (§5, FR-811i).
    let body = report_body(
        RUNS,
        knowledge_ref("project", refs.project),
        "passed",
        "test_outcome",
        &run_at(8),
    );
    let mine = assert_accepted("the owner's report", &post(&pg, &pg.owner, RUNS, &body));
    let theirs = assert_accepted(
        "a co-member's identical report",
        &post(&pg, &pg.member, RUNS, &body),
    );

    assert_ne!(mine, theirs);
    assert_eq!(reports_total(&pg), 2);
    assert_eq!(
        reports_where(
            &pg,
            &format!(
                "reference_key = 'knowledge:project:{}' AND account_id IN ('{}', '{}')",
                refs.project, pg.owner.id, pg.member.id
            )
        ),
        2,
        "both rows name the same reference and different accounts"
    );
    assert_eq!(
        stored_authorities(&pg),
        vec!["remote_attested".to_string()],
        "and neither account gained authority from the other's agreement — two \
         attestations are two attestations, not a deterministic check (FR-811i)"
    );
}

#[test]
fn two_accounts_reporting_one_team_reference_are_two_reports() {
    let pg = pg!();
    let refs = seed_refs(&pg);

    // Team knowledge is the sharper case: it belongs to no project and to no
    // single owner, so an implementation keying identity on the record alone
    // has nothing else to fall back on and would collapse these two.
    let body = report_body(
        ATTESTATIONS,
        knowledge_ref("team", refs.team),
        "passed",
        "command_outcome",
        &run_at(9),
    );
    assert_accepted(
        "the owner's team report",
        &post(&pg, &pg.owner, ATTESTATIONS, &body),
    );
    assert_accepted(
        "a second account's identical team report",
        &post(&pg, &pg.member, ATTESTATIONS, &body),
    );

    assert_eq!(
        reports_where(
            &pg,
            &format!("reference_key = 'knowledge:team:{}'", refs.team)
        ),
        2
    );
    assert_eq!(stored_authorities(&pg), vec!["remote_attested".to_string()]);
}

#[test]
fn the_domain_is_part_of_report_identity() {
    let pg = pg!();
    let ids = pg.seed_identical_ids(&pg.owner);

    // One UUID, four references, one account, one verifier kind, one `run_at`.
    // Every component of the natural key except the reference is deliberately
    // held constant, so anything that keys on the bare UUID collapses all four
    // into one row and the count below says so (SC-767).
    let at = run_at(10);
    for reference in [
        knowledge_ref("project", ids.project_memory),
        knowledge_ref("personal", ids.personal),
        knowledge_ref("team", ids.team),
        pattern_ref(ids.pattern),
    ] {
        let body = report_body(RUNS, reference.clone(), "passed", "schema_version", &at);
        assert_accepted(
            &format!("a report about {reference}"),
            &post(&pg, &pg.owner, RUNS, &body),
        );
    }

    assert_eq!(reports_total(&pg), 4, "four references are four reports");
    let mut keys = pg
        .server
        .query_column("SELECT reference_key FROM verification_reports ORDER BY reference_key");
    keys.sort();
    let mut expected = ids.reference_keys().to_vec();
    expected.sort();
    assert_eq!(
        keys, expected,
        "each report carries the complete logical reference; a bare UUID would \
         make these four one (FR-819a, SC-767)"
    );
}

// ---------------------------------------------------------------------------
// 6. Derived state, in the transaction that accepted the report
// ---------------------------------------------------------------------------

/// Post one report per verdict, in order, against one project memory.
///
/// Each gets its own `run_at`, so every step is a genuinely new run rather than
/// a retry — which is the distinction §5 exists to protect.
fn drive(pg: &Pg, memory: Uuid, verdicts: &[&str]) -> String {
    for (i, verdict) in verdicts.iter().enumerate() {
        let at = run_at(40 + i as u32);
        let body = report_body(
            RUNS,
            knowledge_ref("project", memory),
            verdict,
            "test_outcome",
            &at,
        );
        let outcome = post(pg, &pg.owner, RUNS, &body);
        assert_accepted(&format!("step {i} of the sequence ({verdict})"), &outcome);
    }
    state_of(pg, memory)
}

fn fresh_memory(pg: &Pg, session: Uuid, label: &str) -> Uuid {
    let id = Uuid::now_v7();
    pg.server.execute(&format!(
        "INSERT INTO memories (id, project_id, type, scope, scope_key, content,
                               origin_session_id)
         VALUES ('{id}', '{}', 'fact', 'project', '{}', '{label}', '{session}')",
        pg.project, pg.project
    ));
    id
}

#[test]
fn the_derivation_walks_the_published_transition_table() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);

    // §6's table, every cell that a report history can reach, expressed as the
    // sequence that reaches it. The prefix establishes the "current" column and
    // the last verdict is the one under test.
    //
    // `drifted` has no producer here on purpose: no verdict transitions *to*
    // `drifted` in §6, so it cannot be the current state of a record whose
    // history is reports alone. Its row is covered separately below.
    let cells: &[(&str, &[&str], &str)] = &[
        ("unverified + passed", &["passed"], "verified"),
        ("unverified + failed", &["failed"], "unverified"),
        ("unverified + inconclusive", &["inconclusive"], "unverified"),
        ("verified + passed", &["passed", "passed"], "verified"),
        ("verified + failed", &["passed", "failed"], "conflicted"),
        (
            "verified + inconclusive",
            &["passed", "inconclusive"],
            "verified",
        ),
        (
            "conflicted + passed",
            &["passed", "failed", "passed"],
            "needs_recheck",
        ),
        (
            "conflicted + failed",
            &["passed", "failed", "failed"],
            "conflicted",
        ),
        (
            "conflicted + inconclusive",
            &["passed", "failed", "inconclusive"],
            "conflicted",
        ),
        (
            "needs_recheck + passed",
            &["passed", "failed", "passed", "passed"],
            "verified",
        ),
        (
            "needs_recheck + failed",
            &["passed", "failed", "passed", "failed"],
            "needs_recheck",
        ),
        (
            "needs_recheck + inconclusive",
            &["passed", "failed", "passed", "inconclusive"],
            "needs_recheck",
        ),
    ];

    for (cell, verdicts, expected) in cells {
        let memory = fresh_memory(&pg, session, cell);
        assert_eq!(
            &drive(&pg, memory, verdicts),
            expected,
            "§6 cell `{cell}`: the sequence {verdicts:?} must derive `{expected}`"
        );
    }
}

#[test]
fn a_contradiction_costs_one_more_deliberate_run() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    let memory = fresh_memory(&pg, session, "the release job signs images");

    let passed = report_body(
        RUNS,
        knowledge_ref("project", memory),
        "passed",
        "test_outcome",
        &run_at(11),
    );
    let mut failed = passed.clone();
    failed["verdict"] = json!("failed");
    failed["run_at"] = json!(run_at(12));

    assert_accepted("the passing run", &post(&pg, &pg.owner, RUNS, &passed));
    assert_eq!(state_of(&pg, memory), "verified");
    assert_accepted(
        "the contradicting run",
        &post(&pg, &pg.owner, RUNS, &failed),
    );
    assert_eq!(
        state_of(&pg, memory),
        "conflicted",
        "a `failed` report against a `verified` record is a contradiction, not a \
         demotion — silently dropping to `unverified` would lose the fact that \
         the two runs disagree (§6)"
    );

    // The attack §5 names: resubmit the *original* passing run. Same reference,
    // same account, same verifier kind, same `run_at` — one report, replayed.
    let (reply, status) = post(&pg, &pg.owner, RUNS, &passed);
    assert_eq!(status, 200, "{reply}");
    assert_eq!(reply["applied"], json!("duplicate"), "{reply}");
    assert_eq!(
        state_of(&pg, memory),
        "conflicted",
        "a duplicate changes no state. Without that, §6's requirement of a \
         *second, subsequent* passed run is satisfied by sending one run twice \
         and the guarantee is decorative (§5)"
    );

    // Only a genuinely new run moves it, and it lands on `needs_recheck` rather
    // than back on `verified`.
    let mut again = passed.clone();
    again["run_at"] = json!(run_at(13));
    assert_accepted(
        "a second, subsequent passing run",
        &post(&pg, &pg.owner, RUNS, &again),
    );
    assert_eq!(
        state_of(&pg, memory),
        "needs_recheck",
        "exit from `conflicted` never lands directly on `verified` (§6)"
    );
}

#[test]
fn basis_and_count_and_timestamp_are_maintained_by_the_server() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    let memory = fresh_memory(&pg, session, "the release job signs images");

    // Three accepted reports of two distinct kinds, one of them replayed.
    let kinds = ["test_outcome", "file_exists", "test_outcome"];
    for (i, kind) in kinds.iter().enumerate() {
        let body = report_body(
            RUNS,
            knowledge_ref("project", memory),
            "passed",
            kind,
            &run_at(50 + i as u32),
        );
        assert_accepted(&format!("report {i}"), &post(&pg, &pg.owner, RUNS, &body));
        // The replay: identical to the report just accepted.
        let (reply, _) = post(&pg, &pg.owner, RUNS, &body);
        assert_eq!(reply["applied"], json!("duplicate"), "{reply}");
    }

    let summary = project_summary(&pg, memory);
    let mut basis = summary.basis.clone();
    basis.sort();
    assert_eq!(
        basis,
        vec!["file_exists".to_string(), "test_outcome".to_string()],
        "`verification_basis` accumulates the verifier *kinds* of accepted \
         reports, deduplicated (§4) — never a subject, a value or a locator"
    );
    assert_eq!(
        summary.evidence_fact_count, 3,
        "`evidence_fact_count` counts accepted reports and never a \
         client-supplied number; the three replays must not have counted (§4, §5)"
    );
    assert_eq!(
        summary.last_verified_at, "2026-08-30T09:52:00Z",
        "`last_verified_at` is the `run_at` of the most recent accepted passed \
         report, not the time the server received it (§6)"
    );
    assert_eq!(summary.authority, "remote_attested");
}

#[test]
fn a_refused_report_moves_no_state_and_leaves_no_row() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    let memory = fresh_memory(&pg, session, "the release job signs images");

    let good = report_body(
        RUNS,
        knowledge_ref("project", memory),
        "passed",
        "test_outcome",
        &run_at(14),
    );
    assert_accepted(
        "the report that is accepted",
        &post(&pg, &pg.owner, RUNS, &good),
    );
    let before = project_summary(&pg, memory);
    assert_eq!(before.state, "verified");

    // §10.8 puts the insert and the re-derivation in one transaction. From
    // outside the server the observable consequence is this pair: a request
    // that was refused leaves *neither* a report nor a moved state, and a
    // request that succeeded leaves both. A derivation running outside the
    // transaction would eventually show one without the other.
    for (what, mutate) in [
        (
            "a failing report that also names an authority",
            json!({ "authority": "cairn", "verdict": "failed" }),
        ),
        (
            "a failing report that also names a report id",
            json!({ "report_id": Uuid::now_v7(), "verdict": "failed" }),
        ),
        (
            "a failing report with a verdict outside the vocabulary",
            json!({ "verdict": "probably" }),
        ),
        (
            "a failing report with a verifier kind outside the vocabulary",
            json!({ "verdict": "failed", "verifier_kind": "vibes" }),
        ),
    ] {
        let mut body = good.clone();
        body["run_at"] = json!(run_at(60));
        for (k, v) in mutate.as_object().expect("an object of overrides") {
            body[k.as_str()] = v.clone();
        }
        let (reply, status) = post(&pg, &pg.owner, RUNS, &body);
        assert_eq!(status, 400, "{what}: got {status} with {reply}");
        assert_eq!(
            project_summary(&pg, memory),
            before,
            "{what}: a refused report must move nothing — not the state, not the \
             basis, not the count, not the timestamp (FR-811d, §10)"
        );
        assert_eq!(
            reports_total(&pg),
            1,
            "{what}: and it must leave no row behind"
        );
    }
}

#[test]
fn a_record_with_no_accepted_report_carries_no_summary() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    let reported = fresh_memory(&pg, session, "the release job signs images");
    let untouched = fresh_memory(&pg, session, "the rollback hook clears the cache");

    assert_accepted(
        "the one report in this test",
        &post(
            &pg,
            &pg.owner,
            RUNS,
            &report_body(
                RUNS,
                knowledge_ref("project", reported),
                "passed",
                "runtime_state",
                &run_at(15),
            ),
        ),
    );

    let quiet = project_summary(&pg, untouched);
    assert_eq!(
        quiet.state, "",
        "a record nobody reported on is not `verified`, and the server does not \
         invent a state it cannot justify (FR-811d)"
    );
    assert_eq!(quiet.authority, "");
    assert_eq!(quiet.last_verified_at, "");
    assert!(quiet.basis.is_empty());
    assert_eq!(quiet.evidence_fact_count, 0);

    // The converse, so the assertion above is not satisfied by a server that
    // derives nothing at all.
    assert_eq!(project_summary(&pg, reported).state, "verified");
}

#[test]
fn a_drifted_record_is_never_promoted_by_a_report_that_did_not_pass() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);

    // §6's `drifted` row is the one the report history cannot reach on its own:
    // no verdict transitions *to* `drifted`, so a record can only arrive there
    // from a local run or a pre-cutover value. Rather than pre-setting the
    // column and asserting an outcome that depends on whether the server folds
    // the whole history or applies one step to the stored value — a choice §6
    // does not settle — this asserts the part that holds either way, which is
    // also the part that matters: a report that did not pass never produces
    // `verified`, and one that did always does.
    for (verdict, forbidden) in [("failed", "verified"), ("inconclusive", "verified")] {
        let memory = fresh_memory(&pg, session, "the release job signs images");
        pg.server.execute(&format!(
            "UPDATE memories SET verification = 'drifted' WHERE id = '{memory}'"
        ));
        let body = report_body(
            RUNS,
            knowledge_ref("project", memory),
            verdict,
            "file_digest",
            &run_at(70),
        );
        assert_accepted(
            &format!("a {verdict} report on a drifted record"),
            &post(&pg, &pg.owner, RUNS, &body),
        );
        let after = state_of(&pg, memory);
        assert_ne!(
            after, forbidden,
            "a `{verdict}` report must never establish `{forbidden}`; the record \
             was `drifted` and nothing about this report contradicts that (§6)"
        );
        assert!(
            after == "drifted" || after == "unverified",
            "a `{verdict}` report against a `drifted` record leaves it `drifted` \
             (one step from the stored value) or `unverified` (a fold over a \
             history containing no pass); got `{after}`"
        );
    }

    let memory = fresh_memory(&pg, session, "the rollback hook clears the cache");
    pg.server.execute(&format!(
        "UPDATE memories SET verification = 'drifted' WHERE id = '{memory}'"
    ));
    assert_accepted(
        "a passing report on a drifted record",
        &post(
            &pg,
            &pg.owner,
            RUNS,
            &report_body(
                RUNS,
                knowledge_ref("project", memory),
                "passed",
                "file_digest",
                &run_at(71),
            ),
        ),
    );
    assert_eq!(
        state_of(&pg, memory),
        "verified",
        "and a pass does establish `verified` from `drifted`, so the assertions \
         above are not satisfied by a server that derives nothing"
    );
}

// ---------------------------------------------------------------------------
// 7. The report binds through the record, not through the reporter
// ---------------------------------------------------------------------------

#[test]
fn the_report_row_binds_to_the_referenced_record_not_the_reporters_context() {
    let pg = pg!();
    let refs = seed_refs(&pg);

    for (i, (reference, project, owner)) in [
        (
            knowledge_ref("project", refs.project),
            Some(pg.project.to_string()),
            None,
        ),
        (
            knowledge_ref("personal", refs.personal),
            None,
            Some(pg.owner.id.to_string()),
        ),
        (knowledge_ref("team", refs.team), None, None),
        (
            pattern_ref(refs.pattern),
            None,
            Some(pg.owner.id.to_string()),
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let body = report_body(
            RUNS,
            reference.clone(),
            "passed",
            "git_ref",
            &run_at(80 + i as u32),
        );
        let id = assert_accepted(
            &format!("a report about {reference}"),
            &post(&pg, &pg.owner, RUNS, &body),
        );

        let bound = pg.server.text(&format!(
            "SELECT COALESCE(project_id::text, '') || '|' || COALESCE(owner_user_id::text, '')
               FROM verification_reports WHERE report_id = '{id}'"
        ));
        let (got_project, got_owner) = bound.split_once('|').expect("two bound columns");
        assert_eq!(
            got_project,
            project.unwrap_or_default(),
            "{reference}: `project_id` comes from the referenced record. Personal, \
             team and pattern records are project-independent, and naming the \
             project the reporter happened to be in would leak it (§7, FR-822)"
        );
        assert_eq!(
            got_owner,
            owner.unwrap_or_default(),
            "{reference}: `owner_user_id` is the record's owner, not the account \
             that reported. They coincide on the personal and pattern rows only \
             because the fixture owner owns both; a server reading this column \
             from the credential is caught by the team row, which has no owner"
        );
        assert_eq!(
            pg.server.text(&format!(
                "SELECT account_id::text FROM verification_reports WHERE report_id = '{id}'"
            )),
            pg.owner.id.to_string(),
            "{reference}: `account_id` is the authenticated reporter, and it is \
             the only column that is (Principle XI)"
        );
    }
}

// ---------------------------------------------------------------------------
// T136 — the hostile route choice
//
// Reserved for the explicit regression T136 adds once T133 has these tests
// passing: a caller who deliberately picks the stronger-*sounding* URL, with
// the most deterministic-looking verifier kind and every field it can name,
// and still cannot reach `remote_cairn` or `cairn` (SC-765). It belongs at the
// end of this file rather than folded into
// `no_request_on_either_route_produces_cairn_or_remote_cairn` above, because
// that test states the invariant for the ordinary population and T136 states
// it for an adversary — and a regression that lives inside a matrix loop stops
// being findable by the name of the thing it protects.
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// T136 — the hostile route choice (SC-765)
// ---------------------------------------------------------------------------

/// A caller who wants stronger provenance cannot get it by picking the
/// stronger-sounding URL, or by dressing the payload to match it.
///
/// # Why this is its own test
///
/// The tests above prove each rule separately: both routes assign
/// `remote_attested`, an `authority` field is refused, a `verifier_kind` of
/// `cairn` is not a kind. This one plays the adversary — somebody who has read
/// the contract, understands that `cairn` is the authority they want, and tries
/// every combination the surface offers to reach it.
///
/// The distinction it is built around is the one SC-765 turns on: **`/runs`
/// sounds like Cairn ran a check, and it is not.** It is an authenticated client
/// *saying* one ran. Both routes are equally strong precisely because neither
/// establishes execution, and a build that quietly made `/runs` mean more would
/// pass every test above while failing this one.
///
/// **Falsified by** returning anything but `remote_attested` from
/// `assign_authority`, by giving it an argument to branch on, or by mapping
/// `verifier_kind` onto authority anywhere.
#[test]
fn no_combination_of_route_payload_and_kind_reaches_cairn() {
    let pg = pg!();
    let refs = seed_refs(&pg);

    // Every shape an adversary has to work with: the two routes, the four
    // references, and payloads that variously assert the authority outright,
    // spell it as a verifier kind, bury it one level down, or name the sibling
    // value that has no producer at all.
    let references = [
        ("project", knowledge_ref("project", refs.project)),
        ("personal", knowledge_ref("personal", refs.personal)),
        ("team", knowledge_ref("team", refs.team)),
        ("pattern", pattern_ref(refs.pattern)),
    ];
    let dressings: [(&str, fn(&mut Value)); 6] = [
        ("plain", |_| {}),
        ("authority=cairn", |b| b["authority"] = json!("cairn")),
        ("authority=remote_cairn", |b| {
            b["authority"] = json!("remote_cairn")
        }),
        ("verification_authority=cairn", |b| {
            b["verification_authority"] = json!("cairn")
        }),
        ("nested authority", |b| {
            b["provenance"] = json!({ "verification_authority": "cairn" })
        }),
        // The subtlest one: no refused field anywhere, just a verifier kind
        // named after the authority. Nothing here is a lie the screen can catch
        // — it is a legal-looking payload whose only hope is that the server
        // maps kind onto authority.
        ("verifier_kind=cairn", |b| {
            b["verifier_kind"] = json!("cairn")
        }),
    ];

    let mut accepted = 0;
    let mut minute = 0;
    for (which, reference) in &references {
        for route in [RUNS, ATTESTATIONS] {
            for (label, dress) in &dressings {
                let mut body = report_body(
                    route,
                    reference.clone(),
                    "passed",
                    "test_outcome",
                    &run_at(120 + minute),
                );
                minute += 1;
                dress(&mut body);
                let (reply, status) = post(&pg, &pg.owner, route, &body);

                // A dressed payload is refused; a plain one is accepted. Either
                // way the interesting assertion is the same one, made below over
                // everything the server actually stored.
                if *label == "plain" {
                    assert_eq!(
                        status, 200,
                        "{route} {which} {label}: a legal report must still be \
                         accepted, or the negative assertion below is vacuous — \
                         got {reply}"
                    );
                    assert_eq!(
                        reply["authority"],
                        json!("remote_attested"),
                        "{route} {which}: the stronger-sounding route assigned a \
                         stronger authority — a URL is caller-selected input and \
                         cannot establish what ran (§4)"
                    );
                    accepted += 1;
                } else {
                    assert_ne!(
                        status, 200,
                        "{route} {which} {label}: accepted a payload reaching for \
                         an authority the caller cannot establish — got {reply}"
                    );
                }
            }
        }
    }
    assert_eq!(
        accepted, 8,
        "the eight legal reports (two routes, four references) must all have \
         landed, or this test proves nothing about what the server stores"
    );

    // The whole of it, read off the server's own rows rather than its replies:
    // one authority exists, and it is the weak one.
    assert_eq!(
        stored_authorities(&pg),
        vec!["remote_attested".to_string()],
        "an authority other than `remote_attested` reached the database. \
         `cairn` is assignable only by a deterministic check the server itself \
         ran, and `remote_cairn` has no producer in baseline Feature 005 \
         (SC-765)"
    );
    assert_eq!(
        reports_where(&pg, "authority IN ('cairn', 'remote_cairn', 'attested')"),
        0,
        "a report carries an authority no HTTP route may produce"
    );

    // And the summaries derived from them say the same thing. An authority that
    // could not be stored on a report but appeared on the record it produced
    // would be the same overclaim one derivation later.
    assert_eq!(
        pg.server.count(
            "SELECT count(*) FROM memories
              WHERE verification_authority IS NOT NULL
                AND verification_authority <> 'remote_attested'"
        ),
        0,
        "a project summary carries an authority no report established"
    );
    assert_eq!(
        pg.server.count(
            "SELECT count(*) FROM knowledge_verification
              WHERE verification_authority IS NOT NULL
                AND verification_authority <> 'remote_attested'"
        ),
        0,
        "a non-project summary carries an authority no report established"
    );
}

/// A knowledge reference arriving without its domain is refused, over HTTP.
///
/// # Why this exists separately from the unit test
///
/// `Reference::parse` has a unit test for exactly this, and a mutation that
/// made a domainless reference default to `project` was caught by it — and by
/// **nothing in this file or in the summaries file**. Both suites always send a
/// domain, because both were written to exercise what a correct client does, so
/// a server that quietly guessed would have shipped with two full e2e suites
/// green.
///
/// The guess is not a small thing to get wrong. The same UUID can name a
/// project memory, a personal note, a team entry and a pattern at once, so
/// defaulting to `project` files one record's verification against another's —
/// and does it silently, because the caller asked about *a* record and got a
/// `200` about a different one.
///
/// **Falsified by** giving `Reference::parse` any default for a missing domain.
#[test]
fn a_reference_that_names_no_domain_is_refused_over_http() {
    let pg = pg!();
    let refs = seed_refs(&pg);

    for route in [RUNS, ATTESTATIONS] {
        let mut body = report_body(
            route,
            // Deliberately malformed: the id of a real project memory, with the
            // domain left out. A server that defaults would accept this and
            // verify the project record — which is the *plausible* wrong answer,
            // and the reason a bare id must be refused rather than resolved.
            json!({ "knowledge_id": refs.project }),
            "passed",
            "test_outcome",
            &run_at(200),
        );
        body["run_at"] = json!(run_at(200));
        let (reply, status) = post(&pg, &pg.owner, route, &body);
        assert_eq!(
            status, 400,
            "{route} accepted a knowledge reference with no domain. A bare id \
             names a project memory, a personal note, a team entry and a pattern \
             at once, so it names none of them: {reply}"
        );
    }

    // And nothing was written on the way to refusing.
    assert_eq!(
        reports_where(&pg, &format!("knowledge_id = '{}'", refs.project)),
        0,
        "a refused reference still produced a report"
    );
}
