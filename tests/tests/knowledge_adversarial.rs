//! Adversarial cases for canonical knowledge and verification (Checkpoint O).
//!
//! Every test here was written as a *counterexample* during the independent
//! review of the finished branch, and every one of them failed. They are kept
//! because the shapes they describe — relation cycles arriving from two
//! machines, a coarse value key beside a third value, a rebuild running over a
//! state no run produced — are exactly the shapes the ordinary suite was built
//! from one side of.
//!
//! What they have in common is that none of them is reachable by one machine
//! acting alone. They are what multi-device convergence produces: two stores
//! that each decided something reasonable, merged.

use cairn_core::domain::{
    EvidenceCollector, EvidenceKind, EvidenceRole, MemoryScope, MemoryState, Reconciliation,
    RelationBasis, RelationKind, VerificationState, VerifierKind, VerifyResult, VerifyTrigger,
};
use cairn_core::knowledge::{
    content_norm_digest, derive_subject, normalize_topic_key, normalize_value_key, MemoryFacts,
    Relation,
};
use cairn_e2e::store_fixture::Fixture;
use cairn_store::evidence::{self, NewEvidence, NewRun};
use cairn_store::knowledge::{
    rebuild_supersession, record_relation, relations_for_project, NewRelation,
};
use uuid::Uuid;

fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

fn keyed(n: u128, topic: &str, value: &str, content: &str) -> MemoryFacts {
    MemoryFacts {
        topic_key: normalize_topic_key(topic),
        value_key: normalize_value_key(value),
        content_norm_digest: Some(content_norm_digest(content)),
        origin_session_id: id(1000 + n),
        ..MemoryFacts::active(id(n), MemoryScope::Project, "p1")
    }
}

/// Every member of a subject is either an answer or somebody's duplicate.
///
/// This is the property a conflicted subject exists to provide: it is the view
/// an agent consults *because* it does not know which answer is right, so a
/// member that appears in neither list has been decided against silently.
fn accounted_for(view: &cairn_core::knowledge::SubjectView, member: Uuid) -> bool {
    view.answers.contains(&member)
        || view
            .accounting
            .iter()
            .any(|a| a.duplicates.contains(&member))
}

// ---------------------------------------------------------------------------
// The derivation (`cairn-core::knowledge::derive_subject`)
// ---------------------------------------------------------------------------

/// A coarse value key beside a third value keeps every statement (FR-334).
///
/// `jwt`/HS256 and `jwt`/RS256 share a value key and say different things. On
/// their own they are `Corroborated` and both are answers. Add a third value and
/// the subject becomes conflicted — and the conflicted branch used to keep one
/// representative per *key*, recording only byte-identical members as its
/// duplicates. RS256 was neither, so it vanished from the one view whose entire
/// job is to show every competing statement.
#[test]
fn a_conflicted_subject_loses_no_statement() {
    let hs = keyed(
        1,
        "auth.strategy",
        "jwt",
        "JWT uses HS256 with a shared secret.",
    );
    let rs = keyed(
        2,
        "auth.strategy",
        "jwt",
        "JWT uses RS256 with rotating keys.",
    );
    let oauth = keyed(3, "auth.strategy", "oauth", "Auth is OAuth2 device flow.");

    let v = derive_subject(&[hs.clone(), rs.clone(), oauth.clone()], &[]);

    assert_eq!(v.reconciliation, Reconciliation::Conflicted);
    for m in [&hs, &rs, &oauth] {
        assert!(
            accounted_for(&v, m.id),
            "memory {} appears neither as an answer nor as a duplicate: answers={:?} \
             accounting={:?}",
            m.id,
            v.answers,
            v.accounting
        );
    }
}

/// A supersession cycle elects nobody (FR-303, SC-302, D78).
///
/// Two machines offline, each deciding the other's proposal was replaced. Every
/// `supersedes` target used to be dropped, so both cancelled out — and the third
/// value, which nobody had argued about at all, was left standing alone as the
/// settled answer. That is a winner no session chose, in the one place Cairn
/// promises never to produce one.
#[test]
fn a_supersession_cycle_produces_no_winner() {
    let a = keyed(1, "infra.db", "postgresql", "PostgreSQL.");
    let b = keyed(2, "infra.db", "cockroachdb", "CockroachDB.");
    let c = keyed(3, "infra.db", "mysql", "MySQL.");

    // Without the cycle: a plain three-way conflict.
    let before = derive_subject(&[a.clone(), b.clone(), c.clone()], &[]);
    assert_eq!(before.reconciliation, Reconciliation::Conflicted);
    assert_eq!(before.answers.len(), 3);

    let cycle = [
        Relation::new(
            RelationKind::Supersedes,
            a.id,
            b.id,
            RelationBasis::ExplicitAgent,
        ),
        Relation::new(
            RelationKind::Supersedes,
            b.id,
            a.id,
            RelationBasis::ExplicitAgent,
        ),
    ];
    let after = derive_subject(&[a.clone(), b.clone(), c.clone()], &cycle);

    assert!(
        !after.is_settled(),
        "a mutual supersession between {} and {} left {} as the settled answer ({:?})",
        a.id,
        b.id,
        c.id,
        after.reconciliation
    );
    assert_eq!(
        after.reconciliation,
        Reconciliation::Conflicted,
        "the cycle is a disagreement about which replaces which, and reads as one"
    );
    for m in [&a, &b, &c] {
        assert!(
            accounted_for(&after, m.id),
            "{} was dropped by the cycle",
            m.id
        );
    }
}

/// A duplicate cycle is not a conflict.
///
/// The same claim recorded on two machines, each having pointed `duplicates` at
/// the other. Dropping both left no member standing, and the subject reported
/// two byte-identical statements as a conflict between them.
#[test]
fn a_duplicate_cycle_collapses_instead_of_conflicting() {
    let a = keyed(
        1,
        "infra.db",
        "postgresql",
        "The production database is PostgreSQL.",
    );
    let b = keyed(
        2,
        "infra.db",
        "postgresql",
        "the production database is postgresql",
    );
    assert_eq!(
        a.content_norm_digest, b.content_norm_digest,
        "identical claims"
    );

    let rels = [
        Relation::new(
            RelationKind::Duplicates,
            b.id,
            a.id,
            RelationBasis::DeterministicRule,
        ),
        Relation::new(
            RelationKind::Duplicates,
            a.id,
            b.id,
            RelationBasis::DeterministicRule,
        ),
    ];
    let v = derive_subject(&[a.clone(), b.clone()], &rels);

    assert_ne!(
        v.reconciliation,
        Reconciliation::Conflicted,
        "two byte-identical claims reported as Conflicted"
    );
    assert_eq!(v.answers.len(), 1, "one claim, one answer: {:?}", v.answers);
    assert!(accounted_for(&v, a.id));
    assert!(accounted_for(&v, b.id));
}

/// A proposal that supersedes itself says nothing.
///
/// The write path accepts the relation, and acting on it deleted the only
/// member the subject had.
#[test]
fn a_self_supersession_is_ignored() {
    let a = keyed(1, "infra.db", "postgresql", "PostgreSQL.");
    let r = Relation::new(
        RelationKind::Supersedes,
        a.id,
        a.id,
        RelationBasis::ExplicitAgent,
    );
    let v = derive_subject(std::slice::from_ref(&a), &[r]);

    assert_eq!(v.reconciliation, Reconciliation::Settled);
    assert_eq!(v.answers, vec![a.id]);
}

/// A chain deeper than one link still settles on its head — the control for the
/// cycle handling, which must not disturb the ordinary case.
#[test]
fn a_four_deep_chain_settles_on_the_head() {
    let a = keyed(1, "infra.db", "v1", "One.");
    let b = keyed(2, "infra.db", "v2", "Two.");
    let c = keyed(3, "infra.db", "v3", "Three.");
    let d = keyed(4, "infra.db", "v4", "Four.");
    let rels = [
        Relation::new(
            RelationKind::Supersedes,
            b.id,
            a.id,
            RelationBasis::ExplicitUser,
        ),
        Relation::new(
            RelationKind::Supersedes,
            c.id,
            b.id,
            RelationBasis::ExplicitUser,
        ),
        Relation::new(
            RelationKind::Supersedes,
            d.id,
            c.id,
            RelationBasis::ExplicitUser,
        ),
    ];
    let v = derive_subject(&[a, b, c, d.clone()], &rels);

    assert_eq!(v.reconciliation, Reconciliation::Settled);
    assert_eq!(v.answers, vec![d.id]);
}

/// A relation naming a member this subject does not have is ignored.
#[test]
fn a_dangling_endpoint_is_ignored() {
    let a = keyed(1, "infra.db", "postgresql", "PostgreSQL.");
    let ghost = id(999);
    let rels = [
        Relation::new(
            RelationKind::Supersedes,
            ghost,
            a.id,
            RelationBasis::ExplicitUser,
        ),
        Relation::new(
            RelationKind::Duplicates,
            a.id,
            ghost,
            RelationBasis::DeterministicRule,
        ),
    ];
    let v = derive_subject(std::slice::from_ref(&a), &rels);

    assert_eq!(v.reconciliation, Reconciliation::Settled);
    assert_eq!(v.answers, vec![a.id]);
}

/// Two successors for one memory are a conflict, not a race.
#[test]
fn two_successors_keep_both_in_the_derivation() {
    let old = keyed(1, "infra.db", "v1", "One.");
    let s1 = keyed(2, "infra.db", "v2", "Two.");
    let s2 = keyed(3, "infra.db", "v3", "Three.");
    let rels = [
        Relation::new(
            RelationKind::Supersedes,
            s1.id,
            old.id,
            RelationBasis::ExplicitUser,
        ),
        Relation::new(
            RelationKind::Supersedes,
            s2.id,
            old.id,
            RelationBasis::ExplicitUser,
        ),
    ];
    let v = derive_subject(&[old, s1.clone(), s2.clone()], &rels);

    assert_eq!(v.reconciliation, Reconciliation::Conflicted);
    assert_eq!(v.answers, vec![s1.id, s2.id]);
}

// ---------------------------------------------------------------------------
// The store, through the paths sync actually takes
// ---------------------------------------------------------------------------

/// An imported supersession cycle leaves both memories alive.
///
/// The derivation alone is not enough here: `rebuild_supersession` writes
/// `state = 'superseded'` into the rows, and a memory that is not active never
/// reaches `derive_subject` at all. Both halves have to know that a cycle
/// decides nothing, or the subject reads as history — the disagreement erased
/// rather than reported.
#[test]
fn an_imported_supersession_cycle_leaves_both_members_active() {
    let (rt, f) = Fixture::blocking();
    rt.block_on(async {
        let a = f
            .propose(
                Uuid::now_v7(),
                Some("infra.db"),
                Some("postgresql"),
                "PostgreSQL.",
            )
            .await;
        let b = f
            .propose(
                Uuid::now_v7(),
                Some("infra.db"),
                Some("cockroachdb"),
                "CockroachDB.",
            )
            .await;

        // Exactly what cairnd's `import_relation` does: record it, then rebuild.
        for (from, to) in [(a.memory.id, b.memory.id), (b.memory.id, a.memory.id)] {
            record_relation(
                &f.store,
                NewRelation {
                    project_id: f.project,
                    from,
                    to,
                    kind: RelationKind::Supersedes,
                    decided_by_session: Uuid::now_v7(),
                    basis: RelationBasis::ExplicitAgent,
                    basis_evidence_id: None,
                    rationale: None,
                },
            )
            .await
            .expect("record");
        }
        let written = relations_for_project(&f.store, f.project)
            .await
            .expect("rels")
            .iter()
            .filter(|r| r.kind == RelationKind::Supersedes)
            .count();
        assert_eq!(
            written, 2,
            "both halves of the cycle are recorded, as they arrive"
        );

        rebuild_supersession(&f.store, f.project)
            .await
            .expect("rebuild");

        let read = f.subject("infra.db").await;
        assert_ne!(
            read.view.reconciliation,
            Reconciliation::Historical,
            "a mutual supersession erased every member of the subject"
        );
        assert_eq!(read.view.reconciliation, Reconciliation::Conflicted);
        assert!(
            read.members.iter().all(|m| m.state == MemoryState::Active),
            "the rebuild moved a live memory to superseded with no author behind the move: {:?}",
            read.members
                .iter()
                .map(|m| (m.id, m.state))
                .collect::<Vec<_>>()
        );
    });
}

/// Two successors of one memory are arbitrated by identifier in the row — the
/// control that shows the cycle handling did not change the ordinary case.
#[test]
fn two_imported_successors_are_arbitrated_by_uuid() {
    let (rt, f) = Fixture::blocking();
    rt.block_on(async {
        let old = f
            .propose(Uuid::now_v7(), Some("infra.db"), Some("v1"), "One.")
            .await;
        let s1 = f
            .propose(Uuid::now_v7(), Some("infra.db"), Some("v2"), "Two.")
            .await;
        let s2 = f
            .propose(Uuid::now_v7(), Some("infra.db"), Some("v3"), "Three.")
            .await;

        for from in [s1.memory.id, s2.memory.id] {
            record_relation(
                &f.store,
                NewRelation {
                    project_id: f.project,
                    from,
                    to: old.memory.id,
                    kind: RelationKind::Supersedes,
                    decided_by_session: Uuid::now_v7(),
                    basis: RelationBasis::ExplicitAgent,
                    basis_evidence_id: None,
                    rationale: None,
                },
            )
            .await
            .expect("record");
        }

        rebuild_supersession(&f.store, f.project)
            .await
            .expect("rebuild");
        let link: Option<String> =
            sqlx::query_scalar("SELECT superseded_by_id FROM memories WHERE id = ?1")
                .bind(old.memory.id.to_string())
                .fetch_one(f.store.pool())
                .await
                .expect("link");

        let lower = s1.memory.id.min(s2.memory.id);
        assert_eq!(
            link.as_deref(),
            Some(lower.to_string().as_str()),
            "expected the lexicographically lowest successor to win"
        );
    });
}

/// The rebuild never resurrects a verification a recheck is owed on (FR-478).
///
/// `verification` is a derived value, and the rebuild recomputes it from the
/// recorded runs. Drift marking used to set `needs_recheck` and record nothing,
/// so the last successful run was still the newest thing the rebuild could see
/// and it restored `verified`. Since `doctor --rebuild-derived` calls this for
/// every memory, running the release-readiness check erased every drift marker
/// in the project.
#[test]
fn the_rebuild_does_not_resurrect_a_drift_marked_verification() {
    let (rt, f) = Fixture::blocking();
    rt.block_on(async {
        let session = Uuid::now_v7();
        let m = f
            .propose(
                session,
                Some("service.api_port"),
                Some("8080"),
                "The API listens on 8080.",
            )
            .await;

        let fact = evidence::record(
            &f.store,
            NewEvidence {
                project_id: f.project,
                kind: EvidenceKind::Configuration,
                collector: EvidenceCollector::Cairn,
                subject: "API port",
                observed_value: "8080",
                source_locator: "config/app.toml",
                fingerprint: "fp-8080",
                observation_id: None,
                repo_branch: "main",
                repo_commit: Some("abc123"),
                collected_by_session: session,
            },
            256,
            256,
        )
        .await
        .expect("evidence");

        evidence::attach_to_memory(
            &f.store,
            m.memory.id,
            fact.id,
            EvidenceRole::Supports,
            session,
        )
        .await
        .expect("attach");
        evidence::record_run(
            &f.store,
            NewRun {
                project_id: f.project,
                memory_id: Some(m.memory.id),
                criterion_id: None,
                verifier: VerifierKind::Configuration,
                evidence_id: Some(fact.id),
                expected_digest: Some("fp-8080"),
                observed_digest: Some("fp-8080"),
                result: VerifyResult::Verified,
                detail: None,
                repo_branch: "main",
                repo_commit: Some("abc123"),
                trigger: VerifyTrigger::OnDemand,
            },
        )
        .await
        .expect("run");

        let established = evidence::rebuild_verification(&f.store, m.memory.id)
            .await
            .expect("rb");
        assert_eq!(established.0, VerificationState::Verified);

        evidence::set_verification(&f.store, m.memory.id, VerificationState::NeedsRecheck)
            .await
            .expect("mark");

        // `doctor --rebuild-derived` runs this over every memory in the project.
        let rebuilt = evidence::rebuild_verification(&f.store, m.memory.id)
            .await
            .expect("rb2");
        assert_eq!(
            rebuilt.0,
            VerificationState::NeedsRecheck,
            "the rebuild restored {:?} (authority {:?}) over a recheck that was owed",
            rebuilt.0,
            rebuilt.1
        );
        assert_eq!(
            rebuilt.1, None,
            "a recheck that is owed carries no authority"
        );
    });
}

/// An imported verification survives the rebuild (T104, FR-478).
///
/// The second head of the same defect. Runs never cross the wire — a peer sends
/// the summary and this machine records `remote_cairn` — so an imported memory
/// has no local run at all. Deriving from local runs anyway rewrote every synced
/// verification in the project to `unverified`, and then reported the damage as
/// a difference the rebuild had found.
#[test]
fn the_rebuild_does_not_erase_an_imported_verification() {
    let (rt, f) = Fixture::blocking();
    rt.block_on(async {
        let m = f
            .propose(
                Uuid::now_v7(),
                Some("infra.db"),
                Some("postgresql"),
                "PostgreSQL.",
            )
            .await;

        // Exactly what cairnd/src/sync.rs writes when a peer's memory arrives
        // carrying a verification summary.
        sqlx::query(
            "UPDATE memories SET verification = 'verified',
                                 verification_authority = 'remote_cairn'
              WHERE id = ?1",
        )
        .bind(m.memory.id.to_string())
        .execute(f.store.pool())
        .await
        .expect("import");

        let rebuilt = evidence::rebuild_verification(&f.store, m.memory.id)
            .await
            .expect("rb");
        assert_eq!(
            rebuilt.0,
            VerificationState::Verified,
            "the rebuild erased a verification it never derived"
        );

        let row: (String, Option<String>) = sqlx::query_as(
            "SELECT verification, verification_authority FROM memories WHERE id = ?1",
        )
        .bind(m.memory.id.to_string())
        .fetch_one(f.store.pool())
        .await
        .expect("row");
        assert_eq!(row.0, "verified");
        assert_eq!(row.1.as_deref(), Some("remote_cairn"));
    });
}

/// `verified` is never persisted without an authority (FR-370).
///
/// `derive_authority` already refused to invent one — "a successful run that
/// consulted no evidence establishes nothing" — but the state was written
/// beside the `None` anyway, leaving a claim on the row with nothing behind it.
#[test]
fn a_run_with_no_evidence_does_not_verify() {
    let (rt, f) = Fixture::blocking();
    rt.block_on(async {
        let session = Uuid::now_v7();
        let m = f
            .propose(session, Some("infra.db"), Some("postgresql"), "PostgreSQL.")
            .await;

        evidence::record_run(
            &f.store,
            NewRun {
                project_id: f.project,
                memory_id: Some(m.memory.id),
                criterion_id: None,
                verifier: VerifierKind::CommandOutcome,
                evidence_id: None,
                expected_digest: None,
                observed_digest: None,
                result: VerifyResult::Verified,
                detail: None,
                repo_branch: "main",
                repo_commit: None,
                trigger: VerifyTrigger::OnDemand,
            },
        )
        .await
        .expect("run");

        let out = evidence::rebuild_verification(&f.store, m.memory.id)
            .await
            .expect("rb");
        assert_ne!(out.0, VerificationState::Verified);
        assert_eq!(out.1, None);

        let row: (String, Option<String>) = sqlx::query_as(
            "SELECT verification, verification_authority FROM memories WHERE id = ?1",
        )
        .bind(m.memory.id.to_string())
        .fetch_one(f.store.pool())
        .await
        .expect("row");
        assert!(
            !(row.0 == "verified" && row.1.is_none()),
            "`verified` with no authority persisted: {row:?}"
        );
    });
}

/// The documented coarse-value-key case, unchanged — the control for the
/// conflicted-branch fix.
#[test]
fn a_two_member_coarse_value_key_is_still_corroborated() {
    let (rt, f) = Fixture::blocking();
    rt.block_on(async {
        f.propose(
            Uuid::now_v7(),
            Some("auth.strategy"),
            Some("jwt"),
            "JWT uses HS256.",
        )
        .await;
        f.propose(
            Uuid::now_v7(),
            Some("auth.strategy"),
            Some("jwt"),
            "JWT uses RS256.",
        )
        .await;

        let read = f.subject("auth.strategy").await;
        assert_eq!(read.view.reconciliation, Reconciliation::Corroborated);
        assert_eq!(read.view.answers.len(), 2);
        assert!(read.members.iter().all(|m| m.state == MemoryState::Active));
    });
}
