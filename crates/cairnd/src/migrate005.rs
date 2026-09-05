//! The client's migration path from Feature 004 to Feature 005
//! (`contracts/migration-cutover.md` §4–§9; FR-861–FR-878).
//!
//! # Two mechanisms, and this is the second one
//!
//! The server's authority mode is an admin's single switch (§1–§3). This is the
//! other machine entirely: a per-store, resumable procedure that hands durable
//! knowledge over and then — only then — stops treating the local copy as the
//! authority. The two never call each other, and FR-876e requires retiring the
//! first without removing the second, which is why a store can run this path
//! before its server cuts over or long afterwards.
//!
//! # The ordering is the safety property
//!
//! ```text
//! inspect → claim patterns → drain → possession → switch → recheck → demote
//! ```
//!
//! Every arrow is a precondition, and two of them carry the whole guarantee:
//!
//! - **Nothing is demoted before possession is established.** "Delivered" and
//!   "durably held" are different facts, and only the second authorizes
//!   demotion (§5). A store that demoted on a `200` from the drain would be
//!   trusting an acknowledgement with the last copy of a record.
//! - **Possession is re-checked immediately before demotion** (§12.3). Phases 3
//!   and 5 are separated by a switch that can take arbitrary time, and a
//!   server-side loss in between would otherwise demote the last copy on the
//!   strength of a check that has gone stale.
//!
//! # Resumability is the ordinary mode
//!
//! Interruption at any point leaves `migration_state` at whatever phase last
//! committed `done`; re-running re-enters at the first phase that is not done
//! and never skips ahead (§7). Every phase is independently idempotent, so
//! "resume" and "run again" are the same operation.
//!
//! # What this module deliberately does not do
//!
//! It does not decide ownership of a legacy pattern. `reusable_patterns` has no
//! owner column, a Feature 004 store may have been used with several accounts,
//! and there is therefore no truthful automatic assignment (§4.1a). Ownership
//! comes from an explicit claim the authenticated account makes, persisted
//! before anything is delivered, and read back at retry time rather than
//! recomputed from whichever credential is active then.

use cairn_core::domain::{KnowledgeDomain, RelationKind, RelationRef};
use cairn_store::migrate::{
    self as mig, ClaimOutcome, Phase, PhaseState, RetainedReason, RetainedRef,
};
use cairn_store::{Result, Store, StoreError};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::str::FromStr;
use uuid::Uuid;

use crate::sync::{legacy_row_eligibility, pattern_eligibility, Eligibility, PatternEligibility};

/// The possession check's per-call bound (§5), matching the batch-bounding
/// discipline `safe-events.md` §7 already sets.
pub const POSSESSION_BATCH: usize = 500;

/// The entity types the drain carries (§12.0). `pattern_application` is
/// deliberately absent: it is machine-local evidence (FR-707) and never drains.
pub const DRAINED: [&str; 5] = [
    "memory",
    "memory_relation",
    "personal_knowledge",
    "team_knowledge",
    "pattern",
];

// ---------------------------------------------------------------------------
// Naming a record
// ---------------------------------------------------------------------------

/// One record, named by its own reference shape (§5).
///
/// A relation is **not** a `KnowledgeRef`: it has no id of its own, it *is* the
/// `(from, to, kind)` triple, and giving it a surrogate id would create a
/// second way to name one edge for two writers to disagree about.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum RecordRef {
    Knowledge { domain: KnowledgeDomain, id: Uuid },
    Pattern(Uuid),
    Relation(RelationRef),
}

impl RecordRef {
    /// The wire shape the possession endpoint reads and answers with.
    pub fn to_json(&self) -> Value {
        match self {
            RecordRef::Knowledge { domain, id } => json!({
                "ref_kind": "knowledge", "domain": domain.as_str(), "id": id
            }),
            RecordRef::Pattern(id) => json!({ "ref_kind": "pattern", "id": id }),
            RecordRef::Relation(r) => json!({
                "ref_kind": "relation",
                "from": r.from_memory_id,
                "to": r.to_memory_id,
                "kind": r.kind.as_str(),
            }),
        }
    }

    pub fn from_json(v: &Value) -> Option<Self> {
        let id = |k: &str| {
            v.get(k)
                .and_then(Value::as_str)
                .and_then(|s| s.parse().ok())
        };
        match v.get("ref_kind").and_then(Value::as_str)? {
            "knowledge" => Some(RecordRef::Knowledge {
                domain: KnowledgeDomain::from_str(v.get("domain")?.as_str()?).ok()?,
                id: id("id")?,
            }),
            "pattern" => Some(RecordRef::Pattern(id("id")?)),
            "relation" => Some(RecordRef::Relation(RelationRef {
                from_memory_id: id("from")?,
                to_memory_id: id("to")?,
                kind: RelationKind::from_str(v.get("kind")?.as_str()?).ok()?,
            })),
            _ => None,
        }
    }

    /// The same record, as the local retained-local table names it.
    pub fn retained(&self) -> RetainedRef {
        match self {
            RecordRef::Knowledge { domain, id } => RetainedRef::Knowledge {
                domain: *domain,
                id: *id,
            },
            RecordRef::Pattern(id) => RetainedRef::Pattern(*id),
            RecordRef::Relation(r) => RetainedRef::Relation(*r),
        }
    }
}

// ---------------------------------------------------------------------------
// What the migration needs from a server
// ---------------------------------------------------------------------------

/// One record offered to the migration-scoped drain.
#[derive(Debug, Clone)]
pub struct DrainItem {
    pub entity_type: &'static str,
    pub entity_id: String,
    pub payload: Value,
    /// How the record will be named at possession time. Carried alongside so a
    /// refusal can be retained under the same reference the check would use,
    /// rather than reconstructed from the entity type twice.
    pub reference: RecordRef,
}

/// What the server said about one offered record.
#[derive(Debug, Clone)]
pub struct DrainOutcome {
    pub entity_id: String,
    pub accepted: bool,
    /// The server's own rejection reason, verbatim (§4.3).
    pub reason: Option<String>,
}

/// The three answers, and only three (§5, §12.5).
#[derive(Debug, Default, Clone)]
pub struct Possession {
    pub held: Vec<RecordRef>,
    pub missing: Vec<RecordRef>,
    pub indeterminate: Vec<RecordRef>,
}

/// The server operations the migration performs, as an interface.
///
/// Injected rather than called directly so the state machine can be driven
/// against a server that refuses, loses a record between two checks, or answers
/// `indeterminate` — none of which a real server can be asked to do on cue, and
/// all of which the contract has a defined behaviour for.
#[allow(async_fn_in_trait)]
pub trait Remote {
    /// Register this store's migration and return its token (§12.1).
    async fn register(&self, writer_id: &str) -> Result<String>;
    async fn drain(&self, token: &str, items: &[DrainItem]) -> Result<Vec<DrainOutcome>>;
    async fn possession(&self, records: &[RecordRef]) -> Result<Possession>;
    async fn complete(&self, token: &str) -> Result<()>;
}

// ---------------------------------------------------------------------------
// Phase 1 — inspect (§4.1)
// ---------------------------------------------------------------------------

/// What the store holds, counted before anything changes.
#[derive(Debug, Default, Clone, serde::Serialize)]
pub struct InspectReport {
    /// Row counts per drained record type, by the record's own state column.
    pub records: BTreeMap<String, i64>,
    /// Outbox rows by state.
    pub outbox: BTreeMap<String, i64>,
    /// Queued rows carrying no `authored_by_user_id`.
    pub outbox_without_author: i64,
    /// Project memories marked `local_only`: nothing canonical to defer to.
    pub local_only_memories: i64,
    /// Local patterns eligible for an ownership claim, with their **historical
    /// owner labelled `unknown`** — shipped `reusable_patterns` has no owner
    /// column, and inferring one from the current credential is exactly the
    /// untruth §4.1a exists to prevent.
    pub patterns_eligible_for_claim: Vec<Uuid>,
}

pub async fn inspect(store: &Store) -> Result<InspectReport> {
    mig::phase_begin(store, Phase::Inspect).await?;

    let mut report = InspectReport::default();
    for (label, sql) in [
        (
            "memory",
            "SELECT state, count(*) FROM memories WHERE deleted_at IS NULL GROUP BY state",
        ),
        (
            "memory_relation",
            "SELECT 'active', count(*) FROM memory_relations WHERE deleted_at IS NULL",
        ),
        (
            "personal_knowledge",
            "SELECT CASE WHEN forgotten_at IS NULL THEN 'active' ELSE 'forgotten' END,
                    count(*) FROM personal_knowledge GROUP BY 1",
        ),
        (
            "team_knowledge",
            "SELECT state, count(*) FROM team_knowledge GROUP BY state",
        ),
        (
            "pattern",
            "SELECT 'active', count(*) FROM reusable_patterns WHERE deleted_at IS NULL",
        ),
    ] {
        let rows: Vec<(String, i64)> = sqlx::query_as(sql)
            .fetch_all(store.pool())
            .await
            .map_err(StoreError::Sqlx)?;
        for (state, n) in rows {
            report.records.insert(format!("{label}:{state}"), n);
        }
    }

    let rows: Vec<(String, i64)> =
        sqlx::query_as("SELECT state, count(*) FROM outbox GROUP BY state")
            .fetch_all(store.pool())
            .await
            .map_err(StoreError::Sqlx)?;
    report.outbox = rows.into_iter().collect();

    report.outbox_without_author = scalar(
        store,
        "SELECT count(*) FROM outbox
          WHERE authored_by_user_id IS NULL AND state IN ('pending','failed','blocked')",
    )
    .await?;
    report.local_only_memories = scalar(
        store,
        "SELECT count(*) FROM memories WHERE local_only = 1 AND deleted_at IS NULL",
    )
    .await?;
    report.patterns_eligible_for_claim = mig::unclaimed_patterns(store).await?;

    // The one write phase 1 makes beyond its own row: the store is now
    // migrating. Recorded before the phase is marked done so an interruption
    // between the two re-enters at inspect rather than at a later phase against
    // a store still calling itself `feature_004`.
    if cairn_store::authority::mode(store).await?
        == cairn_store::authority::AuthorityMode::Feature004
    {
        cairn_store::authority::set_mode(store, cairn_store::authority::AuthorityMode::Migrating)
            .await?;
    }
    let counted = report.records.values().sum::<i64>();
    mig::phase_finish(store, Phase::Inspect, PhaseState::Done, counted).await?;
    Ok(report)
}

async fn scalar(store: &Store, sql: &str) -> Result<i64> {
    sqlx::query_scalar(sql)
        .fetch_one(store.pool())
        .await
        .map_err(StoreError::Sqlx)
}

// ---------------------------------------------------------------------------
// Phase 1a — establish legacy pattern ownership, once (§4.1a, FR-867b)
// ---------------------------------------------------------------------------

/// What one local pattern's claim attempt produced.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ClaimReportRow {
    pub local_pattern_id: Uuid,
    /// `claimed` | `already_owned` | `legacy_pattern_already_claimed`
    pub outcome: &'static str,
    pub pattern_id: Option<Uuid>,
    pub owner_user_id: Option<Uuid>,
}

/// Claim the named local patterns for `account`.
///
/// Everything about this is deliberate. The content key and the pattern id are
/// computed **here**, from the local row's content and the authenticated
/// account, and persisted in the same local transaction as the claim — before
/// anything is deliverable. Drain then reads all three persisted values. It
/// never recomputes the owner from the credential active at retry time, which
/// is what makes a credential switch unable to produce a second owner or a
/// second canonical pattern.
///
/// Repeating a claim by the same owner returns the persisted identity and
/// changes nothing. A different account gets `legacy_pattern_already_claimed`
/// and changes nothing.
pub async fn claim_patterns(
    store: &Store,
    account: Uuid,
    selection: &[Uuid],
) -> Result<Vec<ClaimReportRow>> {
    mig::phase_begin(store, Phase::ClaimPatternOwnership).await?;
    let mut out = Vec::new();
    for local in selection {
        let Some((problem, root_cause, approach)) = pattern_content(store, *local).await? else {
            continue;
        };
        let content_key = cairn_core::eventid::content_key(&problem, &root_cause, &approach);
        let pattern_id = cairn_core::eventid::pattern_id(account, &content_key);
        let outcome = mig::claim_pattern(store, *local, account, &content_key, pattern_id).await?;
        let row = match outcome {
            ClaimOutcome::Claimed(c) => {
                // A pattern that was retained only because nobody had claimed
                // it is no longer unclaimed. Leaving the row would report it
                // as retained forever, which is a lie one claim later.
                let _ = mig::release_retained(store, &RetainedRef::Pattern(*local)).await?;
                ClaimReportRow {
                    local_pattern_id: *local,
                    outcome: "claimed",
                    pattern_id: Some(c.pattern_id),
                    owner_user_id: Some(c.owner_user_id),
                }
            }
            ClaimOutcome::AlreadyOwned(c) => ClaimReportRow {
                local_pattern_id: *local,
                outcome: "already_owned",
                pattern_id: Some(c.pattern_id),
                owner_user_id: Some(c.owner_user_id),
            },
            ClaimOutcome::HeldByAnother { owner_user_id } => ClaimReportRow {
                local_pattern_id: *local,
                outcome: "legacy_pattern_already_claimed",
                pattern_id: None,
                owner_user_id: Some(owner_user_id),
            },
        };
        out.push(row);
    }
    // Completing the phase means every legacy pattern is **either** backed by a
    // persisted claim **or** named as retained-local (§4.1a). Closing it after
    // claiming three of five would leave the other two claimed by nobody and
    // reported by nothing — present in the store, absent from the migration,
    // and invisible to `--status`.
    settle_unclaimed_patterns(store).await?;
    let claimed = out
        .iter()
        .filter(|r| r.outcome != "legacy_pattern_already_claimed")
        .count();
    mig::phase_finish(
        store,
        Phase::ClaimPatternOwnership,
        PhaseState::Done,
        claimed as i64,
    )
    .await?;
    Ok(out)
}

/// Close phase 1a for a run that claims nothing further.
///
/// Completing the phase means every legacy pattern is **either** backed by one
/// persisted claim **or** named as retained-local — not that every pattern was
/// claimed (§4.1a). An unclaimed pattern stays readable locally, is reported
/// individually, and is never queued, delivered, demoted or silently
/// attributed to whoever happens to be running the migration.
async fn settle_unclaimed_patterns(store: &Store) -> Result<i64> {
    let mut retained = 0;
    for local in mig::unclaimed_patterns(store).await? {
        if mig::retain(
            store,
            RetainedRef::Pattern(local),
            RetainedReason::OwnerUnclaimed,
        )
        .await?
        {
            retained += 1;
        }
    }
    Ok(retained)
}

async fn pattern_content(store: &Store, id: Uuid) -> Result<Option<(String, String, String)>> {
    let row: Option<(String, String, String)> = sqlx::query_as(
        "SELECT problem, root_cause, approach FROM reusable_patterns
          WHERE id = ?1 AND deleted_at IS NULL",
    )
    .bind(id.to_string())
    .fetch_optional(store.pool())
    .await
    .map_err(StoreError::Sqlx)?;
    Ok(row)
}

// ---------------------------------------------------------------------------
// Key normalization (§8, §12.4; FR-867a, SC-750)
// ---------------------------------------------------------------------------

/// What re-keying changed, and what it found.
#[derive(Debug, Default, Clone, serde::Serialize)]
pub struct KeyReport {
    /// Rows whose stored key differed from its normalized form.
    pub renormalized: i64,
    /// Collisions surfaced as `conflicts_with` relations.
    pub conflicts: i64,
    /// Collisions that were ordinary duplicates — same key, same value.
    pub duplicates: i64,
}

/// Re-normalize the `topic_key` and `value_key` of every record whose authority
/// is moving.
///
/// **Why it is load-bearing.** Duplicate and conflict detection match on
/// normalized keys. A legacy record still carrying an un-normalized key stops
/// colliding with a normalized candidate that means the same thing, so
/// detection degrades silently against exactly the knowledge users already
/// have — which is what SC-750 measures.
///
/// **Through the shipped normalizers, not a copy.** `normalize_topic_key` and
/// `normalize_value_key` from `cairn-core` are the same functions
/// `consolidation.md` §5 applies to new candidates. A migration-local variant
/// would be a second definition of sameness, and the two would diverge on the
/// first change to either. In particular the topic normalizer's treatment of
/// `.` is left exactly as shipped: `.` separates segments, and folding it would
/// rewrite `test.command` to `test_command` across every record in the store.
///
/// **What it is not.** Only the two key columns are rewritten. Domain, scope
/// and authorship are untouched (FR-867).
pub async fn normalize_keys(store: &Store) -> Result<KeyReport> {
    let mut report = KeyReport::default();

    // (table, id column, the columns that scope a collision)
    let tables: [(&str, &str, &[&str]); 3] = [
        ("memories", "id", &["project_id", "scope", "scope_key"]),
        ("personal_knowledge", "id", &["owner_user_id"]),
        ("team_knowledge", "id", &[]),
    ];

    for (table, id_col, scope_cols) in tables {
        let select_scope = if scope_cols.is_empty() {
            "''".to_string()
        } else {
            scope_cols.join(" || '|' || ")
        };
        let rows: Vec<(String, Option<String>, Option<String>, String)> = sqlx::query_as(&format!(
            "SELECT {id_col}, topic_key, value_key, {select_scope}
               FROM {table} WHERE topic_key IS NOT NULL"
        ))
        .fetch_all(store.pool())
        .await
        .map_err(StoreError::Sqlx)?;

        // scope + normalized topic → the rows already seen there
        let mut seen: BTreeMap<(String, String), Vec<(String, Option<String>)>> = BTreeMap::new();

        for (id, topic, value, scope) in rows {
            let Some(topic) = topic else { continue };
            let normalized_topic = cairn_core::knowledge::normalize_topic_key(&topic);
            let normalized_value = value
                .as_deref()
                .and_then(cairn_core::knowledge::normalize_value_key);
            // A key the shipped normalizer refuses is left exactly as it is.
            // Migration corrects keys; it does not delete a record for having
            // one it cannot express, and it does not invent a replacement.
            let Some(normalized_topic) = normalized_topic else {
                continue;
            };

            if normalized_topic != topic || normalized_value != value {
                sqlx::query(&format!(
                    "UPDATE {table} SET topic_key = ?1, value_key = ?2 WHERE {id_col} = ?3"
                ))
                .bind(&normalized_topic)
                .bind(normalized_value.as_deref())
                .bind(&id)
                .execute(store.pool())
                .await
                .map_err(StoreError::Sqlx)?;
                report.renormalized += 1;
            }

            let bucket = seen
                .entry((scope.clone(), normalized_topic.clone()))
                .or_default();
            for (other_id, other_value) in bucket.iter() {
                if other_value == &normalized_value {
                    // Same key, same value: an ordinary duplicate. Left to the
                    // reinforcement path that handles every other duplicate;
                    // migration records that it saw one and discards neither.
                    report.duplicates += 1;
                } else {
                    record_conflict(store, table, other_id, &id).await?;
                    report.conflicts += 1;
                }
            }
            bucket.push((id, normalized_value));
        }
    }

    Ok(report)
}

/// Surface a collision as a conflict, through the relation table that already
/// carries every other conflict for this record type.
///
/// `deterministic_rule` is the basis because a key collision *is* a
/// deterministic rule firing — the same basis `consolidation.md` §5 rows 6–7
/// assign when the same collision is found on a new candidate. Nothing is
/// discarded to resolve it: both records stay, related.
async fn record_conflict(store: &Store, table: &str, from: &str, to: &str) -> Result<()> {
    let now = chrono::Utc::now().to_rfc3339();
    match table {
        "memories" => {
            let project: Option<String> =
                sqlx::query_scalar("SELECT project_id FROM memories WHERE id = ?1")
                    .bind(from)
                    .fetch_optional(store.pool())
                    .await
                    .map_err(StoreError::Sqlx)?;
            let session: Option<String> =
                sqlx::query_scalar("SELECT origin_session_id FROM memories WHERE id = ?1")
                    .bind(from)
                    .fetch_optional(store.pool())
                    .await
                    .map_err(StoreError::Sqlx)?;
            let (Some(project), Some(session)) = (project, session) else {
                return Ok(());
            };
            sqlx::query(
                "INSERT OR IGNORE INTO memory_relations
                     (from_memory_id, to_memory_id, kind, project_id, decided_by_session,
                      decided_at, basis)
                 VALUES (?1, ?2, 'conflicts_with', ?3, ?4, ?5, 'deterministic_rule')",
            )
            .bind(from)
            .bind(to)
            .bind(project)
            .bind(session)
            .bind(&now)
            .execute(store.pool())
            .await
            .map_err(StoreError::Sqlx)?;
        }
        "personal_knowledge" | "team_knowledge" => {
            let relations = format!("{table}_relations");
            let writer = writer_id(store).await?;
            sqlx::query(&format!(
                "INSERT OR IGNORE INTO {relations}
                     (from_id, to_id, kind, basis, decided_by_writer, decided_at)
                 VALUES (?1, ?2, 'conflicts_with', 'deterministic_rule', ?3, ?4)"
            ))
            .bind(from)
            .bind(to)
            .bind(writer)
            .bind(&now)
            .execute(store.pool())
            .await
            .map_err(StoreError::Sqlx)?;
        }
        _ => {}
    }
    Ok(())
}

async fn writer_id(store: &Store) -> Result<String> {
    let id: Option<String> = sqlx::query_scalar("SELECT writer_id FROM writer_identity LIMIT 1")
        .fetch_optional(store.pool())
        .await
        .map_err(StoreError::Sqlx)?;
    Ok(id.unwrap_or_else(|| "unknown-writer".to_string()))
}

// ---------------------------------------------------------------------------
// Phase 2 — drain (§4.2, §4.3, §12.0)
// ---------------------------------------------------------------------------

/// One record the drain did not deliver, reported individually (FR-873).
#[derive(Debug, Clone, serde::Serialize)]
pub struct BlockedRow {
    pub entity_type: String,
    pub entity_id: String,
    /// `no_recorded_author` | `owner_unclaimed` | `author_mismatch`
    /// | `server_rejected: <reason>` | `capability_blocked`
    pub reason: String,
}

#[derive(Debug, Default, Clone, serde::Serialize)]
pub struct DrainReport {
    pub delivered: i64,
    pub blocked: Vec<BlockedRow>,
    /// Every record the server accepted, for phase 3 to check possession of.
    #[serde(skip)]
    pub delivered_refs: Vec<RecordRef>,
}

/// Push every knowledge-bearing record this account may hand over.
///
/// **The payload is built from the record, not from the queued row.** A legacy
/// outbox payload was serialized before this migration re-keyed anything, so
/// replaying it would deliver the un-normalized keys §8 exists to correct, and
/// SC-750 would fail against the very corpus it measures. The queued row still
/// decides *eligibility* — it is where the recorded author lives — and is
/// marked delivered on success; it just does not decide content.
pub async fn drain<R: Remote>(
    store: &Store,
    remote: &R,
    token: &str,
    account: Uuid,
) -> Result<DrainReport> {
    mig::phase_begin(store, Phase::Drain).await?;
    let mut report = DrainReport::default();
    let mut items: Vec<DrainItem> = Vec::new();

    // Where a record has a queued row, that row's namespace and recorded author
    // decide who may drain it.
    let queued = queued_rows(store).await?;

    for (id, project_id, payload, reference) in project_memories(store).await? {
        match queued.get(&id) {
            Some((namespace, author)) => {
                match legacy_row_eligibility(namespace, *author, account) {
                    Eligibility::Eligible => {}
                    Eligibility::NoRecordedAuthor => {
                        report.blocked.push(BlockedRow {
                            entity_type: "memory".into(),
                            entity_id: id.clone(),
                            reason: "no_recorded_author".into(),
                        });
                        continue;
                    }
                    Eligibility::AuthorMismatch { .. } => {
                        report.blocked.push(BlockedRow {
                            entity_type: "memory".into(),
                            entity_id: id.clone(),
                            reason: "author_mismatch".into(),
                        });
                        continue;
                    }
                }
            }
            None => {}
        }
        let _ = project_id;
        items.push(DrainItem {
            entity_type: "memory",
            entity_id: id,
            payload,
            reference,
        });
    }

    for (key, payload, reference) in relations(store).await? {
        items.push(DrainItem {
            entity_type: "memory_relation",
            entity_id: key,
            payload,
            reference,
        });
    }

    for (id, owner, payload, reference) in personal_rows(store).await? {
        if owner != account {
            report.blocked.push(BlockedRow {
                entity_type: "personal_knowledge".into(),
                entity_id: id,
                reason: "author_mismatch".into(),
            });
            continue;
        }
        items.push(DrainItem {
            entity_type: "personal_knowledge",
            entity_id: id,
            payload,
            reference,
        });
    }

    for (id, proposer, state, payload, reference) in team_rows(store).await? {
        // A proposal is its author's to hand over. An authoritative or retired
        // entry belongs to the corpus every account on the server shares.
        if state == "proposed" && proposer != account {
            report.blocked.push(BlockedRow {
                entity_type: "team_knowledge".into(),
                entity_id: id,
                reason: "author_mismatch".into(),
            });
            continue;
        }
        items.push(DrainItem {
            entity_type: "team_knowledge",
            entity_id: id,
            payload,
            reference,
        });
    }

    for local in local_patterns(store).await? {
        let claim = mig::pattern_claim(store, local).await?;
        match pattern_eligibility(claim.as_ref(), account) {
            PatternEligibility::Eligible {
                pattern_id,
                content_key,
            } => {
                let Some(payload) = pattern_payload(store, local, &content_key).await? else {
                    continue;
                };
                items.push(DrainItem {
                    entity_type: "pattern",
                    entity_id: pattern_id.to_string(),
                    payload,
                    reference: RecordRef::Pattern(pattern_id),
                });
            }
            PatternEligibility::OwnerUnclaimed => report.blocked.push(BlockedRow {
                entity_type: "pattern".into(),
                entity_id: local.to_string(),
                reason: "owner_unclaimed".into(),
            }),
            PatternEligibility::AuthorMismatch { .. } => report.blocked.push(BlockedRow {
                entity_type: "pattern".into(),
                entity_id: local.to_string(),
                reason: "author_mismatch".into(),
            }),
        }
    }

    // `pattern_application` is the one shape that must never reach here: it is
    // machine-local evidence (FR-707), and the six local names it travels with
    // are refused at the privacy boundary. Asserting the set rather than
    // trusting the five call sites above keeps a later addition from quietly
    // widening what migration sends.
    items.retain(|i| DRAINED.contains(&i.entity_type));

    // Bounded the same way possession is, so one enormous store does not send
    // one enormous request.
    for chunk in items.chunks(POSSESSION_BATCH) {
        let outcomes = remote.drain(token, chunk).await?;
        let by_id: BTreeMap<&str, &DrainOutcome> =
            outcomes.iter().map(|o| (o.entity_id.as_str(), o)).collect();
        for item in chunk {
            match by_id.get(item.entity_id.as_str()) {
                Some(o) if o.accepted => {
                    report.delivered += 1;
                    report.delivered_refs.push(item.reference.clone());
                    mark_delivered(store, &item.entity_id).await?;
                }
                Some(o) => {
                    let reason = o.reason.clone().unwrap_or_else(|| "unspecified".into());
                    report.blocked.push(BlockedRow {
                        entity_type: item.entity_type.into(),
                        entity_id: item.entity_id.clone(),
                        reason: format!("server_rejected: {reason}"),
                    });
                    // A record the server explicitly refused is retained with
                    // that reason and reported. It is a defect signal, not a
                    // reason to lose the only copy (§6, second row).
                    mig::retain(
                        store,
                        item.reference.retained(),
                        RetainedReason::ServerRefused,
                    )
                    .await?;
                }
                None => report.blocked.push(BlockedRow {
                    entity_type: item.entity_type.into(),
                    entity_id: item.entity_id.clone(),
                    reason: "server_rejected: no answer for this record".into(),
                }),
            }
        }
    }

    // Local-only project memories were never eligible to move: there is nothing
    // canonical for them to defer to, so they are retained rather than
    // transferred, permanently excluded from demotion (§6, first row).
    for id in local_only_memories(store).await? {
        mig::retain(
            store,
            RetainedRef::Knowledge {
                domain: KnowledgeDomain::Project,
                id,
            },
            RetainedReason::LocalOnly,
        )
        .await?;
    }

    let state = if report.blocked.is_empty() {
        PhaseState::Done
    } else {
        // Blocked, and the migration still proceeds for what did drain: one
        // un-transferable record must not hold a whole store hostage (§12.2).
        PhaseState::Blocked
    };
    mig::phase_finish(store, Phase::Drain, state, report.blocked.len() as i64).await?;
    Ok(report)
}

async fn mark_delivered(store: &Store, entity_id: &str) -> Result<()> {
    sqlx::query(
        "UPDATE outbox SET state = 'delivered', delivered_at = ?2
          WHERE entity_id = ?1 AND state IN ('pending','in_flight','failed','blocked')",
    )
    .bind(entity_id)
    .bind(chrono::Utc::now().to_rfc3339())
    .execute(store.pool())
    .await
    .map_err(StoreError::Sqlx)?;
    Ok(())
}

/// Queued rows still awaiting delivery, by entity id: `(namespace, author)`.
async fn queued_rows(store: &Store) -> Result<BTreeMap<String, (String, Option<Uuid>)>> {
    let rows: Vec<(String, String, Option<String>)> = sqlx::query_as(
        "SELECT entity_id, namespace, authored_by_user_id FROM outbox
          WHERE state IN ('pending','in_flight','failed','blocked')",
    )
    .fetch_all(store.pool())
    .await
    .map_err(StoreError::Sqlx)?;
    Ok(rows
        .into_iter()
        .map(|(id, ns, author)| (id, (ns, author.and_then(|a| a.parse().ok()))))
        .collect())
}

// ---------------------------------------------------------------------------
// Reading the records out of the store
// ---------------------------------------------------------------------------

type MemoryRow = (String, String, Value, RecordRef);

/// Project memories in linked projects, with the **server's** project id.
///
/// The server needs the id it knows the project by, and an unlinked project has
/// none — so a memory in one is not drainable at all. It is not lost: the join
/// below drops it here and the local-only sweep at the end of the drain retains
/// it, which is the honest disposition for a record with nothing canonical to
/// defer to (§6, first row).
async fn project_memories(store: &Store) -> Result<Vec<MemoryRow>> {
    let rows: Vec<(
        String,
        String,
        String,
        String,
        String,
        String,
        String,
        Option<String>,
        Option<String>,
        String,
    )> = sqlx::query_as(
        "SELECT m.id, p.server_project_id, m.type, m.scope, m.scope_key, m.content,
                    m.state, m.topic_key, m.value_key, m.origin_session_id
               FROM memories m
               JOIN projects p ON p.id = m.project_id
              WHERE m.deleted_at IS NULL AND m.local_only = 0
                AND p.server_project_id IS NOT NULL
              ORDER BY m.created_at, m.id",
    )
    .fetch_all(store.pool())
    .await
    .map_err(StoreError::Sqlx)?;
    Ok(rows
        .into_iter()
        .filter_map(
            |(id, project, kind, scope, scope_key, content, state, topic, value, session)| {
                let uuid: Uuid = id.parse().ok()?;
                Some((
                    id.clone(),
                    project.clone(),
                    json!({
                        "id": id, "project_id": project, "type": kind, "scope": scope,
                        "scope_key": scope_key, "content": content, "state": state,
                        "topic_key": topic, "value_key": value,
                        // The server requires a memory to name the session it
                        // came from, and a migrated memory's provenance is the
                        // session that wrote it, not the one migrating it.
                        "provenance": { "session_id": session,
                                        "observation_ids": [],
                                        "evidence_count": 0 },
                    }),
                    RecordRef::Knowledge {
                        domain: KnowledgeDomain::Project,
                        id: uuid,
                    },
                ))
            },
        )
        .collect())
}

async fn local_only_memories(store: &Store) -> Result<Vec<Uuid>> {
    let rows: Vec<String> = sqlx::query_scalar(
        "SELECT m.id FROM memories m
           JOIN projects p ON p.id = m.project_id
          WHERE m.deleted_at IS NULL
            AND (m.local_only = 1 OR p.server_project_id IS NULL)",
    )
    .fetch_all(store.pool())
    .await
    .map_err(StoreError::Sqlx)?;
    Ok(rows.into_iter().filter_map(|s| s.parse().ok()).collect())
}

async fn relations(store: &Store) -> Result<Vec<(String, Value, RecordRef)>> {
    let rows: Vec<(String, String, String, String, String, String)> = sqlx::query_as(
        "SELECT r.from_memory_id, r.to_memory_id, r.kind, p.server_project_id, r.basis,
                r.decided_by_session
           FROM memory_relations r
           JOIN projects p ON p.id = r.project_id
          WHERE r.deleted_at IS NULL AND p.server_project_id IS NOT NULL",
    )
    .fetch_all(store.pool())
    .await
    .map_err(StoreError::Sqlx)?;
    let mut out = Vec::new();
    for (from, to, kind, project, basis, session) in rows {
        let (Ok(from_id), Ok(to_id), Ok(relation_kind)) = (
            from.parse::<Uuid>(),
            to.parse::<Uuid>(),
            RelationKind::from_str(&kind),
        ) else {
            continue;
        };
        let reference = RelationRef {
            from_memory_id: from_id,
            to_memory_id: to_id,
            kind: relation_kind,
        };
        out.push((
            // The natural key, spelled the one way the whole system spells it.
            reference.relation_key(),
            json!({
                "from_memory_id": from, "to_memory_id": to, "kind": kind,
                "project_id": project, "basis": basis,
                "decided_by_session": session,
            }),
            RecordRef::Relation(reference),
        ));
    }
    Ok(out)
}

async fn personal_rows(store: &Store) -> Result<Vec<(String, Uuid, Value, RecordRef)>> {
    let rows: Vec<(
        String,
        String,
        String,
        String,
        Option<String>,
        Option<String>,
        String,
        i64,
    )> = sqlx::query_as(
        "SELECT id, owner_user_id, knowledge_type, content, topic_key, value_key,
                    writer_id, writer_seq
               FROM personal_knowledge WHERE forgotten_at IS NULL ORDER BY created_at, id",
    )
    .fetch_all(store.pool())
    .await
    .map_err(StoreError::Sqlx)?;
    Ok(rows
        .into_iter()
        .filter_map(|(id, owner, kind, content, topic, value, writer, seq)| {
            let uuid: Uuid = id.parse().ok()?;
            Some((
                id.clone(),
                owner.parse().ok()?,
                json!({
                    "id": id, "knowledge_type": kind, "content": content,
                    "topic_key": topic, "value_key": value,
                    "writer_id": writer, "writer_seq": seq, "applicability": [],
                }),
                RecordRef::Knowledge {
                    domain: KnowledgeDomain::Personal,
                    id: uuid,
                },
            ))
        })
        .collect())
}

type TeamRow = (String, Uuid, String, Value, RecordRef);

async fn team_rows(store: &Store) -> Result<Vec<TeamRow>> {
    let rows: Vec<(
        String,
        String,
        String,
        Option<String>,
        Option<String>,
        String,
        String,
        String,
        i64,
    )> = sqlx::query_as(
        "SELECT id, knowledge_type, content, topic_key, value_key, state,
                    proposed_by_user_id, writer_id, writer_seq
               FROM team_knowledge ORDER BY created_at, id",
    )
    .fetch_all(store.pool())
    .await
    .map_err(StoreError::Sqlx)?;
    Ok(rows
        .into_iter()
        .filter_map(
            |(id, kind, content, topic, value, state, proposer, writer, seq)| {
                let uuid: Uuid = id.parse().ok()?;
                Some((
                    id.clone(),
                    proposer.parse().ok()?,
                    state.clone(),
                    json!({
                        "id": id, "knowledge_type": kind, "content": content,
                        "topic_key": topic, "value_key": value, "state": state,
                        "proposed_by_user_id": proposer, "writer_id": writer,
                        "writer_seq": seq, "applicability": [],
                    }),
                    RecordRef::Knowledge {
                        domain: KnowledgeDomain::Team,
                        id: uuid,
                    },
                ))
            },
        )
        .collect())
}

async fn local_patterns(store: &Store) -> Result<Vec<Uuid>> {
    let rows: Vec<String> =
        sqlx::query_scalar("SELECT id FROM reusable_patterns WHERE deleted_at IS NULL ORDER BY id")
            .fetch_all(store.pool())
            .await
            .map_err(StoreError::Sqlx)?;
    Ok(rows.into_iter().filter_map(|s| s.parse().ok()).collect())
}

/// The safe shape of a local pattern.
///
/// Six local names never appear here — `signals`, `signal_digest`,
/// `origin_ref`, `sanitization_report`, `source_memory_id`, `origin_deleted` —
/// because they are refused at the privacy boundary and the server has no
/// column for them. `trust` is absent for a related reason: the server assigns
/// `sanitized`, the one level it can establish, and `validated` is a state this
/// machine earned privately from applications the server cannot see.
async fn pattern_payload(store: &Store, local: Uuid, content_key: &str) -> Result<Option<Value>> {
    let row: Option<(String, String, String, String, String, String)> = sqlx::query_as(
        "SELECT title, problem, root_cause, approach, constraints, applicability
           FROM reusable_patterns WHERE id = ?1 AND deleted_at IS NULL",
    )
    .bind(local.to_string())
    .fetch_optional(store.pool())
    .await
    .map_err(StoreError::Sqlx)?;
    Ok(row.map(
        |(title, problem, root_cause, approach, constraints, applicability)| {
            json!({
                "title": title,
                "problem": problem,
                "root_cause": root_cause,
                "approach": approach,
                "constraints": serde_json::from_str::<Value>(&constraints).unwrap_or(json!([])),
                "applicability": serde_json::from_str::<Value>(&applicability).unwrap_or(json!([])),
                "content_key": content_key,
            })
        },
    ))
}

// ---------------------------------------------------------------------------
// Phase 3 — possession (§5), phase 4 — switch (§4), phase 5 — demote (§12.3)
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Clone, serde::Serialize)]
pub struct PossessionReport {
    pub held: i64,
    pub missing: i64,
    pub indeterminate: i64,
}

/// Ask the server which records it durably holds, and retain the rest.
///
/// Checked for **every** delivered record plus every record this store already
/// believed canonical, because a store that only checked what it just sent
/// would demote a pre-existing replica on no evidence at all.
///
/// The three answers are not interchangeable. `missing` means the server
/// genuinely does not hold it for this caller. `indeterminate` means the server
/// will not say — the caller cannot see the record — and answering `missing`
/// there would be a lie the client acts on by keeping a writable copy of
/// something the server owns (§12.5).
pub async fn verify_possession<R: Remote>(
    store: &Store,
    remote: &R,
    account: Uuid,
    delivered: &[RecordRef],
) -> Result<(PossessionReport, Vec<RecordRef>)> {
    mig::phase_begin(store, Phase::VerifyPossession).await?;

    let mut candidates: Vec<RecordRef> = delivered.to_vec();
    candidates.extend(believed_canonical(store, account).await?);
    candidates.sort();
    candidates.dedup();

    let mut report = PossessionReport::default();
    let mut held = Vec::new();
    for chunk in candidates.chunks(POSSESSION_BATCH) {
        let answers = remote.possession(chunk).await?;
        report.held += answers.held.len() as i64;
        report.missing += answers.missing.len() as i64;
        report.indeterminate += answers.indeterminate.len() as i64;
        for r in answers.missing {
            // Nothing canonical to defer to, so the local copy is the only copy
            // and stays one. The reason is `local_only` rather than
            // `server_refused`, because `missing` is not evidence that the
            // server refused anything: a record blocked before it was ever
            // offered — a team proposal this account did not write, a pattern
            // nobody claimed — has had no refusal from anyone. The drain
            // records `server_refused` itself, for records it actually offered
            // and the server actually declined, and a record already retained
            // under any reason keeps the one it has.
            if mig::is_retained(store, &r.retained()).await?.is_none() {
                mig::retain(store, r.retained(), RetainedReason::LocalOnly).await?;
            }
        }
        for r in answers.indeterminate {
            mig::retain(store, r.retained(), RetainedReason::PossessionIndeterminate).await?;
        }
        held.extend(answers.held);
    }

    // Blocked only if nothing could be checked at all — one unconfirmed record
    // is a retained record, not a stopped migration.
    let state =
        if !candidates.is_empty() && report.held + report.missing + report.indeterminate == 0 {
            PhaseState::Blocked
        } else {
            PhaseState::Done
        };
    mig::phase_finish(store, Phase::VerifyPossession, state, report.held).await?;
    Ok((report, held))
}

/// Everything this store already treated as canonical, named by reference.
async fn believed_canonical(store: &Store, account: Uuid) -> Result<Vec<RecordRef>> {
    let mut out = Vec::new();
    for (id, _project, _payload, reference) in project_memories(store).await? {
        let _ = id;
        out.push(reference);
    }
    for (_key, _payload, reference) in relations(store).await? {
        out.push(reference);
    }
    for (_id, owner, _payload, reference) in personal_rows(store).await? {
        if owner == account {
            out.push(reference);
        }
    }
    for (_id, _proposer, _state, _payload, reference) in team_rows(store).await? {
        out.push(reference);
    }
    for claim in mig::pattern_claims(store).await? {
        if claim.owner_user_id == account {
            out.push(RecordRef::Pattern(claim.pattern_id));
        }
    }
    Ok(out)
}

/// Move the store to server authority, with the retained set written in the
/// same transaction.
///
/// One statement, and it either commits or it does not — there is no partial
/// state in which the store believes the server owns knowledge it has not
/// finished naming the exceptions to.
pub async fn switch_authority(store: &Store) -> Result<()> {
    mig::phase_begin(store, Phase::SwitchAuthority).await?;
    cairn_store::authority::set_mode(
        store,
        cairn_store::authority::AuthorityMode::ServerAuthoritative,
    )
    .await?;
    let retained = mig::retained(store).await?.len() as i64;
    mig::phase_finish(store, Phase::SwitchAuthority, PhaseState::Done, retained).await?;
    Ok(())
}

/// Demote the local replicas of records the server still holds — and re-check
/// that it still holds them, in this step (§12.3).
///
/// Phases 3 and 5 are separated by an authority switch that can take arbitrary
/// wall-clock time, and a server-side loss in between would otherwise demote
/// the last copy of a record on the strength of a check that has gone stale.
/// FR-872 is satisfied at the moment that matters rather than only at the
/// moment of the earlier check.
///
/// Anything that has stopped being held is **retained instead of demoted**.
pub async fn demote<R: Remote>(
    store: &Store,
    remote: &R,
    to_demote: &[RecordRef],
) -> Result<(i64, i64)> {
    mig::phase_begin(store, Phase::Demote).await?;
    let mut demoted = 0i64;
    let mut withheld = 0i64;
    for chunk in to_demote.chunks(POSSESSION_BATCH) {
        let answers = remote.possession(chunk).await?;
        let still_held: std::collections::BTreeSet<RecordRef> =
            answers.held.iter().cloned().collect();
        for r in chunk {
            if still_held.contains(r) {
                demoted += 1;
            } else {
                withheld += 1;
                let reason = if answers.indeterminate.contains(r) {
                    RetainedReason::PossessionIndeterminate
                } else {
                    // Held a moment ago and not held now. Something happened
                    // on the server between the two checks, which is exactly
                    // what §12.3 re-checks for, and the honest reading is that
                    // this local copy is the only one again.
                    RetainedReason::LocalOnly
                };
                mig::retain(store, r.retained(), reason).await?;
            }
        }
    }
    mig::phase_finish(store, Phase::Demote, PhaseState::Done, demoted).await?;
    Ok((demoted, withheld))
}

// ---------------------------------------------------------------------------
// The whole run, resumable (§7)
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Clone, serde::Serialize)]
pub struct RunReport {
    pub resumed_at: Option<String>,
    pub keys: KeyReport,
    pub drain: DrainReport,
    pub possession: PossessionReport,
    pub demoted: i64,
    pub withheld_from_demotion: i64,
    pub mode: String,
}

/// Run the migration, entering at the first phase that is not done.
///
/// **It never skips ahead.** `first_unfinished` is consulted between phases
/// rather than once at the start, so an interruption inside a phase re-enters
/// at that phase on the next run and a phase whose precondition has not been
/// met cannot be reached by a store that crashed at the wrong moment.
pub async fn run<R: Remote>(
    store: &Store,
    remote: &R,
    account: Uuid,
    writer_id: &str,
) -> Result<RunReport> {
    let mut report = RunReport::default();
    report.resumed_at = mig::first_unfinished(store)
        .await?
        .map(|p| p.as_str().to_string());

    // Registration is what opens the migration-scoped drain route, and it is
    // deliberately not gated by the server's authority mode: a store upgrading
    // *after* cutover must still be able to hand its knowledge over, or the
    // refusal would be self-perpetuating (§12.1, FR-876d).
    let token = remote.register(writer_id).await?;

    let mut delivered: Vec<RecordRef> = Vec::new();
    let mut held: Vec<RecordRef> = Vec::new();

    // Walked in `Phase::all()` order, skipping only what is already `done`.
    //
    // Deliberately not `while let Some(p) = first_unfinished()`: a phase that
    // ends `blocked` is not `done`, so that loop re-enters it forever. Blocked
    // is "finished for now" — `--retry-retained` is how it is revisited — and a
    // single ordered pass expresses that without a special case, while still
    // resuming at the first unfinished phase and never skipping one.
    for phase in Phase::all().iter().copied() {
        let done = matches!(
            mig::phase(store, phase).await?.map(|p| p.state),
            Some(PhaseState::Done)
        );
        if done {
            continue;
        }
        match phase {
            Phase::Inspect => {
                inspect(store).await?;
            }
            Phase::ClaimPatternOwnership => {
                // `--run` claims nothing on the user's behalf. It closes the
                // phase by naming every unclaimed pattern as retained-local,
                // which is what "completing this phase" means (§4.1a).
                mig::phase_begin(store, Phase::ClaimPatternOwnership).await?;
                let retained = settle_unclaimed_patterns(store).await?;
                mig::phase_finish(
                    store,
                    Phase::ClaimPatternOwnership,
                    PhaseState::Done,
                    retained,
                )
                .await?;
            }
            Phase::Drain => {
                // Re-keying happens here, immediately before the records are
                // built into payloads, so what is handed over carries the
                // normalized keys rather than the legacy ones.
                report.keys = normalize_keys(store).await?;
                report.drain = drain(store, remote, &token, account).await?;
                delivered = report.drain.delivered_refs.clone();
            }
            Phase::VerifyPossession => {
                let (p, h) = verify_possession(store, remote, account, &delivered).await?;
                report.possession = p;
                held = h;
            }
            Phase::SwitchAuthority => switch_authority(store).await?,
            Phase::Demote => {
                // Only records confirmed held at phase 3, minus anything that
                // has since been retained.
                let mut candidates = Vec::new();
                for r in &held {
                    if mig::is_retained(store, &r.retained()).await?.is_none() {
                        candidates.push(r.clone());
                    }
                }
                let (demoted, withheld) = demote(store, remote, &candidates).await?;
                report.demoted = demoted;
                report.withheld_from_demotion = withheld;
            }
        }
    }
    remote.complete(&token).await?;
    report.mode = cairn_store::authority::mode(store)
        .await?
        .as_str()
        .to_string();
    Ok(report)
}

// ---------------------------------------------------------------------------
// Status and retry (§12.2)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, serde::Serialize)]
pub struct RetainedRow {
    pub ref_kind: &'static str,
    pub reference: String,
    pub reason: &'static str,
    pub detected_at: String,
    /// Whether the local copy may still be written.
    ///
    /// A record retained as `possession_indeterminate` is read-only: the server
    /// may hold it but would not confirm to this caller, and writing it would
    /// create a second truth (FR-712).
    pub writable: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct StatusReport {
    pub mode: String,
    pub phases: Vec<Value>,
    pub retained: Vec<RetainedRow>,
    /// True when every retained record has been transferred or deleted, so the
    /// store is server-authoritative with no exceptions left to name.
    pub complete: bool,
}

pub async fn status(store: &Store) -> Result<StatusReport> {
    let mode = cairn_store::authority::mode(store)
        .await?
        .as_str()
        .to_string();
    let phases = mig::phases(store)
        .await?
        .into_iter()
        .map(|p| {
            json!({
                "phase": p.phase.as_str(),
                "state": p.state.as_str(),
                "detail_count": p.detail_count,
                "started_at": p.started_at,
                "finished_at": p.finished_at,
            })
        })
        .collect();
    let retained: Vec<RetainedRow> = mig::retained(store)
        .await?
        .into_iter()
        .map(|r| RetainedRow {
            ref_kind: match r.reference {
                RetainedRef::Knowledge { .. } => "knowledge",
                RetainedRef::Pattern(_) => "pattern",
                RetainedRef::Relation(_) => "relation",
            },
            reference: r.reference.dedupe_key(),
            reason: r.reason.as_str(),
            detected_at: r.detected_at,
            writable: !matches!(r.reason, RetainedReason::PossessionIndeterminate),
        })
        .collect();
    let complete = mode == "server_authoritative" && retained.is_empty();
    Ok(StatusReport {
        mode,
        phases,
        retained,
        complete,
    })
}

/// Re-attempt every retained record, on demand and never automatically.
///
/// A retained record is never re-synchronized on a timer: it is an exception
/// somebody should look at, and quietly retrying it forever would turn a
/// reported problem into a background one. `--retry-retained` is the explicit
/// ask, and a record that finally comes back `held` stops being an exception.
pub async fn retry_retained<R: Remote>(
    store: &Store,
    remote: &R,
    account: Uuid,
    writer_id: &str,
) -> Result<(i64, i64)> {
    let token = remote.register(writer_id).await?;
    let retained = mig::retained(store).await?;
    let mut released = 0i64;
    let mut still = 0i64;

    // Anything the server refused or never received is offered again before it
    // is re-checked; an indeterminate record only needs the check.
    let redrainable: Vec<_> = retained
        .iter()
        .filter(|r| {
            matches!(
                r.reason,
                RetainedReason::ServerRefused | RetainedReason::OwnerUnclaimed
            )
        })
        .collect();
    if !redrainable.is_empty() {
        drain(store, remote, &token, account).await?;
    }

    let refs: Vec<RecordRef> = retained
        .iter()
        .filter_map(|r| match r.reference {
            RetainedRef::Knowledge { domain, id } => Some(RecordRef::Knowledge { domain, id }),
            RetainedRef::Pattern(id) => Some(RecordRef::Pattern(id)),
            RetainedRef::Relation(rel) => Some(RecordRef::Relation(rel)),
        })
        .collect();
    for chunk in refs.chunks(POSSESSION_BATCH) {
        let answers = remote.possession(chunk).await?;
        for r in answers.held {
            // A `local_only` record is retained by the user's choice, not by a
            // failure, and is never released by a possession answer.
            if mig::is_retained(store, &r.retained()).await? == Some(RetainedReason::LocalOnly) {
                still += 1;
                continue;
            }
            if mig::release_retained(store, &r.retained()).await? {
                released += 1;
            }
        }
        still += (answers.missing.len() + answers.indeterminate.len()) as i64;
    }
    remote.complete(&token).await?;
    Ok((released, still))
}

// ---------------------------------------------------------------------------
// The real server, behind the interface
// ---------------------------------------------------------------------------

/// [`Remote`] over the migration routes of a real Cairn server.
///
/// Errors arrive as [`StoreError::Refused`] carrying the server's own code, so
/// the state machine has one error type to reason about and a refusal keeps
/// the code the contract names rather than being flattened into a message.
pub struct HttpRemote {
    client: crate::sync::Client,
}

impl HttpRemote {
    pub fn new(client: crate::sync::Client) -> Self {
        Self { client }
    }

    async fn post(&self, path: &str, body: Value) -> Result<Value> {
        self.client.post(path, &body).await.map_err(|e| {
            // `WireError`'s code is already the stable one the contract names —
            // `upgrade_required`, `migration_not_registered` — and the state
            // machine branches on it, so it must survive the crossing.
            StoreError::Refused {
                code: leak(e.code),
                // The route is in the message because a migration makes four
                // different calls and a bare "server rejected the request" sends
                // the reader to the wrong one.
                message: format!("{path}: {}", e.message),
            }
        })
    }
}

/// Give a wire code a `'static` lifetime.
///
/// `StoreError::Refused` holds a `&'static str` because every code in the store
/// is a literal. A code that arrived over the network is not, and the set of
/// distinct codes a process sees is tiny and bounded by the contract, so
/// leaking one is a few bytes rather than a leak in any meaningful sense.
fn leak(code: String) -> &'static str {
    Box::leak(code.into_boxed_str())
}

impl Remote for HttpRemote {
    async fn register(&self, writer_id: &str) -> Result<String> {
        let v = self
            .post("/api/migration/register", json!({ "writer_id": writer_id }))
            .await?;
        v.get("migration_token")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| StoreError::Corrupt("registration returned no token".into()))
    }

    async fn drain(&self, token: &str, items: &[DrainItem]) -> Result<Vec<DrainOutcome>> {
        let body = json!({
            "migration_token": token,
            "items": items.iter().map(|i| json!({
                "entity_type": i.entity_type,
                "entity_id": i.entity_id,
                "operation": "upsert",
                "payload": i.payload,
            })).collect::<Vec<_>>(),
        });
        let v = self.post("/api/migration/drain", body).await?;
        Ok(v.get("results")
            .and_then(Value::as_array)
            .map(|rows| {
                rows.iter()
                    .filter_map(|r| {
                        Some(DrainOutcome {
                            entity_id: r.get("entity_id")?.as_str()?.to_string(),
                            accepted: r.get("accepted").and_then(Value::as_bool).unwrap_or(false),
                            reason: r.get("reason").and_then(Value::as_str).map(str::to_string),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default())
    }

    async fn possession(&self, records: &[RecordRef]) -> Result<Possession> {
        let body = json!({
            "records": records.iter().map(RecordRef::to_json).collect::<Vec<_>>(),
        });
        let v = self.post("/api/migration/possession", body).await?;
        let list = |k: &str| -> Vec<RecordRef> {
            v.get(k)
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(RecordRef::from_json).collect())
                .unwrap_or_default()
        };
        Ok(Possession {
            held: list("held"),
            missing: list("missing"),
            indeterminate: list("indeterminate"),
        })
    }

    async fn complete(&self, token: &str) -> Result<()> {
        self.post(
            "/api/migration/complete",
            json!({ "migration_token": token }),
        )
        .await
        .map(|_| ())
    }
}
