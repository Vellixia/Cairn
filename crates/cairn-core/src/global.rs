//! The two project-independent knowledge domains (FR-431–FR-465).
//!
//! Personal knowledge follows one user across every project and device they
//! touch. Team knowledge is the server-wide default, proposed by any member and
//! made authoritative only by an administrator.
//!
//! Neither record type has a field for a project identifier, an evidence
//! reference, an observation identifier, or verification of any kind — not an
//! authority, not a state, not a timestamp (FR-513, FR-517). That absence is
//! Layer A: there is nowhere to put those values, so no validator could miss
//! one and no caller could bypass one. It is the same argument
//! `reusable_patterns` rests on — a record that cannot name a project cannot
//! leak one.
//!
//! What these types *do* carry in free text — `content`, `topic_key`,
//! `value_key`, and every applicability value — is Layer B, guarded by
//! [`crate::validate::validate_global_content`] at all five entry points
//! (FR-550). The distinction is not pedantry: an earlier draft of this design
//! described a free-text column as structurally incapable of holding a path,
//! and it was not true.

use crate::domain::{ApplicabilityFact, MemoryType, TeamState};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A single user's durable, project-less knowledge (FR-431–FR-446).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersonalKnowledge {
    pub id: Uuid,
    pub owner_user_id: Uuid,
    pub knowledge_type: MemoryType,
    pub content: String,
    pub topic_key: Option<String>,
    pub value_key: Option<String>,
    /// Set only when this record was created by promotion (D418).
    ///
    /// **Local only, never transmitted** (D434, FR-551). The server knows every
    /// project identity it holds, so a transmitted digest of one is a lookup
    /// away from being reversed — and the digest is salted per machine, which
    /// means two devices of the same user compute different digests for the
    /// same source project. That divergence is intentional: it is the cost of
    /// keeping the digest off the wire, and `FR-552` requires it documented
    /// rather than discovered.
    #[serde(skip)]
    pub origin_digest: Option<String>,
    pub applicability: Vec<ApplicabilityFact>,
    /// This store's writer identity.
    ///
    /// Crosses the wire and has a column on both sides (D448, FR-582), unlike
    /// `origin_digest` above. A peer needs to see it to detect a gap in this
    /// writer's stream (FR-492), and a gap detector that cannot see the
    /// sequence detects nothing.
    pub writer_id: Uuid,
    /// This row's position in its writer's own stream.
    ///
    /// **Diagnostic only** (FR-583). No importer may consult it as an ordering
    /// key, a tiebreak, or a conflict-resolution input — which is enforced by
    /// keeping it off every reconciliation input type rather than by asking
    /// nicely, the same discipline that keeps timestamps out of `MemoryFacts`.
    pub writer_seq: i64,
    pub created_at: DateTime<Utc>,
    pub superseded_by_id: Option<Uuid>,
    pub forgotten_at: Option<DateTime<Utc>>,
}

/// The server-wide default knowledge; begins `Proposed` (FR-451–FR-465).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamKnowledge {
    pub id: Uuid,
    pub knowledge_type: MemoryType,
    pub content: String,
    pub topic_key: Option<String>,
    pub value_key: Option<String>,
    /// Local only, never transmitted — see [`PersonalKnowledge::origin_digest`].
    #[serde(skip)]
    pub origin_digest: Option<String>,
    pub applicability: Vec<ApplicabilityFact>,
    /// Advanced only by compare-and-swap on the expected state (D409, FR-454),
    /// never by last-write-wins.
    pub state: TeamState,
    /// Kept as a traceable reference only, never as project-identifying
    /// content (FR-459).
    pub proposed_by_user_id: Uuid,
    pub ratified_by_user_id: Option<Uuid>,
    pub ratified_at: Option<DateTime<Utc>>,
    /// Same wire-crossing, diagnostic-only stamp as personal knowledge's
    /// (D448, FR-582, FR-583).
    pub writer_id: Uuid,
    pub writer_seq: i64,
    pub created_at: DateTime<Utc>,
    pub superseded_by_id: Option<Uuid>,
    /// Who retired it, alongside `retired_at` (FR-457).
    ///
    /// The column existed on both schemas from the start; this field did not,
    /// so the record type could not carry the answer even where the database
    /// held it — `cairn team list` had no way to say who removed a piece of
    /// guidance, and the synchronized mirror had nothing to copy. A timestamp
    /// alone does not record who acted, and retirement is the transition most
    /// worth attributing: it removes guidance from every account on the server.
    pub retired_by_user_id: Option<Uuid>,
    pub retired_at: Option<DateTime<Utc>>,
}

/// The salted digest of a promotion's source project (FR-516, D434).
///
/// Reuses the approach `reusable_patterns` already uses for `origin_ref`
/// (`cairn-store/src/patterns.rs`, `paths::machine_salt`): salted rather than a
/// bare digest, because a project id is a UUID and a bare digest of one is a
/// lookup away from being reversed by anyone holding the id — which the server
/// does, for every project.
///
/// The salt is per machine, so two devices of the same user produce **different**
/// digests for the same source project. That is correct and intentional: it is
/// what makes the digest useless to a party that did not compute it, and the
/// price is that origin recognition is per-machine only (FR-552).
pub fn origin_digest(machine_salt: &str, project_id: Uuid) -> String {
    crate::digest(&format!("{machine_salt}:{project_id}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SC-441's first two clauses. The third — that no digest reaches the wire
    /// — is asserted structurally in `tests/tests/privacy_payloads.rs`, because
    /// it is a claim about serialized forms rather than about this function.
    #[test]
    fn one_machine_agrees_with_itself_and_disagrees_with_another() {
        let project = Uuid::now_v7();
        let other_project = Uuid::now_v7();
        let machine_a = "salt-a";
        let machine_b = "salt-b";

        // Two promotions from the same project on one machine share a digest.
        assert_eq!(
            origin_digest(machine_a, project),
            origin_digest(machine_a, project)
        );
        // Different projects on one machine do not.
        assert_ne!(
            origin_digest(machine_a, project),
            origin_digest(machine_a, other_project)
        );
        // The same project on a second machine gives a *different* digest, and
        // this divergence is the design, not a defect (D434).
        assert_ne!(
            origin_digest(machine_a, project),
            origin_digest(machine_b, project)
        );
        // The second machine still agrees with itself.
        assert_eq!(
            origin_digest(machine_b, project),
            origin_digest(machine_b, project)
        );
    }

    /// The digest must not be reversible by someone holding the project id
    /// alone — which the server always is. Asserted as: the digest of a project
    /// id with no salt is not the digest we produce.
    #[test]
    fn the_digest_is_not_a_bare_digest_of_the_project_id() {
        let project = Uuid::now_v7();
        assert_ne!(
            origin_digest("salt", project),
            crate::digest(&project.to_string())
        );
    }
}
