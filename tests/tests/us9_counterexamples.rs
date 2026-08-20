//! T117, T124 — what a pattern's counters can and cannot be talked into
//! (FR-402, FR-403, FR-404, FR-405, SC-313, SC-314).
//!
//! Trust here is the thing most easily gamed by accident: a team that hits one
//! problem ten times, an agent that reads Cairn's own suggestion and agrees
//! with it, a pattern applied where it came from. Each of those looks like
//! confirmation and is not, and each has a test below.

use cairn_core::domain::{
    EvidenceCollector, EvidenceKind, MemoryScope, MemoryType, PatternDiscovery, PatternOutcome,
    VerifierKind, VerifyResult, VerifyTrigger,
};
use cairn_core::wire::codes;
use cairn_store::outbox::SyncPolicy;
use cairn_store::patterns::{self, Candidate, NewApplication, Promotion};
use cairn_store::{evidence, repo, Store, StoreError};
use uuid::Uuid;

const LOCAL: SyncPolicy = SyncPolicy {
    linked: false,
    server_project_id: None,
};

struct Bench {
    store: Store,
    /// The project the pattern was promoted from.
    origin: Uuid,
    pattern: Uuid,
    _dir: tempfile::TempDir,
}

async fn project(store: &Store, name: &str) -> Uuid {
    let id = Uuid::now_v7();
    let now = chrono::Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO projects (id, name, git_common_dir, repository_remote, linked,
                               server_project_id, created_at, updated_at, deleted_at)
         VALUES (?1, ?2, ?3, NULL, 0, NULL, ?4, ?4, NULL)",
    )
    .bind(id.to_string())
    .bind(name)
    .bind(format!("/fixture/{id}/.git"))
    .bind(&now)
    .execute(store.pool())
    .await
    .expect("project");
    id
}

fn signals() -> Vec<String> {
    vec![
        "could not find an available non-overlapping ipv4 address pool".to_string(),
        "docker bridge network create failure".to_string(),
    ]
}

/// A promoted pattern, and the project it came from.
async fn bench() -> Bench {
    cairn_e2e::shared_home();
    let dir = tempfile::tempdir().expect("dir");
    let store = Store::open(&dir.path().join("cairn.sqlite3"))
        .await
        .expect("store");

    let origin = project(&store, "origin-project").await;
    let session = Uuid::now_v7();
    let scope_key = origin.to_string();
    let memory = repo::create_memory(
        &store,
        repo::NewMemory {
            project_id: origin,
            kind: MemoryType::Procedure,
            scope: MemoryScope::Project,
            scope_key: &scope_key,
            content: "Expand the daemon's address pools when bridge creation fails.",
            origin_session_id: session,
            local_only: false,
            evidence: &[],
            topic_key: None,
            value_key: None,
            importance: cairn_core::Importance::Normal,
        },
        LOCAL,
    )
    .await
    .expect("memory");

    let fact = evidence::record(
        &store,
        evidence::NewEvidence {
            project_id: origin,
            kind: EvidenceKind::Configuration,
            collector: EvidenceCollector::Cairn,
            subject: "docker default address pools",
            observed_value: "exhausted",
            source_locator: "docker/daemon.json#default-address-pools",
            fingerprint: "digest:pools",
            observation_id: None,
            repo_branch: "main",
            repo_commit: None,
            collected_by_session: session,
        },
        256,
        256,
    )
    .await
    .expect("fact");
    evidence::attach_to_memory(
        &store,
        memory.id,
        fact.id,
        cairn_core::domain::EvidenceRole::Supports,
        session,
    )
    .await
    .expect("attach");
    evidence::record_run(
        &store,
        evidence::NewRun {
            project_id: origin,
            memory_id: Some(memory.id),
            criterion_id: None,
            verifier: VerifierKind::Configuration,
            evidence_id: Some(fact.id),
            expected_digest: Some("digest:pools"),
            observed_digest: Some("digest:pools"),
            result: VerifyResult::Verified,
            detail: None,
            repo_branch: "main",
            repo_commit: None,
            trigger: VerifyTrigger::OnDemand,
        },
    )
    .await
    .expect("run");
    evidence::rebuild_verification(&store, memory.id)
        .await
        .expect("rebuild");

    let s = signals();
    let promoted = patterns::promote(
        &store,
        memory.id,
        Candidate {
            title: "Docker cannot allocate a non-overlapping bridge network",
            problem: "Container creation fails because no bridge subnet is free.",
            signals: &s,
            applicability: &["Docker bridge networking in use".to_string()],
            root_cause: "The daemon's default address pools are fully allocated.",
            approach: "Expand default-address-pools and restart the daemon.",
            constraints: &["existing networks are not migrated".to_string()],
        },
        2,
        false,
    )
    .await
    .expect("the gate runs");

    let pattern = match promoted {
        Promotion::Promoted(p) => p.id,
        Promotion::Refused { class, message } => {
            panic!("the bench pattern was refused as `{class}`: {message}")
        }
    };

    Bench {
        store,
        origin,
        pattern,
        _dir: dir,
    }
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime")
}

async fn apply(
    b: &Bench,
    project_id: Uuid,
    outcome: PatternOutcome,
    discovery: PatternDiscovery,
    evidence_id: Option<Uuid>,
    alternative_cause: Option<&str>,
) -> Result<cairn_core::domain::PatternTrust, StoreError> {
    let s = signals();
    patterns::record_outcome(
        &b.store,
        NewApplication {
            pattern_id: b.pattern,
            project_id,
            session_id: Uuid::now_v7(),
            signals: &s,
            outcome,
            discovery,
            alternative_cause,
            evidence_id,
        },
    )
    .await
}

/// Ten sessions in one project describing one incident count **once**
/// (FR-402, SC-314).
///
/// This is the anti-poisoning property. Without the unique key, a team that
/// hits one problem repeatedly would walk a pattern up the trust ladder by
/// repetition alone, and repetition is not corroboration — it is the same
/// evidence, counted again.
#[test]
fn one_incident_counts_once() {
    runtime().block_on(async {
        let b = bench().await;
        let applying = project(&b.store, "applying-project").await;

        let first = apply(
            &b,
            applying,
            PatternOutcome::Resolved,
            PatternDiscovery::Independent,
            Some(Uuid::now_v7()),
            None,
        )
        .await
        .expect("the first application is recorded");
        assert_eq!(first.as_str(), "validated");

        // Nine more sessions, same project, same incident.
        for session in 2..=10 {
            let again = apply(
                &b,
                applying,
                PatternOutcome::Resolved,
                PatternDiscovery::Independent,
                Some(Uuid::now_v7()),
                None,
            )
            .await;
            match again {
                Err(StoreError::Refused { code, .. }) => assert_eq!(
                    code,
                    codes::OUTCOME_ALREADY_RECORDED,
                    "session {session} was refused for the wrong reason"
                ),
                Err(e) => panic!("session {session}: {e}"),
                Ok(_) => panic!(
                    "session {session} recorded a second row for one incident, \
                     which is how repetition becomes false corroboration"
                ),
            }
        }

        let counters = patterns::counters(&b.store, b.pattern)
            .await
            .expect("counters");
        assert_eq!(
            counters.applications, 1,
            "ten sessions, one incident, one row"
        );
        assert_eq!(counters.distinct_projects_applied, 1);
        assert_eq!(counters.qualifying_successes, 1);
        assert_eq!(
            patterns::pattern(&b.store, b.pattern)
                .await
                .expect("pattern")
                .trust
                .as_str(),
            "validated",
            "one genuine success in one other project is a validation, and ten \
             retellings of it are still one"
        );
    });
}

/// Cairn's own suggestion, agreed with, is not confirmation (FR-403).
#[test]
fn suggested_does_not_validate() {
    runtime().block_on(async {
        let b = bench().await;
        let reader = project(&b.store, "reading-project").await;

        let trust = apply(
            &b,
            reader,
            PatternOutcome::Resolved,
            PatternDiscovery::CairnSuggested,
            // No deterministic evidence: the agent read the suggestion and
            // agreed with it.
            None,
            None,
        )
        .await
        .expect("recorded");

        assert_eq!(
            trust.as_str(),
            "sanitized",
            "an agent agreeing with Cairn's suggestion is Cairn confirming Cairn"
        );
        let counters = patterns::counters(&b.store, b.pattern)
            .await
            .expect("counters");
        assert_eq!(counters.applications, 1, "it is still an application");
        assert_eq!(counters.qualifying_successes, 0, "and not a validation");
    });
}

/// The same suggestion, confirmed by local evidence, does validate.
///
/// The sibling of the case above, differing in exactly the way that matters.
/// Without it, `suggested_does_not_validate` would also pass against an
/// implementation that ignored `cairn_suggested` applications entirely.
#[test]
fn a_suggestion_confirmed_by_local_evidence_validates() {
    runtime().block_on(async {
        let b = bench().await;
        let reader = project(&b.store, "confirming-project").await;
        let trust = apply(
            &b,
            reader,
            PatternOutcome::Resolved,
            PatternDiscovery::CairnSuggested,
            Some(Uuid::now_v7()),
            None,
        )
        .await
        .expect("recorded");
        assert_eq!(trust.as_str(), "validated");
    });
}

/// A pattern cannot validate itself where it came from (FR-402).
#[test]
fn the_origin_project_does_not_validate_its_own_pattern() {
    runtime().block_on(async {
        let b = bench().await;
        let trust = apply(
            &b,
            b.origin,
            PatternOutcome::Resolved,
            PatternDiscovery::Independent,
            Some(Uuid::now_v7()),
            None,
        )
        .await
        .expect("recorded");

        assert_eq!(trust.as_str(), "sanitized");
        let counters = patterns::counters(&b.store, b.pattern)
            .await
            .expect("counters");
        assert_eq!(counters.applications, 1);
        assert_eq!(
            counters.qualifying_successes, 0,
            "an application at home is history, not evidence"
        );
    });
}

/// A counterexample contests the pattern without deleting it or decreasing
/// anything (FR-404, FR-405, SC-313).
#[test]
fn a_counterexample_contests_without_erasing() {
    runtime().block_on(async {
        let b = bench().await;
        let succeeded = project(&b.store, "it-worked-here").await;
        let differed = project(&b.store, "different-cause-here").await;

        apply(
            &b,
            succeeded,
            PatternOutcome::Resolved,
            PatternDiscovery::Independent,
            Some(Uuid::now_v7()),
            None,
        )
        .await
        .expect("success");
        let trust = apply(
            &b,
            differed,
            PatternOutcome::NotApplicable,
            PatternDiscovery::Independent,
            Some(Uuid::now_v7()),
            Some("A VPN route collision produced the same symptom."),
        )
        .await
        .expect("counterexample");

        assert_eq!(
            trust.as_str(),
            "contested",
            "contested is decided before validated, so both sides are reported"
        );

        let counters = patterns::counters(&b.store, b.pattern)
            .await
            .expect("counters");
        assert_eq!(
            counters.qualifying_successes, 1,
            "the success is not decreased by the counterexample"
        );
        assert_eq!(counters.counterexamples, 1);

        // Retained, not deleted.
        let still_here = patterns::pattern(&b.store, b.pattern).await;
        assert!(
            still_here.is_ok(),
            "a counterexample must not delete a pattern"
        );

        // And the alternative cause travels with future suggestions.
        let causes = patterns::alternative_causes(&b.store, b.pattern)
            .await
            .expect("causes");
        assert_eq!(causes.len(), 1);
        assert!(
            causes[0].contains("VPN route collision"),
            "the alternative cause must be available to the next suggestion: {causes:?}"
        );

        // No surface reports any of this as a number of verifications.
        let rendered = counters.render();
        assert!(
            !rendered.to_lowercase().contains("verif"),
            "counts are never presented as verifications: {rendered}"
        );
    });
}
