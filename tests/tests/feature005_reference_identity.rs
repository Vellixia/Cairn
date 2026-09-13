//! One UUID in four domains stays four identities, everywhere (T034, SC-766,
//! SC-767).
//!
//! The adversarial input is the whole test: `project`, `personal`, `team` and
//! `pattern` records **deliberately sharing one UUID**. A suite using four
//! random ids would pass against a schema keyed on the bare UUID, which is
//! precisely the bug — and the consequence of that bug is not a mangled report,
//! it is a personal record served where a project record was asked for.
//!
//! So every surface that carries a reference is checked with the same id in all
//! four places: candidate results, retrieval traces, delivery dedup,
//! verification summaries and run reports. And every illegal
//! `(ref_kind, domain)` combination is checked to be refused by the database
//! rather than by application code, because FR-819a asks for the complete
//! logical reference to participate in *database* identity, and a CHECK that
//! exists on three tables out of five is a hole nobody notices.

use cairn_core::domain::{KnowledgeDomain, KnowledgeRef, PatternRef, Reference};
use cairn_e2e::feature005::Pg;
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

/// Every table that carries a polymorphic reference, and how to insert one.
///
/// Enumerated rather than sampled: a table added later without the CHECK is
/// exactly what this file exists to catch, and a test naming three tables would
/// pass on the day a fourth arrived.
const POLYMORPHIC_TABLES: &[&str] = &[
    "retrieval_trace_items",
    "verification_reports",
    "knowledge_verification",
    "delivered_context",
];

fn trace(pg: &Pg, session: Uuid) -> Uuid {
    let trace = Uuid::now_v7();
    pg.server.execute(&format!(
        "INSERT INTO retrieval_traces
             (trace_id, project_id, session_id, account_id, trigger, delivery_point,
              degradation_level, latency_ms, delivery_state)
         VALUES ('{trace}', '{}', '{session}', '{}', 'session_open', 'session_open',
                 'full', 5, 'generated')",
        pg.project, pg.owner.id
    ));
    trace
}

/// Insert one reference into `table`, returning the statement's outcome.
fn insert_ref(
    pg: &Pg,
    table: &str,
    trace_id: Uuid,
    session: Uuid,
    ref_kind: &str,
    domain: Option<&str>,
    id: Uuid,
) -> Result<(), String> {
    let d = domain.map(|d| format!("'{d}'")).unwrap_or("NULL".into());
    let sql = match table {
        "retrieval_trace_items" => format!(
            "INSERT INTO retrieval_trace_items
                 (trace_id, ref_kind, domain, knowledge_id, status, source_updated_at)
             VALUES ('{trace_id}', '{ref_kind}', {d}, '{id}', 'selected', now())"
        ),
        "verification_reports" => format!(
            "INSERT INTO verification_reports
                 (report_id, ref_kind, domain, knowledge_id, account_id, verdict,
                  verifier_kind, authority, run_at)
             VALUES ('{}', '{ref_kind}', {d}, '{id}', '{}', 'passed', 'command',
                     'remote_attested', now())",
            Uuid::now_v7(),
            pg.owner.id
        ),
        "knowledge_verification" => format!(
            "INSERT INTO knowledge_verification (ref_kind, domain, knowledge_id)
             VALUES ('{ref_kind}', {d}, '{id}')"
        ),
        "delivered_context" => format!(
            "INSERT INTO delivered_context
                 (session_id, ref_kind, domain, knowledge_id, delivered_at,
                  source_updated_at, delivery_point)
             VALUES ('{session}', '{ref_kind}', {d}, '{id}', now(), now(), 'session_open')"
        ),
        other => panic!("unknown polymorphic table {other}"),
    };
    pg.try_execute(&sql)
}

// ---------------------------------------------------------------------------
// Four identities from one UUID
// ---------------------------------------------------------------------------

#[test]
fn one_uuid_is_four_reference_keys_on_every_table_that_carries_one() {
    let pg = pg!();
    let ids = pg.seed_identical_ids(&pg.owner);
    let session = pg.session_for(&pg.owner);

    for table in POLYMORPHIC_TABLES {
        let trace_id = trace(&pg, session);
        // `knowledge_verification` covers the two domains without their own
        // columns; project knowledge keeps the five columns migration 2 gave
        // it, so it is legitimately absent there.
        let domains: Vec<(&str, Option<&str>)> = if *table == "knowledge_verification" {
            vec![
                ("knowledge", Some("personal")),
                ("knowledge", Some("team")),
                ("pattern", None),
            ]
        } else {
            vec![
                ("knowledge", Some("project")),
                ("knowledge", Some("personal")),
                ("knowledge", Some("team")),
                ("pattern", None),
            ]
        };
        for (ref_kind, domain) in &domains {
            insert_ref(&pg, table, trace_id, session, ref_kind, *domain, ids.id).unwrap_or_else(
                |e| panic!("{table} refused a legal {ref_kind}/{domain:?} reference: {e}"),
            );
        }
        let rows = pg.server.count(&format!(
            "SELECT count(*) FROM {table} WHERE knowledge_id = '{}'",
            ids.id
        ));
        assert_eq!(
            rows,
            domains.len() as i64,
            "{table} collapsed references that share a UUID; a schema keyed on \
             the bare id serves a personal record where a project record was asked for"
        );
        let keys = pg.server.count(&format!(
            "SELECT count(DISTINCT reference_key) FROM {table}
              WHERE knowledge_id = '{}'",
            ids.id
        ));
        assert_eq!(
            keys,
            domains.len() as i64,
            "{table} produced duplicate keys"
        );
    }
}

#[test]
fn the_generated_key_matches_what_the_rust_side_derives() {
    let pg = pg!();
    let id = Uuid::now_v7();
    let session = pg.session_for(&pg.owner);
    let trace_id = trace(&pg, session);

    // The SQL and the Rust derivations have to agree exactly, or a reference
    // written by one and looked up by the other names nothing.
    for (reference, ref_kind, domain) in [
        (
            Reference::Knowledge(KnowledgeRef::project(id)),
            "knowledge",
            Some("project"),
        ),
        (
            Reference::Knowledge(KnowledgeRef::personal(id)),
            "knowledge",
            Some("personal"),
        ),
        (
            Reference::Knowledge(KnowledgeRef::team(id)),
            "knowledge",
            Some("team"),
        ),
        (Reference::Pattern(PatternRef(id)), "pattern", None),
    ] {
        insert_ref(
            &pg,
            "retrieval_trace_items",
            trace_id,
            session,
            ref_kind,
            domain,
            id,
        )
        .expect("legal reference");
        let stored = pg.server.query_column(&format!(
            "SELECT reference_key FROM retrieval_trace_items
              WHERE trace_id = '{trace_id}' AND ref_kind = '{ref_kind}'
                AND domain IS NOT DISTINCT FROM {}",
            domain.map(|d| format!("'{d}'")).unwrap_or("NULL".into())
        ));
        assert_eq!(
            stored,
            vec![reference.reference_key()],
            "the database and `Reference::reference_key` disagree"
        );
    }
}

#[test]
fn a_candidate_result_keeps_the_domain_of_what_it_produced() {
    let pg = pg!();
    let id = Uuid::now_v7();
    let run = Uuid::now_v7();
    pg.server.execute(&format!(
        "INSERT INTO consolidation_runs (run_id, project_id, started_at, extractor_kind, state)
         VALUES ('{run}', '{}', now(), 'deterministic', 'finished')",
        pg.project
    ));

    // The same id as a project result and as a personal one are two different
    // outcomes, and a reader following the result has to land in the right
    // table.
    for (n, domain) in ["project", "personal", "team"].iter().enumerate() {
        pg.server.execute(&format!(
            "INSERT INTO knowledge_candidates
                 (candidate_id, run_id, project_id, proposed_kind, proposed_domain,
                  topic_key, value_key, content, decision, result_ref_kind, result_domain,
                  result_knowledge_id)
             VALUES ('{}', '{run}', '{}', 'fact', '{domain}', 'k{n}', 'v', 'c',
                     'accepted', 'knowledge', '{domain}', '{id}')",
            Uuid::now_v7(),
            pg.project
        ));
    }
    pg.server.execute(&format!(
        "INSERT INTO knowledge_candidates
             (candidate_id, run_id, project_id, proposed_kind, proposed_domain,
              topic_key, value_key, content, decision, result_ref_kind, result_domain,
              result_knowledge_id)
         VALUES ('{}', '{run}', '{}', 'fact', 'personal', 'kp', 'v', 'c',
                 'accepted', 'pattern', NULL, '{id}')",
        Uuid::now_v7(),
        pg.project
    ));

    assert_eq!(
        pg.server.count(&format!(
            "SELECT count(DISTINCT (result_ref_kind, result_domain))
               FROM knowledge_candidates WHERE result_knowledge_id = '{id}'"
        )),
        4,
        "four different results collapsed into fewer"
    );
}

// ---------------------------------------------------------------------------
// Illegal shapes, on every table
// ---------------------------------------------------------------------------

#[test]
fn every_table_refuses_knowledge_without_a_domain_and_a_pattern_with_one() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    for table in POLYMORPHIC_TABLES {
        let trace_id = trace(&pg, session);
        // A bare UUID pretending to be an identity.
        assert!(
            insert_ref(
                &pg,
                table,
                trace_id,
                session,
                "knowledge",
                None,
                Uuid::now_v7()
            )
            .is_err(),
            "{table} accepted a knowledge reference with no domain, so a bare \
             UUID is a cross-domain identity there (SC-766)"
        );
        // A fourth domain, encoded through the pattern discriminator.
        assert!(
            insert_ref(
                &pg,
                table,
                trace_id,
                session,
                "pattern",
                Some("personal"),
                Uuid::now_v7()
            )
            .is_err(),
            "{table} accepted a PatternRef carrying a domain"
        );
        // A discriminator outside the vocabulary.
        assert!(
            insert_ref(
                &pg,
                table,
                trace_id,
                session,
                "memory",
                Some("project"),
                Uuid::now_v7()
            )
            .is_err(),
            "{table} accepted an unknown ref_kind"
        );
    }
}

#[test]
fn the_reference_key_is_what_participates_in_row_identity() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    let id = Uuid::now_v7();

    // Two domains, one id: two rows. Then the *same* reference twice: one row.
    // Both halves matter — the first says the domain is in the key, the second
    // says the key is the key.
    for domain in ["project", "personal"] {
        insert_ref(
            &pg,
            "delivered_context",
            Uuid::nil(),
            session,
            "knowledge",
            Some(domain),
            id,
        )
        .expect("legal");
    }
    assert_eq!(
        pg.server.count(&format!(
            "SELECT count(*) FROM delivered_context WHERE knowledge_id = '{id}'"
        )),
        2
    );
    assert!(
        insert_ref(
            &pg,
            "delivered_context",
            Uuid::nil(),
            session,
            "knowledge",
            Some("project"),
            id
        )
        .is_err(),
        "the same reference was delivered twice into one session"
    );
}

#[test]
fn two_reporters_of_one_record_are_two_reports() {
    let pg = pg!();
    let id = Uuid::now_v7();
    let run_at = "2026-09-02T10:00:00Z";
    let report = |account: Uuid| {
        format!(
            "INSERT INTO verification_reports
                 (report_id, ref_kind, domain, knowledge_id, account_id, verdict,
                  verifier_kind, authority, run_at)
             VALUES ('{}', 'knowledge', 'team', '{id}', '{account}', 'passed',
                     'command', 'remote_attested', '{run_at}')",
            Uuid::now_v7()
        )
    };
    // Two accounts reporting one record are two pieces of evidence; one account
    // retrying its own run is one.
    pg.server.execute(&report(pg.owner.id));
    pg.server.execute(&report(pg.member.id));
    assert!(pg.try_execute(&report(pg.owner.id)).is_err());
    assert_eq!(
        pg.server.count(&format!(
            "SELECT count(*) FROM verification_reports WHERE knowledge_id = '{id}'"
        )),
        2
    );
}

#[test]
fn a_pattern_reference_still_resolves_to_a_personal_domain_record() {
    let pg = pg!();
    let ids = pg.seed_identical_ids(&pg.owner);
    // The NULL domain slot means "this row holds a PatternRef", never "this
    // record has no domain". The row it names says `personal` explicitly.
    assert_eq!(
        pg.server.text(&format!(
            "SELECT domain FROM shared_patterns WHERE pattern_id = '{}'",
            ids.pattern
        )),
        "personal"
    );
    assert_eq!(
        Reference::Pattern(PatternRef(ids.pattern)).canonical_domain(),
        KnowledgeDomain::Personal
    );
    assert_eq!(
        Reference::Pattern(PatternRef(ids.pattern)).domain_slot(),
        None
    );
}
