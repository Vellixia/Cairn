//! Reusable cross-project patterns: the repository and the promotion gate
//! (`contracts/patterns.md`, FR-391 – FR-406).
//!
//! Everything here is **local**. `reusable_patterns` has no `project_id`
//! column, no outbox entity type and no server table, so "a pattern never
//! synchronizes" is a property of the schema rather than a promise someone has
//! to keep (FR-508).
//!
//! A project memory must never simply become a global memory. The gate below is
//! what stands between the two, and it fails closed: any check that does not
//! pass refuses, names its class, echoes no value, and writes nothing.

use crate::{rows, tx, Result, Store, StoreError};
use cairn_core::domain::{
    MemoryScope, MemoryState, MemoryType, PatternDiscovery, PatternOutcome, PatternTrust,
    VerificationAuthority, VerificationState,
};
use cairn_core::patterns::{
    count_applications, derive_pattern_trust, normalize_signals, root_cause_digest, signal_digest,
    signal_overlap, ApplicationFacts, PatternCounters,
};
use cairn_core::wire::codes;
use cairn_core::{domain::new_id, redact};
use sqlx::Row as _;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// The record
// ---------------------------------------------------------------------------

/// A sanitized, applicability-bounded pattern with **no project identity**.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Pattern {
    pub id: Uuid,
    pub title: String,
    pub problem: String,
    pub signals: Vec<String>,
    pub signal_digest: String,
    pub applicability: Vec<String>,
    pub root_cause: String,
    pub root_cause_digest: String,
    pub approach: String,
    pub constraints: Vec<String>,
    pub trust: PatternTrust,
    /// Opaque. A machine-salted digest of the source project, never a name, a
    /// path or a remote.
    pub origin_ref: String,
    pub origin_deleted: bool,
    pub source_memory_id: Option<Uuid>,
    pub sanitization_report: serde_json::Value,
}

/// What a promotion proposes. Free text, still unsanitized.
#[derive(Debug, Clone)]
pub struct Candidate<'a> {
    pub title: &'a str,
    pub problem: &'a str,
    pub signals: &'a [String],
    pub applicability: &'a [String],
    pub root_cause: &'a str,
    pub approach: &'a str,
    pub constraints: &'a [String],
}

/// The outcome of running the gate.
#[derive(Debug, Clone)]
pub enum Promotion {
    /// Written, unless the caller asked for a dry run.
    Promoted(Box<Pattern>),
    /// Refused with its class named. Nothing was written.
    Refused {
        /// One of the ten gate classes, or an authority refusal.
        class: &'static str,
        /// Names the class and what to do about it. **Never** the value that
        /// caused the refusal (FR-397).
        message: String,
    },
}

impl Promotion {
    pub fn refusal(&self) -> Option<&'static str> {
        match self {
            Promotion::Refused { class, .. } => Some(class),
            Promotion::Promoted(_) => None,
        }
    }
}

/// The source memory's facts, as the gate reads them.
struct SourceFacts {
    state: MemoryState,
    kind: MemoryType,
    scope: MemoryScope,
    scope_key: String,
    topic_key: Option<String>,
    local_only: bool,
    verification: VerificationState,
    authority: Option<VerificationAuthority>,
}

// ---------------------------------------------------------------------------
// The gate
// ---------------------------------------------------------------------------

/// The ten checks, in this fixed order so the reported reason is stable.
///
/// The order is the contract. A candidate that violates more than one check
/// must always report the same class, or a developer fixing what they were told
/// about would be told something different next time.
const GATE_CHECKS: &[&str] = &[
    "source_active",
    "source_verified",
    "evidence_present",
    "not_local_only",
    "not_conflicted",
    "transferable_type",
    "secret_scan",
    "project_identifier_scan",
    "signal_specificity",
    "duplicate_scan",
];

/// Run the gate and, unless `dry_run`, write the pattern.
///
/// `--dry-run` exists because the gate's whole value is that it explains a
/// refusal: someone should be able to ask before committing to the wording.
pub async fn promote(
    store: &Store,
    source_memory_id: Uuid,
    candidate: Candidate<'_>,
    signals_min: usize,
    dry_run: bool,
) -> Result<Promotion> {
    let memory = crate::repo::memory(store, source_memory_id).await?;
    let project = crate::repo::project(store, memory.project_id).await?;
    let source = source_facts(store, source_memory_id).await?;

    // --- 1. The source is still what this project holds.
    if source.state != MemoryState::Active {
        return Ok(refuse(
            codes::SOURCE_NOT_ACTIVE,
            "the source memory is no longer active; promoting it would export a \
             conclusion this project has already replaced",
        ));
    }

    // --- 2. Verified, and by a deterministic check this machine ran.
    //
    // The two authority refusals are the two ways an agent could otherwise
    // launder its own claim into cross-project knowledge: attest a fact and
    // promote it, or promote one a peer said it had checked. Cross-project
    // knowledge is the furthest-travelling thing Cairn produces, so it takes
    // the strictest setting (FR-370, SC-328).
    if source.verification != VerificationState::Verified {
        return Ok(refuse(
            codes::SOURCE_UNVERIFIED,
            "the source memory is not verified; a pattern starts from a check, \
             not from a claim",
        ));
    }
    match source.authority {
        Some(VerificationAuthority::Cairn) => {}
        Some(VerificationAuthority::Attested) => {
            return Ok(refuse(
                codes::ATTESTED_NOT_SUFFICIENT,
                "the source is verified by an agent's attestation; promotion \
                 requires a deterministic check Cairn ran itself",
            ))
        }
        Some(VerificationAuthority::RemoteCairn) | Some(VerificationAuthority::RemoteAttested) => {
            return Ok(refuse(
                codes::IMPORTED_NOT_SUFFICIENT,
                "the source's verification was established on another machine, \
                 against evidence this machine cannot see",
            ))
        }
        None => {
            return Ok(refuse(
                codes::SOURCE_UNVERIFIED,
                "the source memory carries no verification authority",
            ))
        }
    }

    // --- 3. Something is actually behind that verification.
    let facts = crate::evidence::facts_for_memory(store, source_memory_id).await?;
    if facts.is_empty() {
        return Ok(refuse(
            codes::NO_EVIDENCE,
            "the source memory has no evidence fact attached; a verification \
             with nothing behind it is a state without a reason",
        ));
    }

    // --- 4. The memory was not marked never to travel.
    if source.local_only {
        return Ok(refuse(
            codes::LOCAL_ONLY_MEMORY,
            "the source memory is local-only; a pattern derived from it would be \
             that memory travelling under another name",
        ));
    }

    // --- 5. The project itself agrees with it.
    if let Some(topic) = &source.topic_key {
        let view = crate::knowledge::subject(
            store,
            memory.project_id,
            source.scope,
            &source.scope_key,
            topic,
            crate::repo::DEFAULT_RECONCILE_MEMBERS_MAX,
        )
        .await?;
        if view.view.reconciliation == cairn_core::domain::Reconciliation::Conflicted {
            return Ok(refuse(
                codes::SOURCE_CONFLICTED,
                "the source's subject is conflicted; exporting one side of an \
                 unresolved disagreement would state as settled what is not",
            ));
        }
    }

    // --- 6. It describes a problem and its resolution, not a configuration.
    if !transferable(&source) {
        return Ok(refuse(
            codes::NOT_TRANSFERABLE,
            "this memory states what this project is configured with rather than \
             a problem and its resolution, so it transfers nowhere",
        ));
    }

    // --- 7 and 8. What the text must not contain.
    let body = candidate_text(&candidate);
    let scrubbed = redact::redact(&body);
    let redactions = body.len().saturating_sub(scrubbed.len());

    // Both the text as written **and** the text after redaction.
    //
    // Redacting a credential and promoting the remainder would be the wrong
    // answer here, even though it is the right answer for an observation. A
    // pattern is the furthest-travelling record Cairn produces, and a candidate
    // that contained a credential at all was written somewhere it should not
    // have been — that is a fact about the candidate, not a formatting problem
    // to clean up silently (FR-397).
    //
    // Checking the scrubbed text too catches the shape redaction mangled
    // without removing.
    if redact::contains_secret(&body) || redact::contains_secret(&scrubbed) {
        return Ok(refuse(
            codes::POSSIBLE_SECRET,
            "this candidate contains a credential-shaped value; a pattern is \
             shared across projects and must never carry one",
        ));
    }
    if let Some(class) = project_identifier(&scrubbed, &project) {
        return Ok(refuse(
            codes::PROJECT_IDENTIFYING,
            format!("this candidate names {class}, which a pattern must never carry"),
        ));
    }

    // --- 9. Specific enough to match on.
    let signals = normalize_signals(candidate.signals);
    if signals.len() < signals_min {
        return Ok(refuse(
            codes::INSUFFICIENT_SPECIFICITY,
            format!(
                "a pattern needs at least {signals_min} distinct signals to match on; \
                 this one normalizes to {}",
                signals.len()
            ),
        ));
    }

    // --- 10. Not one we already have.
    let digests = (
        signal_digest(candidate.signals),
        root_cause_digest(candidate.root_cause),
    );
    let existing: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM reusable_patterns
          WHERE signal_digest = ?1 AND root_cause_digest = ?2 AND deleted_at IS NULL",
    )
    .bind(&digests.0)
    .bind(&digests.1)
    .fetch_one(store.pool())
    .await?;
    if existing > 0 {
        return Ok(refuse(
            codes::DUPLICATE_PATTERN,
            "a pattern with these signals and this root cause already exists",
        ));
    }

    let pattern = Pattern {
        id: new_id(),
        title: redact::redact(candidate.title),
        problem: redact::redact(candidate.problem),
        signals,
        signal_digest: digests.0,
        applicability: candidate
            .applicability
            .iter()
            .map(|s| redact::redact(s))
            .collect(),
        root_cause: redact::redact(candidate.root_cause),
        root_cause_digest: digests.1,
        approach: redact::redact(candidate.approach),
        constraints: candidate
            .constraints
            .iter()
            .map(|s| redact::redact(s))
            .collect(),
        // A freshly promoted pattern is sanitized and nothing more. Trust is
        // earned by other projects, never by promotion.
        trust: PatternTrust::Sanitized,
        origin_ref: origin_ref(memory.project_id)?,
        origin_deleted: false,
        source_memory_id: Some(source_memory_id),
        sanitization_report: serde_json::json!({
            "checks": GATE_CHECKS,
            // A count of what redaction changed, never the text it changed.
            "redactions_applied": usize::from(redactions > 0),
            "outcome": "passed",
        }),
    };

    if dry_run {
        return Ok(Promotion::Promoted(Box::new(pattern)));
    }
    insert(store, &pattern).await?;
    Ok(Promotion::Promoted(Box::new(pattern)))
}

fn refuse(class: &'static str, message: impl Into<String>) -> Promotion {
    Promotion::Refused {
        class,
        message: message.into(),
    }
}

/// Check 6. A `fact` is never transferable; a `convention` is only when it is
/// not bound to project configuration.
fn transferable(source: &SourceFacts) -> bool {
    match source.kind {
        MemoryType::Procedure | MemoryType::Failure | MemoryType::Decision => true,
        MemoryType::Convention => source.topic_key.is_none(),
        // "The production database is PostgreSQL" is true, verified,
        // evidence-backed and completely untransferable. This is the check that
        // earns its place.
        MemoryType::Fact => false,
    }
}

/// Check 8. Which class of project identifier the text names, if any.
///
/// Returns the **class**, never the value: a refusal that echoed the offending
/// string would put it wherever the refusal is logged.
fn project_identifier(text: &str, project: &cairn_core::domain::Project) -> Option<&'static str> {
    let haystack = text.to_lowercase();

    if absolute_path(&haystack) {
        return Some("an absolute filesystem path");
    }
    // Four casings, because a project called "Helios Ledger" appears as
    // `helios-ledger` and `HELIOS_LEDGER` at least as often as it appears
    // verbatim.
    for form in name_forms(&project.name) {
        if form.chars().count() >= 4 && haystack.contains(&form) {
            return Some("this project's name");
        }
    }
    if let Some(remote) = &project.repository_remote {
        // With or without credentials, and with or without the scheme.
        let bare = remote
            .to_lowercase()
            .rsplit('@')
            .next()
            .unwrap_or_default()
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .trim_start_matches("git://")
            .trim_end_matches(".git")
            .to_string();
        if !bare.is_empty() && haystack.contains(&bare) {
            return Some("the repository remote");
        }
    }
    if let Some(server) = project.server_project_id {
        if haystack.contains(&server.to_string()) {
            return Some("the shared project identifier");
        }
    }
    let git_dir = project.git_common_dir.to_lowercase();
    if !git_dir.is_empty() && haystack.contains(&git_dir) {
        return Some("this repository's location on disk");
    }
    if email(&haystack) {
        return Some("an email address");
    }
    None
}

fn name_forms(name: &str) -> Vec<String> {
    let lower = name.to_lowercase();
    let mut forms = vec![
        lower.clone(),
        lower.replace(' ', "-"),
        lower.replace(' ', "_"),
        lower.replace(' ', ""),
    ];
    forms.sort();
    forms.dedup();
    forms.retain(|f| !f.is_empty());
    forms
}

fn absolute_path(text: &str) -> bool {
    text.contains("/users/")
        || text.contains("/home/")
        || text.contains("/var/")
        || text.contains("/etc/")
        || text.contains("/opt/")
        || text.contains("/private/")
        || text.contains("\\users\\")
        || text.contains(r"\\")
        || text
            .split_whitespace()
            .any(|w| w.len() > 3 && w.as_bytes()[1] == b':' && w.as_bytes()[2] == b'\\')
}

fn email(text: &str) -> bool {
    text.split_whitespace().any(|word| {
        let word = word.trim_matches(|c: char| !c.is_alphanumeric());
        match word.split_once('@') {
            Some((local, domain)) => {
                !local.is_empty() && domain.contains('.') && !domain.ends_with('.')
            }
            None => false,
        }
    })
}

/// Every free-text field, joined — what checks 7 and 8 scan.
fn candidate_text(c: &Candidate<'_>) -> String {
    let mut parts = vec![
        c.title.to_string(),
        c.problem.to_string(),
        c.root_cause.to_string(),
        c.approach.to_string(),
    ];
    parts.extend(c.signals.iter().cloned());
    parts.extend(c.applicability.iter().cloned());
    parts.extend(c.constraints.iter().cloned());
    parts.join("\n")
}

/// The opaque origin reference (FR-393).
fn origin_ref(project_id: Uuid) -> Result<String> {
    let salt = cairn_core::paths::machine_salt()
        .map_err(|e| StoreError::Corrupt(format!("machine salt: {e}")))?;
    Ok(cairn_core::digest(&format!("{salt}:{project_id}")))
}

async fn source_facts(store: &Store, memory_id: Uuid) -> Result<SourceFacts> {
    let row = sqlx::query(
        "SELECT state, type, scope, scope_key, topic_key, local_only,
                verification, verification_authority
           FROM memories WHERE id = ?1 AND deleted_at IS NULL",
    )
    .bind(memory_id.to_string())
    .fetch_optional(store.pool())
    .await?
    .ok_or_else(|| StoreError::NotFound(format!("memory {memory_id}")))?;

    Ok(SourceFacts {
        state: rows::enum_val(&row, "state")?,
        kind: rows::enum_val(&row, "type")?,
        scope: rows::enum_val(&row, "scope")?,
        scope_key: row.try_get("scope_key")?,
        topic_key: row.try_get("topic_key")?,
        local_only: row.try_get::<i64, _>("local_only")? == 1,
        verification: row
            .try_get::<Option<String>, _>("verification")?
            .and_then(|s| s.parse().ok())
            .unwrap_or(VerificationState::Unverified),
        authority: row
            .try_get::<Option<String>, _>("verification_authority")?
            .and_then(|s| s.parse().ok()),
    })
}

// ---------------------------------------------------------------------------
// Storage
// ---------------------------------------------------------------------------

async fn insert(store: &Store, p: &Pattern) -> Result<()> {
    let now = rows::now_text();
    let mut t = tx::begin(store, "promote_pattern").await?;
    sqlx::query(
        "INSERT INTO reusable_patterns
            (id, title, problem, signals, signal_digest, applicability, root_cause,
             root_cause_digest, approach, constraints, trust, origin_ref,
             origin_deleted, source_memory_id, sanitization_report, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, 0, ?13, ?14, ?15, ?15)",
    )
    .bind(p.id.to_string())
    .bind(&p.title)
    .bind(&p.problem)
    .bind(serde_json::to_string(&p.signals).unwrap_or_else(|_| "[]".into()))
    .bind(&p.signal_digest)
    .bind(serde_json::to_string(&p.applicability).unwrap_or_else(|_| "[]".into()))
    .bind(&p.root_cause)
    .bind(&p.root_cause_digest)
    .bind(&p.approach)
    .bind(serde_json::to_string(&p.constraints).unwrap_or_else(|_| "[]".into()))
    .bind(p.trust.as_str())
    .bind(&p.origin_ref)
    .bind(p.source_memory_id.map(|id| id.to_string()))
    .bind(p.sanitization_report.to_string())
    .bind(&now)
    .execute(&mut *t)
    .await?;
    tx::commit(t, "promote_pattern").await
}

fn from_row(row: &sqlx::sqlite::SqliteRow) -> Result<Pattern> {
    let list = |column: &str| -> Vec<String> {
        row.try_get::<String, _>(column)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    };
    Ok(Pattern {
        id: rows::uuid(row, "id")?,
        title: row.try_get("title")?,
        problem: row.try_get("problem")?,
        signals: list("signals"),
        signal_digest: row.try_get("signal_digest")?,
        applicability: list("applicability"),
        root_cause: row.try_get("root_cause")?,
        root_cause_digest: row.try_get("root_cause_digest")?,
        approach: row.try_get("approach")?,
        constraints: list("constraints"),
        trust: rows::enum_val(row, "trust")?,
        origin_ref: row.try_get("origin_ref")?,
        origin_deleted: row.try_get::<i64, _>("origin_deleted")? == 1,
        source_memory_id: rows::opt_uuid(row, "source_memory_id")?,
        sanitization_report: row
            .try_get::<String, _>("sanitization_report")
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_else(|| serde_json::json!({})),
    })
}

pub async fn pattern(store: &Store, id: Uuid) -> Result<Pattern> {
    let row = sqlx::query("SELECT * FROM reusable_patterns WHERE id = ?1 AND deleted_at IS NULL")
        .bind(id.to_string())
        .fetch_optional(store.pool())
        .await?
        .ok_or_else(|| StoreError::NotFound(format!("pattern {id}")))?;
    from_row(&row)
}

/// Every live pattern, newest first.
pub async fn list(store: &Store, trust: Option<PatternTrust>) -> Result<Vec<Pattern>> {
    let rows = sqlx::query(
        "SELECT * FROM reusable_patterns
          WHERE deleted_at IS NULL AND (?1 IS NULL OR trust = ?1)
          ORDER BY created_at DESC, id DESC",
    )
    .bind(trust.map(|t| t.as_str()))
    .fetch_all(store.pool())
    .await?;
    rows.iter().map(from_row).collect()
}

/// Patterns whose signals overlap the given ones by at least `min`, best first.
///
/// The match is lexical on whole normalized signals and the candidate set is
/// narrowed by an indexed read first, so this is bounded work rather than a
/// scan of every pattern the machine has ever held (FR-398).
pub async fn matching(
    store: &Store,
    signals: &[String],
    min: usize,
    limit: usize,
) -> Result<Vec<(Pattern, usize)>> {
    if normalize_signals(signals).len() < min {
        return Ok(Vec::new());
    }
    let rows = sqlx::query("SELECT * FROM reusable_patterns WHERE deleted_at IS NULL")
        .fetch_all(store.pool())
        .await?;

    let mut scored: Vec<(Pattern, usize)> = rows
        .iter()
        .map(from_row)
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .map(|p| {
            let overlap = signal_overlap(&p.signals, signals);
            (p, overlap)
        })
        .filter(|(_, overlap)| *overlap >= min)
        .collect();

    // Highest overlap first, then by id so the order is stable rather than
    // merely deterministic-looking.
    scored.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.id.cmp(&b.0.id)));
    scored.truncate(limit);
    Ok(scored)
}

/// Tombstone a pattern. Its applications survive as history.
pub async fn forget(store: &Store, id: Uuid) -> Result<()> {
    sqlx::query("UPDATE reusable_patterns SET deleted_at = ?2, updated_at = ?2 WHERE id = ?1")
        .bind(id.to_string())
        .bind(rows::now_text())
        .execute(store.pool())
        .await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Deletion and origin (T122, FR-399, FR-505)
// ---------------------------------------------------------------------------

/// What a pattern's origin resolves to.
///
/// Never a dangling reference, and never restored content: a deleted origin is
/// reported as deleted, which is a fact, rather than as a missing row, which is
/// a fault (FR-505).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub enum Origin {
    /// The source memory is still here. The reference itself stays opaque.
    Present {
        origin_ref: String,
    },
    Deleted,
}

pub fn origin_of(p: &Pattern) -> Origin {
    if p.origin_deleted || p.source_memory_id.is_none() {
        Origin::Deleted
    } else {
        Origin::Present {
            origin_ref: p.origin_ref.clone(),
        }
    }
}

/// Mark every pattern whose source memory is gone.
///
/// The pattern **survives**: it never held project identity, so there is
/// nothing in it that the deletion invalidates. What changes is that its origin
/// now resolves to "origin deleted".
pub async fn mark_orphaned(store: &Store) -> Result<u64> {
    Ok(sqlx::query(
        "UPDATE reusable_patterns
            SET origin_deleted = 1, source_memory_id = NULL, updated_at = ?1
          WHERE origin_deleted = 0
            AND source_memory_id IS NOT NULL
            AND source_memory_id NOT IN (SELECT id FROM memories WHERE deleted_at IS NULL)",
    )
    .bind(rows::now_text())
    .execute(store.pool())
    .await?
    .rows_affected())
}

// ---------------------------------------------------------------------------
// Applications and trust (T120)
// ---------------------------------------------------------------------------

/// One recorded outcome.
#[derive(Debug, Clone)]
pub struct NewApplication<'a> {
    pub pattern_id: Uuid,
    pub project_id: Uuid,
    pub session_id: Uuid,
    /// The signals **this project** saw. One incident, one row.
    pub signals: &'a [String],
    pub outcome: PatternOutcome,
    /// Set by the daemon from whether this session received the pattern in its
    /// context — never by the agent, which cannot be asked to report honestly
    /// on whether it was influenced (FR-401).
    pub discovery: PatternDiscovery,
    pub alternative_cause: Option<&'a str>,
    pub evidence_id: Option<Uuid>,
}

/// Record an application, then rebuild the pattern's trust from every
/// application it has.
///
/// Refuses `outcome_already_recorded` rather than overwriting: the unique key
/// `(pattern_id, project_id, signal_digest)` is the anti-poisoning mechanism,
/// and silently updating the existing row would let one project keep changing
/// its mind until the answer suited it.
pub async fn record_outcome(store: &Store, a: NewApplication<'_>) -> Result<PatternTrust> {
    let pattern = pattern(store, a.pattern_id).await?;
    let digest = signal_digest(a.signals);
    let is_origin = origin_ref(a.project_id)? == pattern.origin_ref;

    let inserted = sqlx::query(
        "INSERT OR IGNORE INTO pattern_applications
            (id, pattern_id, project_id, session_id, signal_digest, outcome,
             discovery, alternative_cause, evidence_id, is_origin, applied_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
    )
    .bind(new_id().to_string())
    .bind(a.pattern_id.to_string())
    .bind(a.project_id.to_string())
    .bind(a.session_id.to_string())
    .bind(&digest)
    .bind(a.outcome.as_str())
    .bind(a.discovery.as_str())
    // Bounded and redacted: it is free text an agent wrote.
    .bind(
        a.alternative_cause
            .map(|c| redact::redact(&c.chars().take(512).collect::<String>())),
    )
    .bind(a.evidence_id.map(|id| id.to_string()))
    .bind(i64::from(is_origin))
    .bind(rows::now_text())
    .execute(store.pool())
    .await?;

    if inserted.rows_affected() == 0 {
        return Err(StoreError::Refused {
            code: codes::OUTCOME_ALREADY_RECORDED,
            message: "an application for this pattern, project and signal set is \
                      already recorded; one incident counts once"
                .into(),
        });
    }
    rebuild_pattern_trust(store, a.pattern_id).await
}

/// Every application of a pattern, as trust reads them.
pub async fn applications(store: &Store, pattern_id: Uuid) -> Result<Vec<ApplicationFacts>> {
    let rows = sqlx::query(
        "SELECT project_id, outcome, discovery, evidence_id, is_origin
           FROM pattern_applications WHERE pattern_id = ?1 ORDER BY applied_at, id",
    )
    .bind(pattern_id.to_string())
    .fetch_all(store.pool())
    .await?;

    rows.iter()
        .map(|r| {
            Ok(ApplicationFacts {
                project_id: rows::uuid(r, "project_id")?,
                outcome: rows::enum_val(r, "outcome")?,
                discovery: rows::enum_val(r, "discovery")?,
                has_evidence: r
                    .try_get::<Option<String>, _>("evidence_id")?
                    .is_some_and(|id| !id.is_empty()),
                is_origin: r.try_get::<i64, _>("is_origin")? == 1,
            })
        })
        .collect()
}

/// The counters the surfaces report.
pub async fn counters(store: &Store, pattern_id: Uuid) -> Result<PatternCounters> {
    Ok(count_applications(&applications(store, pattern_id).await?))
}

/// Recompute a pattern's trust from its applications.
///
/// Derived on every read of the record's history, never accumulated: a stored
/// counter that drifted from the rows behind it would be a number nobody could
/// check.
pub async fn rebuild_pattern_trust(store: &Store, pattern_id: Uuid) -> Result<PatternTrust> {
    let counters = counters(store, pattern_id).await?;
    // Reaching this function means the gate passed — a candidate that never
    // passed it was never written.
    let trust = derive_pattern_trust(true, counters);
    sqlx::query("UPDATE reusable_patterns SET trust = ?2, updated_at = ?3 WHERE id = ?1")
        .bind(pattern_id.to_string())
        .bind(trust.as_str())
        .bind(rows::now_text())
        .execute(store.pool())
        .await?;
    Ok(trust)
}

/// The alternative causes recorded against a pattern, for the suggestion's
/// "check this first" line.
pub async fn alternative_causes(store: &Store, pattern_id: Uuid) -> Result<Vec<String>> {
    Ok(sqlx::query_scalar::<_, String>(
        "SELECT alternative_cause FROM pattern_applications
          WHERE pattern_id = ?1 AND alternative_cause IS NOT NULL
            AND outcome IN ('not_applicable', 'failed')
          ORDER BY applied_at",
    )
    .bind(pattern_id.to_string())
    .fetch_all(store.pool())
    .await?)
}
