//! Where a verification summary lives, what it is allowed to contain, and who
//! may cause or read one (T126, US6, SC-766, SC-767, FR-811a, FR-811c).
//!
//! Three properties, each one edit away from failing quietly:
//!
//! 1. **Two storage locations, one derivation.** Project knowledge already has
//!    the five verification columns migration 2 gave `memories`, so it keeps
//!    them. `personal_knowledge`, `team_knowledge` and `shared_patterns` have
//!    none and gain none — five columns on four tables is four places for a
//!    derivation to drift — so their summaries live in
//!    `knowledge_verification`, keyed by the canonical reference
//!    (`contracts/verification-summary.md` §7.4). Both are asserted
//!    *physically*, against the tables, because a reader that resolved by
//!    domain and a writer that did not would look correct through the API
//!    until the two disagreed.
//!
//! 2. **One UUID is four identities.** The sharpest test in the file seeds a
//!    single UUID as a project memory, a personal record, a team record and a
//!    pattern, verifies each, and demands four summaries that do not touch each
//!    other. Anything keying on the bare UUID collapses them, and the
//!    consequence is not a mangled row — it is a personal record's verification
//!    shown against a colleague's project memory (SC-767). A `PatternRef`
//!    carries `ref_kind = 'pattern'` and a null domain slot, which says the row
//!    holds a pattern reference and *not* that the pattern has no domain: the
//!    `shared_patterns` row it resolves to is `domain = 'personal'`.
//!
//! 3. **No raw evidence, anywhere.** A summary carries a state, an authority, a
//!    timestamp and counts (FR-811c). Not an observed value, a command, its
//!    output, a file path, a locator or a digest. Asserted by seeding a
//!    distinctive string into every field that might carry one and then
//!    searching the *whole database* for it, because a count of rows would pass
//!    on a server that stored the string and merely declined to serve it back.
//!
//! And the ordering rule that makes the first three safe to state: §10's checks
//! run in order, so a caller who may not see the referenced record is refused
//! before the payload is judged, and is refused the same way an absent record
//! is refused.

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

const RUNS: &str = "/api/verification/runs";
const ATTESTATIONS: &str = "/api/verification/attestations";

fn knowledge_ref(domain: &str, id: Uuid) -> Value {
    json!({ "domain": domain, "knowledge_id": id })
}

fn pattern_ref(id: Uuid) -> Value {
    json!({ "pattern_id": id })
}

fn report_body(route: &str, reference: Value, verdict: &str, kind: &str, run_at: &str) -> Value {
    let mut body = json!({
        "memory_ref": reference,
        "verdict": verdict,
        "verifier_kind": kind,
        "run_at": run_at,
    });
    if route == ATTESTATIONS {
        body["attesting_agent"] = json!("claude-code");
    }
    body
}

fn post(pg: &Pg, who: &Account, route: &str, body: &Value) -> (Value, u16) {
    post_json_status_bearer(&pg.server.base, route, body, &who.token)
}

fn run_at(n: u32) -> String {
    format!("2026-08-30T10:{n:02}:00Z")
}

fn assert_accepted(what: &str, outcome: &(Value, u16)) {
    let (body, status) = outcome;
    assert_eq!(
        *status, 200,
        "{what}: expected the report to be accepted, got {status} with {body}"
    );
}

/// A refusal that discloses nothing about the record.
///
/// `404` for both "no such record" and "not yours", which is the answer
/// `commands::project_of_record` already gives and for the same reason: a route
/// that answered `404` for a missing record and `403` for a colleague's lets
/// anyone with an account enumerate ids one guess at a time (FR-894a).
fn assert_hidden(what: &str, outcome: (Value, u16)) {
    let (body, status) = outcome;
    assert_eq!(
        status, 404,
        "{what}: a caller who may not see the record must get the same answer an \
         absent record gets — got {status} with {body}"
    );
}

// ---------------------------------------------------------------------------
// Reading the two storage locations
// ---------------------------------------------------------------------------

/// One summary from `knowledge_verification`, addressed by its reference key.
///
/// `reference_key` rather than `(ref_kind, domain, knowledge_id)` because the
/// generated column *is* the identity (§7.4) and reading through it is the only
/// way a test can tell a collapsed key from a working one.
#[derive(Debug, PartialEq, Eq)]
struct Summary {
    verification: String,
    authority: String,
    last_verified_at: String,
    basis: String,
    evidence_fact_count: i64,
}

fn summary_rows(pg: &Pg, reference_key: &str) -> Vec<Summary> {
    pg.server
        .query_column(&format!(
            "SELECT verification
                 || '|' || COALESCE(verification_authority, '')
                 || '|' || COALESCE(to_char(last_verified_at AT TIME ZONE 'UTC',
                                            'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"'), '')
                 || '|' || verification_basis::text
                 || '|' || evidence_fact_count::text
               FROM knowledge_verification WHERE reference_key = '{reference_key}'"
        ))
        .into_iter()
        .map(|raw| {
            let p: Vec<&str> = raw.splitn(5, '|').collect();
            Summary {
                verification: p[0].to_string(),
                authority: p[1].to_string(),
                last_verified_at: p[2].to_string(),
                basis: p[3].to_string(),
                evidence_fact_count: p[4].parse().unwrap_or(-1),
            }
        })
        .collect()
}

fn summary(pg: &Pg, reference_key: &str) -> Summary {
    let mut rows = summary_rows(pg, reference_key);
    assert_eq!(
        rows.len(),
        1,
        "expected exactly one `knowledge_verification` row for `{reference_key}`, \
         found {}",
        rows.len()
    );
    rows.remove(0)
}

fn summary_count(pg: &Pg) -> i64 {
    pg.server
        .count("SELECT count(*) FROM knowledge_verification")
}

/// The project domain's summary, read from `memories`' own five columns.
fn project_summary(pg: &Pg, memory: Uuid) -> Summary {
    let raw = pg.server.text(&format!(
        "SELECT COALESCE(verification, '')
             || '|' || COALESCE(verification_authority, '')
             || '|' || COALESCE(to_char(last_verified_at AT TIME ZONE 'UTC',
                                        'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"'), '')
             || '|' || verification_basis::text
             || '|' || evidence_fact_count::text
           FROM memories WHERE id = '{memory}'"
    ));
    let p: Vec<&str> = raw.splitn(5, '|').collect();
    Summary {
        verification: p[0].to_string(),
        authority: p[1].to_string(),
        last_verified_at: p[2].to_string(),
        basis: p[3].to_string(),
        evidence_fact_count: p[4].parse().unwrap_or(-1),
    }
}

// ---------------------------------------------------------------------------
// Fixture records
// ---------------------------------------------------------------------------

fn seed_project_memory(pg: &Pg, session: Uuid, content: &str) -> Uuid {
    let id = Uuid::now_v7();
    pg.server.execute(&format!(
        "INSERT INTO memories (id, project_id, type, scope, scope_key, content,
                               origin_session_id)
         VALUES ('{id}', '{}', 'fact', 'project', '{}', '{content}', '{session}')",
        pg.project, pg.project
    ));
    id
}

fn seed_personal(pg: &Pg, owner: &Account, content: &str) -> Uuid {
    let id = Uuid::now_v7();
    pg.server.execute(&format!(
        "INSERT INTO personal_knowledge
             (id, owner_user_id, knowledge_type, content, writer_id, writer_seq)
         VALUES ('{id}', '{}', 'fact', '{content}', 'summary-fixture-{id}', 1)",
        owner.id
    ));
    id
}

/// A team row in a chosen state, with the CHECK the schema imposes satisfied.
///
/// `authoritative` is the ratified state's name in `team_knowledge`
/// (`0003_collaborative_global_memory.sql`), and the table refuses a
/// non-proposed row without a ratifier and a ratification time — so the fixture
/// supplies both rather than leaving a row the database would reject.
fn seed_team(pg: &Pg, author: &Account, state: &str, content: &str) -> Uuid {
    let id = Uuid::now_v7();
    let ratified = if state == "proposed" {
        "NULL, NULL".to_string()
    } else {
        format!("'{}', now()", author.id)
    };
    pg.server.execute(&format!(
        "INSERT INTO team_knowledge
             (id, knowledge_type, content, state, proposed_by_user_id,
              ratified_by_user_id, ratified_at, writer_id, writer_seq)
         VALUES ('{id}', 'convention', '{content}', '{state}', '{}', {ratified},
                 'summary-fixture-{id}', 2)",
        author.id
    ));
    id
}

fn seed_pattern(pg: &Pg, owner: &Account, content: &str) -> Uuid {
    let id = Uuid::now_v7();
    assert!(
        pg.seed_pattern_with_id(owner, id, content),
        "the pattern fixture needs `shared_patterns`, which server schema v4 adds"
    );
    id
}

// ---------------------------------------------------------------------------
// 1. Project knowledge keeps its own columns; nothing else gains any
// ---------------------------------------------------------------------------

#[test]
fn a_project_report_writes_memories_columns_and_creates_no_summary_row() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    let memory = seed_project_memory(&pg, session, "the release job signs images");

    assert_accepted(
        "a project-domain report",
        &post(
            &pg,
            &pg.owner,
            RUNS,
            &report_body(
                RUNS,
                knowledge_ref("project", memory),
                "passed",
                "test_outcome",
                &run_at(1),
            ),
        ),
    );

    let stored = project_summary(&pg, memory);
    assert_eq!(stored.verification, "verified");
    assert_eq!(stored.authority, "remote_attested");
    assert_eq!(
        stored.last_verified_at, "2026-08-30T10:01:00Z",
        "the project summary is the `memories` row's own `last_verified_at`, \
         which migration 2 already gave it (§1, §7.4)"
    );
    assert_eq!(stored.evidence_fact_count, 1);

    assert_eq!(
        summary_count(&pg),
        0,
        "and no `knowledge_verification` row was created. Two summaries for one \
         record is two answers to one question, and the reader resolving by \
         domain would see only one of them (§7.4)"
    );
    assert_eq!(
        summary_rows(&pg, &format!("knowledge:project:{memory}")).len(),
        0,
        "in particular there is no `knowledge:project:…` key — the table's own \
         CHECK admits only `personal` and `team`, and a row here would mean the \
         project domain had quietly acquired a second home"
    );
}

#[test]
fn personal_team_and_pattern_reports_write_knowledge_verification() {
    let pg = pg!();
    let personal = seed_personal(&pg, &pg.owner, "the owner prefers one signer");
    let team = seed_team(
        &pg,
        &pg.owner,
        "authoritative",
        "the team signs every release image",
    );
    let pattern = seed_pattern(&pg, &pg.owner, "sign images before release");

    let cases = [
        (
            knowledge_ref("personal", personal),
            format!("knowledge:personal:{personal}"),
            "file_exists",
            1_u32,
        ),
        (
            knowledge_ref("team", team),
            format!("knowledge:team:{team}"),
            "git_ref",
            2,
        ),
        (
            pattern_ref(pattern),
            format!("pattern:{pattern}"),
            "command_outcome",
            3,
        ),
    ];

    for (reference, key, kind, at) in &cases {
        assert_accepted(
            &format!("a report about {reference}"),
            &post(
                &pg,
                &pg.owner,
                RUNS,
                &report_body(RUNS, reference.clone(), "passed", kind, &run_at(*at)),
            ),
        );
        let stored = summary(&pg, key);
        assert_eq!(
            stored.verification, "verified",
            "{key}: the derivation is the same code for every domain; only the \
             storage location differs (§7.4)"
        );
        assert_eq!(
            stored.authority, "remote_attested",
            "{key}: a personal, team or pattern summary from a client report may \
             reach `remote_attested` and never further (§7.4, FR-826)"
        );
        assert_eq!(stored.basis, format!("[\"{kind}\"]"));
        assert_eq!(stored.evidence_fact_count, 1);
    }

    assert_eq!(summary_count(&pg), 3);
    assert_eq!(
        pg.server
            .count("SELECT count(*) FROM memories WHERE verification IS NOT NULL"),
        0,
        "and no `memories` row was touched: personal, team and pattern records \
         are not project knowledge, and writing a project column for one would \
         attribute it to a project it does not belong to (FR-822)"
    );

    // The three non-project tables gain no columns of their own. This is the
    // half of §7.4 that a passing API test cannot see: a build that added
    // `verification` to `personal_knowledge` would satisfy every assertion
    // above while creating the fourth place for the derivation to drift.
    for table in ["personal_knowledge", "team_knowledge", "shared_patterns"] {
        for column in [
            "verification",
            "verification_authority",
            "verification_basis",
            "evidence_fact_count",
            "last_verified_at",
        ] {
            assert!(
                !pg.column_exists(table, column),
                "`{table}.{column}` exists; §7.4 keeps every non-project summary \
                 in `knowledge_verification` precisely so there is one place to \
                 write and one place to read"
            );
        }
    }
}

#[test]
fn a_pattern_summary_carries_the_pattern_discriminator_and_a_null_domain() {
    let pg = pg!();
    let pattern = seed_pattern(&pg, &pg.owner, "sign images before release");

    assert_accepted(
        "a report about a pattern",
        &post(
            &pg,
            &pg.owner,
            ATTESTATIONS,
            &report_body(
                ATTESTATIONS,
                pattern_ref(pattern),
                "passed",
                "runtime_state",
                &run_at(4),
            ),
        ),
    );

    let shape = pg.server.text(&format!(
        "SELECT ref_kind || '|' || COALESCE(domain, '<null>') || '|' || reference_key
           FROM knowledge_verification WHERE knowledge_id = '{pattern}'"
    ));
    assert_eq!(
        shape,
        format!("pattern|<null>|pattern:{pattern}"),
        "a `PatternRef` is stored with the pattern discriminator and a null \
         reference-domain slot (SC-766). The null says the row holds a \
         `PatternRef` rather than a `KnowledgeRef`; it does not say the pattern \
         is domain-less"
    );

    // And the record it resolves to is still personal, which is the sentence
    // §7.4 ends on and the thing the null slot is most likely to be misread as
    // contradicting.
    assert_eq!(
        pg.server.text(&format!(
            "SELECT domain FROM shared_patterns WHERE pattern_id = '{pattern}'"
        )),
        "personal",
        "the resolved `shared_patterns` row keeps `domain = personal`"
    );

    // Likewise the report row.
    assert_eq!(
        pg.server.text(&format!(
            "SELECT ref_kind || '|' || COALESCE(domain, '<null>')
               FROM verification_reports WHERE knowledge_id = '{pattern}'"
        )),
        "pattern|<null>"
    );
}

// ---------------------------------------------------------------------------
// 2. One UUID, four identities, four summaries
// ---------------------------------------------------------------------------

#[test]
fn one_uuid_becomes_four_summaries_that_do_not_collide() {
    let pg = pg!();
    let ids = pg.seed_identical_ids(&pg.owner);
    let keys = ids.reference_keys();

    // `seed_identical_ids` leaves the team row `proposed`, which only its author
    // or an administrator may see (§7). The fixture owner proposed it, so the
    // owner is the right reporter for all four here; the co-member's refusal is
    // the authorization test further down.
    //
    // Every report differs from the others in verifier kind and `run_at` as
    // well as reference, so a collapsed key does not merely produce one row —
    // it produces one row with visibly the wrong basis and the wrong timestamp,
    // and the assertion names which.
    let plan = [
        (
            knowledge_ref("project", ids.project_memory),
            "file_exists",
            10_u32,
        ),
        (knowledge_ref("personal", ids.personal), "file_digest", 11),
        (knowledge_ref("team", ids.team), "git_commit", 12),
        (pattern_ref(ids.pattern), "schema_version", 13),
    ];
    for (reference, kind, at) in &plan {
        assert_accepted(
            &format!("a report about {reference}"),
            &post(
                &pg,
                &pg.owner,
                RUNS,
                &report_body(RUNS, reference.clone(), "passed", kind, &run_at(*at)),
            ),
        );
    }

    // The project half lives in `memories`.
    let project = project_summary(&pg, ids.project_memory);
    assert_eq!(project.verification, "verified");
    assert_eq!(project.basis, "[\"file_exists\"]");
    assert_eq!(project.last_verified_at, "2026-08-30T10:10:00Z");
    assert_eq!(project.evidence_fact_count, 1);

    // The other three live in `knowledge_verification`, one row each, and the
    // count is asserted before the rows so a collapse fails here rather than in
    // a lookup that would report "no row" and read like a different bug.
    assert_eq!(
        summary_count(&pg),
        3,
        "three non-project references are three summaries. One row means the \
         reference key collapsed to the bare UUID and a personal record's \
         verification is now answering for a team record (SC-767)"
    );
    for (key, kind, at) in [
        (&keys[1], "file_digest", "2026-08-30T10:11:00Z"),
        (&keys[2], "git_commit", "2026-08-30T10:12:00Z"),
        (&keys[3], "schema_version", "2026-08-30T10:13:00Z"),
    ] {
        let stored = summary(&pg, key);
        assert_eq!(
            stored.basis,
            format!("[\"{kind}\"]"),
            "{key}: each summary accumulated only its own report's verifier kind; \
             two kinds in one basis is two references sharing a row"
        );
        assert_eq!(
            stored.last_verified_at, at,
            "{key}: and only its own run's timestamp"
        );
        assert_eq!(
            stored.evidence_fact_count, 1,
            "{key}: and only its own count"
        );
    }

    // The four reports stay four, keyed by the complete logical reference.
    let mut stored_keys = pg
        .server
        .query_column("SELECT reference_key FROM verification_reports");
    stored_keys.sort();
    let mut expected = keys.to_vec();
    expected.sort();
    assert_eq!(stored_keys, expected);

    // And the project key is nowhere in `knowledge_verification`, which is the
    // asymmetry §7.4 chose deliberately.
    assert_eq!(
        summary_rows(&pg, &keys[0]).len(),
        0,
        "`{}` must not appear in `knowledge_verification`: the project domain \
         reads `memories`, and a row here would be a second answer",
        keys[0]
    );
}

#[test]
fn a_pattern_id_named_as_personal_knowledge_produces_no_personal_summary() {
    let pg = pg!();
    // A pattern with **no** personal record sharing its id, so there is nothing
    // legitimate for a `personal` reference to resolve to. If the server
    // resolves a `KnowledgeRef(personal, x)` against `shared_patterns` — the
    // coercion this test exists for — it would find the pattern and file a
    // personal summary for it.
    let pattern = seed_pattern(&pg, &pg.owner, "sign images before release");
    assert_eq!(
        pg.server.count(&format!(
            "SELECT count(*) FROM personal_knowledge WHERE id = '{pattern}'"
        )),
        0,
        "the fixture depends on there being no personal record with this id"
    );

    let (body, status) = post(
        &pg,
        &pg.owner,
        RUNS,
        &report_body(
            RUNS,
            knowledge_ref("personal", pattern),
            "passed",
            "file_exists",
            &run_at(20),
        ),
    );
    assert_eq!(
        status, 404,
        "a `personal` reference resolves against `personal_knowledge` and nothing \
         else; a pattern id is not a personal knowledge id (`data-model.md` §6.1). \
         Got {status} with {body}"
    );

    // The invariant, stated independently of how the refusal is spelled: no
    // request naming a pattern's id in the personal domain may produce a
    // personal summary for it. A build that "helpfully" resolved the pattern
    // would give a `PatternRef` a second, `KnowledgeRef`-shaped identity, and
    // the two would then drift.
    assert_eq!(
        summary_rows(&pg, &format!("knowledge:personal:{pattern}")).len(),
        0,
        "a pattern must not acquire a personal-knowledge summary"
    );
    assert_eq!(
        summary_count(&pg),
        0,
        "and it must not have acquired its pattern summary either — the request \
         named a personal reference and was refused, so nothing was verified"
    );
}

// ---------------------------------------------------------------------------
// 3. No raw evidence reaches the server
// ---------------------------------------------------------------------------

/// A string no other fixture in this suite produces.
///
/// The assertion is its **absence** from the whole database. A count of stored
/// rows would pass on a server that accepted the value and merely declined to
/// serve it back, which is the failure mode FR-811c is about
/// (`Server::dump` exists for exactly this, SC-119).
const NEEDLE: &str = "zqx-evidence-needle-9f4c1e";

#[test]
fn no_field_carrying_raw_evidence_is_accepted_or_stored() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    let memory = seed_project_memory(&pg, session, "the release job signs images");

    // Every name §3 rejects by example plus the refused-name list the rest of
    // this server already enforces recursively. One at a time, and then all at
    // once, on both routes.
    let evidence_fields = [
        "observed_value",
        "source_locator",
        "value_digest",
        "fingerprint",
        "command",
        "path",
        "details",
        "detail",
        "summary",
        "exit_code",
        "observations",
        "relevant_paths",
        "content_norm_digest",
    ];

    let mut attempted = 0;
    for (i, field) in evidence_fields.iter().enumerate() {
        for route in [RUNS, ATTESTATIONS] {
            let mut body = report_body(
                route,
                knowledge_ref("project", memory),
                "passed",
                "test_outcome",
                &run_at(30 + i as u32),
            );
            body[*field] = json!(format!("{NEEDLE}-{field}"));
            let (reply, status) = post(&pg, &pg.owner, route, &body);
            assert_eq!(
                status, 400,
                "{route} carrying `{field}`: a summary carries a state, an \
                 authority, a timestamp and counts — never an observed value, a \
                 locator, a digest, a command, its output or a local path \
                 (FR-811c, §3). Got {status} with {reply}"
            );
            assert!(
                !reply.to_string().contains(NEEDLE),
                "{route} carrying `{field}`: the refusal echoed the value back. A \
                 boundary that names the offending field is helpful; one that \
                 repeats its content has transmitted exactly what it refused"
            );
            attempted += 1;

            // Nested, because wrapping is the obvious defeat for a top-level
            // check and the sibling boundaries on this server all search
            // recursively (`events::carries_refused_name`).
            let mut nested = report_body(
                route,
                knowledge_ref("project", memory),
                "passed",
                "test_outcome",
                &run_at(30 + i as u32),
            );
            nested["memory_ref"][*field] = json!(format!("{NEEDLE}-nested-{field}"));
            let (reply, status) = post(&pg, &pg.owner, route, &nested);
            assert_eq!(
                status, 400,
                "{route} carrying a nested `{field}`: refused at any depth. Got \
                 {status} with {reply}"
            );
            attempted += 1;
        }
    }
    assert_eq!(attempted, evidence_fields.len() * 4);

    // A legal report on the same record, so the negative below is not satisfied
    // by a server that refuses everything.
    assert_accepted(
        "the legal report",
        &post(
            &pg,
            &pg.owner,
            RUNS,
            &report_body(
                RUNS,
                knowledge_ref("project", memory),
                "passed",
                "test_outcome",
                &run_at(59),
            ),
        ),
    );
    assert_eq!(project_summary(&pg, memory).verification, "verified");

    let dump = pg.server.dump();
    assert!(
        !dump.contains(NEEDLE),
        "the needle reached the database. The endpoints are one view of what the \
         server accepted; the tables are the other, and a privacy assertion made \
         only against the API passes on a server that stored the value and \
         declined to serve it (SC-119, FR-811c)"
    );
}

#[test]
fn the_verification_tables_have_no_column_shaped_like_evidence() {
    let pg = pg!();

    // The regression the test above cannot catch: a later migration adding a
    // column for "just the digest, it's only a hash". FR-811c is about the
    // shape of the record, not only about today's payload validation.
    let refused = [
        "observed_value",
        "source_locator",
        "value_digest",
        "fingerprint",
        "command",
        "path",
        "details",
        "detail",
        "summary",
        "exit_code",
        "observations",
        "relevant_paths",
        "content_norm_digest",
        "evidence",
        "output",
    ];
    for table in ["verification_reports", "knowledge_verification"] {
        for column in refused {
            assert!(
                !pg.column_exists(table, column),
                "`{table}.{column}` exists. A verification summary carries a \
                 state, an authority, a timestamp and counts (FR-811c); a column \
                 by this name is a place for raw evidence to live"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// 4. Authorization, in §10's order
// ---------------------------------------------------------------------------

#[test]
fn an_outsider_may_not_report_on_a_project_memory() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    let memory = seed_project_memory(&pg, session, "the release job signs images");

    let body = report_body(
        RUNS,
        knowledge_ref("project", memory),
        "passed",
        "test_outcome",
        &run_at(40),
    );
    assert_hidden(
        "an authenticated non-member reporting on a project memory",
        post(&pg, &pg.outsider, RUNS, &body),
    );

    // The same answer an id that does not exist gets, so the route is not an
    // enumeration oracle.
    let absent = report_body(
        RUNS,
        knowledge_ref("project", Uuid::now_v7()),
        "passed",
        "test_outcome",
        &run_at(41),
    );
    assert_hidden(
        "a member naming a memory that does not exist",
        post(&pg, &pg.owner, RUNS, &absent),
    );

    assert_eq!(
        pg.server.count("SELECT count(*) FROM verification_reports"),
        0
    );
    assert_eq!(
        project_summary(&pg, memory).verification,
        "",
        "a refused report establishes nothing (FR-811d)"
    );
}

#[test]
fn a_co_member_may_not_report_on_a_colleagues_personal_record() {
    let pg = pg!();
    let personal = seed_personal(&pg, &pg.owner, "the owner prefers one signer");

    // The personal domain's check is not membership but identity: the caller
    // **is** the owner, or the record is not theirs to speak about (§7). A
    // project co-member is the sharpest case, because every other Feature 005
    // surface treats them as entitled.
    let body = report_body(
        RUNS,
        knowledge_ref("personal", personal),
        "passed",
        "file_exists",
        &run_at(42),
    );
    assert_hidden(
        "a project co-member reporting on the owner's personal record",
        post(&pg, &pg.member, RUNS, &body),
    );
    assert_hidden(
        "an outsider reporting on the owner's personal record",
        post(&pg, &pg.outsider, RUNS, &body),
    );
    assert_eq!(summary_count(&pg), 0);

    assert_accepted(
        "the owner reporting on the owner's own record",
        &post(&pg, &pg.owner, RUNS, &body),
    );
    assert_eq!(
        summary(&pg, &format!("knowledge:personal:{personal}")).verification,
        "verified",
        "and the owner's own report is accepted, so the refusals above are not \
         a route that refuses everyone"
    );
}

#[test]
fn a_proposed_team_record_admits_only_its_author() {
    let pg = pg!();
    let proposed = seed_team(&pg, &pg.owner, "proposed", "sign every release image");
    let ratified = seed_team(
        &pg,
        &pg.owner,
        "authoritative",
        "the team signs every release image",
    );

    // §7: team knowledge reaches every member of the server's single team, but
    // a `proposed` row additionally requires author-or-admin — the same
    // predicate `global::team_visibility_predicate` already enforces on the
    // read path. A reporting route that skipped it would disclose the existence
    // of a colleague's unratified proposal.
    assert_hidden(
        "a second account reporting on somebody else's proposed team entry",
        post(
            &pg,
            &pg.member,
            RUNS,
            &report_body(
                RUNS,
                knowledge_ref("team", proposed),
                "passed",
                "git_ref",
                &run_at(43),
            ),
        ),
    );
    assert_accepted(
        "the proposal's author reporting on it",
        &post(
            &pg,
            &pg.owner,
            RUNS,
            &report_body(
                RUNS,
                knowledge_ref("team", proposed),
                "passed",
                "git_ref",
                &run_at(44),
            ),
        ),
    );

    // A ratified entry is the whole team's, so any authenticated account may
    // report on it — including one that is a member of no project, because team
    // knowledge is project-independent by construction (FR-822).
    assert_accepted(
        "a co-member reporting on a ratified team entry",
        &post(
            &pg,
            &pg.member,
            RUNS,
            &report_body(
                RUNS,
                knowledge_ref("team", ratified),
                "passed",
                "git_ref",
                &run_at(45),
            ),
        ),
    );
    assert_accepted(
        "an account in no project reporting on a ratified team entry",
        &post(
            &pg,
            &pg.outsider,
            RUNS,
            &report_body(
                RUNS,
                knowledge_ref("team", ratified),
                "passed",
                "git_ref",
                &run_at(46),
            ),
        ),
    );

    assert_eq!(summary_count(&pg), 2, "one summary per team reference");
    assert_eq!(
        pg.server.count(&format!(
            "SELECT count(*) FROM verification_reports
              WHERE reference_key = 'knowledge:team:{ratified}'"
        )),
        2,
        "two accounts reporting one ratified entry are two reports (FR-811i)"
    );
}

#[test]
fn a_pattern_is_reportable_only_by_its_owner() {
    let pg = pg!();
    let pattern = seed_pattern(&pg, &pg.owner, "sign images before release");
    let body = report_body(
        RUNS,
        pattern_ref(pattern),
        "passed",
        "command_outcome",
        &run_at(47),
    );

    // A pattern resolves to a personal-domain record, so its check is the
    // personal one: the caller **is** the owner (§7, `data-model.md` §6.2).
    assert_hidden(
        "a project co-member reporting on the owner's pattern",
        post(&pg, &pg.member, RUNS, &body),
    );
    assert_eq!(summary_count(&pg), 0);
    assert_accepted(
        "the owner reporting on it",
        &post(&pg, &pg.owner, RUNS, &body),
    );
    assert_eq!(
        summary(&pg, &format!("pattern:{pattern}")).verification,
        "verified"
    );
}

#[test]
fn authorization_is_decided_before_the_payload_is_judged() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    let memory = seed_project_memory(&pg, session, "the release job signs images");

    // §10 numbers its checks: resolve the record (2), resolve the domain
    // binding (3), *then* validate the vocabularies (4) and refuse a payload
    // that names authority or a report id (5). The order is the contract's, and
    // it is observable: a non-member who sends a malformed body must be told
    // the same thing a non-member who sends a perfect one is told. A `400`
    // about the verdict would confirm that the caller got as far as validation,
    // which is a step past the record they may not see.
    for (what, mutate) in [
        (
            "a verdict outside the vocabulary",
            json!({ "verdict": "probably" }),
        ),
        (
            "a verifier kind outside the vocabulary",
            json!({ "verifier_kind": "vibes" }),
        ),
        ("a named authority", json!({ "authority": "cairn" })),
        ("a named report id", json!({ "report_id": Uuid::now_v7() })),
    ] {
        let mut body = report_body(
            RUNS,
            knowledge_ref("project", memory),
            "passed",
            "test_outcome",
            &run_at(48),
        );
        for (k, v) in mutate.as_object().expect("an object of overrides") {
            body[k.as_str()] = v.clone();
        }
        assert_hidden(
            &format!("an outsider sending {what}"),
            post(&pg, &pg.outsider, RUNS, &body),
        );
    }
    assert_eq!(
        pg.server.count("SELECT count(*) FROM verification_reports"),
        0
    );
}

#[test]
fn a_caller_who_may_not_see_a_record_cannot_read_its_summary() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    let memory = seed_project_memory(&pg, session, "the release job signs images");
    let personal = seed_personal(&pg, &pg.owner, "the owner prefers one signer");
    let pattern = seed_pattern(&pg, &pg.owner, "sign images before release");

    for (i, reference) in [
        knowledge_ref("project", memory),
        knowledge_ref("personal", personal),
        pattern_ref(pattern),
    ]
    .into_iter()
    .enumerate()
    {
        assert_accepted(
            &format!("the owner's report about {reference}"),
            &post(
                &pg,
                &pg.owner,
                RUNS,
                &report_body(
                    RUNS,
                    reference,
                    "passed",
                    "file_exists",
                    &run_at(50 + i as u32),
                ),
            ),
        );
    }

    // Reporting and reading are the same entitlement (§10, FR-894a). The
    // project detail route answers a non-member the way it answers a stranger
    // asking about a record that does not exist.
    let (body, status) = get_json_status_bearer(
        &pg.server.base,
        &format!("/api/memories/{memory}"),
        &pg.outsider.token,
    );
    assert_eq!(
        status, 404,
        "a non-member reading a project memory's detail — and with it its \
         verification summary — must be refused: got {status} with {body}"
    );

    // The two owner-scoped reads take no parameter that could name an owner, so
    // the guarantee is structural rather than a check: a co-member's own view
    // simply does not contain the owner's records, and therefore cannot contain
    // their summaries.
    for path in ["/api/personal/knowledge", "/api/patterns"] {
        let (body, status) = get_json_status_bearer(&pg.server.base, path, &pg.member.token);
        assert_eq!(status, 200, "{path}: {body}");
        let rendered = body.to_string();
        for id in [personal, pattern] {
            assert!(
                !rendered.contains(&id.to_string()),
                "{path} disclosed `{id}` to an account that does not own it, and a \
                 verification summary rendered beside it would disclose that the \
                 record was checked and when"
            );
        }
        assert!(
            !rendered.contains("remote_attested"),
            "{path} carried a verification summary for records the caller may not \
             see"
        );
    }
}
