//! Promotion orchestration: project memory becomes personal or team knowledge
//! (T075, T076; FR-506, FR-520, FR-545, FR-548).
//!
//! This is the **second and fourth** of the five entry points capable of
//! creating global content, and the only two that have a source memory to gate.
//! It composes two pure functions and adds nothing of its own to either:
//!
//! - [`cairn_core::promotion::evaluate_promotion`] — the eight-check gate, which
//!   needs project-memory context (`source_state`, the project's identity,
//!   evidence metadata, the promoter's membership) that no other entry point
//!   has.
//! - [`cairn_core::validate::validate_global_content`] — the shared validator,
//!   reached through the gate's check 1 rather than called twice.
//!
//! The gate does **not** re-implement the validator's classes (FR-579), and this
//! module does not re-implement either. What it adds is the I/O the pure
//! functions deliberately refuse to do: reading the source memory, reading the
//! machine salt, and writing the result.
//!
//! **Nothing is persisted unless both pass** (FR-548). The gate runs to
//! completion before any write begins, so a refusal leaves no record, no partial
//! record and no queued outbox entry — there is no window in which the row
//! existed rather than a window that was rolled back.

use cairn_core::domain::{ApplicabilityFact, ApplicabilityKind, MemoryState, PromotionTarget};
use cairn_core::promotion::{evaluate_promotion, PromotionRejection};
use cairn_core::validate::ProjectIdentity;
use cairn_core::wire::{codes, WireError};
use cairn_store::global::{create_personal, propose_team, NewPersonalKnowledge, NewTeamKnowledge};
use cairn_store::Store;
use uuid::Uuid;

/// What the gate needs to know about the memory being promoted.
///
/// Read from the store in one query rather than passed piecemeal, so a caller
/// cannot supply a half-populated view that makes an unanswerable check look
/// answerable — which is exactly what check 8 (`evaluation_incomplete`) exists
/// to refuse.
struct Source {
    state: MemoryState,
    content: String,
    topic_key: Option<String>,
    value_key: Option<String>,
    knowledge_type: cairn_core::domain::MemoryType,
}

/// Promote one project memory (FR-506).
///
/// `target` defaults to `Pattern` at the tool surface, so a caller that names
/// none gets today's behaviour unchanged; this function is reached only for the
/// two global targets.
///
/// `owner_user_id` is the acting user: the owner of the resulting personal
/// record, or the proposer of the resulting team proposal. There is no path on
/// which one user promotes on another's behalf.
#[allow(clippy::too_many_arguments)]
pub async fn promote(
    store: &Store,
    memory_id: Uuid,
    target: PromotionTarget,
    owner_user_id: Uuid,
    project_id: Uuid,
    project_identities: &[ProjectIdentity],
    promoter_is_project_member: bool,
    applicability: Vec<(ApplicabilityKind, String)>,
) -> Result<Uuid, WireError> {
    let source = source_facts(store, memory_id).await?;

    // The machine salt is read here rather than inside the gate, because the
    // gate is pure and reading a file is not. Its absence is a refusal, not a
    // default: a promotion whose origin digest could not be computed is one
    // whose check 7 could not run (FR-518).
    let salt = cairn_core::paths::machine_salt().map_err(|e| {
        WireError::new(
            codes::STORAGE_UNAVAILABLE,
            format!("the machine salt could not be read: {e}"),
        )
    })?;

    let approval = evaluate_promotion(
        &source.content,
        source.topic_key.as_deref(),
        source.value_key.as_deref(),
        &applicability,
        project_identities,
        Some(project_id),
        Some(&salt),
        target,
        promoter_is_project_member,
        Some(source.state),
    )
    .map_err(refusal)?;

    match target {
        PromotionTarget::Personal => {
            // The store call runs `validate_global_content` again on its own —
            // it is the same function the gate's check 1 already ran, and it is
            // that path's own guarantee rather than a duplicate implementation.
            // Passing here twice is cheap and means the store's invariant holds
            // whoever calls it, including a future path that does not go through
            // this gate at all.
            let mut record = NewPersonalKnowledge::direct(
                owner_user_id,
                source.knowledge_type,
                &source.content,
                source.topic_key.as_deref(),
                source.value_key.as_deref(),
                approval
                    .sanitized_applicability
                    .iter()
                    .map(|(kind, value)| ApplicabilityFact {
                        kind: *kind,
                        value: value.clone(),
                    })
                    .collect(),
            );
            // T076: the origin digest is the **only** thing promotion adds to
            // the record. There is deliberately no verification field to carry
            // over, in either direction — a personal record has nowhere to hold
            // one (FR-513, FR-517), which is why the gate's `verification_reset`
            // check resets nothing and exists only to make that absence visible
            // at the moment promotion happens.
            record.origin_digest = Some(approval.origin_digest);

            let outcome = create_personal(store, record, project_identities)
                .await
                .map_err(|e| WireError::new(codes::INVALID_REQUEST, e.to_string()))?;
            Ok(outcome.record.id)
        }
        PromotionTarget::Team => {
            // The **fourth** entry point (T124). Structurally identical to
            // personal promotion above and deliberately so: the same gate, the
            // same validator, the same origin digest, the same refusal shape.
            // Two things differ, and both are the point.
            //
            // First, the record always lands `proposed` — `propose_team` has no
            // parameter that could make it authoritative, so "never
            // authoritative on promotion" (FR-515) holds by there being no way
            // to ask, not by this call site choosing well.
            //
            // Second, the promoter must be a member of the source project. That
            // is check 5 (`not_a_member`) inside the gate, which has already run
            // by the time control reaches here — promoting a project's knowledge
            // into server-wide guidance is a claim about that project, and a
            // non-member has no standing to make it.
            let mut record = NewTeamKnowledge::direct(
                owner_user_id,
                source.knowledge_type,
                &source.content,
                source.topic_key.as_deref(),
                source.value_key.as_deref(),
                approval
                    .sanitized_applicability
                    .iter()
                    .map(|(kind, value)| ApplicabilityFact {
                        kind: *kind,
                        value: value.clone(),
                    })
                    .collect(),
            );
            // As on the personal side: the digest is the only thing promotion
            // adds, and there is no verification field to carry over (T076,
            // FR-513).
            record.origin_digest = Some(approval.origin_digest);

            let outcome = propose_team(store, record, project_identities)
                .await
                .map_err(|e| WireError::new(codes::INVALID_REQUEST, e.to_string()))?;
            Ok(outcome.record.id)
        }
        PromotionTarget::Pattern => Err(WireError::invalid(
            "pattern promotion does not go through this path",
        )),
    }
}

/// Report a gate refusal synchronously, in the same response that asked for the
/// promotion (FR-520).
///
/// Never through a separate channel the caller must poll: an agent that
/// promoted something and was told "we'll let you know" has no way to act on the
/// answer. The class travels alongside the check name, and neither can hold the
/// offending content — both are `&'static str` drawn from fixed vocabularies.
fn refusal(rejection: PromotionRejection) -> WireError {
    let detail = match rejection.class {
        Some(class) => format!("{} ({class})", rejection.check),
        None => rejection.check.to_string(),
    };
    WireError::new(
        codes::INVALID_REQUEST,
        format!("promotion refused: {detail}"),
    )
}

async fn source_facts(store: &Store, memory_id: Uuid) -> Result<Source, WireError> {
    let row = sqlx::query_as::<_, (String, String, Option<String>, Option<String>, String)>(
        "SELECT state, content, topic_key, value_key, type
           FROM memories WHERE id = ?1 AND deleted_at IS NULL",
    )
    .bind(memory_id.to_string())
    .fetch_optional(store.pool())
    .await
    .map_err(|e| WireError::new(codes::STORAGE_UNAVAILABLE, e.to_string()))?;

    let Some((state, content, topic_key, value_key, kind)) = row else {
        return Err(WireError::not_found("no such memory"));
    };
    Ok(Source {
        state: state.parse().unwrap_or(MemoryState::Stale),
        content,
        topic_key,
        value_key,
        knowledge_type: kind.parse().unwrap_or(cairn_core::domain::MemoryType::Fact),
    })
}
