//! Server schema v4 — the migration, and every constraint that carries meaning
//! (T008, `data-model.md` §6, SC-766).
//!
//! The theme running through this file is that Feature 005's correctness is
//! largely *structural*. Three things in particular cannot be left to
//! application validation:
//!
//! - **The canonical `reference_key`.** Project, personal and team knowledge
//!   are three tables, so the same UUID can legitimately exist in all three.
//!   Every reference therefore carries its domain, and the generated key is
//!   what takes part in row identity. A test that used four random UUIDs would
//!   pass against a schema that keyed on the bare id, which is exactly the bug.
//!   So the fixture gives all four records *deliberately the same* UUID.
//! - **The trace lifecycle.** `requested → generated → transmitted | failed` is
//!   how Cairn avoids claiming it delivered context it only generated. Each
//!   transition has a NULL that must not be allowed to hide it.
//! - **The pattern's domain and trust.** A `shared_patterns` row is a
//!   personal-domain record with exactly one establishable trust value, and
//!   both are CHECKed to a single value so no writer can widen them.
//!
//! Every test skips, loudly, without `CAIRN_TEST_DATABASE_URL`. A vacuous pass
//! here would be a schema nobody checked.

use cairn_e2e::feature005::{Pg, SERVER_SCHEMA_V3, SERVER_SCHEMA_V4};
use cairn_e2e::Server;

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

/// Every table v4 introduces.
const V4_TABLES: &[&str] = &[
    "safe_events",
    "consolidation_session",
    "consolidation_work",
    "consolidation_runs",
    "knowledge_candidates",
    "candidate_source_events",
    "retrieval_traces",
    "retrieval_trace_items",
    "verification_reports",
    "knowledge_verification",
    "legacy_verification_audit",
    "shared_patterns",
    "integration_health",
    "delivered_context",
    "capture_dispositions",
    "server_authority",
];

// ---------------------------------------------------------------------------
// The migration
// ---------------------------------------------------------------------------

#[test]
fn v3_migrates_to_v4_and_keeps_what_v3_held() {
    let Some(mut old) = Server::start_at_schema(SERVER_SCHEMA_V3) else {
        eprintln!("skipped: CAIRN_TEST_DATABASE_URL is not set");
        return;
    };
    assert_eq!(
        old.count("SELECT COALESCE(MAX(version), 0) FROM schema_migrations"),
        SERVER_SCHEMA_V3
    );
    for table in V4_TABLES {
        assert_eq!(
            old.count(&format!(
                "SELECT count(*) FROM information_schema.tables
                  WHERE table_schema = 'public' AND table_name = '{table}'"
            )),
            0,
            "v3 already has {table}, so v4 is not what introduces it"
        );
    }

    // A project, a session and a memory, exactly as a v3 installation holds
    // them. `origin_kind` does not exist yet, which is the point: after the
    // migration it must exist and must be NULL, because nobody recorded a
    // provenance for a row written before the distinction did.
    let (user, _) = old.new_user("v3-owner");
    let project = uuid::Uuid::now_v7();
    let session = uuid::Uuid::now_v7();
    let memory = uuid::Uuid::now_v7();
    old.execute(&format!(
        "INSERT INTO projects (id, name) VALUES ('{project}', 'a v3 project')"
    ));
    old.execute(&format!(
        "INSERT INTO project_members (project_id, user_id) VALUES ('{project}', '{user}')"
    ));
    old.execute(&format!(
        "INSERT INTO sessions (id, project_id, user_id, agent, branch, status, started_at)
         VALUES ('{session}', '{project}', '{user}', 'claude-code', 'main', 'active', now())"
    ));
    old.execute(&format!(
        "INSERT INTO memories (id, project_id, type, scope, scope_key, content,
                               origin_session_id)
         VALUES ('{memory}', '{project}', 'fact', 'project', '{project}',
                 'a v3 memory', '{session}')"
    ));

    let new = old.upgraded();

    assert_eq!(
        new.count("SELECT COALESCE(MAX(version), 0) FROM schema_migrations"),
        SERVER_SCHEMA_V4
    );
    for table in V4_TABLES {
        assert_eq!(
            new.count(&format!(
                "SELECT count(*) FROM information_schema.tables
                  WHERE table_schema = 'public' AND table_name = '{table}'"
            )),
            1,
            "v4 did not create {table}"
        );
    }
    assert_eq!(
        new.text(&format!(
            "SELECT content FROM memories WHERE id = '{memory}'"
        )),
        "a v3 memory",
        "the migration rewrote an existing row"
    );
    assert_eq!(
        new.count(&format!(
            "SELECT count(*) FROM memories WHERE id = '{memory}' AND origin_kind IS NULL"
        )),
        1,
        "the migration invented a provenance for a row written before Cairn tracked one"
    );
}

#[test]
fn the_migration_initializes_server_authority_before_its_own_cutover() {
    let pg = pg!();
    assert_eq!(
        pg.server.count("SELECT count(*) FROM server_authority"),
        1,
        "server_authority is a single-row table"
    );
    assert_eq!(
        pg.server
            .text("SELECT mode FROM server_authority WHERE id = 1"),
        "pre_cutover",
        "a fresh deployment was initialized as if a migration had already \
         established canonical possession"
    );
    assert_eq!(
        pg.server
            .count("SELECT count(*) FROM server_authority WHERE cutover_at IS NULL"),
        1
    );

    pg.refuses(
        "a second server_authority row",
        "INSERT INTO server_authority (id, mode) VALUES (2, 'pre_cutover')",
    );
    pg.refuses(
        "an unknown authority mode",
        "UPDATE server_authority SET mode = 'whenever_i_feel_like_it' WHERE id = 1",
    );
}

// ---------------------------------------------------------------------------
// The canonical reference key (SC-766, SC-767)
// ---------------------------------------------------------------------------

#[test]
fn four_records_sharing_one_uuid_produce_four_distinct_reference_keys() {
    let pg = pg!();
    let ids = pg.seed_identical_ids(&pg.owner);
    assert!(
        pg.table_exists("shared_patterns"),
        "the pattern half of this fixture needs v4"
    );

    let session = pg.session_for(&pg.owner);
    for (ref_kind, domain) in [
        ("knowledge", Some("project")),
        ("knowledge", Some("personal")),
        ("knowledge", Some("team")),
        ("pattern", None),
    ] {
        let domain_sql = domain.map(|d| format!("'{d}'")).unwrap_or("NULL".into());
        pg.server.execute(&format!(
            "INSERT INTO delivered_context
                 (session_id, ref_kind, domain, knowledge_id, delivered_at,
                  source_updated_at, delivery_point)
             VALUES ('{session}', '{ref_kind}', {domain_sql}, '{}', now(), now(),
                     'session_open')",
            ids.id
        ));
    }

    // Four rows, not one. A schema keyed on the bare UUID would have collapsed
    // them, and a personal record would then be suppressed as "already
    // delivered" because a project record with the same id had been.
    assert_eq!(
        pg.server.count(&format!(
            "SELECT count(*) FROM delivered_context WHERE session_id = '{session}'"
        )),
        4
    );
    let mut keys = pg.server.query_column(&format!(
        "SELECT reference_key FROM delivered_context
          WHERE session_id = '{session}' ORDER BY reference_key"
    ));
    keys.sort();
    let mut expected = ids.reference_keys().to_vec();
    expected.sort();
    assert_eq!(
        keys, expected,
        "the generated key is not the canonical form"
    );
}

#[test]
fn every_polymorphic_table_generates_the_same_reference_key() {
    let pg = pg!();
    let id = uuid::Uuid::now_v7();
    let session = pg.session_for(&pg.owner);
    let trace = uuid::Uuid::now_v7();
    pg.server.execute(&format!(
        "INSERT INTO retrieval_traces
             (trace_id, project_id, session_id, account_id, trigger, delivery_point,
              degradation_level, latency_ms, delivery_state)
         VALUES ('{trace}', '{}', '{session}', '{}', 'session_open', 'session_open',
                 'full', 12, 'generated')",
        pg.project, pg.owner.id
    ));

    pg.server.execute(&format!(
        "INSERT INTO retrieval_trace_items
             (trace_id, ref_kind, domain, knowledge_id, status, source_updated_at)
         VALUES ('{trace}', 'knowledge', 'team', '{id}', 'selected', now())"
    ));
    pg.server.execute(&format!(
        "INSERT INTO verification_reports
             (report_id, ref_kind, domain, knowledge_id, account_id, verdict,
              verifier_kind, authority, run_at)
         VALUES ('{}', 'knowledge', 'team', '{id}', '{}', 'passed', 'command',
                 'remote_attested', now())",
        uuid::Uuid::now_v7(),
        pg.owner.id
    ));
    pg.server.execute(&format!(
        "INSERT INTO knowledge_verification (ref_kind, domain, knowledge_id)
         VALUES ('knowledge', 'team', '{id}')"
    ));
    pg.server.execute(&format!(
        "INSERT INTO delivered_context
             (session_id, ref_kind, domain, knowledge_id, delivered_at,
              source_updated_at, delivery_point)
         VALUES ('{session}', 'knowledge', 'team', '{id}', now(), now(), 'prompt_time')"
    ));

    let expected = format!("knowledge:team:{id}");
    for table in [
        "retrieval_trace_items",
        "verification_reports",
        "knowledge_verification",
        "delivered_context",
    ] {
        assert_eq!(
            pg.server.query_column(&format!(
                "SELECT reference_key FROM {table} WHERE knowledge_id = '{id}'"
            )),
            vec![expected.clone()],
            "{table} generates a different reference key from the others; \
             two tables that disagree about identity cannot be joined honestly"
        );
    }
}

#[test]
fn every_polymorphic_table_refuses_a_mismatched_discriminator() {
    let pg = pg!();
    let id = uuid::Uuid::now_v7();
    let session = pg.session_for(&pg.owner);
    let trace = uuid::Uuid::now_v7();
    pg.server.execute(&format!(
        "INSERT INTO retrieval_traces
             (trace_id, project_id, session_id, account_id, trigger, delivery_point,
              degradation_level, latency_ms, delivery_state)
         VALUES ('{trace}', '{}', '{session}', '{}', 'session_open', 'session_open',
                 'full', 3, 'generated')",
        pg.project, pg.owner.id
    ));

    // A knowledge reference with no domain is the bug the discriminator exists
    // to prevent: it is a bare UUID pretending to be an identity.
    pg.refuses(
        "a trace item naming knowledge without a domain",
        &format!(
            "INSERT INTO retrieval_trace_items
                 (trace_id, ref_kind, domain, knowledge_id, status, source_updated_at)
             VALUES ('{trace}', 'knowledge', NULL, '{id}', 'selected', now())"
        ),
    );
    // A pattern carrying a domain would encode a fourth domain, which
    // Constitution IV does not have.
    pg.refuses(
        "a trace item giving a PatternRef a domain",
        &format!(
            "INSERT INTO retrieval_trace_items
                 (trace_id, ref_kind, domain, knowledge_id, status, source_updated_at)
             VALUES ('{trace}', 'pattern', 'personal', '{id}', 'selected', now())"
        ),
    );
    pg.refuses(
        "a delivered-context row naming knowledge without a domain",
        &format!(
            "INSERT INTO delivered_context
                 (session_id, ref_kind, domain, knowledge_id, delivered_at,
                  source_updated_at, delivery_point)
             VALUES ('{session}', 'knowledge', NULL, '{id}', now(), now(), 'session_open')"
        ),
    );
    pg.refuses(
        "a verification report giving a PatternRef a domain",
        &format!(
            "INSERT INTO verification_reports
                 (report_id, ref_kind, domain, knowledge_id, account_id, verdict,
                  verifier_kind, authority, run_at)
             VALUES ('{}', 'pattern', 'team', '{id}', '{}', 'passed', 'command',
                     'remote_attested', now())",
            uuid::Uuid::now_v7(),
            pg.owner.id
        ),
    );
    // `knowledge_verification` covers the two domains without their own
    // columns; project knowledge keeps the five columns migration 2 gave it.
    pg.refuses(
        "a knowledge_verification row claiming the project domain",
        &format!(
            "INSERT INTO knowledge_verification (ref_kind, domain, knowledge_id)
             VALUES ('knowledge', 'project', '{id}')"
        ),
    );
    pg.refuses(
        "an unknown ref_kind",
        &format!(
            "INSERT INTO knowledge_verification (ref_kind, domain, knowledge_id)
             VALUES ('memory', 'team', '{id}')"
        ),
    );
}

#[test]
fn two_accounts_may_report_one_record_but_one_account_may_not_report_it_twice() {
    let pg = pg!();
    let id = uuid::Uuid::now_v7();
    let run_at = "2026-09-02T10:00:00Z";
    let report = |account: uuid::Uuid| {
        format!(
            "INSERT INTO verification_reports
                 (report_id, ref_kind, domain, knowledge_id, account_id, verdict,
                  verifier_kind, authority, run_at)
             VALUES ('{}', 'knowledge', 'team', '{id}', '{account}', 'passed',
                     'command', 'remote_attested', '{run_at}')",
            uuid::Uuid::now_v7()
        )
    };
    pg.server.execute(&report(pg.owner.id));
    // Two accounts reporting the same record are two pieces of evidence.
    pg.server.execute(&report(pg.member.id));
    // The same account retrying its own run is one.
    pg.refuses("a duplicate report from one account", &report(pg.owner.id));

    assert_eq!(
        pg.server.count(&format!(
            "SELECT count(*) FROM verification_reports WHERE knowledge_id = '{id}'"
        )),
        2
    );
}

// ---------------------------------------------------------------------------
// The trace lifecycle
// ---------------------------------------------------------------------------

fn trace_sql(
    pg: &Pg,
    session: uuid::Uuid,
    state: &str,
    degradation: &str,
    latency: &str,
    failure: &str,
) -> String {
    format!(
        "INSERT INTO retrieval_traces
             (trace_id, project_id, session_id, account_id, trigger, delivery_point,
              degradation_level, latency_ms, delivery_state, failure_reason)
         VALUES ('{}', '{}', '{session}', '{}', 'session_open', 'session_open',
                 {degradation}, {latency}, '{state}', {failure})",
        uuid::Uuid::now_v7(),
        pg.project,
        pg.owner.id
    )
}

#[test]
fn a_requested_trace_exists_before_anything_is_generated() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    // The Principle X row: an authenticated retrieval is recorded the moment it
    // is asked for, carrying no latency, no degradation level and no result,
    // because none of those exist yet.
    pg.server.execute(&trace_sql(
        &pg,
        session,
        "requested",
        "NULL",
        "NULL",
        "NULL",
    ));
    assert_eq!(
        pg.server
            .count("SELECT count(*) FROM retrieval_traces WHERE delivery_state = 'requested'"),
        1
    );
    assert_eq!(
        pg.server.text(
            "SELECT acknowledgement_state FROM retrieval_traces
              WHERE delivery_state = 'requested'"
        ),
        "unavailable",
        "receipt defaulted to something other than no evidence"
    );
}

#[test]
fn the_trace_lifecycle_constraints_refuse_every_state_that_would_overclaim() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);

    pg.refuses(
        "a requested trace already carrying a latency it cannot have measured",
        &trace_sql(&pg, session, "requested", "NULL", "5", "NULL"),
    );
    pg.refuses(
        "a generated trace with no latency",
        &trace_sql(&pg, session, "generated", "'full'", "NULL", "NULL"),
    );
    pg.refuses(
        "a generated trace claiming no degradation level",
        &trace_sql(&pg, session, "generated", "NULL", "7", "NULL"),
    );
    pg.refuses(
        "a transmitted trace claiming no degradation level",
        &trace_sql(&pg, session, "transmitted", "NULL", "7", "NULL"),
    );
    pg.refuses(
        "a failed trace with no reason",
        &trace_sql(&pg, session, "failed", "NULL", "7", "NULL"),
    );
    pg.refuses(
        "a successful trace carrying a failure reason",
        &trace_sql(&pg, session, "generated", "'full'", "7", "'timeout'"),
    );
    pg.refuses(
        "an unknown delivery state",
        &trace_sql(&pg, session, "delivered", "'full'", "7", "NULL"),
    );
    pg.refuses(
        "an unknown acknowledgement state",
        &format!(
            "INSERT INTO retrieval_traces
                 (trace_id, project_id, session_id, account_id, trigger, delivery_point,
                  degradation_level, latency_ms, delivery_state, acknowledgement_state)
             VALUES ('{}', '{}', '{session}', '{}', 'session_open', 'session_open',
                     'full', 7, 'generated', 'probably')",
            uuid::Uuid::now_v7(),
            pg.project,
            pg.owner.id
        ),
    );

    // A failed retrieval is a legal, recordable outcome — it just has to say
    // why. Its degradation level stays NULL because nothing was built.
    pg.server.execute(&trace_sql(
        &pg,
        session,
        "failed",
        "NULL",
        "40",
        "'server_unreachable'",
    ));
    assert_eq!(
        pg.server
            .count("SELECT count(*) FROM retrieval_traces WHERE delivery_state = 'failed'"),
        1,
        "a failed retrieval must stay visible rather than disappearing"
    );
}

#[test]
fn trace_items_are_deleted_with_their_trace_and_carry_a_source_timestamp() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    let trace = uuid::Uuid::now_v7();
    pg.server.execute(&format!(
        "INSERT INTO retrieval_traces
             (trace_id, project_id, session_id, account_id, trigger, delivery_point,
              degradation_level, latency_ms, delivery_state)
         VALUES ('{trace}', '{}', '{session}', '{}', 'session_open', 'session_open',
                 'full', 9, 'generated')",
        pg.project, pg.owner.id
    ));
    pg.server.execute(&format!(
        "INSERT INTO retrieval_trace_items
             (trace_id, ref_kind, domain, knowledge_id, status, source_updated_at)
         VALUES ('{trace}', 'knowledge', 'project', '{}', 'considered', now())",
        uuid::Uuid::now_v7()
    ));

    // `source_updated_at` is mandatory: the delivery-dedup upsert compares
    // against the version that was actually selected, and by the time an
    // outcome is reported the source row may have moved on.
    pg.refuses(
        "a trace item with no source timestamp",
        &format!(
            "INSERT INTO retrieval_trace_items
                 (trace_id, ref_kind, domain, knowledge_id, status)
             VALUES ('{trace}', 'knowledge', 'project', '{}', 'considered')",
            uuid::Uuid::now_v7()
        ),
    );

    // The 90-day retention sweep deletes traces; the items must go with them
    // rather than being left as orphans referencing nothing.
    pg.server.execute(&format!(
        "DELETE FROM retrieval_traces WHERE trace_id = '{trace}'"
    ));
    assert_eq!(
        pg.server.count(&format!(
            "SELECT count(*) FROM retrieval_trace_items WHERE trace_id = '{trace}'"
        )),
        0
    );
}

// ---------------------------------------------------------------------------
// Patterns — owner-only, single-domain, single-trust
// ---------------------------------------------------------------------------

#[test]
fn a_shared_pattern_is_a_personal_domain_record_with_one_establishable_trust() {
    let pg = pg!();
    let id = uuid::Uuid::now_v7();
    pg.server.execute(&format!(
        "INSERT INTO shared_patterns
             (pattern_id, owner_user_id, title, problem, root_cause, approach, content_key)
         VALUES ('{id}', '{}', 't', 'p', 'r', 'a', 'ck-1')",
        pg.owner.id
    ));
    // The defaults say what the record is, without the writer having to.
    assert_eq!(
        pg.server.text(&format!(
            "SELECT domain FROM shared_patterns WHERE pattern_id = '{id}'"
        )),
        "personal",
        "a pattern is a personal-domain record, not a domain-less one"
    );
    assert_eq!(
        pg.server.text(&format!(
            "SELECT trust FROM shared_patterns WHERE pattern_id = '{id}'"
        )),
        "sanitized"
    );

    pg.refuses(
        "a pattern claiming the team domain",
        &format!("UPDATE shared_patterns SET domain = 'team' WHERE pattern_id = '{id}'"),
    );
    pg.refuses(
        "a pattern claiming the project domain",
        &format!("UPDATE shared_patterns SET domain = 'project' WHERE pattern_id = '{id}'"),
    );
    // `validated` and `contested` are derived from `pattern_applications`,
    // which stay local-only. The server has no evidence for them, and a client
    // asserting one would be asserting a state earned privately on a record the
    // server cannot check.
    for overclaim in ["validated", "contested", "candidate"] {
        pg.refuses(
            &format!("a pattern asserting trust '{overclaim}'"),
            &format!("UPDATE shared_patterns SET trust = '{overclaim}' WHERE pattern_id = '{id}'"),
        );
    }
}

#[test]
fn a_pattern_must_have_an_owner_and_the_same_owner_cannot_hold_it_twice() {
    let pg = pg!();
    pg.refuses(
        "a pattern with no owner",
        &format!(
            "INSERT INTO shared_patterns
                 (pattern_id, owner_user_id, title, problem, root_cause, approach,
                  content_key)
             VALUES ('{}', NULL, 't', 'p', 'r', 'a', 'ck')",
            uuid::Uuid::now_v7()
        ),
    );
    pg.refuses(
        "a pattern owned by an account that does not exist",
        &format!(
            "INSERT INTO shared_patterns
                 (pattern_id, owner_user_id, title, problem, root_cause, approach,
                  content_key)
             VALUES ('{}', '{}', 't', 'p', 'r', 'a', 'ck')",
            uuid::Uuid::now_v7(),
            uuid::Uuid::now_v7()
        ),
    );

    let insert = |id: uuid::Uuid, owner: uuid::Uuid, key: &str| {
        format!(
            "INSERT INTO shared_patterns
                 (pattern_id, owner_user_id, title, problem, root_cause, approach,
                  content_key)
             VALUES ('{id}', '{owner}', 't', 'p', 'r', 'a', '{key}')"
        )
    };
    pg.server
        .execute(&insert(uuid::Uuid::now_v7(), pg.owner.id, "same-content"));
    // Promotion is idempotent: the same owner and the same normalized content
    // derive the same identity, so a repeat is an upsert rather than a second
    // record (SC-760).
    pg.refuses(
        "one owner holding two copies of one pattern",
        &insert(uuid::Uuid::now_v7(), pg.owner.id, "same-content"),
    );
    // Two people whose patterns read alike own two patterns.
    pg.server
        .execute(&insert(uuid::Uuid::now_v7(), pg.member.id, "same-content"));
    assert_eq!(
        pg.server
            .count("SELECT count(*) FROM shared_patterns WHERE content_key = 'same-content'"),
        2
    );
}

#[test]
fn the_pattern_table_has_no_column_that_could_name_where_it_came_from() {
    let pg = pg!();
    // Each of these is a refused field name or a project-naming column, and
    // each absence is load-bearing rather than incidental: a
    // project-independent record must not be able to name the project it was
    // learned in.
    for column in [
        "signals",
        "signal_digest",
        "origin_ref",
        "sanitization_report",
        "source_memory_id",
        "origin_deleted",
        "project_id",
    ] {
        assert!(
            !pg.column_exists("shared_patterns", column),
            "shared_patterns has a `{column}` column, which is exactly the shape \
             the safe replacement record exists to avoid"
        );
    }
}

// ---------------------------------------------------------------------------
// Nullable project bindings, and the FTS indexes
// ---------------------------------------------------------------------------

#[test]
fn personal_and_team_reports_need_no_project_to_name() {
    let pg = pg!();
    // Personal and team knowledge are project-independent. A report about one
    // has no project to bind to, and inventing one would leak the project the
    // reporter happened to be sitting in.
    pg.server.execute(&format!(
        "INSERT INTO verification_reports
             (report_id, ref_kind, domain, knowledge_id, project_id, account_id, verdict,
              verifier_kind, authority, run_at)
         VALUES ('{}', 'knowledge', 'personal', '{}', NULL, '{}', 'passed', 'command',
                 'remote_attested', now())",
        uuid::Uuid::now_v7(),
        uuid::Uuid::now_v7(),
        pg.owner.id
    ));
    pg.server.execute(&format!(
        "INSERT INTO verification_reports
             (report_id, ref_kind, domain, knowledge_id, project_id, owner_user_id,
              account_id, verdict, verifier_kind, authority, run_at)
         VALUES ('{}', 'pattern', NULL, '{}', NULL, '{}', '{}', 'inconclusive', 'command',
                 'remote_attested', now())",
        uuid::Uuid::now_v7(),
        uuid::Uuid::now_v7(),
        pg.owner.id,
        pg.owner.id
    ));
    assert_eq!(
        pg.server
            .count("SELECT count(*) FROM verification_reports WHERE project_id IS NULL"),
        2
    );
}

#[test]
fn personal_and_team_knowledge_gained_the_text_index_project_memories_already_had() {
    let pg = pg!();
    // FR-806 puts server-side retrieval over these two domains in scope, and
    // neither table had a text index before v4.
    for index in [
        "personal_knowledge_search",
        "team_knowledge_search",
        "shared_patterns_search",
    ] {
        assert!(pg.index_exists(index), "v4 did not create {index}");
    }
    // Mirrors of the existing `memories_search`, which is what makes the three
    // domains searchable the same way rather than three different ways.
    assert!(pg.index_exists("memories_search"));
}

#[test]
fn the_text_index_makes_personal_search_fast_it_does_not_make_it_safe() {
    let pg = pg!();
    let mine = uuid::Uuid::now_v7();
    let theirs = uuid::Uuid::now_v7();
    for (id, owner, seq) in [(mine, pg.owner.id, 1), (theirs, pg.member.id, 2)] {
        pg.server.execute(&format!(
            "INSERT INTO personal_knowledge
                 (id, owner_user_id, knowledge_type, content, writer_id, writer_seq)
             VALUES ('{id}', '{owner}', 'fact',
                     'the deployment pipeline rejects unsigned images',
                     'w-{id}', {seq})"
        ));
    }

    // Both rows match the query. The index does not care who owns them, which
    // is the point of asserting it here: ownership is a filter the read path
    // applies, and a reader who forgot it would see a colleague's personal
    // knowledge with the search still looking correct.
    assert_eq!(
        pg.server.count(
            "SELECT count(*) FROM personal_knowledge
              WHERE to_tsvector('english', content) @@ plainto_tsquery('english', 'unsigned images')"
        ),
        2
    );
    assert_eq!(
        pg.server.count(&format!(
            "SELECT count(*) FROM personal_knowledge
              WHERE owner_user_id = '{}'
                AND to_tsvector('english', content)
                    @@ plainto_tsquery('english', 'unsigned images')",
            pg.owner.id
        )),
        1
    );
}

// ---------------------------------------------------------------------------
// Consolidation, events and the closed vocabularies
// ---------------------------------------------------------------------------

#[test]
fn an_event_is_idempotent_on_its_id_and_unique_on_its_session_ordinal() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    let event = uuid::Uuid::now_v7();
    let insert = |id: uuid::Uuid, seq: i64| {
        format!(
            "INSERT INTO safe_events
                 (event_id, project_id, session_id, account_id, agent, kind, session_seq,
                  contract_version, content, occurred_at)
             VALUES ('{id}', '{}', '{session}', '{}', 'claude-code', 'tool_invoked', {seq},
                     1, '{{}}'::jsonb, now())",
            pg.project, pg.owner.id
        )
    };
    pg.server.execute(&insert(event, 1));
    // A retried batch re-derives the same event id, and the primary key is what
    // turns that into a no-op rather than a duplicate.
    pg.refuses("a re-delivered event", &insert(event, 1));
    // A different event claiming an ordinal already used in the session would
    // make the session's history ambiguous.
    pg.refuses(
        "a second event at ordinal 1",
        &insert(uuid::Uuid::now_v7(), 1),
    );
    pg.server.execute(&insert(uuid::Uuid::now_v7(), 2));

    pg.refuses(
        "an event naming a project that does not exist",
        &format!(
            "INSERT INTO safe_events
                 (event_id, project_id, session_id, account_id, agent, kind, session_seq,
                  contract_version, content, occurred_at)
             VALUES ('{}', '{}', '{session}', '{}', 'claude-code', 'tool_invoked', 9,
                     1, '{{}}'::jsonb, now())",
            uuid::Uuid::now_v7(),
            uuid::Uuid::now_v7(),
            pg.owner.id
        ),
    );
    pg.refuses(
        "an event naming an account that does not exist",
        &format!(
            "INSERT INTO safe_events
                 (event_id, project_id, session_id, account_id, agent, kind, session_seq,
                  contract_version, content, occurred_at)
             VALUES ('{}', '{}', '{session}', '{}', 'claude-code', 'tool_invoked', 10,
                     1, '{{}}'::jsonb, now())",
            uuid::Uuid::now_v7(),
            pg.project,
            uuid::Uuid::now_v7()
        ),
    );
}

#[test]
fn consolidation_work_cannot_exist_without_the_session_row_that_gets_claimed() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    let event = uuid::Uuid::now_v7();
    pg.server.execute(&format!(
        "INSERT INTO safe_events
             (event_id, project_id, session_id, account_id, agent, kind, session_seq,
              contract_version, content, occurred_at)
         VALUES ('{event}', '{}', '{session}', '{}', 'claude-code', 'tool_invoked', 1,
                 1, '{{}}'::jsonb, now())",
        pg.project, pg.owner.id
    ));

    // The lease row is what a worker actually locks — PostgreSQL will not take
    // a locking clause with GROUP BY — so work without one could never be
    // claimed at all.
    pg.refuses(
        "queued work with no session lease row",
        &format!(
            "INSERT INTO consolidation_work
                 (event_id, project_id, session_id, session_seq, state)
             VALUES ('{event}', '{}', '{session}', 1, 'pending')",
            pg.project
        ),
    );

    pg.server.execute(&format!(
        "INSERT INTO consolidation_session (project_id, session_id)
         VALUES ('{}', '{session}')",
        pg.project
    ));
    pg.server.execute(&format!(
        "INSERT INTO consolidation_work
             (event_id, project_id, session_id, session_seq, state)
         VALUES ('{event}', '{}', '{session}', 1, 'pending')",
        pg.project
    ));
    assert_eq!(
        pg.server.text(&format!(
            "SELECT state FROM consolidation_session WHERE session_id = '{session}'"
        )),
        "pending"
    );

    pg.refuses(
        "work queued for an event the server never accepted",
        &format!(
            "INSERT INTO consolidation_work
                 (event_id, project_id, session_id, session_seq, state)
             VALUES ('{}', '{}', '{session}', 2, 'pending')",
            uuid::Uuid::now_v7(),
            pg.project
        ),
    );
    pg.refuses(
        "an unknown work state",
        &format!("UPDATE consolidation_work SET state = 'maybe' WHERE event_id = '{event}'"),
    );
    pg.refuses(
        "an unknown lease state",
        &format!(
            "UPDATE consolidation_session SET state = 'working' WHERE session_id = '{session}'"
        ),
    );
}

#[test]
fn a_candidate_records_both_what_was_proposed_and_what_cairn_decided() {
    let pg = pg!();
    let run = uuid::Uuid::now_v7();
    pg.server.execute(&format!(
        "INSERT INTO consolidation_runs
             (run_id, project_id, started_at, extractor_kind, state)
         VALUES ('{run}', '{}', now(), 'deterministic', 'finished')",
        pg.project
    ));

    let candidate = uuid::Uuid::now_v7();
    pg.server.execute(&format!(
        "INSERT INTO knowledge_candidates
             (candidate_id, run_id, project_id, proposed_kind, proposed_domain,
              topic_key, value_key, content, decision, result_ref_kind, result_domain,
              result_knowledge_id)
         VALUES ('{candidate}', '{run}', '{}', 'failure', 'project',
                 'deploy.images', 'unsigned', 'unsigned images are rejected',
                 'accepted', 'knowledge', 'project', '{}')",
        pg.project,
        uuid::Uuid::now_v7()
    ));

    // A refused candidate names no record, and the CHECK is what stops a
    // half-filled result from reading as a real one.
    pg.refuses(
        "a candidate result naming a domain but no record",
        &format!(
            "INSERT INTO knowledge_candidates
                 (candidate_id, run_id, project_id, proposed_kind, proposed_domain,
                  content, decision, result_ref_kind, result_domain)
             VALUES ('{}', '{run}', '{}', 'fact', 'project', 'c', 'accepted',
                     'knowledge', 'project')",
            uuid::Uuid::now_v7(),
            pg.project
        ),
    );
    pg.refuses(
        "a pattern result carrying a domain",
        &format!(
            "INSERT INTO knowledge_candidates
                 (candidate_id, run_id, project_id, proposed_kind, proposed_domain,
                  content, decision, result_ref_kind, result_domain, result_knowledge_id)
             VALUES ('{}', '{run}', '{}', 'fact', 'personal', 'c', 'accepted',
                     'pattern', 'personal', '{}')",
            uuid::Uuid::now_v7(),
            pg.project,
            uuid::Uuid::now_v7()
        ),
    );
    pg.refuses(
        "an unknown decision",
        &format!(
            "INSERT INTO knowledge_candidates
                 (candidate_id, run_id, project_id, proposed_kind, proposed_domain,
                  content, decision)
             VALUES ('{}', '{run}', '{}', 'fact', 'project', 'c', 'probably')",
            uuid::Uuid::now_v7(),
            pg.project
        ),
    );
    pg.refuses(
        "a candidate belonging to no run",
        &format!(
            "INSERT INTO knowledge_candidates
                 (candidate_id, run_id, project_id, proposed_kind, proposed_domain,
                  content, decision)
             VALUES ('{}', '{}', '{}', 'fact', 'project', 'c', 'refused')",
            uuid::Uuid::now_v7(),
            uuid::Uuid::now_v7(),
            pg.project
        ),
    );

    // Provenance: which events this candidate came from.
    let session = pg.session_for(&pg.owner);
    let event = uuid::Uuid::now_v7();
    pg.server.execute(&format!(
        "INSERT INTO safe_events
             (event_id, project_id, session_id, account_id, agent, kind, session_seq,
              contract_version, content, occurred_at)
         VALUES ('{event}', '{}', '{session}', '{}', 'claude-code', 'test_failed', 1,
                 1, '{{}}'::jsonb, now())",
        pg.project, pg.owner.id
    ));
    pg.server.execute(&format!(
        "INSERT INTO candidate_source_events (candidate_id, event_id)
         VALUES ('{candidate}', '{event}')"
    ));
    pg.refuses(
        "provenance naming an event the server never accepted",
        &format!(
            "INSERT INTO candidate_source_events (candidate_id, event_id)
             VALUES ('{candidate}', '{}')",
            uuid::Uuid::now_v7()
        ),
    );
}

#[test]
fn the_disposition_and_health_vocabularies_are_closed() {
    let pg = pg!();
    let row = |disposition: &str| {
        format!(
            "INSERT INTO capture_dispositions
                 (project_id, account_id, agent, kind, disposition, day, n)
             VALUES ('{}', '{}', 'claude-code', 'tool_invoked', '{disposition}',
                     DATE '2026-09-02', 1)",
            pg.project, pg.owner.id
        )
    };
    // The row that says the agent saw success while Cairn dropped the event.
    pg.server.execute(&row("capture_deadline_exceeded"));
    pg.server.execute(&row("spool_saturated_dropped"));
    pg.refuses("an unrecognized disposition", &row("went_fine_probably"));

    let health = |status: &str, evidence: &str| {
        format!(
            "INSERT INTO integration_health
                 (project_id, account_id, writer_id, agent, capability, stage, status,
                  evidence_kind)
             VALUES ('{}', '{}', 'writer-{status}', 'claude-code', 'event:tool_invoked',
                     'capture', '{status}', {evidence})",
            pg.project, pg.owner.id
        )
    };
    // `no_evidence` is a first-class answer, not an absent row: "we have never
    // observed this" and "this works" must not render the same way.
    pg.server.execute(&health("no_evidence", "NULL"));
    pg.server.execute(&health("supported", "'observation'"));
    pg.server
        .execute(&health("adapter_unimplemented", "'introspection'"));
    pg.refuses(
        "an unrecognized health status",
        &health("probably_fine", "NULL"),
    );
    pg.refuses(
        "an unrecognized evidence kind",
        &health("supported", "'vibes'"),
    );
}
