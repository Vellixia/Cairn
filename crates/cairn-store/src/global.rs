//! Personal global memory: read, write and reconcile (`contracts/global-memory.md`).
//!
//! `personal_knowledge` has no `project_id` column and never gains one (§1 of
//! the contract, D403). That absence is Layer A and is enforced by the schema,
//! not by this module. What this module *does* enforce is Layer B: every
//! free-text field a caller can put a path, a secret or a project name into is
//! screened by [`cairn_core::validate::validate_global_content`] before a
//! single row exists (FR-544, FR-545, FR-579) — this file must never grow a
//! second implementation of that screen.
//!
//! Immutability is the other half of the contract (D405, FR-440): the only
//! `UPDATE … SET content` statement in this module is [`forget_personal`]'s
//! tombstone, exactly as `crates/cairn-store/src/repo.rs`'s `delete_memory`
//! is the only one for `memories`. Reconciliation reuses Feature 003's
//! `classify_proposal`/`derive_subject` unchanged, over `personal_knowledge`'s
//! own relations only (`contracts/global-memory.md` §6).
//!
//! Team knowledge lives in this same file, in its own section below the
//! personal one, following the same layout: the record, write, the one
//! permitted mutation, read, then the subject read. Its lifecycle is the
//! `proposed → authoritative → retired` compare-and-swap
//! (`contracts/global-memory.md` §3, §5b), reusing the revision-guard shape
//! `crate::criteria::check_revision` already proved rather than inventing a
//! second one. Promotion orchestration and every synchronization concern
//! beyond the one server-instance-binding primitive this file owns (routing,
//! per-namespace backoff, capability re-probing) belong to later phases and
//! to files this module does not touch — see the doc comment on
//! [`create_personal`] for exactly where the personal-domain boundary sits,
//! and on [`propose_team`] and [`merge_synced_team`] for team's.

use crate::{rows, tx, Result, Store, StoreError};
use cairn_core::applicability::{applies, normalize_applicability_value};
use cairn_core::domain::{
    new_id, ApplicabilityFact, Importance, MemoryScope, MemoryState, MemoryType, ProjectTrait,
    RelationBasis, RelationKind, SyncNamespace, TeamState, VerificationState,
};
use cairn_core::knowledge::{
    classify_proposal, content_norm_digest, derive_subject, normalize_topic_key,
    normalize_value_key, MemoryFacts, ProposalOutcome, Relation, SubjectView,
};
use cairn_core::validate::{validate_global_content, ProjectIdentity};
use cairn_core::wire::codes;
use chrono::{DateTime, Utc};
use sqlx::sqlite::SqliteRow;
use sqlx::Row;
use std::collections::HashMap;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// The record
// ---------------------------------------------------------------------------

/// Re-exported for callers that only need the record shape.
pub use cairn_core::global::PersonalKnowledge;
/// Re-exported for callers that only need the record shape (team section below).
pub use cairn_core::global::TeamKnowledge;

/// What a direct or promoted personal creation proposes.
pub struct NewPersonalKnowledge<'a> {
    pub owner_user_id: Uuid,
    pub knowledge_type: MemoryType,
    pub content: &'a str,
    pub topic_key: Option<&'a str>,
    pub value_key: Option<&'a str>,
    pub applicability: Vec<ApplicabilityFact>,
    /// `None` for a directly authored entry; `Some` only when this record is
    /// created by promotion (D418). Held on this type — rather than bolted on
    /// by a second insert statement later — so a future promotion path
    /// shares this exact write path instead of a second one that could drift
    /// from it (FR-516). Phase 5 (T068/T074, this module) only ever passes
    /// `None`: promotion orchestration is `crates/cairnd/src/promote.rs`
    /// (T075), out of this file's scope.
    pub origin_digest: Option<String>,
}

impl<'a> NewPersonalKnowledge<'a> {
    /// A directly authored entry — the shape T068/T074 exercise.
    pub fn direct(
        owner_user_id: Uuid,
        knowledge_type: MemoryType,
        content: &'a str,
        topic_key: Option<&'a str>,
        value_key: Option<&'a str>,
        applicability: Vec<ApplicabilityFact>,
    ) -> Self {
        Self {
            owner_user_id,
            knowledge_type,
            content,
            topic_key,
            value_key,
            applicability,
            origin_digest: None,
        }
    }
}

/// What creating a personal entry turned out to mean for its subject.
///
/// Mirrors `crate::repo::CreateOutcome` deliberately: the same reconciliation
/// machinery decided this, so the report it hands back has the same shape.
#[derive(Debug, Clone)]
pub struct PersonalCreateOutcome {
    pub record: PersonalKnowledge,
    pub reconciliation: ProposalOutcome,
    /// The relation this write actually recorded, carried out of the
    /// transaction rather than re-derived from `reconciliation` (see
    /// `crate::repo::CreateOutcome` for why that distinction matters).
    pub relation_recorded: Option<RelationKind>,
    pub matched_value_key: Option<String>,
    /// The subject this entry joined, after normalization; `None` for a
    /// free-form entry or a topic key that failed to normalize (FR-312).
    pub subject: Option<String>,
    /// Notes for an `ok: true` envelope: `invalid_topic_key`,
    /// `value_without_topic`, `corroborating_member`,
    /// `reconciliation_deferred` — the same vocabulary `CreateOutcome` uses.
    pub notes: Vec<&'static str>,
}

/// The per-write reconciliation bound, mirroring
/// `crate::repo::DEFAULT_RECONCILE_MEMBERS_MAX`. Kept as its own constant
/// rather than shared, because the two subjects (`memories`, personal
/// knowledge) are unrelated data and a future change to one bound must not
/// silently move the other.
pub const PERSONAL_RECONCILE_MEMBERS_MAX: usize = 64;

fn personal_bare(row: &SqliteRow) -> Result<PersonalKnowledge> {
    Ok(PersonalKnowledge {
        id: rows::uuid(row, "id")?,
        owner_user_id: rows::uuid(row, "owner_user_id")?,
        knowledge_type: rows::enum_val(row, "knowledge_type")?,
        content: row.try_get("content")?,
        topic_key: row.try_get("topic_key")?,
        value_key: row.try_get("value_key")?,
        origin_digest: row.try_get("origin_digest")?,
        // Loaded separately by every caller of this function — applicability
        // lives in its own table, one-to-many, and is never joined into this
        // row shape (same split `evidence_for` keeps from `memory()`).
        applicability: Vec::new(),
        writer_id: rows::uuid(row, "writer_id")?,
        writer_seq: row.try_get("writer_seq")?,
        created_at: rows::ts(row, "created_at")?,
        superseded_by_id: rows::opt_uuid(row, "superseded_by_id")?,
        forgotten_at: rows::opt_ts(row, "forgotten_at")?,
    })
}

/// Every applicability fact for `ids`, grouped by the record they belong to.
///
/// A record with no rows here is universal (D411, FR-435) — this returns no
/// entry for it, and every caller reads that as an empty `Vec` via
/// `unwrap_or_default`, never as "not loaded yet".
async fn applicability_for(
    store: &Store,
    ids: &[Uuid],
) -> Result<HashMap<Uuid, Vec<ApplicabilityFact>>> {
    let mut map: HashMap<Uuid, Vec<ApplicabilityFact>> = HashMap::new();
    if ids.is_empty() {
        return Ok(map);
    }
    let placeholders = std::iter::repeat_n("?", ids.len())
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "SELECT personal_id, kind, value FROM personal_knowledge_applicability
          WHERE personal_id IN ({placeholders})
          ORDER BY personal_id, kind, value"
    );
    let mut q = sqlx::query(&sql);
    for id in ids {
        q = q.bind(id.to_string());
    }
    let rs = q.fetch_all(store.pool()).await?;
    for r in &rs {
        let pid = rows::uuid(r, "personal_id")?;
        let kind = rows::enum_val(r, "kind")?;
        let value: String = r.try_get("value")?;
        map.entry(pid)
            .or_default()
            .push(ApplicabilityFact { kind, value });
    }
    Ok(map)
}

/// This store's writer identity — one row, seeded once at migration 7
/// (`migrate.rs`'s `finish(7, tx)`). Never regenerated (D407, FR-490).
async fn writer_id(tx: &mut sqlx::SqliteConnection) -> Result<Uuid> {
    let row = sqlx::query("SELECT writer_id FROM writer_identity WHERE id = 1")
        .fetch_one(&mut *tx)
        .await?;
    rows::uuid(&row, "writer_id")
}

/// This writer's next position in `personal_knowledge`'s stream (D408,
/// FR-445, FR-492).
///
/// Computed inside the caller's `BEGIN IMMEDIATE` transaction, which is what
/// makes `MAX + 1` race-free: the transaction already holds the write lock
/// before this runs, so no concurrent writer can insert a row between this
/// read and the insert that follows it (`tx.rs`'s whole reason to exist).
async fn next_personal_writer_seq(tx: &mut sqlx::SqliteConnection, writer: Uuid) -> Result<i64> {
    let max: Option<i64> =
        sqlx::query_scalar("SELECT MAX(writer_seq) FROM personal_knowledge WHERE writer_id = ?1")
            .bind(writer.to_string())
            .fetch_one(&mut *tx)
            .await?;
    Ok(max.unwrap_or(0) + 1)
}

/// The columns `derive_subject`/`classify_proposal` read for one member.
const PERSONAL_MEMBER_COLUMNS: &str =
    "id, owner_user_id, topic_key, value_key, content_norm_digest, writer_id";

/// Map one `personal_knowledge` row to the shared reconciliation type.
///
/// Several fields have no personal-knowledge analog and are filled with a
/// constant the algorithm never actually branches on for this domain — each
/// is commented at its assignment below, so a reader does not have to guess
/// whether the constant is a placeholder or a real decision.
fn personal_facts_from_row(row: &SqliteRow) -> Result<MemoryFacts> {
    Ok(MemoryFacts {
        id: rows::uuid(row, "id")?,
        // The caller already filtered to `forgotten_at IS NULL` in SQL — the
        // same way `memories` queries filter to `deleted_at IS NULL` before a
        // row ever reaches `MemoryFacts` — so every member handed to this
        // mapper is, by construction, current. Personal knowledge has no
        // richer lifecycle yet: no supersede API exists in this phase (no
        // task in Phase 5 asks for one), so `Active` is not a placeholder,
        // it is the only state a member here can be in.
        state: MemoryState::Active,
        // Personal knowledge has no `MemoryScope` of its own (§1 of the
        // contract): "how narrow inside a project" has no meaning for a
        // record with no project. `classify_proposal`/`derive_subject` still
        // require a scope and scope key to compute `scope_overlap`, so every
        // member of one subject is given the *same* constant pair —
        // `owner_user_id` restated as the scope key — which makes
        // `scope_overlap` return `Simultaneous` for every pair, exactly what
        // "these are all members of one subject" already means once SQL has
        // filtered to one `(owner_user_id, topic_key)`.
        scope: MemoryScope::Project,
        scope_key: row.try_get::<String, _>("owner_user_id")?,
        topic_key: row.try_get("topic_key")?,
        value_key: row.try_get("value_key")?,
        content_norm_digest: row.try_get("content_norm_digest")?,
        // Personal knowledge carries no verification field of any kind
        // (D452, FR-513) — there is nothing to read here, so every member is
        // `Unverified` and the representative tiebreak in
        // `MemoryFacts::representative_key` falls straight through to
        // evidence count (always 0, next) and then to the lowest id.
        verification: VerificationState::Unverified,
        verification_authority: None,
        // No evidence concept for personal knowledge: evidence facts are
        // project-scoped and cannot attach to a project-less record (D419).
        evidence_fact_count: 0,
        pinned: false,
        // No importance concept for personal knowledge either. Unused by
        // `classify_proposal`/`derive_subject` — neither reads `importance`
        // — so this constant cannot change which member reconciliation
        // prefers; it exists only because `MemoryFacts` is one shared type.
        importance: Importance::Normal,
        // Personal knowledge has no session; `writer_id` is the closest
        // analog to "who produced this row". Used only for distinct-origin
        // accounting inside `AnswerAccounting`, never for arbitration — the
        // same rule `MemoryFacts::origin_session_id`'s own doc comment
        // states for project memory (FR-322), and personal knowledge's
        // `writer_seq`/`writer_id` diagnostic-only rule (FR-583) is not
        // violated by this: nothing here compares two writer ids to decide
        // a winner, only to count them.
        origin_session_id: rows::uuid(row, "writer_id")?,
    })
}

/// Every current member of one subject, for reconciliation inside the
/// caller's transaction. Mirrors `crate::knowledge::subject_members_tx`
/// exactly, including the "one more than the cap" trick that tells "exactly
/// filled the bound" apart from "hit it and stopped" (FR-474).
async fn personal_members_tx(
    tx: &mut sqlx::SqliteConnection,
    owner_user_id: Uuid,
    topic_key: &str,
    cap: usize,
) -> Result<(Vec<MemoryFacts>, bool)> {
    let sql = format!(
        "SELECT {PERSONAL_MEMBER_COLUMNS} FROM personal_knowledge
          WHERE owner_user_id = ?1 AND topic_key = ?2 AND forgotten_at IS NULL
          ORDER BY id
          LIMIT ?3"
    );
    let rs = sqlx::query(&sql)
        .bind(owner_user_id.to_string())
        .bind(topic_key)
        .bind(cap as i64 + 1)
        .fetch_all(&mut *tx)
        .await?;
    let over = rs.len() > cap;
    let members = rs
        .iter()
        .take(cap)
        .map(personal_facts_from_row)
        .collect::<Result<Vec<_>>>()?;
    Ok((members, over))
}

/// Record one reconciliation decision among personal entries.
///
/// `INSERT OR IGNORE` is the whole merge rule, exactly as
/// `crate::knowledge::record_relation_tx` uses it for `memory_relations`:
/// recording an already-recorded decision is a no-op, which is what makes two
/// devices independently deciding the same duplicate converge to one row
/// rather than an error.
async fn record_personal_relation_tx(
    tx: &mut sqlx::SqliteConnection,
    decided_by_writer: Uuid,
    r: Relation,
) -> Result<bool> {
    let result = sqlx::query(
        "INSERT OR IGNORE INTO personal_knowledge_relations
            (from_id, to_id, kind, basis, decided_by_writer, decided_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )
    .bind(r.from.to_string())
    .bind(r.to.to_string())
    .bind(r.kind.as_str())
    .bind(r.basis.as_str())
    .bind(decided_by_writer.to_string())
    .bind(rows::now_text())
    .execute(&mut *tx)
    .await?;
    Ok(result.rows_affected() > 0)
}

// ---------------------------------------------------------------------------
// Write (T068, T074, T077)
// ---------------------------------------------------------------------------

/// Create a personal entry, screened, reconciled and committed atomically.
///
/// **T074 / FR-548, FR-580.** `project_identities` is the identity tokens of
/// the project currently being worked in, if any — an **empty** slice is
/// legitimate and passes the `project_identifying` check (FR-580): a personal
/// note recorded with no project open is the ordinary case this feature
/// exists for, not a check this function cannot answer. The validator itself
/// draws the line between "nothing to check against" (passes) and "the
/// caller believed it had an identity and handed over something unusable"
/// (refused) — see `validate_global_content`'s own doc comment.
///
/// The screen runs **before this function's first statement**, not inside a
/// transaction this function then rolls back. That is a deliberate choice,
/// not an oversight: `validate_global_content` is pure and total, so nothing
/// is learned by writing first and checking after, and checking first means a
/// refused call touches the database not at all — there is no earlier
/// statement to undo, because none has run (FR-545, invariant: "a rejected
/// creation persists no record and enqueues no outbox entry").
///
/// **A refused call enqueues nothing** (FR-548). The screen runs before the
/// transaction opens, so there is no queue entry to roll back — the same reason
/// there is no record to roll back. A call that succeeds does enqueue, onto the
/// `personal:<server_instance>:<user>` namespace, in this same transaction
/// (T100 — see [`enqueue_personal_tx`]); on a store that has not yet been
/// linked to a server instance there is no namespace to route to and nothing is
/// queued, which [`enqueue_personal_backlog`] is what later collects.
///
/// **T077.** Reconciliation reuses `classify_proposal` unchanged, over this
/// owner's own active `personal_knowledge` rows for the same `topic_key` —
/// the personal analog of `crate::repo::create_memory_reconciled`.
pub async fn create_personal(
    store: &Store,
    new: NewPersonalKnowledge<'_>,
    project_identities: &[ProjectIdentity],
) -> Result<PersonalCreateOutcome> {
    validate_global_content(
        new.content,
        new.topic_key,
        new.value_key,
        &new.applicability,
        project_identities,
    )
    .map_err(|rejection| StoreError::Refused {
        code: rejection.class,
        message: format!("personal knowledge refused: {rejection}"),
    })?;

    // Safe to re-derive rather than error: `validate_global_content` above
    // already ran this exact normalization to screen the normalized form
    // (FR-578), so a failure here would mean the two calls disagree with
    // each other — a bug in this module, not a fact about this write.
    let applicability: Vec<ApplicabilityFact> = new
        .applicability
        .iter()
        .map(|f| {
            let value = normalize_applicability_value(&f.value)
                .expect("validate_global_content already accepted this value");
            ApplicabilityFact {
                kind: f.kind,
                value,
            }
        })
        .collect();

    let mut notes: Vec<&'static str> = Vec::new();
    // An unrepresentable key never refuses the entry (FR-312, same rule
    // `create_memory_reconciled` follows for memories): the entry is stored
    // free-form and the reason is reported as a note.
    let topic_key = match new.topic_key {
        Some(raw) => {
            let normalized = normalize_topic_key(raw);
            if normalized.is_none() {
                notes.push(codes::INVALID_TOPIC_KEY);
            }
            normalized
        }
        None => None,
    };
    let value_key = match (new.value_key, topic_key.as_ref()) {
        (Some(raw), Some(_)) => normalize_value_key(raw),
        (Some(_), None) => {
            notes.push(codes::VALUE_WITHOUT_TOPIC);
            None
        }
        (None, _) => None,
    };
    let digest = content_norm_digest(new.content);

    let id = new_id();
    let now = rows::now_text();

    let mut tx = tx::begin(store, "create_personal").await?;
    let writer = writer_id(&mut tx).await?;
    let writer_seq = next_personal_writer_seq(&mut tx, writer).await?;

    sqlx::query(
        "INSERT INTO personal_knowledge
            (id, owner_user_id, knowledge_type, content, topic_key, value_key,
             content_norm_digest, origin_digest, writer_id, writer_seq, created_at,
             superseded_by_id, forgotten_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, NULL, NULL)",
    )
    .bind(id.to_string())
    .bind(new.owner_user_id.to_string())
    .bind(new.knowledge_type.as_str())
    .bind(new.content)
    .bind(topic_key.as_deref())
    .bind(value_key.as_deref())
    .bind(&digest)
    .bind(new.origin_digest.as_deref())
    .bind(writer.to_string())
    .bind(writer_seq)
    .bind(&now)
    .execute(&mut *tx)
    .await?;

    for fact in &applicability {
        sqlx::query(
            "INSERT INTO personal_knowledge_applicability (personal_id, kind, value)
             VALUES (?1, ?2, ?3)",
        )
        .bind(id.to_string())
        .bind(fact.kind.as_str())
        .bind(&fact.value)
        .execute(&mut *tx)
        .await?;
    }

    // Bounded reconciliation, in this same transaction — a proposal and the
    // decision it implies commit together, so a reader never observes one
    // without the other (same discipline `create_memory_reconciled` keeps).
    let mut reconciliation = ProposalOutcome::Created;
    let mut relation_recorded: Option<RelationKind> = None;
    let mut matched_value_key: Option<String> = None;
    if let Some(topic) = topic_key.as_deref() {
        let (members, over_bound) = personal_members_tx(
            &mut tx,
            new.owner_user_id,
            topic,
            PERSONAL_RECONCILE_MEMBERS_MAX,
        )
        .await?;

        let proposal = MemoryFacts {
            id,
            state: MemoryState::Active,
            scope: MemoryScope::Project,
            scope_key: new.owner_user_id.to_string(),
            topic_key: Some(topic.to_string()),
            value_key: value_key.clone(),
            content_norm_digest: Some(digest.clone()),
            verification: VerificationState::Unverified,
            verification_authority: None,
            evidence_fact_count: 0,
            pinned: false,
            importance: Importance::Normal,
            origin_session_id: writer,
        };

        if over_bound {
            reconciliation = ProposalOutcome::Deferred;
            notes.push(codes::RECONCILIATION_DEFERRED);
        } else {
            let (outcome, relations) =
                classify_proposal(&proposal, &members, PERSONAL_RECONCILE_MEMBERS_MAX);
            let mut kinds: Vec<RelationKind> = Vec::new();
            for r in relations {
                kinds.push(r.kind);
                record_personal_relation_tx(&mut tx, writer, r).await?;
            }
            if matches!(outcome, ProposalOutcome::Corroborating { .. }) {
                notes.push(codes::CORROBORATING_MEMBER);
            }
            relation_recorded = kinds.first().copied();
            matched_value_key = match &outcome {
                ProposalOutcome::Duplicate { of } => members
                    .iter()
                    .find(|m| m.id == *of)
                    .and_then(|m| m.value_key.clone()),
                ProposalOutcome::Corroborating { member } => members
                    .iter()
                    .find(|m| m.id == *member)
                    .and_then(|m| m.value_key.clone()),
                _ => None,
            };
            reconciliation = outcome;
        }
    }

    // T100. Same transaction as the row, so the queue entry and the record it
    // describes commit together or not at all.
    enqueue_personal_tx(
        &mut tx,
        new.owner_user_id,
        id,
        writer,
        &personal_payload(
            id,
            new.knowledge_type,
            new.content,
            topic_key.as_deref(),
            value_key.as_deref(),
            writer,
            writer_seq,
            &applicability,
            None,
            None,
        ),
    )
    .await?;

    tx::commit(tx, "create_personal").await?;
    let record = get_personal(store, id, new.owner_user_id).await?;
    Ok(PersonalCreateOutcome {
        record,
        reconciliation,
        relation_recorded,
        matched_value_key,
        subject: topic_key,
        notes,
    })
}

// ---------------------------------------------------------------------------
// The tombstone (T069) — the only permitted content mutation
// ---------------------------------------------------------------------------

/// Forget one personal entry: content cleared, `forgotten_at` set, nothing
/// else touched (FR-440, FR-441).
///
/// This is the **only** `UPDATE … SET content` in this module, the same
/// discipline `repo.rs::delete_memory` keeps for `memories`. `topic_key` and
/// `value_key` are left as they are — clearing them would sever a still-valid
/// reconciliation subject key from history the same way `delete_memory`
/// leaves a memory's own subject key alone.
///
/// Scoped to `owner_user_id` in the same statement that finds the row, not as
/// a check afterward: a caller cannot distinguish "no such record" from
/// "that record belongs to someone else" from the error alone, which is the
/// point (FR-432) — a distinguishable error here would let a caller probe for
/// another user's record by id.
pub async fn forget_personal(store: &Store, id: Uuid, owner_user_id: Uuid) -> Result<()> {
    let mut tx = tx::begin(store, "forget_personal").await?;
    let result = sqlx::query(
        "UPDATE personal_knowledge SET forgotten_at = ?1, content = ''
          WHERE id = ?2 AND owner_user_id = ?3 AND forgotten_at IS NULL",
    )
    .bind(rows::now_text())
    .bind(id.to_string())
    .bind(owner_user_id.to_string())
    .execute(&mut *tx)
    .await?;
    if result.rows_affected() == 0 {
        // Nothing changed — `tx` is simply dropped, which rolls back a
        // transaction that made no writes anyway. Covers three cases alike:
        // no such id, a different owner's id, or an id already forgotten.
        return Err(StoreError::NotFound(format!("personal knowledge {id}")));
    }

    // T100. The tombstone is a change like any other and must reach the user's
    // other devices — a record forgotten on one machine and still recalled on
    // the next is the failure this queue entry prevents. The payload re-reads
    // the row rather than reconstructing it, so the `writer_id`/`writer_seq`
    // pair that travels is the original writer's, not this store's; a peer does
    // gap detection against the writer that produced the record, and a
    // tombstone that renamed its writer would open a gap that never existed.
    let forgotten = sqlx::query(
        "SELECT knowledge_type, content, topic_key, value_key, writer_id, writer_seq,
                superseded_by_id, forgotten_at
           FROM personal_knowledge WHERE id = ?1",
    )
    .bind(id.to_string())
    .fetch_one(&mut *tx)
    .await?;
    let facts = applicability_for_tx(&mut tx, id).await?;
    let payload = personal_payload(
        id,
        rows::enum_val(&forgotten, "knowledge_type")?,
        forgotten.try_get("content")?,
        forgotten
            .try_get::<Option<String>, _>("topic_key")?
            .as_deref(),
        forgotten
            .try_get::<Option<String>, _>("value_key")?
            .as_deref(),
        rows::uuid(&forgotten, "writer_id")?,
        forgotten.try_get("writer_seq")?,
        &facts,
        rows::opt_uuid(&forgotten, "superseded_by_id")?,
        forgotten
            .try_get::<Option<String>, _>("forgotten_at")?
            .as_deref(),
    );
    let writer = writer_id(&mut tx).await?;
    enqueue_personal_tx(&mut tx, owner_user_id, id, writer, &payload).await?;

    tx::commit(tx, "forget_personal").await?;
    store.checkpoint().await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Outbox routing (T100, FR-486, FR-568) — how a global write reaches its lane
// ---------------------------------------------------------------------------
//
// The mechanism this section calls, `outbox::enqueue_global`, has existed since
// Phase 6 and was reachable from nowhere. Its absence was not a missing
// optimization: a personal record written on one device stayed on that device
// forever, and `outbox::known_namespaces` — which is how the daemon discovers
// which lanes to drain — returned nothing for `personal:*` or `team:*` at all,
// so the whole lane was inert rather than slow.
//
// Every enqueue below happens inside the same transaction as the write it
// describes, which is the outbox's entire reason to exist (D9): a crash between
// the row and its queue entry cannot lose one and keep the other.
//
// **A write with no namespace enqueues nothing, and that is not a silent drop.**
// A `personal:*` or `team:*` namespace exists only once this store has been
// linked to a server instance and knows its id, exactly as a `project:*` row
// exists only for a linked project (`SyncPolicy::target`, FR-053). Writes made
// before then are recorded locally and reach the queue when the namespace is
// established, through [`enqueue_personal_backlog`] and
// [`enqueue_team_backlog`] — the same shape `crates/cairnd/src/sync.rs`'s
// `backfill` already uses for a project's pre-link history.

/// The `personal:*` namespace this store currently holds for `owner_user_id`.
///
/// Read out of `sync_cursor`'s key rather than from a column of its own, for
/// the reason [`recorded_team_instance`] gives at length: the namespace key
/// **is** the record of which server instance a lane belongs to, and a second
/// place to store it is a second place for it to disagree.
///
/// One owner has at most one such namespace on this store. A store may hold
/// several `personal:*` lanes — the same human's account on two servers is two
/// identities, §10 — but their `user_uuid`s differ, so the suffix match here
/// selects exactly one.
async fn recorded_personal_namespace(
    tx: &mut sqlx::SqliteConnection,
    owner_user_id: Uuid,
) -> Result<Option<SyncNamespace>> {
    let keys: Vec<String> =
        sqlx::query_scalar("SELECT namespace FROM sync_cursor WHERE namespace LIKE 'personal:%'")
            .fetch_all(&mut *tx)
            .await?;
    let suffix = format!(":{owner_user_id}");
    Ok(keys
        .into_iter()
        .find(|k| k.ends_with(&suffix))
        .and_then(|k| {
            let rest = k.strip_prefix("personal:")?;
            let (instance, _) = rest.split_once(':')?;
            Some(SyncNamespace::Personal(
                Uuid::parse_str(instance).ok()?,
                owner_user_id,
            ))
        }))
}

/// One personal row's applicability facts, read inside the caller's
/// transaction.
///
/// [`applicability_for`] takes `&Store` (the pool, autocommit) and so cannot
/// observe a row this transaction has just written or updated — which is
/// exactly what the enqueue paths below need.
async fn applicability_for_tx(
    tx: &mut sqlx::SqliteConnection,
    id: Uuid,
) -> Result<Vec<ApplicabilityFact>> {
    let rows_ = sqlx::query(
        "SELECT kind, value FROM personal_knowledge_applicability WHERE personal_id = ?1",
    )
    .bind(id.to_string())
    .fetch_all(&mut *tx)
    .await?;
    rows_
        .iter()
        .map(|r| {
            Ok(ApplicabilityFact {
                kind: rows::enum_val(r, "kind")?,
                value: r.try_get("value")?,
            })
        })
        .collect()
}

/// One team row's applicability facts, read inside the caller's transaction.
/// See [`applicability_for_tx`].
async fn team_applicability_for_tx(
    tx: &mut sqlx::SqliteConnection,
    id: Uuid,
) -> Result<Vec<ApplicabilityFact>> {
    let rows_ =
        sqlx::query("SELECT kind, value FROM team_knowledge_applicability WHERE team_id = ?1")
            .bind(id.to_string())
            .fetch_all(&mut *tx)
            .await?;
    rows_
        .iter()
        .map(|r| {
            Ok(ApplicabilityFact {
                kind: rows::enum_val(r, "kind")?,
                value: r.try_get("value")?,
            })
        })
        .collect()
}

/// One `personal_knowledge` row in the shape the server's ingest reads.
///
/// Deliberately **not** the record type: `origin_digest` is local-only and must
/// never cross the wire (D434, FR-551), and the way to guarantee that is for the
/// serialized form to have no field for it rather than for every call site to
/// remember to clear one. `content_norm_digest` is likewise absent — it is a
/// local index over local content, recomputable and not the peer's business.
#[allow(clippy::too_many_arguments)]
fn personal_payload(
    id: Uuid,
    knowledge_type: MemoryType,
    content: &str,
    topic_key: Option<&str>,
    value_key: Option<&str>,
    writer_id: Uuid,
    writer_seq: i64,
    applicability: &[ApplicabilityFact],
    superseded_by_id: Option<Uuid>,
    forgotten_at: Option<&str>,
) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "knowledge_type": knowledge_type.as_str(),
        "content": content,
        "topic_key": topic_key,
        "value_key": value_key,
        "writer_id": writer_id,
        "writer_seq": writer_seq,
        "applicability": applicability
            .iter()
            .map(|f| serde_json::json!({ "kind": f.kind.as_str(), "value": f.value }))
            .collect::<Vec<_>>(),
        "superseded_by_id": superseded_by_id,
        "forgotten_at": forgotten_at,
    })
}

/// One `team_knowledge` row in the shape the server's ingest reads.
///
/// Carries the lifecycle columns as well as the content ones, because a peer
/// that already holds an authoritative copy learns of its retirement only from
/// them. `origin_digest` is absent for the same reason as above.
#[allow(clippy::too_many_arguments)]
fn team_payload(
    id: Uuid,
    knowledge_type: MemoryType,
    content: &str,
    topic_key: Option<&str>,
    value_key: Option<&str>,
    writer_id: Uuid,
    writer_seq: i64,
    applicability: &[ApplicabilityFact],
    state: TeamState,
    proposed_by_user_id: Uuid,
    ratified_by_user_id: Option<Uuid>,
    ratified_at: Option<DateTime<Utc>>,
    retired_at: Option<DateTime<Utc>>,
    superseded_by_id: Option<Uuid>,
) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "knowledge_type": knowledge_type.as_str(),
        "content": content,
        "topic_key": topic_key,
        "value_key": value_key,
        "writer_id": writer_id,
        "writer_seq": writer_seq,
        "applicability": applicability
            .iter()
            .map(|f| serde_json::json!({ "kind": f.kind.as_str(), "value": f.value }))
            .collect::<Vec<_>>(),
        "state": state.as_str(),
        "proposed_by_user_id": proposed_by_user_id,
        "ratified_by_user_id": ratified_by_user_id,
        "ratified_at": ratified_at,
        "retired_at": retired_at,
        "superseded_by_id": superseded_by_id,
    })
}

/// Queue one personal row for delivery, inside the caller's transaction.
///
/// `writer` is **this store's** identity, not necessarily the row's: it joins
/// the idempotency key (§7) so that two devices acting on the same record
/// produce two distinct keys and both deliveries land. The row's own
/// `writer_id`/`writer_seq` travel inside the payload, where they are what the
/// receiving peer does gap detection against.
async fn enqueue_personal_tx(
    tx: &mut sqlx::SqliteConnection,
    owner_user_id: Uuid,
    id: Uuid,
    writer: Uuid,
    payload: &serde_json::Value,
) -> Result<bool> {
    let Some(namespace) = recorded_personal_namespace(tx, owner_user_id).await? else {
        return Ok(false);
    };
    crate::outbox::enqueue_global(
        &mut *tx,
        &namespace,
        cairn_core::domain::OutboxEntityType::PersonalKnowledge,
        id,
        cairn_core::domain::OutboxOperation::Upsert,
        writer,
        owner_user_id,
        payload,
    )
    .await
}

/// Queue one team row for delivery, inside the caller's transaction.
///
/// See [`enqueue_personal_tx`] for why `writer` is this store's identity, and
/// the `authored_by_user_id` column comment for why `proposed_by` — the account
/// — has to be recorded separately from it (FR-594).
async fn enqueue_team_tx(
    tx: &mut sqlx::SqliteConnection,
    id: Uuid,
    writer: Uuid,
    proposed_by: Uuid,
    payload: &serde_json::Value,
) -> Result<bool> {
    let Some(instance) = recorded_team_instance(tx).await? else {
        return Ok(false);
    };
    crate::outbox::enqueue_global(
        &mut *tx,
        &SyncNamespace::Team(instance),
        cairn_core::domain::OutboxEntityType::TeamKnowledge,
        id,
        cairn_core::domain::OutboxOperation::Upsert,
        writer,
        proposed_by,
        payload,
    )
    .await
}

/// Queue every one of this owner's personal rows that is not already queued,
/// for a namespace that has just been established (T100, FR-486).
///
/// The counterpart to `crates/cairnd/src/sync.rs`'s `backfill` for projects,
/// and it exists for the same reason: personal knowledge is local-first, so a
/// user records notes before ever linking a server, and those notes are exactly
/// the ones they most want on their second machine. Without this, everything
/// written before the link would be stranded — recorded, recallable locally,
/// and permanently invisible to every other device.
///
/// Idempotent by the outbox's own key: `enqueue_global` is `INSERT OR IGNORE`
/// on `idempotency_key`, and the key is a digest over the writer, the entity,
/// the operation and the body. Running this twice queues nothing the second
/// time, and running it against a namespace whose rows are already delivered
/// re-queues nothing either, because the delivered row still holds its key.
/// Takes the owning account rather than a [`SyncNamespace`]: the namespace is
/// resolved from `sync_cursor` inside each transaction anyway, so accepting one
/// here would let a caller pass a `Project`or `Team` variant that this function
/// would then have to refuse at runtime. There is nothing to refuse if there is
/// nothing to pass.
/// Attribute personal knowledge written before any account was known to the
/// account this machine has just proven (FR-608).
///
/// **Local-first is only half a promise without this.** A user records personal
/// notes before ever linking a server — the documented reason
/// [`enqueue_personal_backlog`] exists at all — and those notes are owned by
/// [`UNATTRIBUTED_OWNER`](cairn_core::domain::UNATTRIBUTED_OWNER), which is not
/// an account and therefore not a partition anything syncs. Left there they are
/// permanently invisible to every other device and to every later read scoped by
/// account: recorded, recallable on this machine alone, and quietly excluded from
/// the feature they were written for.
///
/// So the first account this machine authenticates as adopts them, and the
/// backlog then queues them like any other row it owns. First, because after one
/// adoption there are none left — the operation is naturally once, without a flag
/// to keep.
///
/// **Why the first account and not a choice.** These notes were written by
/// whoever was using this machine, at a time when the machine could not ask who
/// that was; the first person to sign in is the only answer available, and it is
/// the same answer a user would give. A machine genuinely shared by two people
/// before either signs in is outside what personal memory models — it has no
/// concept of a second local user — and the honest place to state that is here.
pub async fn adopt_unattributed_personal(store: &Store, account: Uuid) -> Result<usize> {
    let adopted =
        sqlx::query("UPDATE personal_knowledge SET owner_user_id = ?1 WHERE owner_user_id = ?2")
            .bind(account.to_string())
            .bind(cairn_core::domain::UNATTRIBUTED_OWNER.to_string())
            .execute(store.pool())
            .await?;
    Ok(adopted.rows_affected() as usize)
}

pub async fn enqueue_personal_backlog(store: &Store, owner_user_id: Uuid) -> Result<usize> {
    let ids: Vec<String> =
        sqlx::query_scalar("SELECT id FROM personal_knowledge WHERE owner_user_id = ?1")
            .bind(owner_user_id.to_string())
            .fetch_all(store.pool())
            .await?;

    let mut queued = 0usize;
    for id in ids {
        let Ok(id) = Uuid::parse_str(&id) else {
            continue;
        };
        let mut tx = tx::begin(store, "enqueue_personal_backlog").await?;
        let row = sqlx::query(
            "SELECT knowledge_type, content, topic_key, value_key, writer_id, writer_seq,
                    superseded_by_id, forgotten_at
               FROM personal_knowledge WHERE id = ?1",
        )
        .bind(id.to_string())
        .fetch_one(&mut *tx)
        .await?;
        let facts = applicability_for_tx(&mut tx, id).await?;
        let payload = personal_payload(
            id,
            rows::enum_val(&row, "knowledge_type")?,
            row.try_get("content")?,
            row.try_get::<Option<String>, _>("topic_key")?.as_deref(),
            row.try_get::<Option<String>, _>("value_key")?.as_deref(),
            rows::uuid(&row, "writer_id")?,
            row.try_get("writer_seq")?,
            &facts,
            rows::opt_uuid(&row, "superseded_by_id")?,
            row.try_get::<Option<String>, _>("forgotten_at")?.as_deref(),
        );
        let writer = writer_id(&mut tx).await?;
        if enqueue_personal_tx(&mut tx, owner_user_id, id, writer, &payload).await? {
            queued += 1;
        }
        tx::commit(tx, "enqueue_personal_backlog").await?;
    }
    Ok(queued)
}

/// Queue every locally proposed team row not already queued, for a `team:*`
/// namespace that has just been established (T100).
///
/// Only rows this store itself proposed: a row pulled from the server is
/// already on the server, and pushing it back would be a delivery that changes
/// nothing. `writer_id` is the discriminator — it is this store's own identity
/// for a locally authored row and some other store's for a pulled one, which is
/// precisely the distinction `writer_identity` exists to make (FR-491).
/// Takes no namespace, for the reason [`enqueue_personal_backlog`] gives.
pub async fn enqueue_team_backlog(store: &Store) -> Result<usize> {
    let mut tx = tx::begin(store, "enqueue_team_backlog").await?;
    let writer = writer_id(&mut tx).await?;
    let ids: Vec<String> = sqlx::query_scalar("SELECT id FROM team_knowledge WHERE writer_id = ?1")
        .bind(writer.to_string())
        .fetch_all(&mut *tx)
        .await?;

    let mut queued = 0usize;
    for id in ids {
        let Ok(id) = Uuid::parse_str(&id) else {
            continue;
        };
        let row = sqlx::query(
            "SELECT knowledge_type, content, topic_key, value_key, writer_id, writer_seq,
                    state, proposed_by_user_id, ratified_by_user_id, ratified_at,
                    retired_at, superseded_by_id
               FROM team_knowledge WHERE id = ?1",
        )
        .bind(id.to_string())
        .fetch_one(&mut *tx)
        .await?;
        let facts = team_applicability_for_tx(&mut tx, id).await?;
        let payload = team_payload(
            id,
            rows::enum_val(&row, "knowledge_type")?,
            row.try_get("content")?,
            row.try_get::<Option<String>, _>("topic_key")?.as_deref(),
            row.try_get::<Option<String>, _>("value_key")?.as_deref(),
            rows::uuid(&row, "writer_id")?,
            row.try_get("writer_seq")?,
            &facts,
            rows::enum_val(&row, "state")?,
            rows::uuid(&row, "proposed_by_user_id")?,
            rows::opt_uuid(&row, "ratified_by_user_id")?,
            rows::opt_ts(&row, "ratified_at")?,
            rows::opt_ts(&row, "retired_at")?,
            rows::opt_uuid(&row, "superseded_by_id")?,
        );
        let proposed_by = rows::uuid(&row, "proposed_by_user_id")?;
        if enqueue_team_tx(&mut tx, id, writer, proposed_by, &payload).await? {
            queued += 1;
        }
    }
    tx::commit(tx, "enqueue_team_backlog").await?;
    Ok(queued)
}

// ---------------------------------------------------------------------------
// Read (T068, T073)
// ---------------------------------------------------------------------------

/// Read one personal entry, scoped to its owner.
///
/// `NotFound` for a wrong owner, identically to a missing id — the same
/// privacy discipline `forget_personal` keeps (FR-432).
pub async fn get_personal(
    store: &Store,
    id: Uuid,
    owner_user_id: Uuid,
) -> Result<PersonalKnowledge> {
    let row = sqlx::query("SELECT * FROM personal_knowledge WHERE id = ?1 AND owner_user_id = ?2")
        .bind(id.to_string())
        .bind(owner_user_id.to_string())
        .fetch_optional(store.pool())
        .await?
        .ok_or_else(|| StoreError::NotFound(format!("personal knowledge {id}")))?;
    let mut record = personal_bare(&row)?;
    record.applicability = applicability_for(store, &[id])
        .await?
        .remove(&id)
        .unwrap_or_default();
    Ok(record)
}

/// A candidate pool wide enough that applicability filtering (applied after
/// the fetch, in Rust) rarely starves a caller's `limit` — see `recall_personal`.
const RECALL_CANDIDATE_CAP: i64 = 500;
pub const RECALL_DEFAULT_LIMIT: i64 = 20;
pub const RECALL_MAX_LIMIT: i64 = 200;

/// Enumerate everything this owner holds, applying **no** applicability
/// predicate (T082, FR-434).
///
/// The counterpart to [`recall_personal`], and a separate function on purpose.
/// The two answer different questions and the difference is not a parameter:
/// recall asks "what applies to the project I am standing in", enumeration asks
/// "what do I hold". Expressing the second as the first with an empty trait
/// slice is not a simplification, it is a wrong answer — `applies` returns
/// `false` for every record that carries a fact when the trait set is empty, so
/// "show me everything" would hide exactly the records a user had bothered to
/// scope. That is what `cairn personal list` did before this existed.
///
/// Forgotten entries stay out (FR-441): a tombstone is not something the owner
/// still holds.
pub async fn list_personal(
    store: &Store,
    owner_user_id: Uuid,
    query: Option<&str>,
    limit: i64,
) -> Result<Vec<PersonalKnowledge>> {
    let limit = limit.clamp(1, RECALL_MAX_LIMIT);

    let mut sql = String::from("SELECT pk.* FROM personal_knowledge pk");
    if query.is_some() {
        sql.push_str(
            " JOIN personal_fts ON personal_fts.rowid = pk.rowid
              WHERE personal_fts MATCH ?",
        );
    } else {
        sql.push_str(" WHERE 1 = 1");
    }
    sql.push_str(" AND pk.owner_user_id = ? AND pk.forgotten_at IS NULL");
    sql.push_str(" ORDER BY pk.created_at DESC LIMIT ?");

    let mut q = sqlx::query(&sql);
    if let Some(text) = query {
        q = q.bind(fts_query(text));
    }
    q = q.bind(owner_user_id.to_string());
    q = q.bind(limit);

    let rs = q.fetch_all(store.pool()).await?;
    let mut records: Vec<PersonalKnowledge> =
        rs.iter().map(personal_bare).collect::<Result<_>>()?;
    let ids: Vec<Uuid> = records.iter().map(|r| r.id).collect();
    let mut app_map = applicability_for(store, &ids).await?;
    for r in &mut records {
        r.applicability = app_map.remove(&r.id).unwrap_or_default();
    }
    Ok(records)
}

/// Recall this owner's personal entries, applying the applicability
/// predicate at read time (T073, FR-434–FR-436).
///
/// `query`, when given, matches through `personal_fts` — the same FTS5
/// external-content index `crate::search::search` joins for `memory_fts`
/// (T070: the triggers migration 0007 defines are what keep it current, and
/// this is real use of them, not only a maintained-but-unread index).
/// `topic_key` is matched exactly, never through FTS, for the same reason
/// `search.rs` treats a memory's topic key as an identity rather than text.
///
/// `project_traits` is applied to the fetched candidates in Rust, not pushed
/// into SQL: `cairn_core::applicability::applies` already states "AND across
/// kinds, OR within a kind" correctly and unambiguously over a variable-length
/// per-record set, and a second, SQL-shaped restatement of that rule would be
/// exactly the kind of second implementation FR-579's discipline exists to
/// prevent for the content validator — the same argument applies here.
pub async fn recall_personal(
    store: &Store,
    owner_user_id: Uuid,
    query: Option<&str>,
    topic_key: Option<&str>,
    project_traits: &[ProjectTrait],
    limit: i64,
) -> Result<Vec<PersonalKnowledge>> {
    let limit = limit.clamp(1, RECALL_MAX_LIMIT) as usize;

    let mut sql = String::from("SELECT pk.* FROM personal_knowledge pk");
    if query.is_some() {
        sql.push_str(
            " JOIN personal_fts ON personal_fts.rowid = pk.rowid
              WHERE personal_fts MATCH ?",
        );
    } else {
        sql.push_str(" WHERE 1 = 1");
    }
    sql.push_str(" AND pk.owner_user_id = ? AND pk.forgotten_at IS NULL");
    if topic_key.is_some() {
        sql.push_str(" AND pk.topic_key = ?");
    }
    sql.push_str(" ORDER BY pk.created_at DESC LIMIT ?");

    let mut q = sqlx::query(&sql);
    if let Some(text) = query {
        q = q.bind(fts_query(text));
    }
    q = q.bind(owner_user_id.to_string());
    if let Some(t) = topic_key {
        q = q.bind(t);
    }
    q = q.bind(RECALL_CANDIDATE_CAP);

    let rs = q.fetch_all(store.pool()).await?;
    let mut records: Vec<PersonalKnowledge> =
        rs.iter().map(personal_bare).collect::<Result<_>>()?;

    let ids: Vec<Uuid> = records.iter().map(|r| r.id).collect();
    let mut app_map = applicability_for(store, &ids).await?;
    for r in &mut records {
        r.applicability = app_map.remove(&r.id).unwrap_or_default();
    }

    records.retain(|r| applies(&r.applicability, project_traits));
    records.truncate(limit);
    Ok(records)
}

/// One free-text field, quoted term by term, `OR`-joined.
///
/// Identical in spirit to `search.rs`'s private `fts_query`: every token
/// becomes a quoted FTS5 literal so a caller's text can never be read as FTS5
/// query syntax (`"`, `*`, `NOT`, `OR`), and terms combine with `OR` so a
/// multi-word query is not required to match every word.
fn fts_query(input: &str) -> String {
    let terms: Vec<String> = input
        .split(|c: char| !c.is_alphanumeric() && c != '_' && c != '-')
        .filter(|t| !t.is_empty())
        .map(|t| format!("\"{}\"", t.replace('"', "")))
        .collect();
    if terms.is_empty() {
        return "\"\"".to_string();
    }
    terms.join(" OR ")
}

// ---------------------------------------------------------------------------
// The subject read (T078)
// ---------------------------------------------------------------------------

/// A subject, read and derived — the personal analog of
/// `crate::knowledge::SubjectRead`.
#[derive(Debug, Clone)]
pub struct PersonalSubjectRead {
    pub view: SubjectView,
    /// Every current member of the subject (`forgotten_at IS NULL`), the
    /// input `view` was derived from.
    pub members: Vec<MemoryFacts>,
}

/// Read one subject's members and derive its canonical answer (T078, FR-442,
/// FR-493).
///
/// Reuses `derive_subject` unchanged, over `personal_knowledge_relations`
/// only — never `memory_relations` or `team_knowledge_relations` (§6 of the
/// contract: "no cross-domain relations" is structural, and this function is
/// one of the places that structure is upheld by which table it queries).
///
/// An unnormalizable `topic_key` names no stored subject, so it is answered
/// the same way an absent one would be: `derive_subject` over no members and
/// no decisions, which is exactly `Reconciliation::Historical` with nothing
/// to show — the correctly typed way to say that, rather than a second
/// "empty" path this function would have to invent.
pub async fn personal_subject(
    store: &Store,
    owner_user_id: Uuid,
    topic_key: &str,
) -> Result<PersonalSubjectRead> {
    let Some(topic) = normalize_topic_key(topic_key) else {
        return Ok(PersonalSubjectRead {
            view: derive_subject(&[], &[]),
            members: Vec::new(),
        });
    };

    let sql = format!(
        "SELECT {PERSONAL_MEMBER_COLUMNS} FROM personal_knowledge
          WHERE owner_user_id = ?1 AND topic_key = ?2 AND forgotten_at IS NULL
          ORDER BY id
          LIMIT ?3"
    );
    let rs = sqlx::query(&sql)
        .bind(owner_user_id.to_string())
        .bind(&topic)
        .bind(PERSONAL_RECONCILE_MEMBERS_MAX as i64)
        .fetch_all(store.pool())
        .await?;
    let members = rs
        .iter()
        .map(personal_facts_from_row)
        .collect::<Result<Vec<_>>>()?;

    let ids: Vec<Uuid> = members.iter().map(|m| m.id).collect();
    let relations = personal_relations_touching(store, &ids).await?;
    let view = derive_subject(&members, &relations);
    Ok(PersonalSubjectRead { view, members })
}

/// Every decision touching any of `ids`, read from `personal_knowledge_relations`
/// only. Mirrors `crate::knowledge::relations_touching` exactly.
async fn personal_relations_touching(store: &Store, ids: &[Uuid]) -> Result<Vec<Relation>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let placeholders = std::iter::repeat_n("?", ids.len())
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "SELECT from_id, to_id, kind, basis FROM personal_knowledge_relations
          WHERE deleted_at IS NULL
            AND (from_id IN ({placeholders}) OR to_id IN ({placeholders}))
          ORDER BY from_id, to_id, kind"
    );
    let mut q = sqlx::query(&sql);
    for id in ids.iter().chain(ids.iter()) {
        q = q.bind(id.to_string());
    }
    let rs = q.fetch_all(store.pool()).await?;
    rs.iter()
        .map(|r| {
            Ok(Relation {
                from: rows::uuid(r, "from_id")?,
                to: rows::uuid(r, "to_id")?,
                kind: rows::enum_val(r, "kind")?,
                basis: rows::enum_val(r, "basis")?,
            })
        })
        .collect()
}

// ===========================================================================
// Team knowledge (Phase 7, T115–T128, T131, T132)
// ===========================================================================
//
// Same two-layer discipline as personal knowledge above: Layer A is the
// schema (`team_knowledge` has no `project_id` column, no evidence
// reference, no observation id, no verification field of any kind — see
// `0007_collaborative_global_memory.sql` and `contracts/global-memory.md`
// §2); Layer B is `validate_global_content`, run before this module's first
// write statement in [`propose_team`] exactly as [`create_personal`] runs it
// (FR-545, FR-548, FR-580).
//
// Team's own shape, beyond that shared discipline:
//
// * **`proposed → authoritative → retired`, compare-and-swap** (§3, §5b).
//   [`ratify_team`] and [`retire_team`] are the *only* two `UPDATE` statements
//   against `team_knowledge` in this crate, and each hardcodes the state it
//   requires the row to already be in (`'proposed'` and `'authoritative'`
//   respectively) in its own `WHERE` clause — the same shape the contract's
//   own SQL gives (§3) and the same shape `crate::criteria::check_revision`
//   already proved for task criteria, adapted from an integer revision to a
//   three-value state. A row not in the required state is refused, naming
//   its actual state (FR-454) — `retired` never re-ratifies (FR-465) because
//   no `WHERE` clause in this module ever requires `state = 'retired'` to
//   proceed to `'authoritative'`; the state `CHECK` plus this predicate
//   together make "un-retire" not a statement this module can execute.
//
// * **No admin check lives here.** `role` is a server-side concept
//   (`cairn-server`'s `users.role`) that this local SQLite schema does not
//   carry at all — `users` here has no `role` column (`0001_init.sql`). This
//   module cannot verify "is this caller an admin" and does not try to; it
//   trusts that [`ratify_team`] and [`retire_team`] are reached only from a
//   caller that already established that (the CLI, backed by an admin
//   token against the server; or the server itself applying its own
//   already-authorized decision back into this local mirror). What this
//   module *does* guarantee structurally is narrower and sufficient: there
//   is no *other* function anywhere in this crate that can move a row to
//   `authoritative`, and nothing in this file is reachable from the MCP
//   six-tool surface (T125, T126 — enforced in `crates/cairn/src/mcp.rs`,
//   outside this file, by that surface simply never calling `ratify_team` or
//   `retire_team` at all).
//
// * **`proposed` is invisible to recall, full stop** (T120, FR-452).
//   [`recall_team`] hardcodes `state = 'authoritative'` with no caller
//   exception — not even the proposer's own. [`list_team`] (T132, FR-464) is
//   the one surface that is role-filtered rather than recall-filtered: a
//   member sees authoritative entries and their own proposals, an admin sees
//   every state. The distinction matters enough that the contract itself
//   states it twice (§5b): search and context are recall; `cairn team list`
//   is a listing.
//
// * **Visibility is server-wide, never membership-scoped** (T131, FR-458,
//   FR-463). [`recall_team`] takes no project or membership argument at all
//   — there is nothing in this file's queries capable of restricting a
//   result by project membership, which is what makes "visible regardless
//   of membership" true by construction rather than by an extra clause that
//   could be forgotten.
//
// * **The server-instance asymmetry** (T116, FR-495, FR-496, FR-567;
//   `sync-namespaces.md` §10). Team knowledge is a claim about one specific
//   server's ratification history, so [`merge_synced_team`] — the one
//   function in this file that receives content whose origin is a
//   specific, named server instance — refuses when that instance differs
//   from the one already recorded for this store's team knowledge, and
//   reports the refusal rather than silently dropping the row. Personal
//   knowledge is never refused on this basis anywhere in this file: personal
//   rows are partitioned by `owner_user_id` alone (see [`create_personal`],
//   [`get_personal`], [`recall_personal`] above, none of which take or check
//   a server-instance argument), which is what lets one store hold more than
//   one identity's personal knowledge side by side. [`propose_team`]
//   deliberately does *not* run the same check: a freshly authored proposal
//   is this device's own new content, not something "sourced from" any
//   server yet, so gating it on a recorded instance would incorrectly
//   refuse a legitimate proposal made after a deliberate relink.

/// Where a proposed row's binding lives (T116, FR-495).
///
/// `sync_cursor` (`0007_collaborative_global_memory.sql`, read by
/// `crate::cursor`) already stores one row per synchronization namespace,
/// keyed on `namespace` — and the team namespace's own key,
/// `SyncNamespace::Team(id).key()` (`"team:<server_instance_id>"`), already
/// names the server instance. No dedicated column or table is needed to
/// "record which server instance this store's team knowledge came from"
/// (FR-495): the namespace key already *is* that record.
///
/// This reads directly against `sync_cursor` inside this call's own
/// transaction rather than going through `crate::cursor`'s helpers: those
/// take `&Store` (the pool, autocommit) and have no way to participate in
/// this function's `BEGIN IMMEDIATE`, and the atomicity of "check the
/// recorded instance and, if absent, establish it" together with the row
/// this transaction is about to write is exactly what this check exists to
/// guarantee. `crate::cursor` remains the read/write surface for every other
/// caller working outside a transaction of its own.
async fn recorded_team_instance(tx: &mut sqlx::SqliteConnection) -> Result<Option<Uuid>> {
    let namespace: Option<String> = sqlx::query_scalar(
        "SELECT namespace FROM sync_cursor WHERE namespace LIKE 'team:%' LIMIT 1",
    )
    .fetch_optional(&mut *tx)
    .await?;
    Ok(namespace.and_then(|ns| {
        ns.strip_prefix("team:")
            .and_then(|id| Uuid::parse_str(id).ok())
    }))
}

/// Refusal code for the server-instance asymmetry (T116, FR-496). Local to
/// this module: `cairn_core::wire::codes` carries no team-synchronization
/// code yet, and this file does not touch `cairn-core`.
pub const SERVER_INSTANCE_MISMATCH: &str = "server_instance_mismatch";

/// Bind this store's team knowledge to `server_instance_id`, or refuse when
/// it is already bound to a different one (T116, FR-495, FR-496; D438,
/// `sync-namespaces.md` §10).
///
/// Called by [`merge_synced_team`] alone — see this section's module-level
/// note above for why [`propose_team`] must not call it too.
async fn bind_team_server_instance_tx(
    tx: &mut sqlx::SqliteConnection,
    server_instance_id: Uuid,
) -> Result<()> {
    match recorded_team_instance(tx).await? {
        Some(recorded) if recorded != server_instance_id => Err(StoreError::Refused {
            code: SERVER_INSTANCE_MISMATCH,
            message: format!(
                "this store's team knowledge came from server instance {recorded}; \
                 refusing to merge team knowledge from a different instance ({server_instance_id})"
            ),
        }),
        Some(_) => Ok(()),
        None => {
            sqlx::query(
                "INSERT INTO sync_cursor (namespace, pull_cursor, last_success_at, backoff_until, server_capability)
                 VALUES (?1, NULL, NULL, NULL, NULL)
                 ON CONFLICT (namespace) DO NOTHING",
            )
            .bind(SyncNamespace::Team(server_instance_id).key())
            .execute(&mut *tx)
            .await?;
            Ok(())
        }
    }
}

/// What proposing a team entry needs (T115, T123). Mirrors
/// [`NewPersonalKnowledge`] with `proposed_by_user_id` in place of
/// `owner_user_id` — there is no owner, only a traceable proposer
/// (FR-459) — and no server-instance argument, for the reason explained
/// above [`bind_team_server_instance_tx`].
pub struct NewTeamKnowledge<'a> {
    pub proposed_by_user_id: Uuid,
    pub knowledge_type: MemoryType,
    pub content: &'a str,
    pub topic_key: Option<&'a str>,
    pub value_key: Option<&'a str>,
    pub applicability: Vec<ApplicabilityFact>,
    /// `None` for a direct proposal; `Some` only when created by promotion
    /// (T124, out of this file's scope — see [`NewPersonalKnowledge::origin_digest`]
    /// for the identical rule on the personal side).
    pub origin_digest: Option<String>,
}

impl<'a> NewTeamKnowledge<'a> {
    /// A directly authored proposal — the shape T123 exercises.
    pub fn direct(
        proposed_by_user_id: Uuid,
        knowledge_type: MemoryType,
        content: &'a str,
        topic_key: Option<&'a str>,
        value_key: Option<&'a str>,
        applicability: Vec<ApplicabilityFact>,
    ) -> Self {
        Self {
            proposed_by_user_id,
            knowledge_type,
            content,
            topic_key,
            value_key,
            applicability,
            origin_digest: None,
        }
    }
}

/// What proposing a team entry turned out to mean for its subject. Mirrors
/// [`PersonalCreateOutcome`].
#[derive(Debug, Clone)]
pub struct TeamCreateOutcome {
    pub record: TeamKnowledge,
    pub reconciliation: ProposalOutcome,
    pub relation_recorded: Option<RelationKind>,
    pub matched_value_key: Option<String>,
    pub subject: Option<String>,
    pub notes: Vec<&'static str>,
}

/// The per-proposal reconciliation bound. See
/// [`PERSONAL_RECONCILE_MEMBERS_MAX`] for why this is its own constant
/// rather than shared.
/// The one definition of "a team entry that currently competes as guidance"
/// (FR-462, FR-463, D431).
///
/// **Written once and used by every canonical read**, because the alternative is
/// what this codebase actually had: `search_team`, `recall_team` and
/// `team_subject` each spelling `state = 'authoritative'` and each therefore
/// independently deciding what "current" means. Three spellings of one predicate
/// is three places for it to drift, and it had already drifted — none of them
/// consulted supersession, so an administrator's explicit `--supersedes` was
/// recorded and changed nothing a reader saw.
///
/// Two clauses, and both are necessary. `state = 'authoritative'` excludes a
/// proposal nobody ratified and an entry an admin retired. `superseded_by_id IS
/// NULL` excludes an entry a later ratification explicitly replaced — which
/// FR-462 names as one of exactly two ways an entry stops competing, the other
/// being retirement.
///
/// `alias` is the table alias the caller's query uses, so the same predicate
/// serves a bare `team_knowledge` query and an FTS join alike.
pub fn team_active_predicate(alias: &str) -> String {
    format!("{alias}.state = 'authoritative' AND {alias}.superseded_by_id IS NULL")
}

pub const TEAM_RECONCILE_MEMBERS_MAX: usize = 64;

/// The one subject-partition key every member of a `team_members_tx` query
/// shares (§6 of the contract: "there is exactly one team per server, so no
/// further partition key exists"). Any constant works here as long as every
/// row a reconciliation call sees carries the same one, which is what makes
/// `scope_overlap` report `Simultaneous` for every pair — see
/// [`personal_facts_from_row`]'s identical use of `owner_user_id` for the
/// personal domain.
const TEAM_SCOPE_KEY: &str = "team";

fn team_bare(row: &SqliteRow) -> Result<TeamKnowledge> {
    Ok(TeamKnowledge {
        id: rows::uuid(row, "id")?,
        knowledge_type: rows::enum_val(row, "knowledge_type")?,
        content: row.try_get("content")?,
        topic_key: row.try_get("topic_key")?,
        value_key: row.try_get("value_key")?,
        origin_digest: row.try_get("origin_digest")?,
        // Loaded separately by every caller — same split personal_bare keeps.
        applicability: Vec::new(),
        state: rows::enum_val(row, "state")?,
        proposed_by_user_id: rows::uuid(row, "proposed_by_user_id")?,
        ratified_by_user_id: rows::opt_uuid(row, "ratified_by_user_id")?,
        ratified_at: rows::opt_ts(row, "ratified_at")?,
        writer_id: rows::uuid(row, "writer_id")?,
        writer_seq: row.try_get("writer_seq")?,
        created_at: rows::ts(row, "created_at")?,
        superseded_by_id: rows::opt_uuid(row, "superseded_by_id")?,
        retired_by_user_id: rows::opt_uuid(row, "retired_by_user_id")?,
        retired_at: rows::opt_ts(row, "retired_at")?,
    })
}

/// Every applicability fact for `ids`, grouped by the record they belong to.
/// Mirrors [`applicability_for`] exactly, over `team_knowledge_applicability`.
async fn team_applicability_for(
    store: &Store,
    ids: &[Uuid],
) -> Result<HashMap<Uuid, Vec<ApplicabilityFact>>> {
    let mut map: HashMap<Uuid, Vec<ApplicabilityFact>> = HashMap::new();
    if ids.is_empty() {
        return Ok(map);
    }
    let placeholders = std::iter::repeat_n("?", ids.len())
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "SELECT team_id, kind, value FROM team_knowledge_applicability
          WHERE team_id IN ({placeholders})
          ORDER BY team_id, kind, value"
    );
    let mut q = sqlx::query(&sql);
    for id in ids {
        q = q.bind(id.to_string());
    }
    let rs = q.fetch_all(store.pool()).await?;
    for r in &rs {
        let tid = rows::uuid(r, "team_id")?;
        let kind = rows::enum_val(r, "kind")?;
        let value: String = r.try_get("value")?;
        map.entry(tid)
            .or_default()
            .push(ApplicabilityFact { kind, value });
    }
    Ok(map)
}

/// This writer's next position in `team_knowledge`'s stream — a stream
/// separate from `personal_knowledge`'s own (each has its own unique index
/// on `(writer_id, writer_seq)`; see [`next_personal_writer_seq`]).
async fn next_team_writer_seq(tx: &mut sqlx::SqliteConnection, writer: Uuid) -> Result<i64> {
    let max: Option<i64> =
        sqlx::query_scalar("SELECT MAX(writer_seq) FROM team_knowledge WHERE writer_id = ?1")
            .bind(writer.to_string())
            .fetch_one(&mut *tx)
            .await?;
    Ok(max.unwrap_or(0) + 1)
}

/// The columns `derive_subject`/`classify_proposal` read for one team member.
const TEAM_MEMBER_COLUMNS: &str = "id, topic_key, value_key, content_norm_digest, writer_id";

/// Map one `team_knowledge` row to the shared reconciliation type. Mirrors
/// [`personal_facts_from_row`]; see its comments for why each filled-in
/// constant is a real decision, not a placeholder.
fn team_facts_from_row(row: &SqliteRow) -> Result<MemoryFacts> {
    Ok(MemoryFacts {
        id: rows::uuid(row, "id")?,
        // The caller already filtered to `state = 'authoritative'` in SQL —
        // reconciliation and subject reads both only ever compare currently
        // authoritative entries against each other (§6 of the contract), so
        // every member handed to this mapper is active by construction.
        state: MemoryState::Active,
        // No `(project_id, scope, scope_key)` partition exists for team
        // knowledge — there is exactly one team per server (§6) — so every
        // member shares the same constant scope and scope key, which is what
        // makes `scope_overlap` report `Simultaneous` for every pair.
        scope: MemoryScope::Project,
        scope_key: TEAM_SCOPE_KEY.to_string(),
        topic_key: row.try_get("topic_key")?,
        value_key: row.try_get("value_key")?,
        content_norm_digest: row.try_get("content_norm_digest")?,
        // No verification field of any kind on team knowledge either
        // (FR-513) — see personal_facts_from_row's identical note.
        verification: VerificationState::Unverified,
        verification_authority: None,
        evidence_fact_count: 0,
        pinned: false,
        importance: Importance::Normal,
        origin_session_id: rows::uuid(row, "writer_id")?,
    })
}

/// Every currently *active* member of one subject, for reconciliation inside
/// the caller's transaction.
///
/// Mirrors [`personal_members_tx`], but over [`team_active_predicate`] rows
/// rather than every non-forgotten row: a proposal reconciles against what is
/// already authoritative and still competing, never against other pending
/// proposals (§6 of the contract) and never against guidance a later
/// ratification already replaced — a conflict recorded against superseded
/// guidance would be a conflict with something no reader can see.
async fn team_members_tx(
    tx: &mut sqlx::SqliteConnection,
    topic_key: &str,
    cap: usize,
) -> Result<(Vec<MemoryFacts>, bool)> {
    let sql = format!(
        "SELECT {TEAM_MEMBER_COLUMNS} FROM team_knowledge tk
          WHERE tk.topic_key = ?1 AND {active}
          ORDER BY tk.id
          LIMIT ?2",
        active = team_active_predicate("tk")
    );
    let rs = sqlx::query(&sql)
        .bind(topic_key)
        .bind(cap as i64 + 1)
        .fetch_all(&mut *tx)
        .await?;
    let over = rs.len() > cap;
    let members = rs
        .iter()
        .take(cap)
        .map(team_facts_from_row)
        .collect::<Result<Vec<_>>>()?;
    Ok((members, over))
}

/// Record one reconciliation decision among team entries. Mirrors
/// [`record_personal_relation_tx`]. Used for the two *automatic* kinds
/// (`duplicates`, `conflicts_with`) recorded at proposal time; the admin-only
/// `supersedes` kind is recorded by [`ratify_team`] through this same
/// function, never inferred (§6, D431).
async fn record_team_relation_tx(
    tx: &mut sqlx::SqliteConnection,
    decided_by_writer: Uuid,
    r: Relation,
) -> Result<bool> {
    let result = sqlx::query(
        "INSERT OR IGNORE INTO team_knowledge_relations
            (from_id, to_id, kind, basis, decided_by_writer, decided_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )
    .bind(r.from.to_string())
    .bind(r.to.to_string())
    .bind(r.kind.as_str())
    .bind(r.basis.as_str())
    .bind(decided_by_writer.to_string())
    .bind(rows::now_text())
    .execute(&mut *tx)
    .await?;
    Ok(result.rows_affected() > 0)
}

// ---------------------------------------------------------------------------
// Write: propose (T115, T123, T127)
// ---------------------------------------------------------------------------

/// Propose a team entry, screened, reconciled and committed atomically —
/// always landing `proposed`, never `authoritative` (FR-451, FR-455).
///
/// **T123 / FR-548, FR-580.** Same discipline as [`create_personal`]:
/// `validate_global_content` runs before this function's first statement, not
/// inside a transaction later rolled back, so a refused proposal touches the
/// database not at all. `project_identities` is the proposer's *current*
/// project's identity tokens — the third of `validate_global_content`'s five
/// entry points.
///
/// **T127.** Reconciliation reuses `classify_proposal` unchanged, over
/// currently `authoritative` team entries for the same `topic_key` only
/// (§6) — never over other pending proposals, and never writing a
/// `supersedes` relation: that kind is admin-only and explicit, recorded
/// only by [`ratify_team`] (D431). This is what keeps two disagreeing
/// authoritative entries both visible (FR-462) — nothing here ever resolves
/// a conflict on an admin's behalf.
///
/// No server-instance argument: see this section's module-level note for why.
pub async fn propose_team(
    store: &Store,
    new: NewTeamKnowledge<'_>,
    project_identities: &[ProjectIdentity],
) -> Result<TeamCreateOutcome> {
    validate_global_content(
        new.content,
        new.topic_key,
        new.value_key,
        &new.applicability,
        project_identities,
    )
    .map_err(|rejection| StoreError::Refused {
        code: rejection.class,
        message: format!("team knowledge proposal refused: {rejection}"),
    })?;

    let applicability: Vec<ApplicabilityFact> = new
        .applicability
        .iter()
        .map(|f| {
            let value = normalize_applicability_value(&f.value)
                .expect("validate_global_content already accepted this value");
            ApplicabilityFact {
                kind: f.kind,
                value,
            }
        })
        .collect();

    let mut notes: Vec<&'static str> = Vec::new();
    let topic_key = match new.topic_key {
        Some(raw) => {
            let normalized = normalize_topic_key(raw);
            if normalized.is_none() {
                notes.push(codes::INVALID_TOPIC_KEY);
            }
            normalized
        }
        None => None,
    };
    let value_key = match (new.value_key, topic_key.as_ref()) {
        (Some(raw), Some(_)) => normalize_value_key(raw),
        (Some(_), None) => {
            notes.push(codes::VALUE_WITHOUT_TOPIC);
            None
        }
        (None, _) => None,
    };
    let digest = content_norm_digest(new.content);

    let id = new_id();
    let now = rows::now_text();

    let mut tx = tx::begin(store, "propose_team").await?;
    let writer = writer_id(&mut tx).await?;
    let writer_seq = next_team_writer_seq(&mut tx, writer).await?;

    sqlx::query(
        "INSERT INTO team_knowledge
            (id, knowledge_type, content, topic_key, value_key, content_norm_digest,
             origin_digest, state, proposed_by_user_id, ratified_by_user_id, ratified_at,
             writer_id, writer_seq, created_at, superseded_by_id, retired_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'proposed', ?8, NULL, NULL, ?9, ?10, ?11, NULL, NULL)",
    )
    .bind(id.to_string())
    .bind(new.knowledge_type.as_str())
    .bind(new.content)
    .bind(topic_key.as_deref())
    .bind(value_key.as_deref())
    .bind(&digest)
    .bind(new.origin_digest.as_deref())
    .bind(new.proposed_by_user_id.to_string())
    .bind(writer.to_string())
    .bind(writer_seq)
    .bind(&now)
    .execute(&mut *tx)
    .await?;

    for fact in &applicability {
        sqlx::query(
            "INSERT INTO team_knowledge_applicability (team_id, kind, value)
             VALUES (?1, ?2, ?3)",
        )
        .bind(id.to_string())
        .bind(fact.kind.as_str())
        .bind(&fact.value)
        .execute(&mut *tx)
        .await?;
    }

    let mut reconciliation = ProposalOutcome::Created;
    let mut relation_recorded: Option<RelationKind> = None;
    let mut matched_value_key: Option<String> = None;
    if let Some(topic) = topic_key.as_deref() {
        let (members, over_bound) =
            team_members_tx(&mut tx, topic, TEAM_RECONCILE_MEMBERS_MAX).await?;

        let proposal = MemoryFacts {
            id,
            state: MemoryState::Active,
            scope: MemoryScope::Project,
            scope_key: TEAM_SCOPE_KEY.to_string(),
            topic_key: Some(topic.to_string()),
            value_key: value_key.clone(),
            content_norm_digest: Some(digest.clone()),
            verification: VerificationState::Unverified,
            verification_authority: None,
            evidence_fact_count: 0,
            pinned: false,
            importance: Importance::Normal,
            origin_session_id: writer,
        };

        if over_bound {
            reconciliation = ProposalOutcome::Deferred;
            notes.push(codes::RECONCILIATION_DEFERRED);
        } else {
            let (outcome, relations) =
                classify_proposal(&proposal, &members, TEAM_RECONCILE_MEMBERS_MAX);
            let mut kinds: Vec<RelationKind> = Vec::new();
            for r in relations {
                kinds.push(r.kind);
                record_team_relation_tx(&mut tx, writer, r).await?;
            }
            if matches!(outcome, ProposalOutcome::Corroborating { .. }) {
                notes.push(codes::CORROBORATING_MEMBER);
            }
            relation_recorded = kinds.first().copied();
            matched_value_key = match &outcome {
                ProposalOutcome::Duplicate { of } => members
                    .iter()
                    .find(|m| m.id == *of)
                    .and_then(|m| m.value_key.clone()),
                ProposalOutcome::Corroborating { member } => members
                    .iter()
                    .find(|m| m.id == *member)
                    .and_then(|m| m.value_key.clone()),
                _ => None,
            };
            reconciliation = outcome;
        }
    }

    // T100. A proposal that never leaves the machine cannot be ratified: the
    // admin who would ratify it is on another device, so this queue entry is
    // the whole path from "someone proposed guidance" to "someone can approve
    // it". Ratification and retirement deliberately do **not** enqueue — they
    // are admin-token API calls against the server (`global-memory.md` §5b),
    // and the server's own ingest refuses to advance `state` from a pushed
    // payload, so a queue entry claiming a state change would be a no-op that
    // read like a delivery.
    enqueue_team_tx(
        &mut tx,
        id,
        writer,
        new.proposed_by_user_id,
        &team_payload(
            id,
            new.knowledge_type,
            new.content,
            topic_key.as_deref(),
            value_key.as_deref(),
            writer,
            writer_seq,
            &applicability,
            TeamState::Proposed,
            new.proposed_by_user_id,
            None,
            None,
            None,
            None,
        ),
    )
    .await?;

    tx::commit(tx, "propose_team").await?;
    // Not `recall_team`: the proposer reading back what they just proposed is
    // not recall, the same way `create_personal`'s own read-back is not
    // (T120 gates recall, not "read the row you just wrote").
    let record = team_row_any_state(store, id).await?;
    Ok(TeamCreateOutcome {
        record,
        reconciliation,
        relation_recorded,
        matched_value_key,
        subject: topic_key,
        notes,
    })
}

/// Fetch one team row by id, regardless of state. Not gated by visibility —
/// used only where the caller is already entitled to see this exact row: the
/// proposer reading back their own new proposal, or [`ratify_team`] /
/// [`retire_team`] / [`merge_synced_team`] reading back the row they just
/// transitioned. Never exposed as a general-purpose lookup, unlike
/// [`get_personal`]: personal privacy is by owner, not by state, so an
/// unrestricted-by-owner personal getter would leak; here an
/// unrestricted-by-state getter would leak a `proposed` row's content to
/// anyone who merely knows its id, which is exactly what FR-452 forbids.
/// One team entry by id, whatever its state.
///
/// Public so the subject read can render an answer's content: `MemoryFacts`
/// carries a content digest and not the content, which is right for the
/// classifier and useless to a human asking what the guidance actually says.
pub async fn team_entry(store: &Store, id: Uuid) -> Result<TeamKnowledge> {
    team_row_any_state(store, id).await
}

async fn team_row_any_state(store: &Store, id: Uuid) -> Result<TeamKnowledge> {
    let row = sqlx::query("SELECT * FROM team_knowledge WHERE id = ?1")
        .bind(id.to_string())
        .fetch_optional(store.pool())
        .await?
        .ok_or_else(|| StoreError::NotFound(format!("team knowledge {id}")))?;
    let mut record = team_bare(&row)?;
    record.applicability = team_applicability_for(store, &[id])
        .await?
        .remove(&id)
        .unwrap_or_default();
    Ok(record)
}

// ---------------------------------------------------------------------------
// The lifecycle: ratify, retire (T119, T121, T122)
// ---------------------------------------------------------------------------

/// Refusal code for the team-lifecycle compare-and-swap (T119, T121, FR-454,
/// SC-415). Local to this module for the same reason
/// [`SERVER_INSTANCE_MISMATCH`] is.
pub const STATE_CONFLICT: &str = "state_conflict";

fn state_conflict(actual: TeamState) -> StoreError {
    StoreError::Refused {
        code: STATE_CONFLICT,
        message: format!(
            "team knowledge is at state {}, not the state this request required",
            actual.as_str()
        ),
    }
}

/// The row's actual state, read inside the caller's already-locked
/// transaction (T119). `NotFound` for a missing id.
async fn team_state_for_update(tx: &mut sqlx::SqliteConnection, id: Uuid) -> Result<TeamState> {
    let row = sqlx::query("SELECT state FROM team_knowledge WHERE id = ?1")
        .bind(id.to_string())
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| StoreError::NotFound(format!("team knowledge {id}")))?;
    rows::enum_val(&row, "state")
}

/// Ratify a proposal: `proposed → authoritative` (T119, T121, FR-453,
/// FR-454, FR-457).
///
/// **The compare-and-swap** (T119, reusing `crate::criteria::check_revision`'s
/// shape): the row is read for its actual state inside this call's own
/// `BEGIN IMMEDIATE` transaction — the same "the write lock is already held
/// before the read" argument that makes `next_personal_writer_seq`'s
/// `MAX + 1` race-free makes this read-then-write race-free too — and the
/// `UPDATE`'s own `WHERE state = 'proposed'` is kept as a second, belt-and-
/// suspenders guard identical to the contract's own SQL (§3). A row not
/// currently `proposed` is refused, **naming its actual state**
/// (`state_conflict`) — this is what T137's concurrent-ratification test and
/// T141's retired-refuses-re-ratification test both rest on: two concurrent
/// callers race on SQLite's write lock, the loser's read observes the
/// winner's already-committed `authoritative` state, and it is refused by
/// name rather than silently reapplied on top.
///
/// **Admin-only** (T121) is enforced by the caller, not here — see this
/// section's module-level note on why this module cannot check a role at
/// all.
///
/// `supersedes`, when given, is the ratifying admin's own explicit act
/// (T127, §6, D431) — recorded here, in the same transaction as the
/// ratification, and nowhere else in this module infers it.
pub async fn ratify_team(
    store: &Store,
    id: Uuid,
    ratified_by_user_id: Uuid,
    supersedes: Option<Uuid>,
) -> Result<TeamKnowledge> {
    let mut tx = tx::begin(store, "ratify_team").await?;
    let actual = team_state_for_update(&mut tx, id).await?;
    if actual != TeamState::Proposed {
        return Err(state_conflict(actual));
    }

    let now = rows::now_text();
    let result = sqlx::query(
        "UPDATE team_knowledge
            SET state = 'authoritative', ratified_by_user_id = ?1, ratified_at = ?2
          WHERE id = ?3 AND state = 'proposed'",
    )
    .bind(ratified_by_user_id.to_string())
    .bind(&now)
    .bind(id.to_string())
    .execute(&mut *tx)
    .await?;
    if result.rows_affected() == 0 {
        // Unreachable under BEGIN IMMEDIATE given the read above already
        // observed 'proposed' inside this same transaction — kept as a
        // named, typed refusal rather than an assertion, so a future change
        // to this transaction's isolation cannot turn into a silent no-op.
        return Err(state_conflict(TeamState::Proposed));
    }

    if let Some(sup) = supersedes {
        let writer = writer_id(&mut tx).await?;
        record_team_relation_tx(
            &mut tx,
            writer,
            Relation {
                from: id,
                to: sup,
                kind: RelationKind::Supersedes,
                basis: RelationBasis::ExplicitUser,
            },
        )
        .await?;
        // **And the pointer, in the same transaction** (FR-462). The relation
        // records *that the admin decided*; the pointer is what every canonical
        // read consults (`team_active_predicate`) and what crosses the wire, so
        // a second device learns the old guidance was replaced without needing
        // the relation itself to synchronize. Recording only the relation is
        // what made `--supersedes` inert: the decision existed and nothing read
        // it.
        //
        // Guarded on the target still being authoritative and not already
        // superseded, so this cannot resurrect a retired entry into a
        // superseded one or overwrite an earlier supersession.
        sqlx::query(
            "UPDATE team_knowledge SET superseded_by_id = ?1
              WHERE id = ?2 AND state = 'authoritative' AND superseded_by_id IS NULL",
        )
        .bind(id.to_string())
        .bind(sup.to_string())
        .execute(&mut *tx)
        .await?;
    }

    tx::commit(tx, "ratify_team").await?;
    // No `store.checkpoint()` here.
    //
    // `PRAGMA wal_checkpoint(TRUNCATE)` takes an exclusive lock and is reserved
    // for **deletions** — it exists so removed content leaves the write-ahead log
    // rather than lingering in an old frame (FR-052, `Store::checkpoint`). This
    // path removes nothing: it changes a lifecycle column and leaves content
    // untouched.
    //
    // Calling it here cost real concurrency. The pull path runs it once per
    // merged row in the background worker, so a machine catching up took an
    // exclusive checkpoint lock repeatedly while foreground commands were
    // writing, and `cairn connect` began failing with "database is locked" about
    // half the time — under a busy timeout that a truncating checkpoint does not
    // wait out.
    team_row_any_state(store, id).await
}

/// Retire an authoritative entry: `authoritative → retired` (T119, T121,
/// FR-456, FR-457, FR-461, FR-465).
///
/// Same compare-and-swap shape as [`ratify_team`], hardcoding
/// `WHERE state = 'authoritative'` — which is exactly what makes retiring a
/// `retired` or still-`proposed` row refuse, naming that actual state,
/// rather than silently doing nothing or half-applying. Content is never
/// touched (FR-461): the `UPDATE` names only `state` and `retired_at`.
pub async fn retire_team(
    store: &Store,
    id: Uuid,
    retired_by_user_id: Uuid,
) -> Result<TeamKnowledge> {
    let mut tx = tx::begin(store, "retire_team").await?;
    let actual = team_state_for_update(&mut tx, id).await?;
    if actual != TeamState::Authoritative {
        return Err(state_conflict(actual));
    }

    let now = rows::now_text();
    // Who **and** when (FR-457). Ratification already recorded both; retirement
    // recorded only the clock, which does not satisfy "every state transition is
    // recorded with who acted and when" — and retirement is the transition most
    // worth attributing, since it removes guidance from every user on the
    // server.
    let result = sqlx::query(
        "UPDATE team_knowledge
            SET state = 'retired', retired_at = ?1, retired_by_user_id = ?3
          WHERE id = ?2 AND state = 'authoritative'",
    )
    .bind(&now)
    .bind(id.to_string())
    .bind(retired_by_user_id.to_string())
    .execute(&mut *tx)
    .await?;
    if result.rows_affected() == 0 {
        return Err(state_conflict(TeamState::Authoritative));
    }

    tx::commit(tx, "retire_team").await?;
    // No `store.checkpoint()` here.
    //
    // `PRAGMA wal_checkpoint(TRUNCATE)` takes an exclusive lock and is reserved
    // for **deletions** — it exists so removed content leaves the write-ahead log
    // rather than lingering in an old frame (FR-052, `Store::checkpoint`). This
    // path removes nothing: it changes a lifecycle column and leaves content
    // untouched.
    //
    // Calling it here cost real concurrency. The pull path runs it once per
    // merged row in the background worker, so a machine catching up took an
    // exclusive checkpoint lock repeatedly while foreground commands were
    // writing, and `cairn connect` began failing with "database is locked" about
    // half the time — under a busy timeout that a truncating checkpoint does not
    // wait out.
    team_row_any_state(store, id).await
}

// ---------------------------------------------------------------------------
// Read: recall (T115, T120, T128, T131)
// ---------------------------------------------------------------------------

/// Recall authoritative team entries, applying the applicability predicate at
/// query time (T128, FR-434–FR-436, FR-460) — the same read-time filter
/// [`recall_personal`] applies, over the same closed vocabulary.
///
/// **Hardcodes `state = 'authoritative'`, unconditionally** (T120, FR-452):
/// there is no argument to this function that could ever surface a
/// `proposed` or `retired` row, not even for the proposer. **Takes no
/// project or membership argument at all** (T131, FR-458, FR-463): an
/// authoritative entry is a server-wide default, so there is nothing here to
/// scope by project membership, which is what makes "visible to a user with
/// zero memberships" true by construction.
pub async fn recall_team(
    store: &Store,
    query: Option<&str>,
    topic_key: Option<&str>,
    project_traits: &[ProjectTrait],
    limit: i64,
) -> Result<Vec<TeamKnowledge>> {
    let limit = limit.clamp(1, RECALL_MAX_LIMIT) as usize;

    let mut sql = String::from("SELECT tk.* FROM team_knowledge tk");
    if query.is_some() {
        sql.push_str(
            " JOIN team_fts ON team_fts.rowid = tk.rowid
              WHERE team_fts MATCH ?",
        );
    } else {
        sql.push_str(" WHERE 1 = 1");
    }
    sql.push_str(&format!(" AND {}", team_active_predicate("tk")));
    if topic_key.is_some() {
        sql.push_str(" AND tk.topic_key = ?");
    }
    sql.push_str(" ORDER BY tk.created_at DESC LIMIT ?");

    let mut q = sqlx::query(&sql);
    if let Some(text) = query {
        q = q.bind(fts_query(text));
    }
    if let Some(t) = topic_key {
        q = q.bind(t);
    }
    q = q.bind(RECALL_CANDIDATE_CAP);

    let rs = q.fetch_all(store.pool()).await?;
    let mut records: Vec<TeamKnowledge> = rs.iter().map(team_bare).collect::<Result<_>>()?;

    let ids: Vec<Uuid> = records.iter().map(|r| r.id).collect();
    let mut app_map = team_applicability_for(store, &ids).await?;
    for r in &mut records {
        r.applicability = app_map.remove(&r.id).unwrap_or_default();
    }

    records.retain(|r| applies(&r.applicability, project_traits));
    records.truncate(limit);
    Ok(records)
}

// ---------------------------------------------------------------------------
// Read: role-filtered listing (T132)
// ---------------------------------------------------------------------------

/// Who is listing team knowledge, and therefore which `proposed` rows they
/// may see (T132, FR-464). This is `list_team`'s own concern, separate from
/// [`recall_team`]'s: a listing, not recall — see this section's
/// module-level note.
///
/// `Admin` is asserted by the caller, not verified here — see this section's
/// module-level note on why this module cannot check a role at all.
pub enum TeamViewer {
    /// An ordinary member: sees authoritative entries and their own
    /// proposals.
    Member(Uuid),
    /// Sees every state.
    Admin,
}

/// List team knowledge, role-filtered (T132, FR-464): a member sees
/// authoritative entries and their own proposals; an admin sees every state.
/// A retired entry is visible to an admin only — not to the member who
/// proposed it, matching FR-464's literal text ("a member sees authoritative
/// entries **and their own proposals**", not every state their own proposal
/// ever passed through).
pub async fn list_team(
    store: &Store,
    viewer: &TeamViewer,
    limit: i64,
) -> Result<Vec<TeamKnowledge>> {
    let limit = limit.clamp(1, RECALL_MAX_LIMIT);
    let rs = match viewer {
        TeamViewer::Admin => {
            sqlx::query("SELECT * FROM team_knowledge ORDER BY created_at DESC LIMIT ?1")
                .bind(limit)
                .fetch_all(store.pool())
                .await?
        }
        TeamViewer::Member(user) => {
            sqlx::query(
                "SELECT * FROM team_knowledge
              WHERE state = 'authoritative'
                 OR (state = 'proposed' AND proposed_by_user_id = ?1)
              ORDER BY created_at DESC LIMIT ?2",
            )
            .bind(user.to_string())
            .bind(limit)
            .fetch_all(store.pool())
            .await?
        }
    };

    let mut records: Vec<TeamKnowledge> = rs.iter().map(team_bare).collect::<Result<_>>()?;
    let ids: Vec<Uuid> = records.iter().map(|r| r.id).collect();
    let mut app_map = team_applicability_for(store, &ids).await?;
    for r in &mut records {
        r.applicability = app_map.remove(&r.id).unwrap_or_default();
    }
    Ok(records)
}

// ---------------------------------------------------------------------------
// The subject read (T115, T127)
// ---------------------------------------------------------------------------

/// A team subject, read and derived — mirrors [`PersonalSubjectRead`].
#[derive(Debug, Clone)]
pub struct TeamSubjectRead {
    pub view: SubjectView,
    /// Every currently authoritative member of the subject — never a
    /// `proposed` or `retired` one (§6).
    pub members: Vec<MemoryFacts>,
}

/// Read one team subject's authoritative members and derive its canonical
/// answer (T127, FR-442, FR-462, FR-493).
///
/// Reuses `derive_subject` unchanged, over `team_knowledge_relations` only —
/// never `personal_knowledge_relations` or `memory_relations` (§6: "no
/// cross-domain relations" is structural, upheld here by which table is
/// queried). Because `record_team_relation_tx` is the only writer of a
/// `supersedes` row for team knowledge, and it is called only from
/// [`ratify_team`] on an explicit admin instruction, two disagreeing
/// authoritative entries on the same subject surface here as
/// `Reconciliation::Conflicted` with both retained — never one silently
/// preferred by ratification order (FR-462, SC-466).
pub async fn team_subject(store: &Store, topic_key: &str) -> Result<TeamSubjectRead> {
    let Some(topic) = normalize_topic_key(topic_key) else {
        return Ok(TeamSubjectRead {
            view: derive_subject(&[], &[]),
            members: Vec::new(),
        });
    };

    let sql = format!(
        "SELECT {TEAM_MEMBER_COLUMNS} FROM team_knowledge tk
          WHERE tk.topic_key = ?1 AND {active}
          ORDER BY tk.id
          LIMIT ?2",
        active = team_active_predicate("tk")
    );
    let rs = sqlx::query(&sql)
        .bind(&topic)
        .bind(TEAM_RECONCILE_MEMBERS_MAX as i64)
        .fetch_all(store.pool())
        .await?;
    let members = rs
        .iter()
        .map(team_facts_from_row)
        .collect::<Result<Vec<_>>>()?;

    let ids: Vec<Uuid> = members.iter().map(|m| m.id).collect();
    let relations = team_relations_touching(store, &ids).await?;
    let view = derive_subject(&members, &relations);
    Ok(TeamSubjectRead { view, members })
}

/// Every decision touching any of `ids`, read from `team_knowledge_relations`
/// only. Mirrors [`personal_relations_touching`].
async fn team_relations_touching(store: &Store, ids: &[Uuid]) -> Result<Vec<Relation>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let placeholders = std::iter::repeat_n("?", ids.len())
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "SELECT from_id, to_id, kind, basis FROM team_knowledge_relations
          WHERE deleted_at IS NULL
            AND (from_id IN ({placeholders}) OR to_id IN ({placeholders}))
          ORDER BY from_id, to_id, kind"
    );
    let mut q = sqlx::query(&sql);
    for id in ids.iter().chain(ids.iter()) {
        q = q.bind(id.to_string());
    }
    let rs = q.fetch_all(store.pool()).await?;
    rs.iter()
        .map(|r| {
            Ok(Relation {
                from: rows::uuid(r, "from_id")?,
                to: rows::uuid(r, "to_id")?,
                kind: rows::enum_val(r, "kind")?,
                basis: rows::enum_val(r, "basis")?,
            })
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Merge: the one server-instance-bound entry point (T116)
// ---------------------------------------------------------------------------

/// One team_knowledge row as synchronized from a specific server instance —
/// the shape the pull-apply caller (`cairnd::sync::merge_pulled_team`) hands to
/// [`merge_synced_team`].
///
/// Carries every field `team_knowledge` stores except `origin_digest`, which is
/// local-only and never transmitted (§6, D434) — a synchronized row therefore
/// never carries one; [`merge_synced_team`] always stores `NULL` for it.
///
/// **`retired_by_user_id` is one of those fields, and it was missing.** FR-457
/// requires every state transition to record who acted *and* when, and the two
/// halves of the lifecycle were treated asymmetrically: ratification's actor
/// crossed the wire, retirement's did not. A device that learned of a retirement
/// by pulling it saw the timestamp and no actor, so "who removed this guidance"
/// was answerable on the server and on the machine that did it, and nowhere
/// else. That is a smaller record than the requirement asks for, and it also
/// falsified this type's own stated invariant.
pub struct SyncedTeamKnowledge {
    pub id: Uuid,
    pub knowledge_type: MemoryType,
    pub content: String,
    pub topic_key: Option<String>,
    pub value_key: Option<String>,
    pub applicability: Vec<ApplicabilityFact>,
    pub state: TeamState,
    pub proposed_by_user_id: Uuid,
    pub ratified_by_user_id: Option<Uuid>,
    pub ratified_at: Option<DateTime<Utc>>,
    pub writer_id: Uuid,
    pub writer_seq: i64,
    pub created_at: DateTime<Utc>,
    pub superseded_by_id: Option<Uuid>,
    /// Who retired it (FR-457). Travels for the same reason
    /// `ratified_by_user_id` does: the transition is not fully recorded without
    /// it.
    pub retired_by_user_id: Option<Uuid>,
    pub retired_at: Option<DateTime<Utc>>,
}

/// Merge one team_knowledge row synchronized from `server_instance_id` into
/// this store (T116, FR-495, FR-496, FR-582).
///
/// **The asymmetric half of D438.** Refuses, reporting the mismatch rather
/// than silently dropping the row, when this store's team knowledge is
/// already bound to a *different* server instance
/// ([`bind_team_server_instance_tx`]) — team knowledge is a claim about one
/// specific server's ratification history, so a different server's guidance
/// (a different deployment, a restored-from-backup instance, a staging
/// server) must never silently blend into this store's team corpus
/// (`sync-namespaces.md` §10). Contrast [`create_personal`], which never
/// takes or checks a server-instance argument at all: personal knowledge is
/// partitioned by owning identity instead, and one store may legitimately
/// hold more than one identity's personal knowledge side by side.
///
/// **No re-validation of content here.** This is not one of
/// `validate_global_content`'s five entry points (direct personal creation,
/// personal promotion, team proposal, team promotion, server-side sync
/// ingest) — it is downstream of the fifth: content was already screened
/// server-side, before this store's peer ever accepted the push that made
/// this row exist to be pulled. Re-running the screen here would be a second
/// implementation of a check that already ran, which is exactly what
/// FR-579's discipline exists to prevent.
///
/// **Content is set once, at first insert, and never updated again.** An id
/// already present in this store has only its lifecycle columns (`state`,
/// `ratified_by_user_id`, `ratified_at`, `retired_at`, `superseded_by_id`)
/// moved to match the incoming row — the same "no `UPDATE … SET content`
/// beyond the tombstone/CAS" discipline §3 of the contract states, upheld
/// here even against an incoming row that (through a bug or a malicious
/// peer) disagreed with the stored content: this function has no SQL
/// statement capable of writing to `content` a second time.
pub async fn merge_synced_team(
    store: &Store,
    server_instance_id: Uuid,
    incoming: SyncedTeamKnowledge,
) -> Result<TeamKnowledge> {
    let mut tx = tx::begin(store, "merge_synced_team").await?;
    bind_team_server_instance_tx(&mut tx, server_instance_id).await?;

    let existing: Option<i64> = sqlx::query_scalar("SELECT 1 FROM team_knowledge WHERE id = ?1")
        .bind(incoming.id.to_string())
        .fetch_optional(&mut *tx)
        .await?;

    if existing.is_none() {
        let digest = content_norm_digest(&incoming.content);
        sqlx::query(
            "INSERT INTO team_knowledge
                (id, knowledge_type, content, topic_key, value_key, content_norm_digest,
                 origin_digest, state, proposed_by_user_id, ratified_by_user_id, ratified_at,
                 writer_id, writer_seq, created_at, superseded_by_id, retired_by_user_id,
                 retired_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
        )
        .bind(incoming.id.to_string())
        .bind(incoming.knowledge_type.as_str())
        .bind(&incoming.content)
        .bind(incoming.topic_key.as_deref())
        .bind(incoming.value_key.as_deref())
        .bind(&digest)
        .bind(incoming.state.as_str())
        .bind(incoming.proposed_by_user_id.to_string())
        .bind(incoming.ratified_by_user_id.map(|u| u.to_string()))
        .bind(incoming.ratified_at.map(rows::ts_text))
        .bind(incoming.writer_id.to_string())
        .bind(incoming.writer_seq)
        .bind(rows::ts_text(incoming.created_at))
        .bind(incoming.superseded_by_id.map(|u| u.to_string()))
        .bind(incoming.retired_by_user_id.map(|u| u.to_string()))
        .bind(incoming.retired_at.map(rows::ts_text))
        .execute(&mut *tx)
        .await?;

        for fact in &incoming.applicability {
            sqlx::query(
                "INSERT INTO team_knowledge_applicability (team_id, kind, value)
                 VALUES (?1, ?2, ?3)",
            )
            .bind(incoming.id.to_string())
            .bind(fact.kind.as_str())
            .bind(&fact.value)
            .execute(&mut *tx)
            .await?;
        }
    } else {
        // **A merge never walks a row's state backwards** (FR-609).
        //
        // Team state only advances: proposed, then authoritative, then retired.
        // A page fetched from the server before a ratification and merged after
        // it carries the row as it was *then*, and this update applied it
        // wholesale — turning a locally ratified entry back into a proposal, and
        // with it dropping the supersession that ratification had recorded. The
        // window is small and the loss is silent: the entry simply stops being
        // guidance, and the one it replaced goes on competing.
        //
        // The server already refuses to let a pushed payload advance state
        // (`global-memory.md` §5b); this is the same principle facing the other
        // way. Delivery order is not something either side can promise, so
        // "later" has to mean further along, not more recently arrived.
        sqlx::query(
            "UPDATE team_knowledge
                SET state = ?1, ratified_by_user_id = ?2, ratified_at = ?3,
                    retired_at = ?4, superseded_by_id = ?5, retired_by_user_id = ?7
              WHERE id = ?6
                AND (CASE state
                        WHEN 'proposed' THEN 0
                        WHEN 'authoritative' THEN 1
                        WHEN 'retired' THEN 2
                        ELSE 0 END)
                    <= (CASE ?1
                        WHEN 'proposed' THEN 0
                        WHEN 'authoritative' THEN 1
                        WHEN 'retired' THEN 2
                        ELSE 0 END)",
        )
        .bind(incoming.state.as_str())
        .bind(incoming.ratified_by_user_id.map(|u| u.to_string()))
        .bind(incoming.ratified_at.map(rows::ts_text))
        .bind(incoming.retired_at.map(rows::ts_text))
        .bind(incoming.superseded_by_id.map(|u| u.to_string()))
        .bind(incoming.id.to_string())
        .bind(incoming.retired_by_user_id.map(|u| u.to_string()))
        .execute(&mut *tx)
        .await?;
    }

    tx::commit(tx, "merge_synced_team").await?;
    // No `store.checkpoint()` here.
    //
    // `PRAGMA wal_checkpoint(TRUNCATE)` takes an exclusive lock and is reserved
    // for **deletions** — it exists so removed content leaves the write-ahead log
    // rather than lingering in an old frame (FR-052, `Store::checkpoint`). This
    // path removes nothing: it changes a lifecycle column and leaves content
    // untouched.
    //
    // Calling it here cost real concurrency. The pull path runs it once per
    // merged row in the background worker, so a machine catching up took an
    // exclusive checkpoint lock repeatedly while foreground commands were
    // writing, and `cairn connect` began failing with "database is locked" about
    // half the time — under a busy timeout that a truncating checkpoint does not
    // wait out.
    team_row_any_state(store, incoming.id).await
}

// ---------------------------------------------------------------------------
// Personal merge (T101 local half, FR-489, FR-567, FR-582)
// ---------------------------------------------------------------------------

/// One `personal_knowledge` row arriving from the server.
///
/// Carries no `origin_digest`, for the same reason [`SyncedTeamKnowledge`] does
/// not: the digest is local to the machine that computed it and never crosses
/// the wire (D434, FR-551). A pulled row therefore always stores `NULL` for it,
/// which is correct rather than lossy — the digest identifies *this* machine's
/// promotion of *this* project, and a different machine's copy of the record
/// was not promoted here.
pub struct SyncedPersonalKnowledge {
    pub id: Uuid,
    pub owner_user_id: Uuid,
    pub knowledge_type: MemoryType,
    pub content: String,
    pub topic_key: Option<String>,
    pub value_key: Option<String>,
    pub applicability: Vec<ApplicabilityFact>,
    pub writer_id: Uuid,
    pub writer_seq: i64,
    pub created_at: DateTime<Utc>,
    pub superseded_by_id: Option<Uuid>,
    pub forgotten_at: Option<DateTime<Utc>>,
}

/// Merge one personal row synchronized from the server into this store
/// (T101, FR-489, FR-567, FR-582).
///
/// **The other half of D438's asymmetry.** [`merge_synced_team`] refuses a row
/// from a second server instance; this function takes no server-instance
/// argument at all and has nothing to refuse. Personal knowledge is partitioned
/// by `owner_user_id` — and a user account is per-server, so two servers are
/// already two owners — which is why a local store may legitimately hold the
/// personal knowledge of more than one identity side by side, with recall
/// surfacing only the one currently authenticated (`sync-namespaces.md` §10,
/// FR-567).
///
/// **Content is set once, at first insert, and never updated again.** As with
/// team knowledge, an id already present here has only its two lifecycle
/// columns — `superseded_by_id` and `forgotten_at` — moved to match. There is
/// no statement in this function capable of writing `content` a second time, so
/// an incoming row that disagreed with the stored content (through a bug, or a
/// peer acting in bad faith) cannot rewrite what this store already recorded.
/// The one exception is the tombstone, and it is not an exception to the rule:
/// forgetting clears content to the empty string, and a `forgotten_at` that
/// arrives here does exactly that, because a record forgotten on one device and
/// still recallable on the next is the failure the tombstone exists to prevent.
///
/// **No content re-validation.** This is downstream of the fifth validator
/// entry point, not a sixth one: the content was screened server-side before
/// the push that created the row this pull is reading. Re-screening here would
/// be a second implementation of a check that already ran, which is what
/// FR-579's single-implementation discipline exists to prevent.
pub async fn merge_synced_personal(
    store: &Store,
    incoming: SyncedPersonalKnowledge,
) -> Result<PersonalKnowledge> {
    let mut tx = tx::begin(store, "merge_synced_personal").await?;

    let existing: Option<i64> =
        sqlx::query_scalar("SELECT 1 FROM personal_knowledge WHERE id = ?1 AND owner_user_id = ?2")
            .bind(incoming.id.to_string())
            .bind(incoming.owner_user_id.to_string())
            .fetch_optional(&mut *tx)
            .await?;

    if existing.is_none() {
        let digest = content_norm_digest(&incoming.content);
        sqlx::query(
            "INSERT INTO personal_knowledge
                (id, owner_user_id, knowledge_type, content, topic_key, value_key,
                 content_norm_digest, origin_digest, writer_id, writer_seq, created_at,
                 superseded_by_id, forgotten_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, ?8, ?9, ?10, ?11, ?12)",
        )
        .bind(incoming.id.to_string())
        .bind(incoming.owner_user_id.to_string())
        .bind(incoming.knowledge_type.as_str())
        .bind(&incoming.content)
        .bind(incoming.topic_key.as_deref())
        .bind(incoming.value_key.as_deref())
        .bind(&digest)
        .bind(incoming.writer_id.to_string())
        .bind(incoming.writer_seq)
        .bind(rows::ts_text(incoming.created_at))
        .bind(incoming.superseded_by_id.map(|u| u.to_string()))
        .bind(incoming.forgotten_at.map(rows::ts_text))
        .execute(&mut *tx)
        .await?;

        for fact in &incoming.applicability {
            sqlx::query(
                "INSERT OR IGNORE INTO personal_knowledge_applicability (personal_id, kind, value)
                 VALUES (?1, ?2, ?3)",
            )
            .bind(incoming.id.to_string())
            .bind(fact.kind.as_str())
            .bind(&fact.value)
            .execute(&mut *tx)
            .await?;
        }
    } else {
        // The tombstone travels as `forgotten_at`, and applying it clears
        // content here exactly as `forget_personal` does locally. `CASE WHEN`
        // rather than an unconditional assignment: a row that arrives without a
        // tombstone must leave content alone, and a statement that could write
        // content in that case is the one this function promises not to have.
        sqlx::query(
            "UPDATE personal_knowledge
                SET superseded_by_id = ?1,
                    forgotten_at = ?2,
                    content = CASE WHEN ?2 IS NULL THEN content ELSE '' END
              WHERE id = ?3 AND owner_user_id = ?4",
        )
        .bind(incoming.superseded_by_id.map(|u| u.to_string()))
        .bind(incoming.forgotten_at.map(rows::ts_text))
        .bind(incoming.id.to_string())
        .bind(incoming.owner_user_id.to_string())
        .execute(&mut *tx)
        .await?;
    }

    tx::commit(tx, "merge_synced_personal").await?;
    // No `store.checkpoint()` here.
    //
    // `PRAGMA wal_checkpoint(TRUNCATE)` takes an exclusive lock and is reserved
    // for **deletions** — it exists so removed content leaves the write-ahead log
    // rather than lingering in an old frame (FR-052, `Store::checkpoint`). This
    // path removes nothing: it changes a lifecycle column and leaves content
    // untouched.
    //
    // Calling it here cost real concurrency. The pull path runs it once per
    // merged row in the background worker, so a machine catching up took an
    // exclusive checkpoint lock repeatedly while foreground commands were
    // writing, and `cairn connect` began failing with "database is locked" about
    // half the time — under a busy timeout that a truncating checkpoint does not
    // wait out.
    get_personal(store, incoming.id, incoming.owner_user_id).await
}

// ---------------------------------------------------------------------------
// Writer-sequence gap detection (T097, T114, FR-492, FR-582, SC-450)
// ---------------------------------------------------------------------------

/// A hole in one writer's own stream, as this store observes it.
///
/// Diagnostic only. Nothing in recall, reconciliation or ordering reads this —
/// `MemoryFacts` has no `writer_seq` field at all, so a tiebreak that consulted
/// one would not compile (§9, FR-583). What it is for is the one question a
/// sequence number can answer and a timestamp cannot: *did something this writer
/// sent never arrive?* A gap nobody reports is indistinguishable from a stream
/// that had no gap, which is why this exists rather than being left implicit in
/// the row set (SC-450).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriterGap {
    pub writer_id: Uuid,
    /// The sequence numbers this store has not seen, between the lowest and the
    /// highest it has, for this writer.
    pub missing: Vec<i64>,
    /// The highest sequence number seen, so a caller can say "3 of 7 missing"
    /// rather than only listing them.
    pub highest_seen: i64,
}

/// Gaps in the personal-knowledge sequence of every writer this store has seen.
pub async fn personal_writer_gaps(store: &Store, owner_user_id: Uuid) -> Result<Vec<WriterGap>> {
    let rs = sqlx::query(
        "SELECT writer_id, writer_seq FROM personal_knowledge
          WHERE owner_user_id = ?1 ORDER BY writer_id, writer_seq",
    )
    .bind(owner_user_id.to_string())
    .fetch_all(store.pool())
    .await?;
    gaps_from_rows(&rs)
}

/// Gaps in the team-knowledge sequence of every writer this store has seen.
pub async fn team_writer_gaps(store: &Store) -> Result<Vec<WriterGap>> {
    let rs = sqlx::query(
        "SELECT writer_id, writer_seq FROM team_knowledge ORDER BY writer_id, writer_seq",
    )
    .fetch_all(store.pool())
    .await?;
    gaps_from_rows(&rs)
}

fn gaps_from_rows(rs: &[SqliteRow]) -> Result<Vec<WriterGap>> {
    let mut by_writer: HashMap<Uuid, Vec<i64>> = HashMap::new();
    for r in rs {
        by_writer
            .entry(rows::uuid(r, "writer_id")?)
            .or_default()
            .push(r.try_get("writer_seq")?);
    }
    let mut out = Vec::new();
    for (writer_id, seqs) in by_writer {
        out.push(WriterGap {
            writer_id,
            missing: missing_between(&seqs),
            highest_seen: seqs.iter().copied().max().unwrap_or(0),
        });
    }
    // The set only becomes a gap report once something is actually missing.
    out.retain(|g| !g.missing.is_empty());
    out.sort_by_key(|g| g.writer_id);
    Ok(out)
}

/// The sequence numbers absent between the lowest and highest seen.
///
/// Bounded from below by the lowest value *seen*, not by 1: a store that pulled
/// into an existing stream legitimately never had that writer's first records,
/// and reporting every number below its window as missing would drown the one
/// gap that matters in a report the size of the writer's whole history. A gap
/// is a hole in what this store has, not the difference between what it has and
/// everything that ever existed.
fn missing_between(seqs: &[i64]) -> Vec<i64> {
    let Some(&low) = seqs.iter().min() else {
        return Vec::new();
    };
    let high = seqs.iter().copied().max().unwrap_or(low);
    let seen: std::collections::HashSet<i64> = seqs.iter().copied().collect();
    (low..=high).filter(|n| !seen.contains(n)).collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_core::domain::ApplicabilityKind;

    async fn store() -> Store {
        Store::open_memory().await.unwrap()
    }

    fn owner() -> Uuid {
        Uuid::now_v7()
    }

    fn no_identities() -> Vec<ProjectIdentity> {
        Vec::new()
    }

    async fn row_count(store: &Store, table: &str) -> i64 {
        sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
            .fetch_one(store.pool())
            .await
            .unwrap()
    }

    // -- T100: outbox routing ----------------------------------------------

    async fn outbox_rows(store: &Store, namespace: &str) -> Vec<(String, String)> {
        sqlx::query_as("SELECT entity_type, idempotency_key FROM outbox WHERE namespace = ?1")
            .bind(namespace.to_string())
            .fetch_all(store.pool())
            .await
            .unwrap()
    }

    /// A personal write on a store that has never been linked queues nothing —
    /// and that is the same rule an unlinked project follows (FR-053), not a
    /// silent drop: [`enqueue_personal_backlog`] is what collects it later.
    #[tokio::test]
    async fn a_personal_write_with_no_namespace_queues_nothing() {
        let store = store().await;
        let owner_user_id = owner();
        create_personal(
            &store,
            NewPersonalKnowledge::direct(
                owner_user_id,
                MemoryType::Fact,
                "prefer the workspace lockfile",
                None,
                None,
                Vec::new(),
            ),
            &no_identities(),
        )
        .await
        .unwrap();
        assert_eq!(row_count(&store, "outbox").await, 0);
    }

    /// With the namespace established, the write and its queue entry land
    /// together.
    #[tokio::test]
    async fn a_personal_write_reaches_its_own_namespace() {
        let store = store().await;
        let owner_user_id = owner();
        let instance = Uuid::now_v7();
        let namespace = SyncNamespace::Personal(instance, owner_user_id);
        crate::cursor::establish(&store, &namespace).await.unwrap();

        create_personal(
            &store,
            NewPersonalKnowledge::direct(
                owner_user_id,
                MemoryType::Convention,
                "prefer the workspace lockfile",
                None,
                None,
                vec![ApplicabilityFact {
                    kind: ApplicabilityKind::Language,
                    value: "rust".into(),
                }],
            ),
            &no_identities(),
        )
        .await
        .unwrap();

        let rows_ = outbox_rows(&store, &namespace.key()).await;
        assert_eq!(rows_.len(), 1, "expected one queued row, got {rows_:?}");
        assert_eq!(rows_[0].0, "personal_knowledge");

        // Project-less by construction: migration 7's CHECK requires it for
        // exactly these entity types.
        let project_ids: Vec<Option<String>> = sqlx::query_scalar("SELECT project_id FROM outbox")
            .fetch_all(store.pool())
            .await
            .unwrap();
        assert!(
            project_ids.iter().all(|p| p.is_none()),
            "a personal row carried a project id: {project_ids:?}"
        );
    }

    /// The payload never carries the origin digest, on any path.
    ///
    /// Asserted on the queue entry rather than on the type, because the type is
    /// `serde_json::Value` here and a field added to `personal_payload` would
    /// pass a type-level check while still reaching the wire.
    #[tokio::test]
    async fn a_queued_personal_payload_carries_no_origin_digest() {
        let store = store().await;
        let owner_user_id = owner();
        let namespace = SyncNamespace::Personal(Uuid::now_v7(), owner_user_id);
        crate::cursor::establish(&store, &namespace).await.unwrap();

        let mut record = NewPersonalKnowledge::direct(
            owner_user_id,
            MemoryType::Fact,
            "the estimator undercounts by four tokens",
            None,
            None,
            Vec::new(),
        );
        record.origin_digest = Some("a-digest-that-must-not-travel".into());
        create_personal(&store, record, &no_identities())
            .await
            .unwrap();

        let body: String = sqlx::query_scalar("SELECT payload FROM outbox")
            .fetch_one(store.pool())
            .await
            .unwrap();
        assert!(
            !body.contains("a-digest-that-must-not-travel") && !body.contains("origin_digest"),
            "the origin digest reached the queue: {body}"
        );
    }

    /// Forgetting queues its own entry: a record forgotten on one device and
    /// still recalled on the next is the failure the tombstone prevents.
    #[tokio::test]
    async fn forgetting_queues_the_tombstone() {
        let store = store().await;
        let owner_user_id = owner();
        let namespace = SyncNamespace::Personal(Uuid::now_v7(), owner_user_id);
        crate::cursor::establish(&store, &namespace).await.unwrap();

        let created = create_personal(
            &store,
            NewPersonalKnowledge::direct(
                owner_user_id,
                MemoryType::Fact,
                "this one gets forgotten",
                None,
                None,
                Vec::new(),
            ),
            &no_identities(),
        )
        .await
        .unwrap();

        forget_personal(&store, created.record.id, owner_user_id)
            .await
            .unwrap();

        let rows_ = outbox_rows(&store, &namespace.key()).await;
        assert_eq!(
            rows_.len(),
            2,
            "the creation and the tombstone are two deliveries, not one: {rows_:?}"
        );
        assert_ne!(
            rows_[0].1, rows_[1].1,
            "the tombstone reused the creation's idempotency key, so it would never be delivered"
        );
    }

    /// The backlog collects everything written before the link, and running it
    /// twice queues nothing more.
    #[tokio::test]
    async fn the_backlog_collects_pre_link_writes_exactly_once() {
        let store = store().await;
        let owner_user_id = owner();
        for content in ["one", "two", "three"] {
            create_personal(
                &store,
                NewPersonalKnowledge::direct(
                    owner_user_id,
                    MemoryType::Fact,
                    content,
                    None,
                    None,
                    Vec::new(),
                ),
                &no_identities(),
            )
            .await
            .unwrap();
        }
        assert_eq!(row_count(&store, "outbox").await, 0);

        let namespace = SyncNamespace::Personal(Uuid::now_v7(), owner_user_id);
        crate::cursor::establish(&store, &namespace).await.unwrap();

        assert_eq!(
            enqueue_personal_backlog(&store, owner_user_id)
                .await
                .unwrap(),
            3
        );
        assert_eq!(row_count(&store, "outbox").await, 3);

        // Idempotent: the keys already exist, so `INSERT OR IGNORE` writes
        // nothing and the queue does not grow.
        enqueue_personal_backlog(&store, owner_user_id)
            .await
            .unwrap();
        assert_eq!(row_count(&store, "outbox").await, 3);
    }

    /// Another owner's rows are not this owner's backlog.
    #[tokio::test]
    async fn the_backlog_never_reaches_another_identitys_rows() {
        let store = store().await;
        let mine = owner();
        let theirs = owner();
        for who in [mine, theirs] {
            create_personal(
                &store,
                NewPersonalKnowledge::direct(
                    who,
                    MemoryType::Fact,
                    "a note",
                    None,
                    None,
                    Vec::new(),
                ),
                &no_identities(),
            )
            .await
            .unwrap();
        }

        let namespace = SyncNamespace::Personal(Uuid::now_v7(), mine);
        crate::cursor::establish(&store, &namespace).await.unwrap();
        assert_eq!(enqueue_personal_backlog(&store, mine).await.unwrap(), 1);
        assert_eq!(row_count(&store, "outbox").await, 1);
    }

    /// A proposal reaches the server; a ratification deliberately does not.
    ///
    /// Ratification is an admin-token API call (`global-memory.md` §5b) and the
    /// server's own ingest refuses to advance `state` from a pushed payload, so
    /// a queue entry claiming one would be a no-op that read like a delivery.
    #[tokio::test]
    async fn a_team_proposal_queues_and_a_ratification_does_not() {
        let store = store().await;
        let instance = Uuid::now_v7();
        let namespace = SyncNamespace::Team(instance);
        crate::cursor::establish(&store, &namespace).await.unwrap();

        let proposed = propose_team(
            &store,
            NewTeamKnowledge::direct(
                owner(),
                MemoryType::Convention,
                "commit messages follow Conventional Commits",
                None,
                None,
                Vec::new(),
            ),
            &no_identities(),
        )
        .await
        .unwrap();

        let after_proposal = outbox_rows(&store, &namespace.key()).await;
        assert_eq!(after_proposal.len(), 1);
        assert_eq!(after_proposal[0].0, "team_knowledge");

        ratify_team(&store, proposed.record.id, owner(), None)
            .await
            .unwrap();
        assert_eq!(
            outbox_rows(&store, &namespace.key()).await.len(),
            1,
            "ratification queued a delivery the server's ingest would ignore"
        );
    }

    // -- T101: merge_synced_personal ---------------------------------------

    fn synced(owner_user_id: Uuid, content: &str, seq: i64) -> SyncedPersonalKnowledge {
        SyncedPersonalKnowledge {
            id: Uuid::now_v7(),
            owner_user_id,
            knowledge_type: MemoryType::Fact,
            content: content.into(),
            topic_key: None,
            value_key: None,
            applicability: Vec::new(),
            writer_id: Uuid::now_v7(),
            writer_seq: seq,
            created_at: Utc::now(),
            superseded_by_id: None,
            forgotten_at: None,
        }
    }

    /// A pulled row lands with its writer identity and sequence intact — the
    /// two fields the receiving peer needs for gap detection (FR-582, SC-450).
    #[tokio::test]
    async fn a_pulled_personal_row_keeps_its_writer_identity() {
        let store = store().await;
        let owner_user_id = owner();
        let incoming = synced(owner_user_id, "from the other laptop", 4);
        let writer = incoming.writer_id;

        let merged = merge_synced_personal(&store, incoming).await.unwrap();
        assert_eq!(merged.content, "from the other laptop");

        let (stored_writer, stored_seq): (String, i64) =
            sqlx::query_as("SELECT writer_id, writer_seq FROM personal_knowledge WHERE id = ?1")
                .bind(merged.id.to_string())
                .fetch_one(store.pool())
                .await
                .unwrap();
        assert_eq!(stored_writer, writer.to_string());
        assert_eq!(stored_seq, 4);
    }

    /// A second delivery of the same row cannot rewrite its content, even when
    /// the incoming copy disagrees.
    #[tokio::test]
    async fn a_second_delivery_cannot_rewrite_content() {
        let store = store().await;
        let owner_user_id = owner();
        let first = synced(owner_user_id, "the original claim", 1);
        let id = first.id;
        merge_synced_personal(&store, first).await.unwrap();

        let mut second = synced(owner_user_id, "a rewritten claim", 1);
        second.id = id;
        let merged = merge_synced_personal(&store, second).await.unwrap();
        assert_eq!(
            merged.content, "the original claim",
            "a redelivery rewrote the stored content"
        );
    }

    /// A tombstone that arrives clears content, exactly as forgetting does
    /// locally.
    #[tokio::test]
    async fn an_arriving_tombstone_clears_content() {
        let store = store().await;
        let owner_user_id = owner();
        let first = synced(owner_user_id, "forgotten elsewhere", 1);
        let id = first.id;
        merge_synced_personal(&store, first).await.unwrap();

        let mut tombstone = synced(owner_user_id, "forgotten elsewhere", 1);
        tombstone.id = id;
        tombstone.forgotten_at = Some(Utc::now());
        let merged = merge_synced_personal(&store, tombstone).await.unwrap();
        assert_eq!(merged.content, "");
        assert!(merged.forgotten_at.is_some());
    }

    /// Two identities' personal knowledge sits side by side, never merged and
    /// never refused — the other half of D438's asymmetry (FR-567).
    #[tokio::test]
    async fn two_identities_personal_knowledge_coexists() {
        let store = store().await;
        let first = owner();
        let second = owner();
        merge_synced_personal(&store, synced(first, "server A", 1))
            .await
            .unwrap();
        merge_synced_personal(&store, synced(second, "server B", 1))
            .await
            .unwrap();
        assert_eq!(row_count(&store, "personal_knowledge").await, 2);
    }

    // -- T097/T114: writer-sequence gap detection --------------------------

    #[test]
    fn a_complete_run_has_no_gap() {
        assert!(missing_between(&[1, 2, 3, 4]).is_empty());
    }

    #[test]
    fn a_withheld_middle_record_is_a_gap() {
        assert_eq!(missing_between(&[1, 2, 4, 5]), vec![3]);
    }

    /// A store that joined an existing stream mid-flight reports the hole in
    /// what it has, not the difference between what it has and all of history.
    #[test]
    fn a_late_joiner_does_not_report_everything_below_its_window() {
        assert_eq!(missing_between(&[7, 9]), vec![8]);
    }

    #[test]
    fn one_record_is_never_a_gap() {
        assert!(missing_between(&[42]).is_empty());
    }

    /// The gap is reported per writer, so two writers whose sequences interleave
    /// do not manufacture holes in each other's streams (§9: never compared
    /// across writers).
    #[tokio::test]
    async fn gaps_are_per_writer_and_never_across_them() {
        let store = store().await;
        let owner_user_id = owner();
        let a = Uuid::now_v7();
        let b = Uuid::now_v7();
        for (writer, seq) in [(a, 1), (a, 3), (b, 1), (b, 2)] {
            let mut row = synced(owner_user_id, "x", seq);
            row.writer_id = writer;
            merge_synced_personal(&store, row).await.unwrap();
        }

        let gaps = personal_writer_gaps(&store, owner_user_id).await.unwrap();
        assert_eq!(gaps.len(), 1, "expected one writer's gap, got {gaps:?}");
        assert_eq!(gaps[0].writer_id, a);
        assert_eq!(gaps[0].missing, vec![2]);
        assert_eq!(gaps[0].highest_seen, 3);
    }

    // -- create_personal ----------------------------------------------------

    #[tokio::test]
    async fn a_valid_direct_creation_succeeds_and_is_retrievable() {
        let store = store().await;
        let owner_user_id = owner();
        let outcome = create_personal(
            &store,
            NewPersonalKnowledge::direct(
                owner_user_id,
                MemoryType::Fact,
                "the retry backoff is exponential",
                Some("retry.backoff"),
                Some("exponential"),
                Vec::new(),
            ),
            &no_identities(),
        )
        .await
        .unwrap();

        assert_eq!(outcome.reconciliation, ProposalOutcome::Created);
        assert_eq!(outcome.subject.as_deref(), Some("retry.backoff"));

        let fetched = get_personal(&store, outcome.record.id, owner_user_id)
            .await
            .unwrap();
        assert_eq!(fetched.content, "the retry backoff is exponential");
        assert_eq!(fetched.owner_user_id, owner_user_id);
        assert!(fetched.forgotten_at.is_none());
        assert!(fetched.origin_digest.is_none());
    }

    #[tokio::test]
    async fn direct_creation_is_refused_when_content_carries_an_absolute_path() {
        let store = store().await;
        let err = create_personal(
            &store,
            NewPersonalKnowledge::direct(
                owner(),
                MemoryType::Fact,
                "the config lives at /etc/cairn/config.toml",
                None,
                None,
                Vec::new(),
            ),
            &no_identities(),
        )
        .await
        .unwrap_err();

        match err {
            StoreError::Refused { code, .. } => assert_eq!(code, "absolute_path"),
            other => panic!("expected a content refusal, got {other:?}"),
        }
        // FR-548: nothing was written, not even a partial row.
        assert_eq!(row_count(&store, "personal_knowledge").await, 0);
    }

    #[tokio::test]
    async fn direct_creation_is_refused_when_an_applicability_value_is_unrepresentable() {
        let store = store().await;
        let bad = vec![ApplicabilityFact {
            kind: ApplicabilityKind::Language,
            value: "has space".to_string(),
        }];
        let err = create_personal(
            &store,
            NewPersonalKnowledge::direct(
                owner(),
                MemoryType::Fact,
                "fine content",
                None,
                None,
                bad,
            ),
            &no_identities(),
        )
        .await
        .unwrap_err();

        match err {
            StoreError::Refused { code, .. } => {
                assert_eq!(code, cairn_core::validate::INVALID_APPLICABILITY)
            }
            other => panic!("expected an invalid_applicability refusal, got {other:?}"),
        }
        assert_eq!(row_count(&store, "personal_knowledge").await, 0);
        assert_eq!(
            row_count(&store, "personal_knowledge_applicability").await,
            0
        );
    }

    #[tokio::test]
    async fn an_empty_project_identities_slice_passes_the_project_identifying_check() {
        // FR-580: the caller is believed when it says there is no project.
        let store = store().await;
        let outcome = create_personal(
            &store,
            NewPersonalKnowledge::direct(
                owner(),
                MemoryType::Fact,
                "acme-widgets uses conventional commits",
                None,
                None,
                Vec::new(),
            ),
            &[],
        )
        .await
        .unwrap();
        assert_eq!(outcome.reconciliation, ProposalOutcome::Created);
    }

    #[tokio::test]
    async fn a_matching_project_identity_refuses_the_same_content_that_an_empty_slice_passed() {
        let store = store().await;
        let identities = vec![ProjectIdentity("acme-widgets".to_string())];
        let err = create_personal(
            &store,
            NewPersonalKnowledge::direct(
                owner(),
                MemoryType::Fact,
                "acme-widgets uses conventional commits",
                None,
                None,
                Vec::new(),
            ),
            &identities,
        )
        .await
        .unwrap_err();
        match err {
            StoreError::Refused { code, .. } => assert_eq!(code, "project_identifying"),
            other => panic!("expected a project_identifying refusal, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn create_personal_records_a_duplicate_relation_for_identical_content_on_one_subject() {
        // T077: the same automatic case `classify_proposal` decides for
        // project memory — identical content after normalization.
        let store = store().await;
        let owner_user_id = owner();
        let first = create_personal(
            &store,
            NewPersonalKnowledge::direct(
                owner_user_id,
                MemoryType::Convention,
                "we use conventional commits",
                Some("ci.commit_style"),
                None,
                Vec::new(),
            ),
            &no_identities(),
        )
        .await
        .unwrap();

        let second = create_personal(
            &store,
            NewPersonalKnowledge::direct(
                owner_user_id,
                MemoryType::Convention,
                "We use conventional commits.",
                Some("ci.commit_style"),
                None,
                Vec::new(),
            ),
            &no_identities(),
        )
        .await
        .unwrap();

        match second.reconciliation {
            ProposalOutcome::Duplicate { of } => assert_eq!(of, first.record.id),
            other => panic!("expected a duplicate, got {other:?}"),
        }
        assert_eq!(second.relation_recorded, Some(RelationKind::Duplicates));
        assert_eq!(row_count(&store, "personal_knowledge_relations").await, 1);
    }

    #[tokio::test]
    async fn create_personal_detects_a_conflict_for_differing_value_keys_on_one_subject() {
        let store = store().await;
        let owner_user_id = owner();
        create_personal(
            &store,
            NewPersonalKnowledge::direct(
                owner_user_id,
                MemoryType::Decision,
                "the default branch is main",
                Some("git.default_branch"),
                Some("main"),
                Vec::new(),
            ),
            &no_identities(),
        )
        .await
        .unwrap();

        let second = create_personal(
            &store,
            NewPersonalKnowledge::direct(
                owner_user_id,
                MemoryType::Decision,
                "the default branch is master",
                Some("git.default_branch"),
                Some("master"),
                Vec::new(),
            ),
            &no_identities(),
        )
        .await
        .unwrap();

        assert!(matches!(
            second.reconciliation,
            ProposalOutcome::ConflictDetected { .. }
        ));
        assert_eq!(second.relation_recorded, Some(RelationKind::ConflictsWith));
    }

    // -- forget_personal ------------------------------------------------------

    #[tokio::test]
    async fn forgetting_clears_content_and_is_the_only_mutation_that_lands() {
        let store = store().await;
        let owner_user_id = owner();
        let created = create_personal(
            &store,
            NewPersonalKnowledge::direct(
                owner_user_id,
                MemoryType::Fact,
                "secret family recipe: nutmeg",
                Some("kitchen.recipe"),
                Some("nutmeg"),
                Vec::new(),
            ),
            &no_identities(),
        )
        .await
        .unwrap();

        forget_personal(&store, created.record.id, owner_user_id)
            .await
            .unwrap();

        // The row survives (tombstoned), not deleted outright.
        let row = sqlx::query("SELECT * FROM personal_knowledge WHERE id = ?1")
            .bind(created.record.id.to_string())
            .fetch_one(store.pool())
            .await
            .unwrap();
        let after = personal_bare(&row).unwrap();
        assert_eq!(after.content, "", "content must be cleared");
        assert!(after.forgotten_at.is_some());
        // Everything else this write did not touch is untouched.
        assert_eq!(after.topic_key.as_deref(), Some("kitchen.recipe"));
        assert_eq!(after.value_key.as_deref(), Some("nutmeg"));
        assert_eq!(after.writer_seq, created.record.writer_seq);
        assert_eq!(after.created_at, created.record.created_at);

        // And a forgotten record is gone from recall (T073's neighbor: it
        // must never come back through the read path either).
        assert!(get_personal(&store, created.record.id, owner_user_id)
            .await
            .is_ok()); // get_personal is a direct-id read, not recall — it
                       // still finds the tombstone row itself.
        let recalled = recall_personal(&store, owner_user_id, None, None, &[], 10)
            .await
            .unwrap();
        assert!(
            recalled.is_empty(),
            "a forgotten record must not be recalled"
        );
    }

    #[tokio::test]
    async fn forgetting_someone_elses_record_is_refused_as_not_found() {
        let store = store().await;
        let created = create_personal(
            &store,
            NewPersonalKnowledge::direct(owner(), MemoryType::Fact, "mine", None, None, Vec::new()),
            &no_identities(),
        )
        .await
        .unwrap();

        let err = forget_personal(&store, created.record.id, owner())
            .await
            .unwrap_err();
        assert!(matches!(err, StoreError::NotFound(_)));

        // Refused: the content must still be there, untouched.
        let still_there = get_personal(&store, created.record.id, created.record.owner_user_id)
            .await
            .unwrap();
        assert_eq!(still_there.content, "mine");
    }

    #[tokio::test]
    async fn forgetting_an_already_forgotten_record_is_refused_rather_than_reapplied() {
        let store = store().await;
        let owner_user_id = owner();
        let created = create_personal(
            &store,
            NewPersonalKnowledge::direct(
                owner_user_id,
                MemoryType::Fact,
                "once",
                None,
                None,
                Vec::new(),
            ),
            &no_identities(),
        )
        .await
        .unwrap();
        forget_personal(&store, created.record.id, owner_user_id)
            .await
            .unwrap();
        let err = forget_personal(&store, created.record.id, owner_user_id)
            .await
            .unwrap_err();
        assert!(matches!(err, StoreError::NotFound(_)));
    }

    // -- recall_personal (T073) ----------------------------------------------

    fn fact(kind: ApplicabilityKind, value: &str) -> ApplicabilityFact {
        ApplicabilityFact {
            kind,
            value: value.to_string(),
        }
    }
    fn ptrait(kind: ApplicabilityKind, value: &str) -> ProjectTrait {
        ProjectTrait {
            kind,
            value: value.to_string(),
        }
    }

    #[tokio::test]
    async fn recall_applies_the_applicability_predicate_at_query_time() {
        use ApplicabilityKind::{Language, Tool};
        let store = store().await;
        let owner_user_id = owner();

        // Universal: no facts at all.
        create_personal(
            &store,
            NewPersonalKnowledge::direct(
                owner_user_id,
                MemoryType::Fact,
                "universal note",
                None,
                None,
                Vec::new(),
            ),
            &no_identities(),
        )
        .await
        .unwrap();
        // Rust-only.
        create_personal(
            &store,
            NewPersonalKnowledge::direct(
                owner_user_id,
                MemoryType::Fact,
                "rust-only note",
                None,
                None,
                vec![fact(Language, "rust")],
            ),
            &no_identities(),
        )
        .await
        .unwrap();
        // Python-only.
        create_personal(
            &store,
            NewPersonalKnowledge::direct(
                owner_user_id,
                MemoryType::Fact,
                "python-only note",
                None,
                None,
                vec![fact(Language, "python")],
            ),
            &no_identities(),
        )
        .await
        .unwrap();

        let rust_project = [ptrait(Language, "rust"), ptrait(Tool, "cargo")];
        let recalled = recall_personal(&store, owner_user_id, None, None, &rust_project, 10)
            .await
            .unwrap();
        let contents: Vec<&str> = recalled.iter().map(|r| r.content.as_str()).collect();
        assert!(contents.contains(&"universal note"));
        assert!(contents.contains(&"rust-only note"));
        assert!(
            !contents.contains(&"python-only note"),
            "a record naming only a non-matching kind must be excluded, not merely ranked lower"
        );

        // With no project traits at all, only the universal record matches.
        let no_traits = recall_personal(&store, owner_user_id, None, None, &[], 10)
            .await
            .unwrap();
        let no_traits_contents: Vec<&str> = no_traits.iter().map(|r| r.content.as_str()).collect();
        assert_eq!(no_traits_contents, vec!["universal note"]);
    }

    #[tokio::test]
    async fn recall_never_returns_another_owners_record() {
        let store = store().await;
        let a = owner();
        let b = owner();
        create_personal(
            &store,
            NewPersonalKnowledge::direct(a, MemoryType::Fact, "a's note", None, None, Vec::new()),
            &no_identities(),
        )
        .await
        .unwrap();

        let recalled = recall_personal(&store, b, None, None, &[], 10)
            .await
            .unwrap();
        assert!(recalled.is_empty());
    }

    #[tokio::test]
    async fn recall_matches_free_text_through_personal_fts() {
        // T070: real use of the FTS index the migration's triggers maintain,
        // not only a maintained-but-unread one.
        let store = store().await;
        let owner_user_id = owner();
        create_personal(
            &store,
            NewPersonalKnowledge::direct(
                owner_user_id,
                MemoryType::Fact,
                "xylophone practice happens on tuesdays",
                None,
                None,
                Vec::new(),
            ),
            &no_identities(),
        )
        .await
        .unwrap();

        let hit = recall_personal(&store, owner_user_id, Some("xylophone"), None, &[], 10)
            .await
            .unwrap();
        assert_eq!(hit.len(), 1);

        let miss = recall_personal(&store, owner_user_id, Some("bassoon"), None, &[], 10)
            .await
            .unwrap();
        assert!(miss.is_empty());
    }

    // -- personal_subject (T078) ---------------------------------------------

    #[tokio::test]
    async fn personal_subject_reports_conflicted_for_differing_value_keys() {
        let store = store().await;
        let owner_user_id = owner();
        create_personal(
            &store,
            NewPersonalKnowledge::direct(
                owner_user_id,
                MemoryType::Decision,
                "default branch is main",
                Some("git.default_branch"),
                Some("main"),
                Vec::new(),
            ),
            &no_identities(),
        )
        .await
        .unwrap();
        create_personal(
            &store,
            NewPersonalKnowledge::direct(
                owner_user_id,
                MemoryType::Decision,
                "default branch is master",
                Some("git.default_branch"),
                Some("master"),
                Vec::new(),
            ),
            &no_identities(),
        )
        .await
        .unwrap();

        let subject = personal_subject(&store, owner_user_id, "git.default_branch")
            .await
            .unwrap();
        assert_eq!(
            subject.view.reconciliation,
            cairn_core::domain::Reconciliation::Conflicted
        );
        assert_eq!(subject.view.answers.len(), 2);
        assert_eq!(subject.members.len(), 2);
    }

    #[tokio::test]
    async fn personal_subject_is_historical_for_an_unnormalizable_topic_key() {
        let store = store().await;
        let subject = personal_subject(&store, owner(), "...").await.unwrap();
        assert_eq!(
            subject.view.reconciliation,
            cairn_core::domain::Reconciliation::Historical
        );
        assert!(subject.members.is_empty());
    }

    #[tokio::test]
    async fn personal_subject_never_reads_another_domains_relations() {
        // Structural sanity check for §1's "no cross-domain relations"
        // invariant, from this module's side: a subject read only ever
        // queries `personal_knowledge_relations`, so a relation recorded
        // against `memory_relations` for an id that happens to collide can
        // never surface here. Exercised by asserting an ordinary personal
        // subject with no personal relations at all reports no decisions.
        let store = store().await;
        let owner_user_id = owner();
        create_personal(
            &store,
            NewPersonalKnowledge::direct(
                owner_user_id,
                MemoryType::Fact,
                "a lone fact",
                Some("solo.fact"),
                None,
                Vec::new(),
            ),
            &no_identities(),
        )
        .await
        .unwrap();
        let subject = personal_subject(&store, owner_user_id, "solo.fact")
            .await
            .unwrap();
        assert!(subject.view.decisions.is_empty());
    }

    // =========================================================================
    // Team knowledge (T115–T128, T131, T132)
    // =========================================================================

    fn member() -> Uuid {
        Uuid::now_v7()
    }
    fn admin() -> Uuid {
        Uuid::now_v7()
    }
    fn instance() -> Uuid {
        Uuid::now_v7()
    }

    async fn propose(
        store: &Store,
        proposer: Uuid,
        content: &str,
        topic: &str,
        value: &str,
    ) -> TeamCreateOutcome {
        propose_team(
            store,
            NewTeamKnowledge::direct(
                proposer,
                MemoryType::Convention,
                content,
                Some(topic),
                Some(value),
                Vec::new(),
            ),
            &no_identities(),
        )
        .await
        .unwrap()
    }

    // -- propose_team ---------------------------------------------------------

    #[tokio::test]
    async fn a_proposal_always_lands_proposed_never_authoritative() {
        let store = store().await;
        let outcome = propose(
            &store,
            member(),
            "we use conventional commits",
            "ci.commit_style",
            "conventional",
        )
        .await;
        assert_eq!(outcome.record.state, TeamState::Proposed);
        assert!(outcome.record.ratified_by_user_id.is_none());
        assert!(outcome.record.ratified_at.is_none());
    }

    #[tokio::test]
    async fn proposal_creation_is_refused_when_content_carries_an_absolute_path() {
        let store = store().await;
        let err = propose_team(
            &store,
            NewTeamKnowledge::direct(
                member(),
                MemoryType::Fact,
                "the config lives at /etc/cairn/config.toml",
                None,
                None,
                Vec::new(),
            ),
            &no_identities(),
        )
        .await
        .unwrap_err();
        match err {
            StoreError::Refused { code, .. } => assert_eq!(code, "absolute_path"),
            other => panic!("expected a content refusal, got {other:?}"),
        }
        assert_eq!(row_count(&store, "team_knowledge").await, 0);
    }

    #[tokio::test]
    async fn proposal_creation_is_refused_when_an_applicability_value_is_unrepresentable() {
        let store = store().await;
        let bad = vec![ApplicabilityFact {
            kind: ApplicabilityKind::Language,
            value: "has space".to_string(),
        }];
        let err = propose_team(
            &store,
            NewTeamKnowledge::direct(member(), MemoryType::Fact, "fine content", None, None, bad),
            &no_identities(),
        )
        .await
        .unwrap_err();
        match err {
            StoreError::Refused { code, .. } => {
                assert_eq!(code, cairn_core::validate::INVALID_APPLICABILITY)
            }
            other => panic!("expected an invalid_applicability refusal, got {other:?}"),
        }
        assert_eq!(row_count(&store, "team_knowledge").await, 0);
        assert_eq!(row_count(&store, "team_knowledge_applicability").await, 0);
    }

    // -- T120: proposed is invisible to all recall, including the proposer's own --

    #[tokio::test]
    async fn a_proposed_entry_is_invisible_to_recall_even_for_its_own_proposer() {
        let store = store().await;
        let proposer = member();
        propose(&store, proposer, "draft guidance", "ci.style", "draft").await;

        let recalled = recall_team(&store, None, None, &[], 10).await.unwrap();
        assert!(
            recalled.is_empty(),
            "a proposed entry must not be recalled, not even by its proposer"
        );

        let via_search = recall_team(&store, Some("draft"), None, &[], 10)
            .await
            .unwrap();
        assert!(via_search.is_empty());
    }

    #[tokio::test]
    async fn ratifying_makes_an_entry_visible_to_recall_for_everyone() {
        let store = store().await;
        let proposer = member();
        let outcome = propose(
            &store,
            proposer,
            "we use trunk-based development",
            "vcs.branching",
            "trunk",
        )
        .await;

        let before = recall_team(&store, None, None, &[], 10).await.unwrap();
        assert!(before.is_empty());

        ratify_team(&store, outcome.record.id, admin(), None)
            .await
            .unwrap();

        let after = recall_team(&store, None, None, &[], 10).await.unwrap();
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].content, "we use trunk-based development");
    }

    // -- T131: server-wide visibility, no membership scoping ------------------

    #[tokio::test]
    async fn an_authoritative_entry_is_recalled_with_no_membership_or_project_argument_at_all() {
        // recall_team takes no user/membership parameter of any kind — this
        // test exercises exactly that signature, so a caller with zero
        // project memberships sees the same result any other caller would.
        let store = store().await;
        let outcome = propose(
            &store,
            member(),
            "CI runs on GitHub Actions",
            "ci.provider",
            "github_actions",
        )
        .await;
        ratify_team(&store, outcome.record.id, admin(), None)
            .await
            .unwrap();

        let recalled = recall_team(&store, None, None, &[], 10).await.unwrap();
        assert_eq!(recalled.len(), 1);
    }

    // -- T121, T137: CAS refuses naming the actual state -----------------------

    #[tokio::test]
    async fn ratifying_an_already_authoritative_entry_is_refused_naming_authoritative() {
        let store = store().await;
        let outcome = propose(
            &store,
            member(),
            "trunk based dev",
            "vcs.branching",
            "trunk",
        )
        .await;
        ratify_team(&store, outcome.record.id, admin(), None)
            .await
            .unwrap();

        // A second ratification attempt against the same id — the sequential
        // stand-in for T137's real-concurrency race: the loser's read
        // observes the winner's already-committed state.
        let err = ratify_team(&store, outcome.record.id, admin(), None)
            .await
            .unwrap_err();
        match err {
            StoreError::Refused { code, message } => {
                assert_eq!(code, STATE_CONFLICT);
                assert!(
                    message.contains("authoritative"),
                    "refusal must name the actual state: {message}"
                );
            }
            other => panic!("expected a state_conflict refusal, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn ratifying_a_still_proposed_id_that_does_not_exist_is_not_found() {
        let store = store().await;
        let err = ratify_team(&store, Uuid::now_v7(), admin(), None)
            .await
            .unwrap_err();
        assert!(matches!(err, StoreError::NotFound(_)));
    }

    #[tokio::test]
    async fn retiring_a_still_proposed_entry_is_refused_naming_proposed() {
        let store = store().await;
        let outcome = propose(&store, member(), "draft", "some.topic", "v").await;
        let err = retire_team(&store, outcome.record.id, Uuid::now_v7())
            .await
            .unwrap_err();
        match err {
            StoreError::Refused { code, message } => {
                assert_eq!(code, STATE_CONFLICT);
                assert!(message.contains("proposed"));
            }
            other => panic!("expected a state_conflict refusal, got {other:?}"),
        }
    }

    // -- T122, T141: retirement is final ---------------------------------------

    #[tokio::test]
    async fn a_retired_entry_refuses_re_ratification_and_its_content_is_unchanged() {
        let store = store().await;
        let outcome = propose(
            &store,
            member(),
            "we use rebase, not merge",
            "vcs.merge_style",
            "rebase",
        )
        .await;
        ratify_team(&store, outcome.record.id, admin(), None)
            .await
            .unwrap();
        retire_team(&store, outcome.record.id, Uuid::now_v7())
            .await
            .unwrap();

        // Retired: gone from recall (FR-456).
        let recalled = recall_team(&store, None, None, &[], 10).await.unwrap();
        assert!(recalled.is_empty());

        // Re-ratification is refused, naming the actual state.
        let err = ratify_team(&store, outcome.record.id, admin(), None)
            .await
            .unwrap_err();
        match err {
            StoreError::Refused { code, message } => {
                assert_eq!(code, STATE_CONFLICT);
                assert!(message.contains("retired"));
            }
            other => panic!("expected a state_conflict refusal, got {other:?}"),
        }

        // Content is untouched by retirement (FR-461).
        let row = team_row_any_state(&store, outcome.record.id).await.unwrap();
        assert_eq!(row.content, "we use rebase, not merge");
        assert_eq!(row.state, TeamState::Retired);
        assert!(row.retired_at.is_some());
    }

    // -- T119: retiring twice is refused, naming retired -----------------------

    #[tokio::test]
    async fn retiring_an_already_retired_entry_is_refused_naming_retired() {
        let store = store().await;
        let outcome = propose(&store, member(), "content", "topic.key", "v").await;
        ratify_team(&store, outcome.record.id, admin(), None)
            .await
            .unwrap();
        retire_team(&store, outcome.record.id, Uuid::now_v7())
            .await
            .unwrap();

        let err = retire_team(&store, outcome.record.id, Uuid::now_v7())
            .await
            .unwrap_err();
        match err {
            StoreError::Refused { code, message } => {
                assert_eq!(code, STATE_CONFLICT);
                assert!(message.contains("retired"));
            }
            other => panic!("expected a state_conflict refusal, got {other:?}"),
        }
    }

    // -- T132: role-filtered listing --------------------------------------------

    #[tokio::test]
    async fn member_listing_shows_authoritative_and_only_their_own_proposals() {
        let store = store().await;
        let alice = member();
        let bob = member();

        let alice_proposal = propose(&store, alice, "alice's draft", "alice.topic", "v").await;
        let bob_proposal = propose(&store, bob, "bob's draft", "bob.topic", "v").await;
        let ratified = propose(&store, alice, "shared truth", "shared.topic", "v").await;
        ratify_team(&store, ratified.record.id, admin(), None)
            .await
            .unwrap();

        let alice_view = list_team(&store, &TeamViewer::Member(alice), 50)
            .await
            .unwrap();
        let ids: Vec<Uuid> = alice_view.iter().map(|r| r.id).collect();
        assert!(ids.contains(&alice_proposal.record.id));
        assert!(ids.contains(&ratified.record.id));
        assert!(
            !ids.contains(&bob_proposal.record.id),
            "a member must not see another member's pending proposal"
        );
    }

    #[tokio::test]
    async fn admin_listing_shows_every_state_including_retired() {
        let store = store().await;
        let proposal = propose(&store, member(), "will be retired", "retire.me", "v").await;
        ratify_team(&store, proposal.record.id, admin(), None)
            .await
            .unwrap();
        retire_team(&store, proposal.record.id, Uuid::now_v7())
            .await
            .unwrap();
        let still_proposed = propose(&store, member(), "still pending", "pending.topic", "v").await;

        let admin_view = list_team(&store, &TeamViewer::Admin, 50).await.unwrap();
        let ids: Vec<Uuid> = admin_view.iter().map(|r| r.id).collect();
        assert!(ids.contains(&proposal.record.id));
        assert!(ids.contains(&still_proposed.record.id));
    }

    #[tokio::test]
    async fn a_retired_entry_is_not_listed_back_to_the_member_who_proposed_it() {
        let store = store().await;
        let proposer = member();
        let outcome = propose(&store, proposer, "retired guidance", "retired.topic", "v").await;
        ratify_team(&store, outcome.record.id, admin(), None)
            .await
            .unwrap();
        retire_team(&store, outcome.record.id, Uuid::now_v7())
            .await
            .unwrap();

        let member_view = list_team(&store, &TeamViewer::Member(proposer), 50)
            .await
            .unwrap();
        assert!(!member_view.iter().any(|r| r.id == outcome.record.id));
    }

    // -- T125/T126, structural: no function anywhere else can write authoritative --

    #[tokio::test]
    async fn no_creation_path_can_land_an_entry_directly_as_authoritative() {
        // propose_team is the only creation path in this module, and its
        // INSERT statement hardcodes 'proposed' with no parameter capable of
        // overriding it. This test exercises that directly: no matter what
        // is passed to propose_team, the result is 'proposed'.
        let store = store().await;
        let outcome = propose(&store, member(), "anything", "any.topic", "v").await;
        assert_eq!(outcome.record.state, TeamState::Proposed);
    }

    // -- T128: applicability filtering, same closed vocabulary as personal ----

    #[tokio::test]
    async fn team_recall_applies_the_applicability_predicate_at_query_time() {
        use ApplicabilityKind::Language;
        let store = store().await;

        let universal = propose_team(
            &store,
            NewTeamKnowledge::direct(
                member(),
                MemoryType::Convention,
                "universal team note",
                None,
                None,
                Vec::new(),
            ),
            &no_identities(),
        )
        .await
        .unwrap();
        let rust_only = propose_team(
            &store,
            NewTeamKnowledge::direct(
                member(),
                MemoryType::Convention,
                "rust-only team note",
                None,
                None,
                vec![ApplicabilityFact {
                    kind: Language,
                    value: "rust".to_string(),
                }],
            ),
            &no_identities(),
        )
        .await
        .unwrap();
        let python_only = propose_team(
            &store,
            NewTeamKnowledge::direct(
                member(),
                MemoryType::Convention,
                "python-only team note",
                None,
                None,
                vec![ApplicabilityFact {
                    kind: Language,
                    value: "python".to_string(),
                }],
            ),
            &no_identities(),
        )
        .await
        .unwrap();

        for outcome in [&universal, &rust_only, &python_only] {
            ratify_team(&store, outcome.record.id, admin(), None)
                .await
                .unwrap();
        }

        let rust_project = [ProjectTrait {
            kind: Language,
            value: "rust".to_string(),
        }];
        let recalled = recall_team(&store, None, None, &rust_project, 10)
            .await
            .unwrap();
        let contents: Vec<&str> = recalled.iter().map(|r| r.content.as_str()).collect();
        assert!(contents.contains(&"universal team note"));
        assert!(contents.contains(&"rust-only team note"));
        assert!(
            !contents.contains(&"python-only team note"),
            "an entry naming only a non-matching kind must be excluded, not merely ranked lower"
        );

        let no_traits = recall_team(&store, None, None, &[], 10).await.unwrap();
        let no_traits_contents: Vec<&str> = no_traits.iter().map(|r| r.content.as_str()).collect();
        assert_eq!(no_traits_contents, vec!["universal team note"]);
    }

    // -- T127: reconciliation, and two disagreeing authoritative entries ------

    #[tokio::test]
    async fn team_proposal_detects_a_conflict_against_an_authoritative_entry() {
        // §6 of the contract: a proposal reconciles against currently
        // *authoritative* entries, never against other pending proposals —
        // so the first entry must be ratified before the second's proposal
        // can detect a conflict against it.
        let store = store().await;
        let first = propose(
            &store,
            member(),
            "default branch is main",
            "git.default_branch",
            "main",
        )
        .await;
        ratify_team(&store, first.record.id, admin(), None)
            .await
            .unwrap();

        let second = propose(
            &store,
            member(),
            "default branch is master",
            "git.default_branch",
            "master",
        )
        .await;

        assert!(matches!(
            second.reconciliation,
            ProposalOutcome::ConflictDetected { .. }
        ));
        assert_eq!(second.relation_recorded, Some(RelationKind::ConflictsWith));
    }

    #[tokio::test]
    async fn two_proposals_on_the_same_subject_do_not_conflict_against_each_other_before_ratification(
    ) {
        // The other half of the same rule: two *unratified* proposals on the
        // same subject do not conflict-check against each other at all —
        // reconciliation only ever compares against authoritative members.
        let store = store().await;
        let first = propose(
            &store,
            member(),
            "default branch is main",
            "git.default_branch",
            "main",
        )
        .await;
        let second = propose(
            &store,
            member(),
            "default branch is master",
            "git.default_branch",
            "master",
        )
        .await;
        assert_eq!(first.reconciliation, ProposalOutcome::Created);
        assert_eq!(second.reconciliation, ProposalOutcome::Created);
        assert_eq!(second.relation_recorded, None);
    }

    #[tokio::test]
    async fn two_disagreeing_authoritative_entries_both_stay_visible_regardless_of_ratification_order(
    ) {
        let store = store().await;
        let first = propose(
            &store,
            member(),
            "default branch is main",
            "git.default_branch",
            "main",
        )
        .await;
        let second = propose(
            &store,
            member(),
            "default branch is master",
            "git.default_branch",
            "master",
        )
        .await;

        // Ratify the *second* one first — order must not matter (FR-462).
        ratify_team(&store, second.record.id, admin(), None)
            .await
            .unwrap();
        ratify_team(&store, first.record.id, admin(), None)
            .await
            .unwrap();

        let subject = team_subject(&store, "git.default_branch").await.unwrap();
        assert_eq!(
            subject.view.reconciliation,
            cairn_core::domain::Reconciliation::Conflicted
        );
        assert_eq!(subject.view.answers.len(), 2);
        assert_eq!(subject.members.len(), 2);

        let recalled = recall_team(&store, None, None, &[], 10).await.unwrap();
        assert_eq!(
            recalled.len(),
            2,
            "both disagreeing authoritative entries must be returned, not just one"
        );
    }

    #[tokio::test]
    async fn ratifying_with_supersedes_records_an_explicit_relation_never_inferred() {
        let store = store().await;
        let old = propose(
            &store,
            member(),
            "we use gitflow",
            "vcs.workflow",
            "gitflow",
        )
        .await;
        ratify_team(&store, old.record.id, admin(), None)
            .await
            .unwrap();

        let new = propose(
            &store,
            member(),
            "we use trunk-based development",
            "vcs.workflow",
            "trunk",
        )
        .await;
        // The proposal itself never writes `supersedes` — only detects
        // conflict/duplicate automatically.
        assert_ne!(new.relation_recorded, Some(RelationKind::Supersedes));

        ratify_team(&store, new.record.id, admin(), Some(old.record.id))
            .await
            .unwrap();

        let row = sqlx::query(
            "SELECT * FROM team_knowledge_relations WHERE from_id = ?1 AND to_id = ?2 AND kind = 'supersedes'",
        )
        .bind(new.record.id.to_string())
        .bind(old.record.id.to_string())
        .fetch_optional(store.pool())
        .await
        .unwrap();
        assert!(
            row.is_some(),
            "an explicit supersedes relation must be recorded by ratify"
        );
    }

    // -- T116: the server-instance asymmetry -----------------------------------

    #[tokio::test]
    async fn merging_synced_team_knowledge_binds_this_store_to_its_first_server_instance() {
        let store = store().await;
        let a = instance();
        let row = SyncedTeamKnowledge {
            id: Uuid::now_v7(),
            knowledge_type: MemoryType::Convention,
            content: "we squash-merge".to_string(),
            topic_key: Some("vcs.merge_style".to_string()),
            value_key: Some("squash".to_string()),
            applicability: Vec::new(),
            state: TeamState::Authoritative,
            proposed_by_user_id: member(),
            ratified_by_user_id: Some(admin()),
            ratified_at: Some(chrono::Utc::now()),
            writer_id: Uuid::now_v7(),
            writer_seq: 1,
            created_at: chrono::Utc::now(),
            superseded_by_id: None,
            retired_by_user_id: None,
            retired_at: None,
        };
        let merged = merge_synced_team(&store, a, row).await.unwrap();
        assert_eq!(merged.content, "we squash-merge");
        assert_eq!(merged.state, TeamState::Authoritative);

        // Now visible via ordinary recall, exactly as a locally-ratified
        // entry would be.
        let recalled = recall_team(&store, None, None, &[], 10).await.unwrap();
        assert_eq!(recalled.len(), 1);
    }

    #[tokio::test]
    async fn merging_from_a_second_server_instance_is_refused_and_writes_nothing() {
        let store = store().await;
        let a = instance();
        let b = instance();
        assert_ne!(a, b);

        let first = SyncedTeamKnowledge {
            id: Uuid::now_v7(),
            knowledge_type: MemoryType::Convention,
            content: "from server A".to_string(),
            topic_key: None,
            value_key: None,
            applicability: Vec::new(),
            state: TeamState::Authoritative,
            proposed_by_user_id: member(),
            ratified_by_user_id: Some(admin()),
            ratified_at: Some(chrono::Utc::now()),
            writer_id: Uuid::now_v7(),
            writer_seq: 1,
            created_at: chrono::Utc::now(),
            superseded_by_id: None,
            retired_by_user_id: None,
            retired_at: None,
        };
        merge_synced_team(&store, a, first).await.unwrap();

        let second_id = Uuid::now_v7();
        let second = SyncedTeamKnowledge {
            id: second_id,
            knowledge_type: MemoryType::Convention,
            content: "from server B".to_string(),
            topic_key: None,
            value_key: None,
            applicability: Vec::new(),
            state: TeamState::Authoritative,
            proposed_by_user_id: member(),
            ratified_by_user_id: Some(admin()),
            ratified_at: Some(chrono::Utc::now()),
            writer_id: Uuid::now_v7(),
            writer_seq: 1,
            created_at: chrono::Utc::now(),
            superseded_by_id: None,
            retired_by_user_id: None,
            retired_at: None,
        };
        let err = merge_synced_team(&store, b, second).await.unwrap_err();
        match err {
            StoreError::Refused { code, message } => {
                assert_eq!(code, SERVER_INSTANCE_MISMATCH);
                assert!(message.contains(&a.to_string()));
                assert!(message.contains(&b.to_string()));
            }
            other => panic!("expected a server_instance_mismatch refusal, got {other:?}"),
        }
        // Nothing from the refused instance was written.
        let row = sqlx::query("SELECT 1 FROM team_knowledge WHERE id = ?1")
            .bind(second_id.to_string())
            .fetch_optional(store.pool())
            .await
            .unwrap();
        assert!(row.is_none());
        // The first instance's row is untouched.
        assert_eq!(row_count(&store, "team_knowledge").await, 1);
    }

    #[tokio::test]
    async fn personal_knowledge_is_never_refused_on_server_instance_grounds_unlike_team() {
        // The other half of the asymmetry (FR-567): two different "identities"
        // (here, simply two different owner_user_ids, standing in for two
        // different (server_instance_id, user_id) pairs) both retain their
        // personal knowledge in the same local store — nothing about
        // create_personal or recall_personal takes or checks a server
        // instance argument at all, so there is nothing to refuse on.
        let store = store().await;
        let identity_a = owner();
        let identity_b = owner();

        create_personal(
            &store,
            NewPersonalKnowledge::direct(
                identity_a,
                MemoryType::Fact,
                "identity a's note",
                None,
                None,
                Vec::new(),
            ),
            &no_identities(),
        )
        .await
        .unwrap();
        create_personal(
            &store,
            NewPersonalKnowledge::direct(
                identity_b,
                MemoryType::Fact,
                "identity b's note",
                None,
                None,
                Vec::new(),
            ),
            &no_identities(),
        )
        .await
        .unwrap();

        // Both identities' knowledge survives, side by side in one store.
        assert_eq!(row_count(&store, "personal_knowledge").await, 2);
        // Recall surfaces only the currently linked identity's rows.
        let a_view = recall_personal(&store, identity_a, None, None, &[], 10)
            .await
            .unwrap();
        assert_eq!(a_view.len(), 1);
        assert_eq!(a_view[0].content, "identity a's note");
        let b_view = recall_personal(&store, identity_b, None, None, &[], 10)
            .await
            .unwrap();
        assert_eq!(b_view.len(), 1);
        assert_eq!(b_view[0].content, "identity b's note");
    }

    // -- FR-517/SC-424: no project identity, evidence, observation or verification field --

    #[tokio::test]
    async fn a_team_record_carries_no_verification_field_of_any_kind() {
        // Structural: MemoryFacts::verification is always Unverified because
        // there is no column to read it from — team_facts_from_row never
        // reads a verification column, because team_knowledge has none.
        let store = store().await;
        let outcome = propose(&store, member(), "content", "some.topic", "v").await;
        ratify_team(&store, outcome.record.id, admin(), None)
            .await
            .unwrap();
        let subject = team_subject(&store, "some.topic").await.unwrap();
        assert_eq!(
            subject.members[0].verification,
            VerificationState::Unverified
        );
        assert!(subject.members[0].verification_authority.is_none());
    }
    /// FR-457 — a retirement records **who** as well as when.
    ///
    /// Ratification already recorded both halves; retirement recorded only the
    /// clock, and a timestamp on its own does not satisfy "every state
    /// transition MUST be recorded with who acted and when". Retirement is the
    /// transition most worth attributing: it removes guidance from every user on
    /// the server, and afterwards the only question anyone asks is who decided
    /// that.
    #[tokio::test]
    async fn retiring_records_who_acted_and_when() {
        let store = Store::open_memory().await.expect("store");
        let proposer = Uuid::now_v7();
        let admin = Uuid::now_v7();
        let retirer = Uuid::now_v7();
        let none: [ProjectIdentity; 0] = [];

        let proposed = propose_team(
            &store,
            NewTeamKnowledge::direct(
                proposer,
                MemoryType::Convention,
                "retry flaky integration tests up to three times",
                Some("ci.retries"),
                Some("three"),
                vec![],
            ),
            &none,
        )
        .await
        .expect("propose");
        ratify_team(&store, proposed.record.id, admin, None)
            .await
            .expect("ratify");
        retire_team(&store, proposed.record.id, retirer)
            .await
            .expect("retire");

        let (who, when): (Option<String>, Option<String>) = sqlx::query_as(
            "SELECT retired_by_user_id, retired_at FROM team_knowledge WHERE id = ?1",
        )
        .bind(proposed.record.id.to_string())
        .fetch_one(store.pool())
        .await
        .expect("row");

        assert_eq!(
            who.as_deref(),
            Some(retirer.to_string().as_str()),
            "the retirement did not record who acted"
        );
        assert!(when.is_some(), "the retirement did not record when");
        // And the ratifier is still recorded separately — the two acts have two
        // actors, and conflating them would lose the earlier one.
        let ratifier: Option<String> =
            sqlx::query_scalar("SELECT ratified_by_user_id FROM team_knowledge WHERE id = ?1")
                .bind(proposed.record.id.to_string())
                .fetch_one(store.pool())
                .await
                .expect("row");
        assert_eq!(ratifier.as_deref(), Some(admin.to_string().as_str()));
    }
}
