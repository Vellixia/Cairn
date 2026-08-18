//! Drift marking — the cheap half of drift (`contracts/evidence-verification.md`
//! §Drift).
//!
//! Two mechanisms, deliberately separate (D54):
//!
//! - **marking**, here, on the capture path: an indexed lookup by exact locator,
//!   capped per event, that sets supported memories to `needs_recheck` and
//!   **nothing else**;
//! - **verifying**, in `verify.rs`, on the existing maintenance tick: bounded,
//!   and the only thing that can move a claim to `verified` or `drifted`.
//!
//! The separation is what keeps a hook inside its 250 ms deadline. Marking is
//! one indexed read and one narrow update; deciding whether the claim still
//! holds is somebody else's problem, later.
//!
//! # What marking must never do
//!
//! ```text
//! evidence changed ──✗──▶ rewrite the memory
//! evidence changed ──✗──▶ create a superseding memory
//! evidence changed ──✗──▶ mark the memory stale or superseded
//! evidence changed ──✓──▶ verification = needs_recheck
//! ```
//!
//! A single changed file must never be able to corrupt a knowledge base
//! (FR-371, FR-372, I6).

use crate::state::Daemon;
use cairn_core::domain::{VerificationState, VerifierKind, VerifyResult, VerifyTrigger};
use cairn_store::evidence;
use sqlx::Row;
use std::str::FromStr;
use uuid::Uuid;

/// What one marking pass did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MarkReport {
    /// Evidence facts looked at. Never more than
    /// `evidence_lookups_per_event_max`.
    pub facts_examined: usize,
    /// Memories moved to `needs_recheck`.
    pub marked: usize,
    /// True when the cap bound. The remaining work defers to the background
    /// pass, and it is **not** an error (FR-374).
    pub deferred: bool,
}

/// Mark the memories a changed path supports as owing a recheck.
///
/// Exact locator equality only — no globbing, no prefix scan. The cap is what
/// keeps this inside Feature 001's capture deadline, and exceeding it defers
/// rather than continuing.
///
/// Failure here is silent by design: capture is fail-soft and always exits 0,
/// so a marking pass that cannot read the store drops its work rather than
/// delaying the agent (FR-475, FR-476).
pub async fn mark_for_path(d: &Daemon, project_id: Uuid, path: &str) -> MarkReport {
    let cap = d.config.read().await.evidence_lookups_per_event_max;
    let mut report = MarkReport::default();

    // One more than the cap, so hitting the bound is distinguishable from
    // exactly filling it.
    let facts = match evidence::facts_by_locator(&d.store, project_id, path, cap + 1).await {
        Ok(f) => f,
        Err(e) => {
            tracing::debug!(error = %e, "drift marking could not read evidence");
            return report;
        }
    };
    if facts.len() > cap {
        report.deferred = true;
    }

    for fact in facts.into_iter().take(cap) {
        report.facts_examined += 1;
        report.marked += mark_supported(
            d,
            &MarkTarget {
                fact_id: fact.id,
                project_id,
                repo_branch: fact.repo_branch,
            },
        )
        .await;
    }
    report
}

/// The same, for a branch or commit change: every commit-pinned fact on that
/// branch is owed a recheck.
///
/// A rebase or a commit change does not invalidate branch knowledge by itself;
/// it marks the evidence pinned to a commit for rechecking (FR-384).
pub async fn mark_for_commit_change(
    d: &Daemon,
    project_id: Uuid,
    branch: &str,
    current_commit: Option<&str>,
) -> MarkReport {
    let cap = d.config.read().await.evidence_lookups_per_event_max;
    let mut report = MarkReport::default();

    // Only the facts pinned to a commit that is no longer current. A fact
    // recorded at the head Cairn is looking at has not moved.
    let ids: Vec<String> = match sqlx::query_scalar(
        "SELECT id FROM evidence_facts
          WHERE project_id = ?1 AND repo_branch = ?2 AND repo_commit IS NOT NULL
            AND deleted_at IS NULL
            AND (?4 IS NULL OR repo_commit <> ?4)
          ORDER BY id
          LIMIT ?3",
    )
    .bind(project_id.to_string())
    .bind(branch)
    .bind(cap as i64 + 1)
    .bind(current_commit)
    .fetch_all(d.store.pool())
    .await
    {
        Ok(ids) => ids,
        Err(e) => {
            tracing::debug!(error = %e, "drift marking could not read commit-pinned evidence");
            return report;
        }
    };
    if ids.len() > cap {
        report.deferred = true;
    }

    for id in ids.into_iter().take(cap) {
        let Ok(fact_id) = Uuid::parse_str(&id) else {
            continue;
        };
        report.facts_examined += 1;
        report.marked += mark_supported(
            d,
            &MarkTarget {
                fact_id,
                project_id,
                // The fact is pinned to this branch: that is how it was
                // selected.
                repo_branch: branch.to_string(),
            },
        )
        .await;
    }
    report
}

/// What one marking pass needs about the fact that moved.
struct MarkTarget {
    fact_id: Uuid,
    project_id: Uuid,
    repo_branch: String,
}

/// Set every memory this fact **supports** to `needs_recheck`, and record why.
///
/// Writes exactly `verification`, plus one appended verification run. Never
/// content, type, scope, provenance or lifecycle state — and never creates a
/// memory (FR-371, I6).
///
/// # Why marking appends a run
///
/// `verification` is a **derived** value: rebuildable from durable records, and
/// on disagreement the records win (D43, FR-478). A state set with no record
/// behind it is not derived, it is asserted — and `rebuild_verification`, which
/// reads only the runs, would restore `verified` over it and call that
/// consistency. `doctor --rebuild-derived` does exactly that for every memory,
/// so an unrecorded marker meant the release-readiness check erased every drift
/// marker in the project.
///
/// The appended run is `inconclusive`, and that is the literal truth: the claim
/// was reopened because the evidence under it moved, and **no verifier ran**, so
/// neither outcome was established (FR-366). Deciding whether the claim still
/// holds remains `verify.rs`'s job — an `inconclusive` run cannot reach
/// `verified` or `drifted`, which is the separation this module exists to keep.
/// Recording it makes `needs_recheck` fall out of the runs on its own.
///
/// A memory that is already `unverified` is left alone: there is nothing to
/// recheck, and moving it would claim a verification it never had. One that is
/// already `drifted` is left alone too — "recheck owed" is weaker than "known
/// to have drifted", and the background pass re-checks `drifted` anyway.
async fn mark_supported(d: &Daemon, target: &MarkTarget) -> usize {
    // The memory, and the verifier that last had anything to say about it. One
    // indexed read: `verification_runs_memory` is `(memory_id, checked_at DESC)`.
    let rows = match sqlx::query(
        "SELECT l.memory_id AS memory_id,
                (SELECT r.verifier FROM verification_runs r
                  WHERE r.memory_id = l.memory_id
                  ORDER BY r.checked_at DESC, r.id DESC
                  LIMIT 1) AS verifier
           FROM memory_evidence_facts l
           JOIN memories m ON m.id = l.memory_id
          WHERE l.evidence_id = ?1 AND l.role = 'supports'
            AND m.deleted_at IS NULL
            AND m.verification = 'verified'",
    )
    .bind(target.fact_id.to_string())
    .fetch_all(d.store.pool())
    .await
    {
        Ok(m) => m,
        Err(_) => return 0,
    };

    let mut marked = 0;
    for row in &rows {
        let Ok(id) = row.try_get::<String, _>("memory_id") else {
            continue;
        };
        let Ok(memory_id) = Uuid::parse_str(&id) else {
            continue;
        };

        // The verifier that established the claim is the one now owed again.
        // Without one there is no run to describe, and nothing to record.
        let verifier = row
            .try_get::<Option<String>, _>("verifier")
            .ok()
            .flatten()
            .and_then(|v| VerifierKind::from_str(&v).ok());
        let Some(verifier) = verifier else { continue };

        if evidence::record_run(
            &d.store,
            evidence::NewRun {
                project_id: target.project_id,
                memory_id: Some(memory_id),
                criterion_id: None,
                verifier,
                evidence_id: Some(target.fact_id),
                expected_digest: None,
                observed_digest: None,
                result: VerifyResult::Inconclusive,
                detail: Some("the evidence under this claim changed; no verifier ran"),
                repo_branch: &target.repo_branch,
                repo_commit: None,
                trigger: VerifyTrigger::Attach,
            },
        )
        .await
        .is_err()
        {
            continue;
        }

        if evidence::set_verification(&d.store, memory_id, VerificationState::NeedsRecheck)
            .await
            .is_ok()
        {
            marked += 1;
        }
    }
    marked
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_core::domain::{
        EvidenceCollector, EvidenceKind, EvidenceRole, VerifierKind, VerifyResult, VerifyTrigger,
    };
    use cairn_store::evidence::{NewEvidence, NewRun};

    /// Everything a drift test needs: a daemon over a real store, a memory, and
    /// a verified claim backed by a fact at a known locator.
    async fn verified_claim(
        d: &Daemon,
        project: Uuid,
        session: Uuid,
        locator: &str,
    ) -> (Uuid, Uuid) {
        let key = project.to_string();
        let m = cairn_store::repo::create_memory_reconciled(
            &d.store,
            cairn_store::repo::NewMemory {
                project_id: project,
                kind: cairn_core::MemoryType::Fact,
                scope: cairn_core::MemoryScope::Project,
                scope_key: &key,
                content: "The API listens on port 8080.",
                origin_session_id: session,
                local_only: false,
                evidence: &[],
                topic_key: Some("service.api_port"),
                value_key: Some("8080"),
                importance: cairn_core::Importance::Normal,
            },
            cairn_store::outbox::SyncPolicy {
                linked: false,
                server_project_id: None,
            },
            64,
        )
        .await
        .expect("memory");

        let fact = evidence::record(
            &d.store,
            NewEvidence {
                project_id: project,
                kind: EvidenceKind::Configuration,
                collector: EvidenceCollector::Cairn,
                subject: "API port",
                observed_value: "8080",
                source_locator: locator,
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
            &d.store,
            m.memory.id,
            fact.id,
            EvidenceRole::Supports,
            session,
        )
        .await
        .expect("attach");
        evidence::record_run(
            &d.store,
            NewRun {
                project_id: project,
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
        evidence::rebuild_verification(&d.store, m.memory.id)
            .await
            .expect("rebuild");

        (m.memory.id, fact.id)
    }

    #[tokio::test]
    async fn marking_moves_verification_and_nothing_else() {
        let fx = crate::testsupport::daemon().await;
        let p = crate::testsupport::project(&fx, "drift", None).await;
        let project = p.id;
        let session = crate::testsupport::session(&fx, &p, "drift-1").await.id;
        let (memory, _) = verified_claim(&fx, project, session, "config/app.yml").await;

        let before: (String, String, String, String, String, Option<String>) = sqlx::query_as(
            "SELECT content, type, scope, scope_key, state, superseded_by_id
               FROM memories WHERE id = ?1",
        )
        .bind(memory.to_string())
        .fetch_one(fx.store.pool())
        .await
        .expect("before");

        let report = mark_for_path(&fx, project, "config/app.yml").await;
        assert_eq!(report.marked, 1);
        assert!(!report.deferred);

        let after: (String, String, String, String, String, Option<String>) = sqlx::query_as(
            "SELECT content, type, scope, scope_key, state, superseded_by_id
               FROM memories WHERE id = ?1",
        )
        .bind(memory.to_string())
        .fetch_one(fx.store.pool())
        .await
        .expect("after");
        assert_eq!(
            before, after,
            "marking changed something other than verification"
        );

        let verification: String =
            sqlx::query_scalar("SELECT verification FROM memories WHERE id = ?1")
                .bind(memory.to_string())
                .fetch_one(fx.store.pool())
                .await
                .expect("verification");
        assert_eq!(verification, "needs_recheck");

        // And no memory was created.
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM memories")
            .fetch_one(fx.store.pool())
            .await
            .expect("count");
        assert_eq!(count, 1, "marking created a memory");
    }

    /// A drift marker is a *derived* state after marking, not an assertion
    /// (D43, FR-478).
    ///
    /// Marking used to set `needs_recheck` and record nothing, so the last
    /// successful run stayed the newest thing `rebuild_verification` could see
    /// and it put `verified` straight back. `doctor --rebuild-derived` calls
    /// that for every memory, which made the release-readiness check erase every
    /// drift marker in the project.
    #[tokio::test]
    async fn a_marked_memory_survives_the_rebuild() {
        let fx = crate::testsupport::daemon().await;
        let p = crate::testsupport::project(&fx, "drift", None).await;
        let project = p.id;
        let session = crate::testsupport::session(&fx, &p, "drift-1").await.id;
        let (memory, fact) = verified_claim(&fx, project, session, "config/app.yml").await;

        assert_eq!(
            mark_for_path(&fx, project, "config/app.yml").await.marked,
            1
        );

        // The marking is on the record, as an `inconclusive` run against the
        // fact that moved: no verifier ran, so neither outcome was established.
        let runs = evidence::runs_for_memory(&fx.store, memory)
            .await
            .expect("runs");
        let latest = runs.first().expect("a run");
        assert_eq!(latest.result, VerifyResult::Inconclusive);
        assert_eq!(latest.evidence_id, Some(fact));

        // Which is why the rebuild agrees rather than overruling.
        let (state, authority) = evidence::rebuild_verification(&fx.store, memory)
            .await
            .expect("rebuild");
        assert_eq!(
            state,
            VerificationState::NeedsRecheck,
            "the rebuild restored {state:?} over a recheck that was owed"
        );
        assert_eq!(authority, None);
    }

    #[tokio::test]
    async fn an_unverified_memory_is_left_alone() {
        // There is nothing to recheck, and moving it would claim a verification
        // it never had.
        let fx = crate::testsupport::daemon().await;
        let p = crate::testsupport::project(&fx, "drift", None).await;
        let project = p.id;
        let session = crate::testsupport::session(&fx, &p, "drift-1").await.id;
        let (memory, _) = verified_claim(&fx, project, session, "config/app.yml").await;
        sqlx::query(
            "UPDATE memories SET verification = 'unverified', verification_authority = NULL
             WHERE id = ?1",
        )
        .bind(memory.to_string())
        .execute(fx.store.pool())
        .await
        .expect("reset");

        let report = mark_for_path(&fx, project, "config/app.yml").await;
        assert_eq!(report.facts_examined, 1, "the fact was still looked at");
        assert_eq!(report.marked, 0);
    }

    #[tokio::test]
    async fn the_lookup_is_exact_and_capped() {
        let fx = crate::testsupport::daemon().await;
        let p = crate::testsupport::project(&fx, "drift", None).await;
        let project = p.id;
        let session = crate::testsupport::session(&fx, &p, "drift-1").await.id;

        for _ in 0..12 {
            verified_claim(&fx, project, session, "config/app.yml").await;
        }

        // A path that merely shares a prefix matches nothing.
        let none = mark_for_path(&fx, project, "config/").await;
        assert_eq!(none.facts_examined, 0);

        // The default cap is 8, so twelve facts defer rather than scanning.
        let report = mark_for_path(&fx, project, "config/app.yml").await;
        assert_eq!(report.facts_examined, 8, "the cap did not bind");
        assert!(
            report.deferred,
            "exceeding the cap must defer, not continue"
        );
    }

    #[tokio::test]
    async fn a_commit_change_marks_commit_pinned_evidence() {
        // FR-384: a rebase does not invalidate branch knowledge by itself; it
        // marks commit-pinned evidence for rechecking.
        let fx = crate::testsupport::daemon().await;
        let p = crate::testsupport::project(&fx, "drift", None).await;
        let project = p.id;
        let session = crate::testsupport::session(&fx, &p, "drift-1").await.id;
        let (memory, _) = verified_claim(&fx, project, session, "config/app.yml").await;

        // The head moved away from the commit the fact was recorded at.
        let report = mark_for_commit_change(&fx, project, "main", Some("def456")).await;
        assert_eq!(report.marked, 1);

        let verification: String =
            sqlx::query_scalar("SELECT verification FROM memories WHERE id = ?1")
                .bind(memory.to_string())
                .fetch_one(fx.store.pool())
                .await
                .expect("verification");
        assert_eq!(verification, "needs_recheck");

        // A different branch's evidence is untouched.
        let other = mark_for_commit_change(&fx, project, "feature/x", Some("def456")).await;
        assert_eq!(other.marked, 0);

        // And a head that has not moved marks nothing: a fact recorded at the
        // commit Cairn is looking at has not drifted.
        let unmoved = mark_for_commit_change(&fx, project, "main", Some("abc123")).await;
        assert_eq!(unmoved.facts_examined, 0);
    }

    #[tokio::test]
    async fn a_path_with_no_evidence_does_nothing_and_is_not_an_error() {
        let fx = crate::testsupport::daemon().await;
        let project = crate::testsupport::project(&fx, "drift", None).await.id;
        let report = mark_for_path(&fx, project, "src/nothing.rs").await;
        assert_eq!(report, MarkReport::default());
    }
}
