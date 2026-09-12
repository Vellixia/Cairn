//! Safe-event ingest (`contracts/safe-events.md` §7, FR-765–FR-780).
//!
//! A boundary of its own, deliberately not `/api/sync/batch`. The sync boundary
//! carries whole entities a client already decided to store; this one carries
//! typed observations the server decides what to do with. Sharing a route would
//! mean one validation order for two different questions.
//!
//! Three properties this module exists to hold:
//!
//! **Idempotency is not client-controlled.** `event_id` travels on the wire and
//! the server recomputes it from `session_id ‖ session_seq`, refusing a
//! mismatch. Without that, a client could submit a colliding id, be answered
//! `duplicate`, and suppress a genuine event — or pre-claim ids it guessed.
//!
//! **Identity is bound from the credential, never from the body.** An event
//! names its session, and a session identifier is body data. The project is
//! derived from the session; the account comes from the token. A client that
//! could name either could attribute another account's work (FR-769, FR-769a).
//!
//! **The server validates independently.** Every check the client ran, this
//! runs again. A client that constructs a field correctly is not the mechanism
//! by which the rule holds (FR-777, FR-777d).
//!
//! Per-event outcomes, so a client can retry precisely what needs retrying
//! (FR-771). `duplicate` is a success: a retry that gets it has achieved
//! exactly what it was for.

use crate::auth::{bind_session, CurrentUser, ReaderContext, SessionBinding, SessionBindingError};
use crate::error::{ApiError, ApiResult};
use crate::AppState;
use axum::extract::State;
use axum::Json;
use cairn_core::event::{
    EventContent, EventKind, SafeCanonicalEvent, BATCH_MAX_EVENTS, CONTRACT_VERSION,
};
use cairn_core::eventid;
use cairn_core::validate::{
    validate_repo_file, validate_safe_event_text, ProjectIdentity, SafeEventField,
};
use cairn_core::vocabulary::{self, SessionVocabulary};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

/// The batch a client posts.
///
/// `deny_unknown_fields` at both levels. The schema is closed, and an unknown
/// field is a client and a server disagreeing about what an event means — which
/// is the drift FR-767 refuses rather than tolerates.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventBatch {
    pub contract_version: u16,
    pub events: Vec<Value>,
}

/// What happened to one event.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct EventOutcome {
    /// The id the client sent, so a client can match outcomes to what it sent
    /// even when the server refused to re-derive that id.
    pub event_id: Option<Uuid>,
    pub status: EventStatus,
    /// A fixed vocabulary term, never the content that caused it (FR-741).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<&'static str>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EventStatus {
    Accepted,
    /// A success. At most one canonical event exists, which is what the retry
    /// was for (FR-770, FR-786).
    Duplicate,
    /// Permanent. The client marks the spool row `refused` and stops retrying.
    Rejected,
}

impl EventOutcome {
    fn accepted(id: Uuid) -> Self {
        Self {
            event_id: Some(id),
            status: EventStatus::Accepted,
            reason: None,
        }
    }
    fn duplicate(id: Uuid) -> Self {
        Self {
            event_id: Some(id),
            status: EventStatus::Duplicate,
            reason: None,
        }
    }
    fn rejected(id: Option<Uuid>, reason: &'static str) -> Self {
        Self {
            event_id: id,
            status: EventStatus::Rejected,
            reason: Some(reason),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct BatchResponse {
    pub results: Vec<EventOutcome>,
}

/// Every name the synchronization boundary refuses, enforced here too.
///
/// FR-777a1 makes this obligation general rather than satisfied by renaming one
/// field: two boundaries on one server disagreeing about the same name is
/// exactly the drift FR-760 forbids for rejection classes. The list is checked
/// **recursively at any depth**, because a refused name nested inside `content`
/// is refused for the same reason it is refused at the top.
const REFUSED_FIELD_NAMES: &[&str] = &[
    "summary",
    "path",
    "command",
    "details",
    "exit_code",
    "observations",
    "observed_value",
    "source_locator",
    "value_digest",
    "fingerprint",
    "relevant_paths",
    "criteria_snapshot",
    "sanitization_report",
    "origin_ref",
    "alternative_cause",
    "signal_digest",
    "pin_reason",
    "rationale",
    "basis_evidence_id",
    "path_fingerprints",
    "task_snapshot_at_bind",
    "detail",
    "prior_value",
    "new_value",
    "content_norm_digest",
    "local_revision",
    // The session refusals apply equally.
    "worktree_path",
    "agent_session_key",
    "daemon_run_id",
    "last_event_at",
    "last_turn_ended_at",
];

/// `outcome` is refused at top level only — nested, it is a legal field name
/// on the sync boundary, and this boundary matches that rule rather than
/// inventing a stricter one that would then disagree with it.
const REFUSED_AT_TOP_LEVEL: &[&str] = &["outcome"];

fn carries_refused_name(value: &Value, top_level: bool) -> bool {
    match value {
        Value::Object(map) => map.iter().any(|(key, child)| {
            REFUSED_FIELD_NAMES.contains(&key.as_str())
                || (top_level && REFUSED_AT_TOP_LEVEL.contains(&key.as_str()))
                || carries_refused_name(child, false)
        }),
        Value::Array(items) => items.iter().any(|item| carries_refused_name(item, false)),
        _ => false,
    }
}

/// `POST /api/events/batch`.
///
/// Authenticated, and the batch's whole session set is authorized before any
/// event is validated. Non-membership is a request-level `403` rather than a
/// per-item rejection: a per-item answer would confirm the session's existence
/// to a non-member (FR-894a).
pub async fn ingest_batch(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(batch): Json<EventBatch>,
) -> ApiResult<Json<BatchResponse>> {
    if batch.contract_version != CONTRACT_VERSION {
        return Err(ApiError::invalid(
            "this server does not support that contract version",
        ));
    }
    if batch.events.len() > BATCH_MAX_EVENTS {
        return Err(ApiError::invalid("a batch carries at most 256 events"));
    }

    let reader = ReaderContext::load(&state.pool, &user).await?;
    let mut results = Vec::with_capacity(batch.events.len());

    // Events are validated in the order they arrive, and the client is required
    // to order a batch by `session_seq`, so an event that establishes a
    // vocabulary token is validated before one that cites it. Sorting here
    // instead would hide a client that had them out of order.
    for raw in &batch.events {
        match ingest_one(&state.pool, &reader, raw).await {
            Ok(outcome) => results.push(outcome),
            // The one failure that is not per-item: a non-member must not learn
            // whether the session exists.
            Err(IngestFailure::Unresolvable) => {
                return Err(ApiError::forbidden("no session you can write to was named"))
            }
            Err(IngestFailure::Database(e)) => return Err(e),
        }
    }
    Ok(Json(BatchResponse { results }))
}

enum IngestFailure {
    /// Request-level. One answer for "no such session" and "not your project",
    /// so a caller cannot use the difference to enumerate session ids.
    Unresolvable,
    Database(ApiError),
}

impl From<ApiError> for IngestFailure {
    fn from(e: ApiError) -> Self {
        IngestFailure::Database(e)
    }
}

impl From<sqlx::Error> for IngestFailure {
    fn from(e: sqlx::Error) -> Self {
        IngestFailure::Database(e.into())
    }
}

/// One event, through the ten steps of §7.1 in order.
///
/// The order is not incidental. Schema before names, names before bounds,
/// bounds before `repo_file`, identity before session binding, session binding
/// before vocabulary — each step's input is the previous step's guarantee, and
/// a reordering would either check something twice or check it against a value
/// that had not been established yet.
async fn ingest_one(
    pool: &PgPool,
    reader: &ReaderContext,
    raw: &Value,
) -> Result<EventOutcome, IngestFailure> {
    // The id as sent, kept so a rejection can name what the client sent even
    // when the event is too malformed to parse.
    let claimed_id = raw
        .get("event_id")
        .and_then(Value::as_str)
        .and_then(|s| Uuid::parse_str(s).ok());

    // 2 — refused names, before deserialization. A strict deserializer would
    // report `unknown_field` for a refused name, which is true but says the
    // wrong thing: the name is not unknown, it is forbidden, and the two call
    // for different fixes.
    if carries_refused_name(raw, true) {
        return Ok(EventOutcome::rejected(claimed_id, "forbidden_field_name"));
    }

    // 1 — schema. Strict: the union is closed.
    let event: SafeCanonicalEvent = match serde_json::from_value(raw.clone()) {
        Ok(event) => event,
        Err(_) => return Ok(EventOutcome::rejected(claimed_id, "unknown_field")),
    };

    if event.contract_version != CONTRACT_VERSION {
        return Ok(EventOutcome::rejected(
            claimed_id,
            "contract_version_unsupported",
        ));
    }

    // 3 — bounds, and the structural rules that go with them.
    if let Err(refusal) = event.validate() {
        return Ok(EventOutcome::rejected(claimed_id, refusal.code()));
    }

    // 4 — `repo_file`, its own vocabulary of refusals.
    if let Some(EventContent::File {
        repo_file,
        repo_file_from,
        ..
    }) = &event.content
    {
        for path in [repo_file, repo_file_from].into_iter().flatten() {
            if let Err(rejection) = validate_repo_file(path) {
                return Ok(EventOutcome::rejected(
                    claimed_id,
                    repo_file_reason(rejection.class),
                ));
            }
        }
    }

    // 5 — identity re-derivation. This is what makes idempotency the server's
    // property rather than the client's.
    let derived = eventid::event_id(event.session_id, event.session_seq);
    if derived != event.event_id {
        return Ok(EventOutcome::rejected(claimed_id, "event_id_mismatch"));
    }

    // 6 — session binding. The project is derived, never asserted.
    let binding = match bind_session(pool, reader, event.session_id).await? {
        Ok(binding) => binding,
        Err(SessionBindingError::Unresolvable) => return Err(IngestFailure::Unresolvable),
        Err(SessionBindingError::NotOwned) => {
            return Ok(EventOutcome::rejected(claimed_id, "session_not_found"))
        }
    };

    // 8 — content screening, before 7, and deliberately: `repo_file` segments
    // and command lines are vocabulary *sources*, so screening them after
    // deriving the vocabulary would let a value the boundary refuses justify a
    // token first. The step numbers in the contract describe the checks, not a
    // constraint that they run in numeric order where one feeds the other.
    if let Some(reason) = screen_event_text(&event) {
        return Ok(EventOutcome::rejected(claimed_id, reason));
    }

    // 7 — vocabulary justification, for the two semantic signals.
    if let Some(reason) = justify_tokens(pool, &event, &binding).await? {
        return Ok(EventOutcome::rejected(claimed_id, reason));
    }

    // 9 and 10 — insert and enqueue, in one transaction, so an accepted event
    // is always eventually consolidated and a rolled-back one never is.
    persist(pool, &event, &binding).await
}

fn repo_file_reason(class: &str) -> &'static str {
    match class {
        "repo_file_absolute" => "repo_file_absolute",
        "repo_file_traversal" => "repo_file_traversal",
        // The remaining path failures share one wire term. A client cannot act
        // differently on "empty segment" than on "drive letter" — both mean the
        // adapter built a value that is not repository-relative — and a
        // narrower vocabulary is a smaller surface to keep in step with the
        // client's own.
        _ => "repo_file_malformed",
    }
}

/// Screen the three free-text fields and the path segments (§7.1 step 8).
///
/// Client-side redaction is where secrets are *removed*; this is where the
/// boundary is *enforced* (FR-777, SC-741). The two are not alternatives.
///
/// No project identities are passed. A safe event is not a global record: it is
/// bound to one project by its session, so the `project_identifying` class has
/// nothing to protect here and an empty slice makes it correctly vacuous
/// (FR-580).
fn screen_event_text(event: &SafeCanonicalEvent) -> Option<&'static str> {
    const NONE: &[ProjectIdentity] = &[];
    let mut checks: Vec<(SafeEventField, &str)> = Vec::new();

    if let Some(vendor_event) = &event.vendor_event {
        checks.push((SafeEventField::Provenance, vendor_event));
    }
    match &event.content {
        Some(EventContent::Command { command_line, .. }) => {
            checks.push((SafeEventField::CommandLine, command_line));
        }
        Some(EventContent::TestInvocation { test_command }) => {
            checks.push((SafeEventField::TestCommand, test_command));
        }
        Some(EventContent::ToolFailure {
            vendor_tool,
            failure_note,
            ..
        }) => {
            checks.push((SafeEventField::Provenance, vendor_tool));
            if let Some(note) = failure_note {
                checks.push((SafeEventField::FailureNote, note));
            }
        }
        Some(EventContent::Tool { vendor_tool, .. }) => {
            checks.push((SafeEventField::Provenance, vendor_tool));
        }
        Some(EventContent::SessionClose { close_reason }) => {
            checks.push((SafeEventField::Provenance, close_reason));
        }
        Some(EventContent::Subagent {
            subagent_ref,
            subagent_kind,
            ..
        }) => {
            checks.push((SafeEventField::Provenance, subagent_ref));
            checks.push((SafeEventField::Provenance, subagent_kind));
        }
        Some(EventContent::File {
            repo_file,
            repo_file_from,
            ..
        }) => {
            // Path segments are a vocabulary source, so they are screened even
            // though the path itself already passed its own validation: a
            // legal repository-relative path can still be named after a secret.
            for path in [repo_file, repo_file_from].into_iter().flatten() {
                for segment in path.split('/') {
                    checks.push((SafeEventField::Provenance, segment));
                }
            }
        }
        _ => {}
    }

    for (field, text) in checks {
        if validate_safe_event_text(field, text, NONE).is_err() {
            return Some("content_screening_failed");
        }
    }
    None
}

/// Recompute the session vocabulary and check both tokens against it.
///
/// Recomputed server-side, from events **the server already holds** with a
/// lower `session_seq`, plus the keys this project's knowledge already
/// establishes. Both sources are required: a server that checked only session
/// events would refuse tokens the client legitimately justified from
/// established project keys, and the refusal is permanent — the decision is
/// destroyed rather than deferred (`contracts/extraction.md` §13.3).
async fn justify_tokens(
    pool: &PgPool,
    event: &SafeCanonicalEvent,
    binding: &SessionBinding,
) -> Result<Option<&'static str>, IngestFailure> {
    let (subject, object) = match &event.content {
        Some(EventContent::Instruction {
            subject_token,
            object_token,
            ..
        })
        | Some(EventContent::Decision {
            subject_token,
            object_token,
            ..
        }) => (subject_token, object_token),
        _ => return Ok(None),
    };

    let vocabulary = session_vocabulary(pool, event, binding).await?;
    if !vocabulary.justifies(subject) || !vocabulary.justifies(object) {
        return Ok(Some("token_not_in_vocabulary"));
    }
    Ok(None)
}

async fn session_vocabulary(
    pool: &PgPool,
    event: &SafeCanonicalEvent,
    binding: &SessionBinding,
) -> Result<SessionVocabulary, IngestFailure> {
    let rows: Vec<(String, Value)> = sqlx::query_as(
        "SELECT kind, content FROM safe_events
          WHERE session_id = $1 AND session_seq < $2
          ORDER BY session_seq",
    )
    .bind(event.session_id)
    .bind(event.session_seq as i64)
    .fetch_all(pool)
    .await?;

    let established: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT topic_key FROM memories
          WHERE project_id = $1 AND topic_key IS NOT NULL AND deleted_at IS NULL
         UNION
         SELECT DISTINCT value_key FROM memories
          WHERE project_id = $1 AND value_key IS NOT NULL AND deleted_at IS NULL",
    )
    .bind(binding.project_id)
    .fetch_all(pool)
    .await?;

    let parsed: Vec<(EventKind, Option<EventContent>)> = rows
        .into_iter()
        .filter_map(|(kind, content)| {
            let kind = kind.parse::<EventKind>().ok()?;
            // A stored row whose content no longer parses contributes nothing
            // rather than refusing the batch. It was accepted under a contract
            // this build may no longer model, and refusing a new event because
            // an old one is unreadable would make a schema change destroy
            // decisions retroactively.
            Some((kind, serde_json::from_value(content).ok()))
        })
        .collect();

    Ok(vocabulary::derive(
        parsed.iter().map(|(k, c)| (*k, c.as_ref())),
        established.iter().map(String::as_str),
    ))
}

/// Insert the event and enqueue its consolidation work, atomically.
///
/// One transaction, because an accepted event that is never enqueued is an
/// event that silently never becomes knowledge, and enqueued work for an event
/// that rolled back is a foreign key violation waiting to happen.
///
/// The lease row is upserted **before** the work row, because
/// `consolidation_work` carries a foreign key to it. The upsert reopens a
/// session already marked `done`: a session that produces more events after
/// consolidation finished has more work, and leaving it `done` would strand it.
async fn persist(
    pool: &PgPool,
    event: &SafeCanonicalEvent,
    binding: &SessionBinding,
) -> Result<EventOutcome, IngestFailure> {
    let mut tx = pool.begin().await?;

    let content = event
        .content
        .as_ref()
        .map(serde_json::to_value)
        .transpose()
        .map_err(|_| ApiError::internal("an accepted event would not re-serialize"))?
        .unwrap_or(Value::Null);

    let inserted = sqlx::query(
        "INSERT INTO safe_events
             (event_id, project_id, session_id, account_id, agent, kind, vendor_event,
              session_seq, contract_version, content, occurred_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
         -- **Unqualified, and it has to be.** `safe_events` has two unique
         -- constraints stating the same fact: `event_id PRIMARY KEY` and
         -- `UNIQUE (session_id, session_seq)`. Naming one of them as the arbiter
         -- means a racing duplicate that slips past the primary key's check
         -- still meets the other index during insertion — and that is not an
         -- `ON CONFLICT` path, it raises. The row is correctly not written
         -- either way, so no data was ever at risk; what broke is the *answer*.
         -- The request failed with `500 internal` instead of reporting
         -- `duplicate`, and it took the per-item outcomes of every event in that
         -- batch that had committed down with it. A retry being told the server
         -- broke, when the retry did exactly what it was for, is the opposite of
         -- what this module promises (FR-771).
         --
         -- Safe precisely here: step 5 re-derives `event_id` from
         -- `(session_id, session_seq)` and refuses a mismatch, so any row that
         -- conflicts on the pair necessarily carries the same id. There is no
         -- genuinely different event an unqualified arbiter could mask.
         ON CONFLICT DO NOTHING",
    )
    .bind(event.event_id)
    .bind(binding.project_id)
    .bind(event.session_id)
    .bind(binding.owner_user_id)
    .bind(event.agent.as_str())
    .bind(event.kind.as_str())
    .bind(event.vendor_event.as_deref())
    .bind(event.session_seq as i64)
    .bind(i32::from(event.contract_version))
    .bind(&content)
    .bind(event.occurred_at)
    .execute(&mut *tx)
    .await?;

    if inserted.rows_affected() == 0 {
        // A redelivery. Nothing to enqueue — the original insert already did —
        // and the transaction commits so a concurrent writer is not blocked.
        tx.commit().await?;
        return Ok(EventOutcome::duplicate(event.event_id));
    }

    // The conflict action is spelled out because all three obvious choices are
    // wrong in a different way.
    //
    // Leaving `state` alone strands every event arriving after a session was
    // marked `done`: the partial index excludes `done`, so the session is never
    // elected again. Setting it to `pending` unconditionally clobbers a live
    // lease and lets a second worker elect a session mid-pass. The `CASE`
    // re-opens only a finished one.
    //
    // `oldest_enqueued_at` is **reset** on re-open and minimised otherwise.
    // Minimising unconditionally carries yesterday's completed work forward as
    // the age of today's first event, so a single fresh event in a long-lived
    // session is instantly age-eligible on the strength of work that was
    // consolidated a day ago — a new generation inheriting the old one's clock.
    // For a generation still `pending` or `claimed` the old value really is the
    // age of work still waiting, so there it is preserved.
    //
    // `eligible_since` is cleared on re-open for the same reason: a new
    // generation has met no threshold of its own, and inheriting the latch
    // would make it eligible before it had.
    sqlx::query(
        "INSERT INTO consolidation_session (project_id, session_id, state, oldest_enqueued_at)
         VALUES ($1, $2, 'pending', now())
         ON CONFLICT (project_id, session_id) DO UPDATE
            SET state = CASE WHEN consolidation_session.state = 'done' THEN 'pending'
                             ELSE consolidation_session.state END,
                oldest_enqueued_at =
                    CASE WHEN consolidation_session.state = 'done'
                         THEN EXCLUDED.oldest_enqueued_at
                         ELSE LEAST(consolidation_session.oldest_enqueued_at,
                                    EXCLUDED.oldest_enqueued_at) END,
                eligible_since =
                    CASE WHEN consolidation_session.state = 'done'
                         THEN NULL
                         ELSE consolidation_session.eligible_since END",
    )
    .bind(binding.project_id)
    .bind(event.session_id)
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "INSERT INTO consolidation_work
             (event_id, project_id, session_id, session_seq, state)
         VALUES ($1, $2, $3, $4, 'pending')",
    )
    .bind(event.event_id)
    .bind(binding.project_id)
    .bind(event.session_id)
    .bind(event.session_seq as i64)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(EventOutcome::accepted(event.event_id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_refused_name_is_found_at_any_depth() {
        // The sync boundary refuses these recursively, and two boundaries on
        // one server disagreeing about one name is the drift FR-777a1 forbids.
        assert!(carries_refused_name(&json!({ "summary": "x" }), true));
        assert!(carries_refused_name(
            &json!({ "content": { "path": "a.rs" } }),
            true
        ));
        assert!(carries_refused_name(
            &json!({ "content": { "items": [ { "exit_code": 1 } ] } }),
            true
        ));
        assert!(!carries_refused_name(
            &json!({ "content": { "repo_file": "a.rs", "exit_status": 1 } }),
            true
        ));
    }

    #[test]
    fn outcome_is_refused_at_the_top_level_only() {
        // Matching the sync boundary exactly rather than inventing a stricter
        // rule that would then disagree with it.
        assert!(carries_refused_name(&json!({ "outcome": "passed" }), true));
        assert!(!carries_refused_name(
            &json!({ "content": { "outcome": "passed" } }),
            true
        ));
    }

    #[test]
    fn the_substitute_names_the_contract_chose_are_all_permitted() {
        // Each of these replaces a refused name. If one were itself refused,
        // the event kind that needs it could never be ingested at all.
        for permitted in [
            "repo_file",
            "repo_file_from",
            "command_line",
            "test_command",
            "exit_status",
            "test_outcome",
            "failure_note",
            "failure_kind",
        ] {
            assert!(
                !carries_refused_name(&json!({ "content": { permitted: "v" } }), true),
                "{permitted} is refused, so its event kind can never be ingested"
            );
        }
    }

    #[test]
    fn a_path_refusal_maps_to_the_wire_term_a_client_can_act_on() {
        assert_eq!(repo_file_reason("repo_file_absolute"), "repo_file_absolute");
        assert_eq!(
            repo_file_reason("repo_file_traversal"),
            "repo_file_traversal"
        );
        for other in [
            "repo_file_empty",
            "repo_file_drive_letter",
            "repo_file_unc",
            "repo_file_empty_segment",
            "repo_file_too_long",
            "repo_file_too_many_segments",
        ] {
            assert_eq!(repo_file_reason(other), "repo_file_malformed");
        }
    }
}
