//! What cutover does to verification it cannot substantiate, and what it
//! leaves alone (T147, User Story 7, `contracts/migration-cutover.md` §2 steps
//! 2-4, §11.9's changes-feed half; SC-746, SC-747, FR-811d, FR-876).
//!
//! Feature 005 makes verification a server-derived fact: a state is only as
//! good as the `verification_reports` rows behind it (§6 of
//! `contracts/verification-summary.md`). Cutover is the moment that rule stops
//! being aspirational for records that predate it. A `memories` or
//! `knowledge_verification` row can carry `verification = 'verified'` from
//! before this feature existed, asserted by a pre-005 client with no report to
//! back it — and once the server is the only authority, a claim with nothing
//! behind it is not a fact the server can keep repeating.
//!
//! So cutover, in the same transaction that flips `server_authority.mode`
//! (`feature005_cutover.rs` covers that switch itself):
//!
//! 1. **Finds exactly the rows a report cannot substantiate** — `verification
//!    <> 'unverified'` with no matching `verification_reports.reference_key` —
//!    and no others. A row with a real report behind it, however it got
//!    there, is left alone.
//! 2. **Writes an untrusted audit row for each one first**, in
//!    `legacy_verification_audit`, so what was previously claimed survives out
//!    of the derivation path rather than being silently lost.
//! 3. **Demotes exactly those rows** to the honest starting state:
//!    `unverified`, with authority, basis, count and timestamp cleared to
//!    their empty values.
//!
//! And two guarantees this file states explicitly because they are the two
//! ways "cutover touches verification" could quietly become "cutover touches
//! more than it should":
//!
//! - **The changes feed stops manufacturing a fact the server cannot stand
//!   behind.** `GET /api/sync/changes` carries a `verification` object per
//!   memory today; after cutover it must not, because that object is exactly
//!   the shape a client could keep reading as "the server verified this" once
//!   the server has stopped deriving it from anything.
//! - **A client's local verification is not reachable from cutover at all.**
//!   Cutover is one server-side transaction over `memories` and
//!   `knowledge_verification`, and the only channel between that transaction
//!   and a client's own locally-derived verification is whatever the client
//!   sends it. Assertion 5 below — the changes feed carries nothing to
//!   receive — is that whole channel, closed. There is no second one to check,
//!   because a server transaction cannot reach into a local SQLite file it
//!   was never sent.
//!
//! What would falsify this file: a substantiated row moved by even one field,
//! an unsubstantiated row left claiming more than `unverified`, a demoted
//! value discarded rather than audited, or a `verification` object still
//! riding the changes feed after cutover.

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

fn make_admin(pg: &Pg, who: &Account) {
    pg.server.execute(&format!(
        "UPDATE users SET role = 'admin' WHERE id = '{}'",
        who.id
    ));
}

fn cutover(pg: &Pg) -> Value {
    make_admin(pg, &pg.owner);
    let (body, status) = post_json_status_bearer(
        &pg.server.base,
        "/api/admin/cutover",
        &json!({}),
        &pg.owner.token,
    );
    assert_eq!(status, 200, "cutover call itself: {body}");
    body
}

fn knowledge_ref(domain: &str, id: Uuid) -> Value {
    json!({ "domain": domain, "knowledge_id": id })
}

/// Substantiate one reference through the ONLY legitimate route — a genuine
/// `/api/verification/runs` report — so its `verification_reports` row and its
/// derived summary are both real, not hand-assembled to look real.
fn substantiate(pg: &Pg, reference: Value, run_at: &str) {
    let (body, status) = post_json_status_bearer(
        &pg.server.base,
        RUNS,
        &json!({
            "memory_ref": reference,
            "verdict": "passed",
            "verifier_kind": "test_outcome",
            "run_at": run_at,
        }),
        &pg.owner.token,
    );
    assert_eq!(status, 200, "seeding a substantiated report: {body}");
}

// ---------------------------------------------------------------------------
// Fixtures: a project memory and a personal/team record, each seeded two ways
// ---------------------------------------------------------------------------

fn seed_memory(pg: &Pg, session: Uuid, label: &str) -> Uuid {
    let id = Uuid::now_v7();
    pg.server.execute(&format!(
        "INSERT INTO memories (id, project_id, type, scope, scope_key, content,
                               origin_session_id)
         VALUES ('{id}', '{}', 'fact', 'project', '{}', '{label}', '{session}')",
        pg.project, pg.project
    ));
    id
}

/// A `memories` row carrying a legacy, unsubstantiated claim — exactly what a
/// pre-005 client could have written through the sync path this feature
/// retires: a state and an authority with no `verification_reports` row
/// behind either.
fn seed_legacy_memory_claim(pg: &Pg, session: Uuid, label: &str) -> Uuid {
    let id = seed_memory(pg, session, label);
    pg.server.execute(&format!(
        "UPDATE memories
            SET verification = 'verified', verification_authority = 'cairn',
                verification_basis = '[\"deterministic_rule\"]'::jsonb,
                evidence_fact_count = 3,
                last_verified_at = '2026-01-01T00:00:00Z'
          WHERE id = '{id}'"
    ));
    id
}

fn seed_personal(pg: &Pg, owner: &Account, label: &str) -> Uuid {
    let id = Uuid::now_v7();
    pg.server.execute(&format!(
        "INSERT INTO personal_knowledge (id, owner_user_id, knowledge_type, content,
                                         writer_id, writer_seq)
         VALUES ('{id}', '{}', 'fact', '{label}', 'cutover-fixture-{id}', 1)",
        owner.id
    ));
    id
}

fn seed_team(pg: &Pg, owner: &Account, label: &str) -> Uuid {
    let id = Uuid::now_v7();
    pg.server.execute(&format!(
        "INSERT INTO team_knowledge (id, knowledge_type, content, state,
                                     proposed_by_user_id, ratified_by_user_id, ratified_at,
                                     writer_id, writer_seq)
         VALUES ('{id}', 'convention', '{label}', 'authoritative', '{}', '{}', now(),
                 'cutover-fixture-{id}', 1)",
        owner.id, owner.id
    ));
    id
}

/// A `knowledge_verification` row carrying a legacy, unsubstantiated claim —
/// the personal/team counterpart of [`seed_legacy_memory_claim`]. Inserted
/// directly rather than through any route, because there has never been a
/// route that lets a caller write this table except the derivation itself
/// (`verifysummary.rs`), and the whole point of this fixture is a value the
/// derivation never produced.
fn seed_legacy_knowledge_claim(pg: &Pg, domain: &str, id: Uuid) {
    pg.server.execute(&format!(
        "INSERT INTO knowledge_verification
             (ref_kind, domain, knowledge_id, verification, verification_authority,
              verification_basis, evidence_fact_count, last_verified_at)
         VALUES ('knowledge', '{domain}', '{id}', 'verified', 'cairn',
                 '[\"deterministic_rule\"]'::jsonb, 5, '2026-01-01T00:00:00Z')"
    ));
}

// ---------------------------------------------------------------------------
// Reading rows back, by value
// ---------------------------------------------------------------------------

/// One verification summary, read wherever it lives — `memories`' own five
/// columns for a project reference, `knowledge_verification` for the other
/// two — so demoted and untouched rows can be compared on equal footing.
#[derive(Debug, PartialEq, Eq, Clone)]
struct Verification {
    state: String,
    authority: String,
    basis: String,
    count: i64,
    last_verified_at: String,
}

fn memory_verification(pg: &Pg, id: Uuid) -> Verification {
    let raw = pg.server.text(&format!(
        "SELECT COALESCE(verification, '')
             || '|' || COALESCE(verification_authority, '')
             || '|' || COALESCE(verification_basis::text, '[]')
             || '|' || evidence_fact_count::text
             || '|' || COALESCE(to_char(last_verified_at AT TIME ZONE 'UTC',
                                        'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"'), '')
           FROM memories WHERE id = '{id}'"
    ));
    parse_verification(&raw)
}

fn knowledge_verification_row(pg: &Pg, domain: &str, id: Uuid) -> Verification {
    let raw = pg.server.text(&format!(
        "SELECT COALESCE(verification, '')
             || '|' || COALESCE(verification_authority, '')
             || '|' || COALESCE(verification_basis::text, '[]')
             || '|' || evidence_fact_count::text
             || '|' || COALESCE(to_char(last_verified_at AT TIME ZONE 'UTC',
                                        'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"'), '')
           FROM knowledge_verification
          WHERE reference_key = 'knowledge:{domain}:{id}'"
    ));
    parse_verification(&raw)
}

fn parse_verification(raw: &str) -> Verification {
    let parts: Vec<&str> = raw.splitn(5, '|').collect();
    Verification {
        state: parts[0].to_string(),
        authority: parts[1].to_string(),
        basis: parts[2].to_string(),
        count: parts[3].parse().unwrap_or(-1),
        last_verified_at: parts[4].to_string(),
    }
}

fn empty_verification() -> Verification {
    Verification {
        state: "unverified".to_string(),
        authority: String::new(),
        basis: "[]".to_string(),
        count: 0,
        last_verified_at: String::new(),
    }
}

// ---------------------------------------------------------------------------
// The four fixtures together
// ---------------------------------------------------------------------------

struct Fixture {
    legacy_memory: Uuid,
    substantiated_memory: Uuid,
    legacy_personal: Uuid,
    substantiated_personal: Uuid,
    legacy_team: Uuid,
    substantiated_team: Uuid,
}

fn seed(pg: &Pg) -> Fixture {
    let session = pg.session_for(&pg.owner);
    let legacy_memory = seed_legacy_memory_claim(pg, session, "a legacy project claim");
    let substantiated_memory = seed_memory(pg, session, "a substantiated project claim");
    substantiate(
        pg,
        knowledge_ref("project", substantiated_memory),
        "2026-08-01T09:00:00Z",
    );

    let legacy_personal = seed_personal(pg, &pg.owner, "a legacy personal claim");
    seed_legacy_knowledge_claim(pg, "personal", legacy_personal);
    let substantiated_personal = seed_personal(pg, &pg.owner, "a substantiated personal claim");
    substantiate(
        pg,
        knowledge_ref("personal", substantiated_personal),
        "2026-08-01T09:01:00Z",
    );

    let legacy_team = seed_team(pg, &pg.owner, "a legacy team claim");
    seed_legacy_knowledge_claim(pg, "team", legacy_team);
    let substantiated_team = seed_team(pg, &pg.owner, "a substantiated team claim");
    substantiate(
        pg,
        knowledge_ref("team", substantiated_team),
        "2026-08-01T09:02:00Z",
    );

    Fixture {
        legacy_memory,
        substantiated_memory,
        legacy_personal,
        substantiated_personal,
        legacy_team,
        substantiated_team,
    }
}

// ---------------------------------------------------------------------------
// 1-2. Exactly the unsubstantiated rows are demoted; the substantiated ones
//      are untouched, by value
// ---------------------------------------------------------------------------

#[test]
fn cutover_demotes_exactly_the_unsubstantiated_rows_and_nothing_else() {
    let pg = pg!();
    let f = seed(&pg);

    // Before cutover: every seeded row carries a real claim, legacy or
    // substantiated alike — the baseline that makes "demoted" mean something.
    for (label, before) in [
        ("legacy memory", memory_verification(&pg, f.legacy_memory)),
        (
            "substantiated memory",
            memory_verification(&pg, f.substantiated_memory),
        ),
        (
            "legacy personal",
            knowledge_verification_row(&pg, "personal", f.legacy_personal),
        ),
        (
            "substantiated personal",
            knowledge_verification_row(&pg, "personal", f.substantiated_personal),
        ),
        (
            "legacy team",
            knowledge_verification_row(&pg, "team", f.legacy_team),
        ),
        (
            "substantiated team",
            knowledge_verification_row(&pg, "team", f.substantiated_team),
        ),
    ] {
        assert_ne!(
            before.state, "unverified",
            "{label}: the fixture must start with a real claim, or the demotion \
             assertion below is vacuous"
        );
    }

    let substantiated_before = [
        memory_verification(&pg, f.substantiated_memory),
        knowledge_verification_row(&pg, "personal", f.substantiated_personal),
        knowledge_verification_row(&pg, "team", f.substantiated_team),
    ];

    cutover(&pg);

    // Exactly the legacy rows are demoted to the empty state.
    let empty = empty_verification();
    assert_eq!(
        memory_verification(&pg, f.legacy_memory),
        empty,
        "the legacy project memory claim was not demoted to the empty state"
    );
    assert_eq!(
        knowledge_verification_row(&pg, "personal", f.legacy_personal),
        empty,
        "the legacy personal claim was not demoted to the empty state"
    );
    assert_eq!(
        knowledge_verification_row(&pg, "team", f.legacy_team),
        empty,
        "the legacy team claim was not demoted to the empty state"
    );

    // And exactly the substantiated rows are untouched, by value.
    let substantiated_after = [
        memory_verification(&pg, f.substantiated_memory),
        knowledge_verification_row(&pg, "personal", f.substantiated_personal),
        knowledge_verification_row(&pg, "team", f.substantiated_team),
    ];
    assert_eq!(
        substantiated_before, substantiated_after,
        "a substantiated record's verification changed under cutover; only \
         records with no backing report may move (§2 step 2)"
    );
    // And it is still a real claim, not merely unchanged-and-empty — otherwise
    // the equality above would be satisfied by demoting everything.
    for (label, v) in [
        ("substantiated memory", &substantiated_after[0]),
        ("substantiated personal", &substantiated_after[1]),
        ("substantiated team", &substantiated_after[2]),
    ] {
        assert_ne!(
            v.state, "unverified",
            "{label}: it was demoted along with the legacy rows, so this test \
             cannot tell 'left alone' from 'demoted the same as everything else'"
        );
    }
}

// ---------------------------------------------------------------------------
// 3. `legacy_verification_audit` holds the demoted values, and nothing is
//    deleted
// ---------------------------------------------------------------------------

#[test]
fn every_demoted_record_leaves_one_audit_row_carrying_what_was_claimed() {
    let pg = pg!();
    let f = seed(&pg);
    cutover(&pg);

    let row = pg.server.text(&format!(
        "SELECT legacy_state || '|' || COALESCE(legacy_authority, '')
             || '|' || to_char(legacy_last_verified_at AT TIME ZONE 'UTC',
                               'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"')
           FROM legacy_verification_audit
          WHERE domain = 'project' AND knowledge_id = '{}'",
        f.legacy_memory
    ));
    assert_eq!(
        row, "verified|cairn|2026-01-01T00:00:00Z",
        "the project audit row does not carry the legacy values that were demoted"
    );

    for (domain, id) in [("personal", f.legacy_personal), ("team", f.legacy_team)] {
        let row = pg.server.text(&format!(
            "SELECT legacy_state || '|' || COALESCE(legacy_authority, '')
                 || '|' || to_char(legacy_last_verified_at AT TIME ZONE 'UTC',
                                   'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"')
               FROM legacy_verification_audit
              WHERE domain = '{domain}' AND knowledge_id = '{id}'"
        ));
        assert_eq!(
            row, "verified|cairn|2026-01-01T00:00:00Z",
            "the {domain} audit row does not carry the legacy values that were demoted"
        );
    }

    // Nothing was deleted anywhere the audit could have come from instead of a
    // demotion: the source rows still exist, merely emptied.
    assert_eq!(
        pg.server.count(&format!(
            "SELECT count(*) FROM memories WHERE id = '{}'",
            f.legacy_memory
        )),
        1,
        "the legacy memory row was deleted rather than demoted"
    );
    assert_eq!(
        pg.server.count(&format!(
            "SELECT count(*) FROM knowledge_verification WHERE reference_key = 'knowledge:personal:{}'",
            f.legacy_personal
        )),
        1,
        "the legacy personal knowledge_verification row was deleted rather than demoted"
    );

    // And no audit row exists for a record that was never demoted.
    assert_eq!(
        pg.server.count(&format!(
            "SELECT count(*) FROM legacy_verification_audit
              WHERE domain = 'project' AND knowledge_id = '{}'",
            f.substantiated_memory
        )),
        0,
        "a substantiated record was audited as if it had been demoted"
    );
}

// ---------------------------------------------------------------------------
// 4. The cutover response's counts match the database
// ---------------------------------------------------------------------------

#[test]
fn the_cutover_responses_counts_match_what_is_in_the_database() {
    let pg = pg!();
    let f = seed(&pg);
    let _ = &f;

    let response = cutover(&pg);
    let demoted = response["demoted"]
        .as_i64()
        .unwrap_or_else(|| panic!("no `demoted` count in {response}"));
    let audited = response["audited"]
        .as_i64()
        .unwrap_or_else(|| panic!("no `audited` count in {response}"));

    let demoted_in_db = pg.server.count(
        "SELECT count(*) FROM (
             SELECT id::text FROM memories WHERE verification = 'unverified'
                                       AND verification_authority IS NULL
                                       AND last_verified_at IS NULL
             UNION ALL
             SELECT reference_key FROM knowledge_verification
              WHERE verification = 'unverified' AND verification_authority IS NULL
                AND last_verified_at IS NULL
         ) t",
    );
    // Every seeded row started at the empty state or was demoted to it by
    // cutover — nothing in this fixture is `unverified` for any other reason —
    // so the rows this query counts are exactly the rows cutover just emptied.
    assert_eq!(
        demoted, demoted_in_db,
        "the cutover response's `demoted` count does not match the rows now at \
         the empty state"
    );

    let audited_in_db = pg
        .server
        .count("SELECT count(*) FROM legacy_verification_audit");
    assert_eq!(
        audited, audited_in_db,
        "the cutover response's `audited` count does not match the number of \
         audit rows actually written"
    );
    assert_eq!(demoted, 3, "the fixture seeds exactly three legacy claims");
    assert_eq!(audited, 3, "the fixture seeds exactly three legacy claims");
}

// ---------------------------------------------------------------------------
// 5-6. The changes feed stops carrying server verification after cutover —
//      and that is the whole of the channel between cutover and a client's
//      own locally-derived verification, because it is the only thing a
//      server transaction ever sends a client
// ---------------------------------------------------------------------------

/// Before cutover, `GET /api/sync/changes` carries a `verification` object per
/// memory; after cutover it carries none at all — not an empty one, an absent
/// key (`migration-cutover.md`, "Changes feed stops carrying server
/// verification").
///
/// This is also the complete answer to "can cutover reach a client's own
/// locally-derived verification?" **No** — a server-side transaction over
/// `memories` and `knowledge_verification` has exactly one channel to a
/// client at all, which is whatever a route sends it, and this is that route.
/// If the `verification` key is absent here, there is nothing left for a
/// client to read as "the server says something about my local verification",
/// and therefore nothing cutover could use to overwrite it. There is no
/// second channel to check.
#[test]
fn the_changes_feed_stops_carrying_server_verification_after_cutover() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    let memory = seed_memory(&pg, session, "a memory read back through changes");
    substantiate(
        &pg,
        knowledge_ref("project", memory),
        "2026-08-01T09:03:00Z",
    );

    let (before, status) = get_json_status_bearer(
        &pg.server.base,
        &format!("/api/sync/changes?project_id={}", pg.project),
        &pg.owner.token,
    );
    assert_eq!(status, 200, "{before}");
    let row = before["memories"]
        .as_array()
        .and_then(|rows| rows.iter().find(|r| r["id"] == json!(memory)))
        .unwrap_or_else(|| panic!("the seeded memory is missing from {before}"));
    assert!(
        row.get("verification").is_some(),
        "before cutover, the changes feed must still carry `verification` — \
         without this the assertion below is vacuous: {row}"
    );
    assert_eq!(
        row["verification"]["state"],
        json!("verified"),
        "the pre-cutover verification object must reflect the real report: {row}"
    );

    cutover(&pg);

    let (after, status) = get_json_status_bearer(
        &pg.server.base,
        &format!("/api/sync/changes?project_id={}", pg.project),
        &pg.owner.token,
    );
    assert_eq!(status, 200, "{after}");
    let row = after["memories"]
        .as_array()
        .and_then(|rows| rows.iter().find(|r| r["id"] == json!(memory)))
        .unwrap_or_else(|| panic!("the seeded memory is missing from {after}"));
    assert!(
        row.get("verification").is_none(),
        "the changes feed still carries a `verification` key after cutover — \
         present-and-empty is not the same guarantee as absent, and a client \
         reading either as 'the server has an answer' is exactly what this \
         must stop: {row}"
    );
}
