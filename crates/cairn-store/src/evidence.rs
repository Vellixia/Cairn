//! Evidence facts, their links, and the append-only verification runs
//! (`contracts/evidence-verification.md`).
//!
//! Everything here is **local**. There is no outbox entity type and no server
//! table for any of it, which is what makes "evidence content never leaves the
//! machine" a property of the schema rather than a promise (FR-502, I8). What a
//! shared memory may say about evidence is a five-key object carrying verifier
//! kinds and one authority enum — no content.
//!
//! # The bound that matters
//!
//! `observed_value` is bounded **after** redaction, never before (FR-354).
//! Bounding first would truncate a credential into something redaction no
//! longer recognizes, and store the fragment.

use crate::{constraints, rows, tx, Result, Store, StoreError};
use cairn_core::domain::{
    EvidenceCollector, EvidenceKind, EvidenceRole, VerificationAuthority, VerificationState,
    VerifierKind, VerifyResult, VerifyTrigger,
};
use cairn_core::verify::{derive_authority, RunFacts};
use sqlx::Row;
use std::str::FromStr;
use uuid::Uuid;

/// A bounded, redacted, attributable record of an observed state of the world.
#[derive(Debug, Clone)]
pub struct NewEvidence<'a> {
    pub project_id: Uuid,
    pub kind: EvidenceKind,
    pub collector: EvidenceCollector,
    /// A label — "database backend", "API port". Bounded and redacted.
    pub subject: &'a str,
    pub observed_value: &'a str,
    /// Repository-relative, or a Git ref name. **Never absolute** (FR-353).
    pub source_locator: &'a str,
    /// What change detection compares. Built by `cairn_core::verify::fingerprint`.
    pub fingerprint: &'a str,
    /// The captured observation this came from, where one exists. The bridge to
    /// Feature 001 provenance.
    pub observation_id: Option<Uuid>,
    pub repo_branch: &'a str,
    pub repo_commit: Option<&'a str>,
    pub collected_by_session: Uuid,
}

/// One stored evidence fact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceFact {
    pub id: Uuid,
    pub project_id: Uuid,
    pub kind: EvidenceKind,
    pub collector: EvidenceCollector,
    pub subject: String,
    /// `None` once the fact is tombstoned: the reference stays resolvable and
    /// reports "evidence deleted" rather than disappearing (FR-358, FR-505).
    pub observed_value: Option<String>,
    pub value_digest: Option<String>,
    pub source_locator: Option<String>,
    pub fingerprint: Option<String>,
    pub observation_id: Option<Uuid>,
    pub repo_branch: String,
    pub repo_commit: Option<String>,
    pub collected_by_session: Uuid,
    pub deleted: bool,
}

impl EvidenceFact {
    /// Whether this fact can establish a claim by a deterministic check Cairn
    /// performed.
    pub fn is_cairn_collected(&self) -> bool {
        self.collector == EvidenceCollector::Cairn
    }
}

/// Why a locator was refused. Each maps to a stable wire code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocatorRefusal {
    Absolute,
    OutsideWorktree,
}

impl LocatorRefusal {
    pub fn code(&self) -> &'static str {
        match self {
            LocatorRefusal::Absolute => cairn_core::wire::codes::ABSOLUTE_LOCATOR,
            LocatorRefusal::OutsideWorktree => cairn_core::wire::codes::EVIDENCE_OUTSIDE_WORKTREE,
        }
    }
}

/// Validate a source locator (FR-353, I9).
///
/// Refused when it begins with `/` or `\`, matches a Windows drive prefix, is a
/// UNC path, or escapes the worktree through `..` after normalization. A stored
/// locator that named an absolute path would carry a machine's directory layout
/// into a column a future feature might widen.
pub fn validate_locator(locator: &str) -> std::result::Result<(), LocatorRefusal> {
    let l = locator.trim();
    if l.starts_with('/') || l.starts_with('\\') {
        return Err(LocatorRefusal::Absolute);
    }
    let bytes = l.as_bytes();
    if bytes.len() > 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        return Err(LocatorRefusal::Absolute);
    }
    if l.starts_with("\\\\") || l.starts_with("//") {
        return Err(LocatorRefusal::Absolute);
    }

    // `..` is checked by walking, not by substring: `src/library/..x` contains
    // the characters without being a traversal.
    let mut depth = 0i32;
    for segment in l.split(['/', '\\']) {
        match segment {
            "" | "." => {}
            ".." => {
                depth -= 1;
                if depth < 0 {
                    return Err(LocatorRefusal::OutsideWorktree);
                }
            }
            _ => depth += 1,
        }
    }
    Ok(())
}

/// Record an evidence fact.
///
/// The value and the locator pass through Feature 001's redaction pipeline and
/// are then bounded — in that order. Cairn never stores raw secret-bearing
/// configuration in order to support a memory; it stores the safe fact, its
/// digest and its locator (FR-354).
pub async fn record(
    store: &Store,
    e: NewEvidence<'_>,
    value_max_bytes: usize,
    locator_max_bytes: usize,
) -> Result<EvidenceFact> {
    validate_locator(e.source_locator)
        .map_err(|r| StoreError::Corrupt(format!("{}: {}", r.code(), e.source_locator)))?;

    // Redact first, bound second. Bounding first would cut a credential into a
    // fragment redaction no longer recognizes — and then store the fragment.
    let subject = bound(&cairn_core::redact::redact(e.subject), 128);
    let observed_value = bound(
        &cairn_core::redact::redact(e.observed_value),
        value_max_bytes,
    );
    let source_locator = bound(
        &cairn_core::redact::redact(e.source_locator),
        locator_max_bytes,
    );
    let value_digest = cairn_core::digest(&observed_value);

    let id = cairn_core::domain::new_id();
    let now = rows::now_text();
    let mut t = tx::begin(store, "record_evidence").await?;
    sqlx::query(
        "INSERT INTO evidence_facts
            (id, project_id, kind, collector, subject, observed_value, value_digest,
             source_locator, fingerprint, observation_id, repo_branch, repo_commit,
             collected_at, collected_by_session, local_only)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, 1)",
    )
    .bind(id.to_string())
    .bind(e.project_id.to_string())
    .bind(e.kind.as_str())
    .bind(e.collector.as_str())
    .bind(&subject)
    .bind(&observed_value)
    .bind(&value_digest)
    .bind(&source_locator)
    .bind(e.fingerprint)
    .bind(e.observation_id.map(|o| o.to_string()))
    .bind(e.repo_branch)
    .bind(e.repo_commit)
    .bind(&now)
    .bind(e.collected_by_session.to_string())
    .execute(&mut *t)
    .await?;
    tx::commit(t, "record_evidence").await?;

    fact(store, id).await
}

/// Truncate on a character boundary, so a multi-byte value never becomes
/// invalid UTF-8 at the bound.
fn bound(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.to_string();
    }
    let mut end = max_bytes;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_string()
}

fn fact_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<EvidenceFact> {
    let corrupt = |e: cairn_core::domain::ParseEnumError| StoreError::Corrupt(e.to_string());
    Ok(EvidenceFact {
        id: rows::uuid(row, "id")?,
        project_id: rows::uuid(row, "project_id")?,
        kind: EvidenceKind::from_str(row.try_get::<String, _>("kind")?.as_str())
            .map_err(corrupt)?,
        collector: EvidenceCollector::from_str(row.try_get::<String, _>("collector")?.as_str())
            .map_err(corrupt)?,
        subject: row.try_get("subject")?,
        observed_value: row.try_get("observed_value")?,
        value_digest: row.try_get("value_digest")?,
        source_locator: row.try_get("source_locator")?,
        fingerprint: row.try_get("fingerprint")?,
        observation_id: row
            .try_get::<Option<String>, _>("observation_id")?
            .and_then(|s| Uuid::parse_str(&s).ok()),
        repo_branch: row.try_get("repo_branch")?,
        repo_commit: row.try_get("repo_commit")?,
        collected_by_session: rows::uuid(row, "collected_by_session")?,
        deleted: row.try_get::<Option<String>, _>("deleted_at")?.is_some(),
    })
}

pub async fn fact(store: &Store, id: Uuid) -> Result<EvidenceFact> {
    let row = sqlx::query("SELECT * FROM evidence_facts WHERE id = ?1")
        .bind(id.to_string())
        .fetch_optional(store.pool())
        .await?
        .ok_or_else(|| StoreError::NotFound(format!("evidence fact {id}")))?;
    fact_from_row(&row)
}

pub async fn facts_for_project(store: &Store, project_id: Uuid) -> Result<Vec<EvidenceFact>> {
    let rows =
        sqlx::query("SELECT * FROM evidence_facts WHERE project_id = ?1 ORDER BY collected_at, id")
            .bind(project_id.to_string())
            .fetch_all(store.pool())
            .await?;
    rows.iter().map(fact_from_row).collect()
}

/// The drift-marking lookup: exact locator equality, capped by the caller.
///
/// No globbing and no prefix scan — the cap is what keeps this inside the
/// capture deadline (FR-374, D54).
pub async fn facts_by_locator(
    store: &Store,
    project_id: Uuid,
    locator: &str,
    cap: usize,
) -> Result<Vec<EvidenceFact>> {
    let rows = sqlx::query(
        "SELECT * FROM evidence_facts
          WHERE project_id = ?1 AND source_locator = ?2 AND deleted_at IS NULL
          ORDER BY id
          LIMIT ?3",
    )
    .bind(project_id.to_string())
    .bind(locator)
    .bind(cap as i64)
    .fetch_all(store.pool())
    .await?;
    rows.iter().map(fact_from_row).collect()
}

/// Tombstone a fact: identity, kind, timestamps and provenance survive; the
/// value, digest, locator and fingerprint are cleared.
///
/// The link row survives too, so the reference resolves to **evidence deleted**
/// rather than disappearing — Feature 001's semantics for observation evidence,
/// extended (FR-358, FR-505).
pub async fn forget(store: &Store, id: Uuid) -> Result<()> {
    let mut t = tx::begin(store, "forget_evidence").await?;
    sqlx::query(
        "UPDATE evidence_facts
            SET observed_value = NULL, value_digest = NULL, source_locator = NULL,
                fingerprint = NULL, deleted_at = ?2
          WHERE id = ?1",
    )
    .bind(id.to_string())
    .bind(rows::now_text())
    .execute(&mut *t)
    .await?;
    tx::commit(t, "forget_evidence").await?;
    Ok(())
}

/// Attach a fact to a memory with a role (FR-359).
pub async fn attach_to_memory(
    store: &Store,
    memory_id: Uuid,
    evidence_id: Uuid,
    role: EvidenceRole,
    session: Uuid,
) -> Result<bool> {
    let mut t = tx::begin(store, "attach_evidence").await?;
    let out = sqlx::query(
        "INSERT OR IGNORE INTO memory_evidence_facts
            (memory_id, evidence_id, role, attached_at, attached_by_session)
         VALUES (?1, ?2, ?3, ?4, ?5)",
    )
    .bind(memory_id.to_string())
    .bind(evidence_id.to_string())
    .bind(role.as_str())
    .bind(rows::now_text())
    .bind(session.to_string())
    .execute(&mut *t)
    .await?;
    tx::commit(t, "attach_evidence").await?;
    Ok(out.rows_affected() > 0)
}

/// Attach a fact to a task criterion.
pub async fn attach_to_criterion(
    store: &Store,
    criterion_id: Uuid,
    evidence_id: Uuid,
    session: Uuid,
) -> Result<bool> {
    let mut t = tx::begin(store, "attach_criterion_evidence").await?;
    let out = sqlx::query(
        "INSERT OR IGNORE INTO criterion_evidence
            (criterion_id, evidence_id, attached_at, attached_by_session)
         VALUES (?1, ?2, ?3, ?4)",
    )
    .bind(criterion_id.to_string())
    .bind(evidence_id.to_string())
    .bind(rows::now_text())
    .bind(session.to_string())
    .execute(&mut *t)
    .await?;
    tx::commit(t, "attach_criterion_evidence").await?;
    Ok(out.rows_affected() > 0)
}

/// Every fact linked to a memory, with its role and whether it is deleted.
pub async fn facts_for_memory(
    store: &Store,
    memory_id: Uuid,
) -> Result<Vec<(EvidenceRole, EvidenceFact)>> {
    let rows = sqlx::query(
        "SELECT l.role, f.* FROM memory_evidence_facts l
           JOIN evidence_facts f ON f.id = l.evidence_id
          WHERE l.memory_id = ?1
          ORDER BY l.role, f.id",
    )
    .bind(memory_id.to_string())
    .fetch_all(store.pool())
    .await?;

    rows.iter()
        .map(|r| {
            let role = EvidenceRole::from_str(r.try_get::<String, _>("role")?.as_str())
                .map_err(|e| StoreError::Corrupt(e.to_string()))?;
            Ok((role, fact_from_row(r)?))
        })
        .collect()
}

/// Every fact linked to a criterion.
pub async fn facts_for_criterion(store: &Store, criterion_id: Uuid) -> Result<Vec<EvidenceFact>> {
    let rows = sqlx::query(
        "SELECT f.* FROM criterion_evidence l
           JOIN evidence_facts f ON f.id = l.evidence_id
          WHERE l.criterion_id = ?1
          ORDER BY f.id",
    )
    .bind(criterion_id.to_string())
    .fetch_all(store.pool())
    .await?;
    rows.iter().map(fact_from_row).collect()
}

// ---------------------------------------------------------------------------
// Verification runs — append-only (FR-364)
// ---------------------------------------------------------------------------

/// One deterministic check, as it is recorded.
#[derive(Debug, Clone)]
pub struct NewRun<'a> {
    pub project_id: Uuid,
    pub memory_id: Option<Uuid>,
    pub criterion_id: Option<Uuid>,
    pub verifier: VerifierKind,
    pub evidence_id: Option<Uuid>,
    pub expected_digest: Option<&'a str>,
    pub observed_digest: Option<&'a str>,
    pub result: VerifyResult,
    /// Bounded and redacted: why inconclusive, or what differed.
    pub detail: Option<&'a str>,
    pub repo_branch: &'a str,
    pub repo_commit: Option<&'a str>,
    pub trigger: VerifyTrigger,
}

/// Append a verification run.
///
/// A later run never rewrites an earlier one; only the memory's or criterion's
/// cached state changes. That is what lets `cairn verify --explain` print the
/// history rather than the last answer.
pub async fn record_run(store: &Store, r: NewRun<'_>) -> Result<Uuid> {
    let id = cairn_core::domain::new_id();
    let mut t = tx::begin(store, "record_run").await?;
    sqlx::query(
        "INSERT INTO verification_runs
            (id, memory_id, criterion_id, project_id, verifier, evidence_id,
             expected_digest, observed_digest, result, detail, repo_branch, repo_commit,
             checked_at, triggered_by)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
    )
    .bind(id.to_string())
    .bind(r.memory_id.map(|m| m.to_string()))
    .bind(r.criterion_id.map(|c| c.to_string()))
    .bind(r.project_id.to_string())
    .bind(r.verifier.as_str())
    .bind(r.evidence_id.map(|e| e.to_string()))
    .bind(r.expected_digest)
    .bind(r.observed_digest)
    .bind(r.result.as_str())
    .bind(r.detail.map(|d| bound(&cairn_core::redact::redact(d), 256)))
    .bind(r.repo_branch)
    .bind(r.repo_commit)
    .bind(rows::now_text())
    .bind(r.trigger.as_str())
    .execute(&mut *t)
    .await?;
    tx::commit(t, "record_run").await?;
    Ok(id)
}

/// One recorded run, as the rebuild and `--explain` read it.
#[derive(Debug, Clone)]
pub struct Run {
    pub id: Uuid,
    pub verifier: VerifierKind,
    pub evidence_id: Option<Uuid>,
    pub result: VerifyResult,
    pub detail: Option<String>,
    pub repo_branch: String,
    pub repo_commit: Option<String>,
    pub checked_at: String,
    pub trigger: VerifyTrigger,
}

fn run_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<Run> {
    let corrupt = |e: cairn_core::domain::ParseEnumError| StoreError::Corrupt(e.to_string());
    Ok(Run {
        id: rows::uuid(row, "id")?,
        verifier: VerifierKind::from_str(row.try_get::<String, _>("verifier")?.as_str())
            .map_err(corrupt)?,
        evidence_id: row
            .try_get::<Option<String>, _>("evidence_id")?
            .and_then(|s| Uuid::parse_str(&s).ok()),
        result: VerifyResult::from_str(row.try_get::<String, _>("result")?.as_str())
            .map_err(corrupt)?,
        detail: row.try_get("detail")?,
        repo_branch: row.try_get("repo_branch")?,
        repo_commit: row.try_get("repo_commit")?,
        checked_at: row.try_get("checked_at")?,
        trigger: VerifyTrigger::from_str(row.try_get::<String, _>("triggered_by")?.as_str())
            .map_err(corrupt)?,
    })
}

/// Every run for a memory, newest first.
pub async fn runs_for_memory(store: &Store, memory_id: Uuid) -> Result<Vec<Run>> {
    let mut conn = store.pool().acquire().await?;
    runs_for_memory_tx(&mut conn, memory_id).await
}

/// The same, through a caller's transaction.
pub async fn runs_for_memory_tx(
    tx: &mut sqlx::SqliteConnection,
    memory_id: Uuid,
) -> Result<Vec<Run>> {
    let rows = sqlx::query(
        "SELECT * FROM verification_runs WHERE memory_id = ?1
          ORDER BY checked_at DESC, id DESC",
    )
    .bind(memory_id.to_string())
    .fetch_all(&mut *tx)
    .await?;
    rows.iter().map(run_from_row).collect()
}

/// Every run for a criterion, newest first.
pub async fn runs_for_criterion(store: &Store, criterion_id: Uuid) -> Result<Vec<Run>> {
    let rows = sqlx::query(
        "SELECT * FROM verification_runs WHERE criterion_id = ?1
          ORDER BY checked_at DESC, id DESC",
    )
    .bind(criterion_id.to_string())
    .fetch_all(store.pool())
    .await?;
    rows.iter().map(run_from_row).collect()
}

/// Recompute a memory's cached verification state and authority from the
/// durable records (`rebuild_verification` + `derive_authority`).
///
/// The state comes from the latest run; the authority from the evidence the
/// runs that **established** it consulted. Both are derived, so a disagreement
/// between the cache and the records is resolved in the records' favour
/// (FR-478).
///
/// Returns the values written.
pub async fn rebuild_verification(
    store: &Store,
    memory_id: Uuid,
) -> Result<(VerificationState, Option<VerificationAuthority>)> {
    let runs = runs_for_memory(store, memory_id).await?;

    // The latest run sets the state. A memory with no run at all is unverified,
    // whatever the cache says.
    let state = match runs.first() {
        None => VerificationState::Unverified,
        Some(latest) => match latest.result {
            VerifyResult::Verified => VerificationState::Verified,
            VerifyResult::Drifted => VerificationState::Drifted,
            // An inconclusive check establishes nothing, so the state is
            // whatever the last conclusive run left — and `unverified` when
            // there was none (FR-366).
            VerifyResult::Inconclusive => runs
                .iter()
                .find_map(|r| match r.result {
                    VerifyResult::Verified => Some(VerificationState::NeedsRecheck),
                    VerifyResult::Drifted => Some(VerificationState::Drifted),
                    VerifyResult::Inconclusive => None,
                })
                .unwrap_or(VerificationState::Unverified),
        },
    };

    // Authority is derived from the collector of the evidence each successful
    // run consulted — never from what a sender claimed.
    let mut facts = Vec::new();
    for run in &runs {
        let collector = match run.evidence_id {
            Some(id) => fact(store, id).await.ok().map(|f| f.collector),
            None => None,
        };
        facts.push(RunFacts {
            verifier: run.verifier,
            result: run.result,
            evidence_collector: collector,
        });
    }
    let authority = derive_authority(state, &facts);

    let last_verified_at = runs
        .iter()
        .find(|r| r.result == VerifyResult::Verified)
        .map(|r| r.checked_at.clone());

    constraints::check_memory_columns(constraints::MemoryColumns {
        verification: Some(state.as_str()),
        verification_authority: authority.map(|a| a.as_str()),
        ..Default::default()
    })?;

    // What the row said before, so an unchanged rebuild can tell it has nothing
    // to say.
    let previous: (Option<String>, Option<String>) =
        sqlx::query_as("SELECT verification, verification_authority FROM memories WHERE id = ?1")
            .bind(memory_id.to_string())
            .fetch_optional(store.pool())
            .await?
            .unwrap_or((None, None));

    sqlx::query(
        "UPDATE memories SET verification = ?2, verification_authority = ?3,
                             last_verified_at = COALESCE(?4, last_verified_at)
         WHERE id = ?1",
    )
    .bind(memory_id.to_string())
    .bind(state.as_str())
    .bind(authority.map(|a| a.as_str()))
    .bind(last_verified_at)
    .execute(store.pool())
    .await?;

    // The **memory's** verification is a shared field, and the evidence behind
    // it is not. Re-queuing the memory is what carries "this was verified, and
    // here is what established it" to a peer; the runs and the facts stay here.
    // Without it the outbox would keep the snapshot taken before the check, and
    // a peer would render a verified memory as unverified indefinitely.
    //
    // **Only when it changed.** This function is also the rebuild path, and
    // `doctor --rebuild-derived` calls it once per memory: queuing regardless
    // would make a release-readiness *check* generate sync traffic proportional
    // to the project, on a project where nothing happened. An unchanged
    // verification has nothing to tell a peer that the peer was not already
    // told.
    let unchanged = previous.0.as_deref().unwrap_or("unverified") == state.as_str()
        && previous.1.as_deref() == authority.map(|a| a.as_str());
    if !unchanged {
        let _ = crate::repo::enqueue_memory_upsert(store, memory_id).await;
    }

    Ok((state, authority))
}

/// Set a memory's verification state directly, for the transitions that are not
/// the result of a run — a fingerprint change, or an attached contradiction.
///
/// Writes exactly `verification`, its authority and `last_verified_at`, and
/// nothing else. Never content, type, scope, provenance or lifecycle state
/// (FR-371, I6).
pub async fn set_verification(
    store: &Store,
    memory_id: Uuid,
    state: VerificationState,
) -> Result<()> {
    let authority = if state == VerificationState::Verified {
        // Reaching `verified` without a run is not expressible: the caller must
        // record the run and rebuild.
        return Err(StoreError::Corrupt(
            "a memory reaches verified only through a recorded run".into(),
        ));
    } else {
        None::<&str>
    };
    constraints::check_memory_columns(constraints::MemoryColumns {
        verification: Some(state.as_str()),
        verification_authority: authority,
        ..Default::default()
    })?;
    sqlx::query(
        "UPDATE memories SET verification = ?2, verification_authority = NULL WHERE id = ?1",
    )
    .bind(memory_id.to_string())
    .bind(state.as_str())
    .execute(store.pool())
    .await?;

    // Losing a verification travels for the same reason gaining one does: a
    // peer still showing `remote_cairn verified` for a memory this machine now
    // knows has drifted is being told something false.
    let _ = crate::repo::enqueue_memory_upsert(store, memory_id).await;
    Ok(())
}

/// What a shared memory may say about its evidence — five keys, no content
/// (`contracts/privacy-sync.md`, FR-502).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct VerificationSummary {
    pub state: VerificationState,
    /// Only ever `cairn` or `attested` on the wire: `remote_*` is what a
    /// *receiver* derives, never what a sender claims (T104).
    pub authority: Option<VerificationAuthority>,
    pub last_verified_at: Option<String>,
    /// A count. Never the facts themselves.
    pub fact_count: usize,
    /// Verifier **kinds** only.
    pub basis: Vec<VerifierKind>,
}

/// Build the summary a memory's sync payload may carry.
pub async fn summary(store: &Store, memory_id: Uuid) -> Result<VerificationSummary> {
    let mut conn = store.pool().acquire().await?;
    summary_tx(&mut conn, memory_id).await
}

/// The same, through a caller's transaction.
///
/// The sync payload is built inside the transaction that wrote the memory, and
/// a pool query there would wait on a connection that transaction is holding.
pub async fn summary_tx(
    tx: &mut sqlx::SqliteConnection,
    memory_id: Uuid,
) -> Result<VerificationSummary> {
    let row = sqlx::query(
        "SELECT verification, verification_authority, last_verified_at
           FROM memories WHERE id = ?1",
    )
    .bind(memory_id.to_string())
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| StoreError::NotFound(format!("memory {memory_id}")))?;

    let state = VerificationState::from_str(row.try_get::<String, _>("verification")?.as_str())
        .map_err(|e| StoreError::Corrupt(e.to_string()))?;
    let authority = row
        .try_get::<Option<String>, _>("verification_authority")?
        .map(|a| VerificationAuthority::from_str(&a))
        .transpose()
        .map_err(|e| StoreError::Corrupt(e.to_string()))?
        // A peer never learns that *this* machine imported the state; it learns
        // what kind of check stands behind it.
        .map(|a| a.on_the_wire());

    let fact_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM memory_evidence_facts l
           JOIN evidence_facts f ON f.id = l.evidence_id
          WHERE l.memory_id = ?1 AND l.role = 'supports' AND f.deleted_at IS NULL",
    )
    .bind(memory_id.to_string())
    .fetch_one(&mut *tx)
    .await? as usize;

    let mut basis: Vec<VerifierKind> = runs_for_memory_tx(&mut *tx, memory_id)
        .await?
        .into_iter()
        .filter(|r| r.result == VerifyResult::Verified)
        .map(|r| r.verifier)
        .collect();
    basis.sort();
    basis.dedup();

    Ok(VerificationSummary {
        state,
        authority,
        last_verified_at: row.try_get("last_verified_at")?,
        fact_count,
        basis,
    })
}

/// Drifted memories, for the Level 0 warning tier.
///
/// Returns `(subject, detail)` per claim whose support moved. Drift is a
/// **state**, never an edit: the memory still says what it said, and this is how
/// the agent is told it no longer holds (FR-371, FR-372).
pub async fn drifted_memories(
    store: &Store,
    project_id: Uuid,
    limit: i64,
) -> Result<Vec<(String, String)>> {
    let rows = sqlx::query(
        "SELECT topic_key, content, verification FROM memories
          WHERE project_id = ?1 AND deleted_at IS NULL
            AND verification IN ('drifted', 'needs_recheck')
            AND state != 'superseded'
          ORDER BY pinned DESC, updated_at DESC
          LIMIT ?2",
    )
    .bind(project_id.to_string())
    .bind(limit)
    .fetch_all(store.pool())
    .await?;

    Ok(rows
        .iter()
        .map(|r| {
            let topic: Option<String> = r.try_get("topic_key").ok().flatten();
            let content: String = r.try_get("content").unwrap_or_default();
            let verification: String = r.try_get("verification").unwrap_or_default();
            (
                topic.unwrap_or_else(|| content.chars().take(48).collect()),
                verification,
            )
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::knowledge::tests_support::{fixture, Fixture};

    const VALUE_MAX: usize = 256;
    const LOCATOR_MAX: usize = 256;

    async fn evidence(
        f: &Fixture,
        kind: EvidenceKind,
        collector: EvidenceCollector,
        value: &str,
        locator: &str,
    ) -> EvidenceFact {
        record(
            &f.store,
            NewEvidence {
                project_id: f.project,
                kind,
                collector,
                subject: "database backend",
                observed_value: value,
                source_locator: locator,
                fingerprint: "fp-1",
                observation_id: None,
                repo_branch: "main",
                repo_commit: Some("abc123"),
                collected_by_session: f.session_a,
            },
            VALUE_MAX,
            LOCATOR_MAX,
        )
        .await
        .expect("record evidence")
    }

    #[test]
    fn a_locator_is_repository_relative_or_it_is_refused() {
        // FR-353, I9. A stored absolute locator would carry a machine's
        // directory layout into a column a future feature might widen.
        for absolute in [
            "/Users/dev/src/repo/config.yml",
            "/etc/passwd",
            "C:\\Users\\dev\\config.yml",
            "\\\\server\\share\\config.yml",
            "//server/share/config.yml",
            "\\config.yml",
        ] {
            assert_eq!(
                validate_locator(absolute),
                Err(LocatorRefusal::Absolute),
                "{absolute} was accepted"
            );
        }

        for escaping in ["../outside.yml", "src/../../outside.yml", "a/b/../../../c"] {
            assert_eq!(
                validate_locator(escaping),
                Err(LocatorRefusal::OutsideWorktree),
                "{escaping} was accepted"
            );
        }

        for ok in [
            "config/app.yml",
            "src/lib.rs",
            "a/b/../c",
            "src/library/..x",
            "refs/heads/main",
            "./config/app.yml",
        ] {
            assert_eq!(validate_locator(ok), Ok(()), "{ok} was refused");
        }
    }

    #[tokio::test]
    async fn a_refused_locator_writes_nothing() {
        let f = fixture().await;
        let err = record(
            &f.store,
            NewEvidence {
                project_id: f.project,
                kind: EvidenceKind::Configuration,
                collector: EvidenceCollector::Cairn,
                subject: "database backend",
                observed_value: "postgresql",
                source_locator: "/etc/cairn/app.yml",
                fingerprint: "fp",
                observation_id: None,
                repo_branch: "main",
                repo_commit: None,
                collected_by_session: f.session_a,
            },
            VALUE_MAX,
            LOCATOR_MAX,
        )
        .await
        .expect_err("an absolute locator must be refused")
        .to_string();
        assert!(err.contains("absolute_locator"), "{err}");
        assert_eq!(
            f.store_count("SELECT COUNT(*) FROM evidence_facts").await,
            0
        );
    }

    #[tokio::test]
    async fn a_credential_is_redacted_before_it_is_bounded() {
        // FR-354. Bounding first would cut the credential into a fragment
        // redaction no longer recognizes, and then store the fragment.
        let f = fixture().await;
        let secret = "postgres://ledger:CORPUSFIXTUREpassword@db.internal:5432/ledger";
        let fact = evidence(
            &f,
            EvidenceKind::Configuration,
            EvidenceCollector::Cairn,
            secret,
            "config/database.yml",
        )
        .await;

        let stored = fact.observed_value.expect("value");
        assert!(
            !stored.contains("CORPUSFIXTUREpassword"),
            "the raw credential was stored: {stored}"
        );
        assert!(
            !cairn_core::redact::contains_secret(&stored),
            "something secret-shaped survived: {stored}"
        );
        assert!(stored.len() <= VALUE_MAX);
    }

    #[tokio::test]
    async fn a_value_is_bounded_on_a_character_boundary() {
        let f = fixture().await;
        let long = "é".repeat(400);
        let fact = record(
            &f.store,
            NewEvidence {
                project_id: f.project,
                kind: EvidenceKind::Configuration,
                collector: EvidenceCollector::Cairn,
                subject: "long",
                observed_value: &long,
                source_locator: "config/app.yml",
                fingerprint: "fp",
                observation_id: None,
                repo_branch: "main",
                repo_commit: None,
                collected_by_session: f.session_a,
            },
            VALUE_MAX,
            LOCATOR_MAX,
        )
        .await
        .expect("record");
        let stored = fact.observed_value.expect("value");
        assert!(stored.len() <= VALUE_MAX);
        assert!(
            !stored.is_empty(),
            "a multi-byte value was truncated to nothing"
        );
    }

    #[tokio::test]
    async fn deleting_a_fact_leaves_the_reference_resolvable() {
        // FR-358, FR-505. Identity, kind, timestamps and provenance survive;
        // the value, digest, locator and fingerprint are cleared.
        let f = fixture().await;
        let m = f
            .propose(
                f.session_a,
                Some("infra.db"),
                Some("postgresql"),
                "PostgreSQL.",
            )
            .await;
        let e = evidence(
            &f,
            EvidenceKind::Configuration,
            EvidenceCollector::Cairn,
            "postgresql",
            "config/database.yml",
        )
        .await;
        attach_to_memory(
            &f.store,
            m.memory.id,
            e.id,
            EvidenceRole::Supports,
            f.session_a,
        )
        .await
        .expect("attach");

        forget(&f.store, e.id).await.expect("forget");

        let after = fact(&f.store, e.id).await.expect("still resolvable");
        assert!(after.deleted, "the tombstone was not recorded");
        assert_eq!(after.observed_value, None);
        assert_eq!(after.value_digest, None);
        assert_eq!(after.source_locator, None);
        assert_eq!(after.fingerprint, None);
        // What survives.
        assert_eq!(after.kind, EvidenceKind::Configuration);
        assert_eq!(after.collected_by_session, f.session_a);
        assert_eq!(after.repo_branch, "main");

        // And the link is still there, reporting a deleted fact rather than
        // disappearing.
        let linked = facts_for_memory(&f.store, m.memory.id)
            .await
            .expect("links");
        assert_eq!(linked.len(), 1);
        assert!(linked[0].1.deleted);
    }

    #[tokio::test]
    async fn the_locator_lookup_is_exact_and_capped() {
        let f = fixture().await;
        for i in 0..5 {
            evidence(
                &f,
                EvidenceKind::Configuration,
                EvidenceCollector::Cairn,
                &format!("value-{i}"),
                "config/app.yml",
            )
            .await;
        }
        evidence(
            &f,
            EvidenceKind::Configuration,
            EvidenceCollector::Cairn,
            "elsewhere",
            "config/other.yml",
        )
        .await;

        let hits = facts_by_locator(&f.store, f.project, "config/app.yml", 8)
            .await
            .expect("lookup");
        assert_eq!(hits.len(), 5, "exact equality only");

        let capped = facts_by_locator(&f.store, f.project, "config/app.yml", 2)
            .await
            .expect("lookup");
        assert_eq!(capped.len(), 2, "the cap binds");

        // No prefix scan: a locator that merely starts the same never matches.
        let none = facts_by_locator(&f.store, f.project, "config/", 8)
            .await
            .expect("lookup");
        assert!(none.is_empty());
    }

    #[tokio::test]
    async fn a_run_never_rewrites_an_earlier_one() {
        // FR-364. Only the cached state moves.
        let f = fixture().await;
        let m = f
            .propose(
                f.session_a,
                Some("infra.db"),
                Some("postgresql"),
                "PostgreSQL.",
            )
            .await;
        let e = evidence(
            &f,
            EvidenceKind::Configuration,
            EvidenceCollector::Cairn,
            "postgresql",
            "config/database.yml",
        )
        .await;

        for result in [
            VerifyResult::Verified,
            VerifyResult::Drifted,
            VerifyResult::Verified,
        ] {
            record_run(
                &f.store,
                NewRun {
                    project_id: f.project,
                    memory_id: Some(m.memory.id),
                    criterion_id: None,
                    verifier: VerifierKind::Configuration,
                    evidence_id: Some(e.id),
                    expected_digest: Some("aaa"),
                    observed_digest: Some("bbb"),
                    result,
                    detail: None,
                    repo_branch: "main",
                    repo_commit: Some("abc123"),
                    trigger: VerifyTrigger::OnDemand,
                },
            )
            .await
            .expect("run");
            tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        }

        let runs = runs_for_memory(&f.store, m.memory.id).await.expect("runs");
        assert_eq!(runs.len(), 3, "a run overwrote another");
        assert_eq!(runs[0].result, VerifyResult::Verified, "newest first");
        assert_eq!(runs[2].result, VerifyResult::Verified);
        assert_eq!(runs[1].result, VerifyResult::Drifted);
    }

    #[tokio::test]
    async fn a_cairn_collected_check_yields_cairn_authority() {
        let f = fixture().await;
        let m = f
            .propose(
                f.session_a,
                Some("infra.db"),
                Some("postgresql"),
                "PostgreSQL.",
            )
            .await;
        let e = evidence(
            &f,
            EvidenceKind::Configuration,
            EvidenceCollector::Cairn,
            "postgresql",
            "config/database.yml",
        )
        .await;
        record_run(
            &f.store,
            NewRun {
                project_id: f.project,
                memory_id: Some(m.memory.id),
                criterion_id: None,
                verifier: VerifierKind::Configuration,
                evidence_id: Some(e.id),
                expected_digest: Some("aaa"),
                observed_digest: Some("aaa"),
                result: VerifyResult::Verified,
                detail: None,
                repo_branch: "main",
                repo_commit: Some("abc123"),
                trigger: VerifyTrigger::OnDemand,
            },
        )
        .await
        .expect("run");

        let (state, authority) = rebuild_verification(&f.store, m.memory.id)
            .await
            .expect("rebuild");
        assert_eq!(state, VerificationState::Verified);
        assert_eq!(authority, Some(VerificationAuthority::Cairn));

        let stored: (String, Option<String>, Option<String>) = sqlx::query_as(
            "SELECT verification, verification_authority, last_verified_at
               FROM memories WHERE id = ?1",
        )
        .bind(m.memory.id.to_string())
        .fetch_one(f.store.pool())
        .await
        .expect("stored");
        assert_eq!(stored.0, "verified");
        assert_eq!(stored.1.as_deref(), Some("cairn"));
        assert!(stored.2.is_some(), "last_verified_at was not recorded");
    }

    #[tokio::test]
    async fn an_attested_check_yields_attested_authority_and_the_deterministic_one_wins() {
        let f = fixture().await;
        let m = f
            .propose(f.session_a, Some("api.port"), Some("8080"), "Port 8080.")
            .await;

        let attested = evidence(
            &f,
            EvidenceKind::RuntimeState,
            EvidenceCollector::Agent,
            "8080",
            "runtime/health",
        )
        .await;
        record_run(
            &f.store,
            NewRun {
                project_id: f.project,
                memory_id: Some(m.memory.id),
                criterion_id: None,
                verifier: VerifierKind::RuntimeState,
                evidence_id: Some(attested.id),
                expected_digest: None,
                observed_digest: None,
                result: VerifyResult::Verified,
                detail: None,
                repo_branch: "main",
                repo_commit: None,
                trigger: VerifyTrigger::Attach,
            },
        )
        .await
        .expect("run");

        let (_, authority) = rebuild_verification(&f.store, m.memory.id)
            .await
            .expect("rebuild");
        assert_eq!(authority, Some(VerificationAuthority::Attested));

        // A deterministic check over Cairn-collected evidence now stands too:
        // the stronger basis is what established it (metric 25c).
        let collected = evidence(
            &f,
            EvidenceKind::Configuration,
            EvidenceCollector::Cairn,
            "8080",
            "config/app.yml",
        )
        .await;
        record_run(
            &f.store,
            NewRun {
                project_id: f.project,
                memory_id: Some(m.memory.id),
                criterion_id: None,
                verifier: VerifierKind::Configuration,
                evidence_id: Some(collected.id),
                expected_digest: Some("aaa"),
                observed_digest: Some("aaa"),
                result: VerifyResult::Verified,
                detail: None,
                repo_branch: "main",
                repo_commit: None,
                trigger: VerifyTrigger::OnDemand,
            },
        )
        .await
        .expect("run");

        let (state, authority) = rebuild_verification(&f.store, m.memory.id)
            .await
            .expect("rebuild");
        assert_eq!(state, VerificationState::Verified);
        assert_eq!(
            authority,
            Some(VerificationAuthority::Cairn),
            "the attested fact outranked a deterministic check"
        );
    }

    #[tokio::test]
    async fn an_inconclusive_run_establishes_nothing() {
        // FR-366: the memory becomes neither verified nor drifted.
        let f = fixture().await;
        let m = f
            .propose(
                f.session_a,
                Some("infra.db"),
                Some("postgresql"),
                "PostgreSQL.",
            )
            .await;
        record_run(
            &f.store,
            NewRun {
                project_id: f.project,
                memory_id: Some(m.memory.id),
                criterion_id: None,
                verifier: VerifierKind::FileDigest,
                evidence_id: None,
                expected_digest: None,
                observed_digest: None,
                result: VerifyResult::Inconclusive,
                detail: Some("the file could not be read"),
                repo_branch: "main",
                repo_commit: None,
                trigger: VerifyTrigger::OnDemand,
            },
        )
        .await
        .expect("run");

        let (state, authority) = rebuild_verification(&f.store, m.memory.id)
            .await
            .expect("rebuild");
        assert_eq!(state, VerificationState::Unverified);
        assert_eq!(authority, None);
    }

    #[tokio::test]
    async fn a_verified_state_cannot_be_set_without_a_run() {
        let f = fixture().await;
        let m = f
            .propose(
                f.session_a,
                Some("infra.db"),
                Some("postgresql"),
                "PostgreSQL.",
            )
            .await;
        assert!(
            set_verification(&f.store, m.memory.id, VerificationState::Verified)
                .await
                .is_err(),
            "a memory reached verified without a recorded run"
        );
        set_verification(&f.store, m.memory.id, VerificationState::NeedsRecheck)
            .await
            .expect("needs_recheck is reachable");
    }

    #[tokio::test]
    async fn the_sync_summary_carries_five_keys_and_no_content() {
        // FR-502: what a shared memory may say about evidence is the state, its
        // authority, the instant, a count, and verifier kinds. Nothing else.
        let f = fixture().await;
        let m = f
            .propose(
                f.session_a,
                Some("infra.db"),
                Some("postgresql"),
                "PostgreSQL.",
            )
            .await;
        let e = evidence(
            &f,
            EvidenceKind::Configuration,
            EvidenceCollector::Cairn,
            "postgresql",
            "config/database.yml",
        )
        .await;
        attach_to_memory(
            &f.store,
            m.memory.id,
            e.id,
            EvidenceRole::Supports,
            f.session_a,
        )
        .await
        .expect("attach");
        record_run(
            &f.store,
            NewRun {
                project_id: f.project,
                memory_id: Some(m.memory.id),
                criterion_id: None,
                verifier: VerifierKind::Configuration,
                evidence_id: Some(e.id),
                expected_digest: Some("aaa"),
                observed_digest: Some("aaa"),
                result: VerifyResult::Verified,
                detail: None,
                repo_branch: "main",
                repo_commit: None,
                trigger: VerifyTrigger::OnDemand,
            },
        )
        .await
        .expect("run");
        rebuild_verification(&f.store, m.memory.id)
            .await
            .expect("rebuild");

        let s = summary(&f.store, m.memory.id).await.expect("summary");
        assert_eq!(s.state, VerificationState::Verified);
        assert_eq!(s.authority, Some(VerificationAuthority::Cairn));
        assert_eq!(s.fact_count, 1);
        assert_eq!(s.basis, vec![VerifierKind::Configuration]);

        let json = serde_json::to_value(&s).expect("serializes");
        // Five keys, and exactly these five. The set is the contract; the
        // serializer's ordering is not.
        let mut keys: Vec<&str> = json
            .as_object()
            .expect("object")
            .keys()
            .map(|k| k.as_str())
            .collect();
        keys.sort();
        assert_eq!(
            keys,
            vec![
                "authority",
                "basis",
                "fact_count",
                "last_verified_at",
                "state"
            ],
            "the payload shape changed"
        );
        let text = serde_json::to_string(&s).expect("serializes");
        for forbidden in [
            "postgresql",
            "config/database.yml",
            "observed_value",
            "source_locator",
        ] {
            assert!(
                !text.contains(forbidden),
                "the summary leaked {forbidden}: {text}"
            );
        }
    }

    #[tokio::test]
    async fn an_imported_authority_is_never_sent_back_as_local() {
        // A peer learns what kind of check stands behind the state, not that
        // this machine imported it (T104).
        let f = fixture().await;
        let m = f
            .propose(
                f.session_a,
                Some("infra.db"),
                Some("postgresql"),
                "PostgreSQL.",
            )
            .await;
        sqlx::query(
            "UPDATE memories SET verification = 'verified', verification_authority = 'remote_attested'
             WHERE id = ?1",
        )
        .bind(m.memory.id.to_string())
        .execute(f.store.pool())
        .await
        .expect("import");

        let s = summary(&f.store, m.memory.id).await.expect("summary");
        assert_eq!(s.authority, Some(VerificationAuthority::Attested));
        assert!(!s.authority.expect("authority").is_imported());
    }
}
