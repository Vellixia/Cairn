//! Deterministic identities for Feature 005 (`data-model.md` §1.4,
//! `contracts/consolidation.md` §7, `contracts/knowledge-commands.md` §3–4).
//!
//! Six identities, one rule: **derive from what is stable, never from a clock
//! and never from a set that changes on retry.**
//!
//! Every one of them is a UUIDv5, which is a digest of a namespace and a name.
//! That makes each derivation reproducible by both sides independently — the
//! daemon assigns an event id, and the server recomputes it and refuses a
//! mismatch, so idempotency is not something a client gets to control. A client
//! that could choose its own id could submit a colliding one, be answered
//! `duplicate`, and suppress a genuine event; or pre-claim ids it guessed.
//!
//! The namespaces are fixed constants and must never change. Changing one
//! re-derives every identity in the system, which would present every existing
//! record as new.

use uuid::Uuid;

/// Namespace for `event_id` — `UUIDv5(ns, session_id ‖ session_seq)`.
pub const CAIRN_EVENT_NS: Uuid = Uuid::from_bytes([
    0x1e, 0x7a, 0x0c, 0x51, 0x4b, 0x9d, 0x5a, 0x2e, 0x9f, 0x38, 0x6d, 0x41, 0xc8, 0x0b, 0x27, 0x11,
]);

/// Namespace for `command_id`.
pub const CAIRN_COMMAND_NS: Uuid = Uuid::from_bytes([
    0x2c, 0x14, 0x8f, 0x63, 0x77, 0xa5, 0x5b, 0x0c, 0xb2, 0x6d, 0x0e, 0x93, 0x54, 0xf1, 0x8a, 0x22,
]);

/// Namespace for `candidate_id`.
pub const CAIRN_CANDIDATE_NS: Uuid = Uuid::from_bytes([
    0x3a, 0x9d, 0x21, 0x74, 0x18, 0xc6, 0x5d, 0x47, 0x84, 0x1b, 0xf7, 0x2a, 0x36, 0x5e, 0x90, 0x33,
]);

/// Namespace for a consolidation refusal's identity.
pub const CAIRN_REFUSAL_NS: Uuid = Uuid::from_bytes([
    0x4f, 0x30, 0xb8, 0x1a, 0x92, 0x5e, 0x5c, 0x83, 0xa7, 0x04, 0x2b, 0xd6, 0x71, 0x8c, 0x45, 0x44,
]);

/// Namespace for a corroboration endpoint's identity.
pub const CAIRN_CORROBORATION_NS: Uuid = Uuid::from_bytes([
    0x5b, 0xc7, 0x46, 0x2f, 0x0d, 0x38, 0x5e, 0x91, 0x93, 0xa5, 0x8f, 0x1c, 0x60, 0x47, 0xe2, 0x55,
]);

/// Namespace for `pattern_id`.
pub const CAIRN_PATTERN_NS: Uuid = Uuid::from_bytes([
    0x6d, 0x52, 0xf9, 0x08, 0xa4, 0x1b, 0x5f, 0x2a, 0xb8, 0xc9, 0x34, 0x7e, 0x05, 0xd3, 0x61, 0x66,
]);

/// The separator between name components.
///
/// A byte that cannot occur in any component — every component is a UUID, a
/// decimal integer, a normalized key or a hex digest — so `a ‖ bc` and
/// `ab ‖ c` cannot produce the same name. Without a separator they could, and
/// two different subjects would share one identity.
const SEP: u8 = 0x1f;

fn derive(namespace: &Uuid, parts: &[&[u8]]) -> Uuid {
    let mut name = Vec::new();
    for (i, part) in parts.iter().enumerate() {
        if i > 0 {
            name.push(SEP);
        }
        name.extend_from_slice(part);
    }
    Uuid::new_v5(namespace, &name)
}

/// `event_id = UUIDv5(CAIRN_EVENT_NS, session_id ‖ session_seq)`.
///
/// Keyed on the **session id**, never the vendor's session key: shipped code
/// frees a vendor key when a session is deleted so it can be reused
/// (`crates/cairn-store/src/repo.rs`), and keying on it would collapse a
/// resumed session's events onto an earlier session's identities.
///
/// `session_seq` comes from `session_event_seq`, a durable counter, and never
/// from `MAX(session_seq)` over the spool. The spool drains and sheds rows
/// under the capacity policy; a counter recovered from it would restart at 1
/// and re-derive an identity a delivered event already used — which the server
/// would answer `duplicate`, silently discarding a real event.
pub fn event_id(session_id: Uuid, session_seq: u64) -> Uuid {
    derive(
        &CAIRN_EVENT_NS,
        &[session_id.as_bytes(), session_seq.to_string().as_bytes()],
    )
}

/// `command_id = UUIDv5(CAIRN_COMMAND_NS, scope_kind ‖ scope_key ‖ command_seq)`.
///
/// The scope kind is part of the name because a command's scope may be a
/// session or the store itself — the CLI issues knowledge commands outside any
/// session — and a session UUID and a store's `writer_id` are different
/// namespaces of key that must not be able to collide.
pub fn command_id(scope_kind: &str, scope_key: &str, command_seq: u64) -> Uuid {
    derive(
        &CAIRN_COMMAND_NS,
        &[
            scope_kind.as_bytes(),
            scope_key.as_bytes(),
            command_seq.to_string().as_bytes(),
        ],
    )
}

/// `candidate_id = UUIDv5(CAIRN_CANDIDATE_NS, project ‖ session ‖ topic ‖ value)`.
///
/// The keys are the **normalized** ones (FR-796a), so a syntactic variant —
/// `Storage Authority`, `storage_authority`, `storage-authority` — cannot
/// produce a second candidate for one subject.
///
/// **The source event set is deliberately not part of this name** (FR-798c).
/// It is not stable across re-execution: a reclaim after a lease expires sweeps
/// in events that arrived meanwhile, and an event that exhausts its attempts
/// leaves the batch. An identity including it would change on retry and produce
/// exactly the duplicate the determinism requirement exists to prevent. Source
/// events are recorded additively as evidence; they are provenance, not
/// identity.
pub fn candidate_id(
    project_id: Uuid,
    session_id: Uuid,
    topic_key: Option<&str>,
    value_key: Option<&str>,
) -> Uuid {
    derive(
        &CAIRN_CANDIDATE_NS,
        &[
            project_id.as_bytes(),
            session_id.as_bytes(),
            topic_key.unwrap_or("").as_bytes(),
            value_key.unwrap_or("").as_bytes(),
        ],
    )
}

/// A refusal's identity, for a candidate that has no normalized keys.
///
/// `UUIDv5(CAIRN_REFUSAL_NS, project ‖ session ‖ reason ‖ digest(proposal))`.
///
/// A candidate refused for a malformed key has no normalized keys, so
/// [`candidate_id`] is unavailable to it — and deriving refusals from the key
/// pair anyway would collapse every `key_normalization_failed` in a session
/// onto one row and undercount refusals, which FR-807 and SC-705 depend on
/// being accurate. The proposal digest is what keeps several distinct
/// malformed proposals distinct.
///
/// `proposal_digest` is a hex digest computed by the caller with
/// [`proposal_digest`], so this function never sees proposal text.
pub fn refusal_id(
    project_id: Uuid,
    session_id: Uuid,
    refusal_reason: &str,
    proposal_digest: &str,
) -> Uuid {
    derive(
        &CAIRN_REFUSAL_NS,
        &[
            project_id.as_bytes(),
            session_id.as_bytes(),
            refusal_reason.as_bytes(),
            proposal_digest.as_bytes(),
        ],
    )
}

/// A corroboration endpoint's identity (FR-798b, FR-798c).
///
/// Reinforcement is a relation between two durable records, so a candidate that
/// reinforces needs a persisted endpoint to reinforce *from*. That endpoint has
/// to survive a mid-pass restart without becoming a second endpoint, so its
/// identity is derived from the same stable triple a candidate's is — project,
/// session, normalized keys — and, like a candidate's, deliberately not from
/// the events that evidenced it.
pub fn corroboration_id(
    project_id: Uuid,
    session_id: Uuid,
    topic_key: Option<&str>,
    value_key: Option<&str>,
) -> Uuid {
    derive(
        &CAIRN_CORROBORATION_NS,
        &[
            project_id.as_bytes(),
            session_id.as_bytes(),
            topic_key.unwrap_or("").as_bytes(),
            value_key.unwrap_or("").as_bytes(),
        ],
    )
}

/// `pattern_id = UUIDv5(CAIRN_PATTERN_NS, owner_user_id ‖ content_key)`.
///
/// Local duplicate identity is `signal_digest + root_cause_digest`, and both
/// are refused field names, so the safe shape derives identity from safe
/// content instead — see [`content_key`]. Promotion is therefore an upsert: a
/// retry and a re-run migration converge on one record (FR-708f, SC-760).
///
/// The owner is part of the name, so two people whose patterns read alike own
/// two patterns rather than sharing one.
pub fn pattern_id(owner_user_id: Uuid, content_key: &str) -> Uuid {
    derive(
        &CAIRN_PATTERN_NS,
        &[owner_user_id.as_bytes(), content_key.as_bytes()],
    )
}

/// A pattern's privacy-safe duplicate identity (`data-model.md` §6.2).
///
/// `digest(normalize(problem) ‖ normalize(root_cause) ‖ normalize(approach))`.
/// Every input is a field that legitimately crosses the boundary, so the
/// identity discloses nothing the record does not already carry.
///
/// The title is deliberately absent. Two patterns that differ only in their
/// title collapse to one, which is correct: the title is a label, and the
/// problem, cause and approach are the pattern.
pub fn content_key(problem: &str, root_cause: &str, approach: &str) -> String {
    digest_parts(&[
        &normalize_for_digest(problem),
        &normalize_for_digest(root_cause),
        &normalize_for_digest(approach),
    ])
}

/// A digest of a refused proposal, for [`refusal_id`].
///
/// Hashed rather than carried, so the identity of a refusal can distinguish two
/// bad proposals without the refusal record holding either one's text
/// (FR-741).
pub fn proposal_digest(proposal: &str) -> String {
    digest_parts(&[&normalize_for_digest(proposal)])
}

/// Collapse the differences a digest must not be sensitive to.
///
/// Case and runs of whitespace only. Deliberately *not* punctuation or word
/// order: two sentences that differ in either are two different claims, and
/// folding them would make one pattern silently overwrite another.
fn normalize_for_digest(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn digest_parts(parts: &[&str]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    for (i, part) in parts.iter().enumerate() {
        if i > 0 {
            hasher.update([SEP]);
        }
        hasher.update(part.as_bytes());
    }
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_event_identity_depends_on_nothing_but_its_session_and_ordinal() {
        let session = Uuid::now_v7();
        let a = event_id(session, 42);
        // Re-derived at a different moment, on a different machine, after a
        // restart: the same id. Nothing here reads a clock.
        assert_eq!(a, event_id(session, 42));
        // A genuinely repeated act gets the next ordinal and is a distinct
        // event, not a suppressed duplicate (FR-738).
        assert_ne!(a, event_id(session, 43));
        assert_ne!(a, event_id(Uuid::now_v7(), 42));
    }

    #[test]
    fn the_separator_keeps_two_different_names_apart() {
        // Without a separator, `session ‖ 12` and a session one byte longer
        // followed by `2` could hash the same name. The components are typed
        // here, so the test states the property directly: differently split
        // components never collide.
        assert_ne!(
            command_id("session", "abc", 12),
            command_id("session", "ab", 12)
        );
        assert_ne!(
            command_id("session", "a", 1),
            command_id("session", "a1", 1)
        );
        // A session-scoped and a store-scoped command with the same key and
        // ordinal are different commands.
        assert_ne!(command_id("session", "k", 1), command_id("store", "k", 1));
    }

    #[test]
    fn a_candidate_is_identified_by_its_subject_and_not_by_its_evidence() {
        let project = Uuid::now_v7();
        let session = Uuid::now_v7();
        let a = candidate_id(project, session, Some("storage.authority"), Some("server"));
        assert_eq!(
            a,
            candidate_id(project, session, Some("storage.authority"), Some("server")),
            "re-executing a reclaimed batch produced a second candidate"
        );
        assert_ne!(
            a,
            candidate_id(project, session, Some("storage.authority"), Some("client"))
        );
        assert_ne!(a, candidate_id(project, session, Some("storage"), None));
        // An absent value key and an empty one are the same absence, but an
        // absent topic with a value is a different subject from the reverse.
        assert_ne!(
            candidate_id(project, session, Some("a"), None),
            candidate_id(project, session, None, Some("a"))
        );
    }

    #[test]
    fn a_corroboration_endpoint_is_stable_but_is_not_its_candidate() {
        let project = Uuid::now_v7();
        let session = Uuid::now_v7();
        let topic = Some("deploy.images");
        let value = Some("unsigned");
        assert_eq!(
            corroboration_id(project, session, topic, value),
            corroboration_id(project, session, topic, value),
            "a mid-pass restart would have added a second corroboration record"
        );
        // Different namespace: the endpoint and the candidate are two records
        // and must not share an id.
        assert_ne!(
            corroboration_id(project, session, topic, value),
            candidate_id(project, session, topic, value)
        );
    }

    #[test]
    fn two_malformed_proposals_in_one_session_are_two_refusals() {
        let project = Uuid::now_v7();
        let session = Uuid::now_v7();
        let a = refusal_id(
            project,
            session,
            "key_normalization_failed",
            &proposal_digest("Storage Authority??"),
        );
        let b = refusal_id(
            project,
            session,
            "key_normalization_failed",
            &proposal_digest("Deploy Pipeline!!"),
        );
        assert_ne!(a, b, "two distinct refusals collapsed onto one row");
        assert_eq!(
            a,
            refusal_id(
                project,
                session,
                "key_normalization_failed",
                &proposal_digest("Storage Authority??")
            )
        );
    }

    #[test]
    fn a_pattern_identity_is_the_owner_and_the_content_not_the_title() {
        let owner = Uuid::now_v7();
        let other = Uuid::now_v7();
        let key = content_key(
            "the pipeline rejects unsigned images",
            "no signer",
            "sign it",
        );
        assert_eq!(
            pattern_id(owner, &key),
            pattern_id(owner, &key),
            "promoting the same pattern twice produced two records"
        );
        // Two people's identical patterns are two patterns.
        assert_ne!(pattern_id(owner, &key), pattern_id(other, &key));
        // The title is a label, not the pattern.
        assert_eq!(
            key,
            content_key(
                "The Pipeline   rejects unsigned images",
                "No signer",
                "Sign it"
            ),
            "case and whitespace changed a pattern's identity"
        );
        // Word order and punctuation are content, not formatting.
        assert_ne!(
            key,
            content_key(
                "unsigned images the pipeline rejects",
                "no signer",
                "sign it"
            )
        );
    }

    #[test]
    fn the_namespaces_are_all_distinct() {
        let all = [
            CAIRN_EVENT_NS,
            CAIRN_COMMAND_NS,
            CAIRN_CANDIDATE_NS,
            CAIRN_REFUSAL_NS,
            CAIRN_CORROBORATION_NS,
            CAIRN_PATTERN_NS,
        ];
        let unique: std::collections::BTreeSet<_> = all.iter().collect();
        assert_eq!(
            unique.len(),
            all.len(),
            "two identity families share a namespace, so one can collide with the other"
        );
    }
}
