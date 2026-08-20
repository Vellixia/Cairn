//! Task work state: the cross-device state identity, derived progress and
//! readiness (`contracts/task-model.md`).
//!
//! Pure. Every value here is computed from records that synchronize, which is
//! what lets two machines agree without a CRDT, a vector clock or an ordering
//! authority: the records converge by identity, and the identity of the
//! resulting state is computed from them.

use crate::domain::{
    BlockerState, CompletionReadiness, CriterionState, CriterionVerification, TaskStatus,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// One acceptance criterion, as the digest and the derived counts see it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CriterionFacts {
    pub id: Uuid,
    pub ordinal: i64,
    pub text: String,
    pub state: CriterionState,
    pub verification: CriterionVerification,
    #[serde(default)]
    pub deleted: bool,
}

/// One blocker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockerFacts {
    pub id: Uuid,
    pub state: BlockerState,
    #[serde(default)]
    pub deleted: bool,
}

/// Everything the cross-device identity is derived from.
///
/// Note what is absent: `local_revision`, every timestamp, and every session
/// identifier. A counter cannot mean the same thing on two machines, and a
/// clock cannot decide anything (D49, D80) — so neither is an input, and the
/// type makes that structural rather than a rule to remember.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskStateFacts {
    pub title: String,
    pub goal: String,
    pub status: TaskStatus,
    pub criteria: Vec<CriterionFacts>,
    pub blockers: Vec<BlockerFacts>,
}

/// Derive a task's cross-device state identity (FR-493).
///
/// ```text
/// SHA-256 of the canonical serialization of
///
///     title_digest, goal_digest, status
///
///  ++ for each non-deleted criterion, ordered by (ordinal, id):
///         criterion_id, ordinal, text_digest, state, verification
///
///  ++ for each non-deleted blocker, ordered by id:
///         blocker_id, state
/// ```
///
/// Properties, each asserted below:
///
/// - **order-independent** — inputs are sorted by stable identifiers, so the
///   sequence in which changes arrived cannot change the result;
/// - **clock-independent** — no timestamp is an input;
/// - **counter-independent** — `local_revision` is not an input;
/// - **content-addressed** — two machines agree exactly when their converged
///   records agree, which is the definition of "the same task state";
/// - **derived** — nothing stores it, so nothing can disagree with it.
///
/// Text is hashed rather than embedded so the serialization stays fixed-width
/// per field and no criterion's prose can shift another field's boundary.
pub fn derive_task_state_digest(facts: &TaskStateFacts) -> String {
    const FIELD: char = '\u{1f}';
    const RECORD: char = '\u{1e}';

    let mut buf = String::new();
    buf.push_str(&crate::digest(&facts.title));
    buf.push(FIELD);
    buf.push_str(&crate::digest(&facts.goal));
    buf.push(FIELD);
    buf.push_str(facts.status.as_str());
    buf.push(RECORD);

    let mut criteria: Vec<&CriterionFacts> = facts.criteria.iter().filter(|c| !c.deleted).collect();
    criteria.sort_by_key(|c| (c.ordinal, c.id));
    for c in criteria {
        buf.push_str(&c.id.to_string());
        buf.push(FIELD);
        buf.push_str(&c.ordinal.to_string());
        buf.push(FIELD);
        buf.push_str(&crate::digest(&c.text));
        buf.push(FIELD);
        buf.push_str(c.state.as_str());
        buf.push(FIELD);
        buf.push_str(c.verification.as_str());
        buf.push(RECORD);
    }

    let mut blockers: Vec<&BlockerFacts> = facts.blockers.iter().filter(|b| !b.deleted).collect();
    blockers.sort_by_key(|b| b.id);
    for b in blockers {
        buf.push_str(&b.id.to_string());
        buf.push(FIELD);
        buf.push_str(b.state.as_str());
        buf.push(RECORD);
    }

    crate::digest(&buf)
}

/// Derived progress, as counts by state. There is no field in which to store a
/// percentage, so an agent cannot write one (FR-486, US11 #5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Progress {
    pub verified: usize,
    /// The honest description of "the agent says it is done and nothing has
    /// checked" — counted separately from `verified`, never folded into it
    /// (FR-483).
    pub satisfied_unverified: usize,
    pub blocked: usize,
    pub pending: usize,
    pub waived: usize,
    pub total: usize,
}

/// Count criteria by state.
pub fn progress(criteria: &[CriterionFacts]) -> Progress {
    let mut p = Progress::default();
    for c in criteria.iter().filter(|c| !c.deleted) {
        p.total += 1;
        match (c.state, c.verification) {
            (CriterionState::Waived, _) => p.waived += 1,
            (CriterionState::Blocked, _) => p.blocked += 1,
            (CriterionState::Satisfied, CriterionVerification::Verified) => p.verified += 1,
            (CriterionState::Satisfied, _) => p.satisfied_unverified += 1,
            (CriterionState::Pending, _) => p.pending += 1,
        }
    }
    p
}

/// Derive completion readiness.
///
/// ```text
/// ready             every non-waived criterion is satisfied AND verified,
///                   AND no blocker is open
/// ready_unverified  every non-waived criterion is satisfied,
///                   AND no blocker is open,
///                   AND at least one is not verified
/// not_ready         otherwise
/// ```
///
/// Derived on read, never stored as authority, and **Cairn never changes a
/// task's status on the basis of it** — completing a task stays an explicit act
/// (FR-487).
pub fn completion_readiness(
    criteria: &[CriterionFacts],
    blockers: &[BlockerFacts],
) -> CompletionReadiness {
    let open_blockers = blockers
        .iter()
        .filter(|b| !b.deleted && b.state == BlockerState::Open)
        .count();
    if open_blockers > 0 {
        return CompletionReadiness::NotReady;
    }

    let considered: Vec<&CriterionFacts> = criteria
        .iter()
        .filter(|c| !c.deleted && c.state != CriterionState::Waived)
        .collect();

    if considered
        .iter()
        .any(|c| c.state != CriterionState::Satisfied)
    {
        return CompletionReadiness::NotReady;
    }
    if considered
        .iter()
        .any(|c| c.verification != CriterionVerification::Verified)
    {
        return CompletionReadiness::ReadyUnverified;
    }
    CompletionReadiness::Ready
}

/// Order criteria for Level 0's bounded detail tier.
///
/// `blocked → satisfied but unverified → pending → verified → waived`, ties by
/// ascending ordinal. Chosen so the ones an agent must act on arrive first: a
/// blocked criterion is what stops progress, a satisfied-but-unverified one is
/// what needs a check, and `verified`/`waived` are what an agent least needs to
/// re-read (`contracts/continuity-context.md` §Criterion action order).
pub fn action_order(criteria: &[CriterionFacts]) -> Vec<&CriterionFacts> {
    let mut out: Vec<&CriterionFacts> = criteria.iter().filter(|c| !c.deleted).collect();
    out.sort_by_key(|c| {
        let verified = c.verification == CriterionVerification::Verified;
        (c.state.action_rank(verified), c.ordinal, c.id)
    });
    out
}

/// The ordinal-ordered projection Feature 001's `tasks.acceptance_criteria`
/// holds (D68, FR-492).
///
/// The one denormalization the feature retains, because five readers consume it
/// and replacing it with a join would break all five at once for no capability
/// gain. It is rewritten in the same transaction as any criterion change, and
/// `rebuild_criteria_projection` asserts the equality (I11, SC-324).
pub fn criteria_projection(criteria: &[CriterionFacts]) -> Vec<String> {
    let mut ordered: Vec<&CriterionFacts> = criteria.iter().filter(|c| !c.deleted).collect();
    ordered.sort_by_key(|c| (c.ordinal, c.id));
    ordered.into_iter().map(|c| c.text.clone()).collect()
}

/// The label a criterion gets at creation: `AC-<ordinal>`.
///
/// **Not renumbered** when a criterion is added or removed. Renumbering would
/// silently change what "AC-2" means in a handoff, a checkpoint or a session's
/// memory (FR-481).
pub fn criterion_label(ordinal: i64) -> String {
    format!("AC-{ordinal}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    fn criterion(n: u128, ordinal: i64, state: CriterionState) -> CriterionFacts {
        CriterionFacts {
            id: id(n),
            ordinal,
            text: format!("criterion {ordinal}"),
            state,
            verification: CriterionVerification::Unverified,
            deleted: false,
        }
    }

    fn task() -> TaskStateFacts {
        TaskStateFacts {
            title: "Ship the thing".into(),
            goal: "It ships".into(),
            status: TaskStatus::InProgress,
            criteria: vec![
                criterion(1, 1, CriterionState::Satisfied),
                criterion(2, 2, CriterionState::Pending),
            ],
            blockers: vec![BlockerFacts {
                id: id(10),
                state: BlockerState::Open,
                deleted: false,
            }],
        }
    }

    #[test]
    fn the_digest_is_order_independent() {
        // The property the offline-convergence scenario rests on: the sequence
        // in which changes arrived cannot change the identity (SC-330).
        let a = task();
        let mut b = task();
        b.criteria.reverse();
        b.blockers.reverse();
        assert_eq!(derive_task_state_digest(&a), derive_task_state_digest(&b));
    }

    #[test]
    fn the_digest_is_counter_and_clock_independent_by_construction() {
        // Neither is an input, so this asserts the *type* rather than a value:
        // there is nowhere to put a counter or a timestamp. The test exists so
        // that adding a field to `TaskStateFacts` has to consider it.
        let json = serde_json::to_string(&task()).unwrap();
        for forbidden in [
            "local_revision",
            "revision",
            "created_at",
            "updated_at",
            "changed_at",
            "session",
        ] {
            assert!(
                !json.contains(forbidden),
                "{forbidden} reached the digest input"
            );
        }
    }

    #[test]
    fn two_machines_with_the_same_converged_records_agree() {
        // Machine A changed AC-1, machine B changed AC-2, both synced.
        let mut a = task();
        a.criteria[0].state = CriterionState::Satisfied;
        a.criteria[1].state = CriterionState::Satisfied;

        let mut b = task();
        // B learned the same two changes in the other order, and holds its
        // records in a different sequence.
        b.criteria = vec![a.criteria[1].clone(), a.criteria[0].clone()];

        assert_eq!(derive_task_state_digest(&a), derive_task_state_digest(&b));
    }

    #[test]
    fn any_material_change_moves_the_digest() {
        let base = derive_task_state_digest(&task());

        let mut title = task();
        title.title = "Ship the other thing".into();
        assert_ne!(derive_task_state_digest(&title), base);

        let mut goal = task();
        goal.goal = "It does not ship".into();
        assert_ne!(derive_task_state_digest(&goal), base);

        let mut status = task();
        status.status = TaskStatus::Done;
        assert_ne!(derive_task_state_digest(&status), base);

        let mut state = task();
        state.criteria[1].state = CriterionState::Satisfied;
        assert_ne!(derive_task_state_digest(&state), base);

        let mut verification = task();
        verification.criteria[0].verification = CriterionVerification::Verified;
        assert_ne!(derive_task_state_digest(&verification), base);

        let mut text = task();
        text.criteria[0].text = "reworded".into();
        assert_ne!(derive_task_state_digest(&text), base);

        let mut blocker = task();
        blocker.blockers[0].state = BlockerState::Cleared;
        assert_ne!(derive_task_state_digest(&blocker), base);

        let mut added = task();
        added
            .criteria
            .push(criterion(3, 3, CriterionState::Pending));
        assert_ne!(derive_task_state_digest(&added), base);
    }

    #[test]
    fn a_tombstoned_record_leaves_the_digest() {
        let mut removed = task();
        removed.criteria[1].deleted = true;

        let mut without = task();
        without.criteria.pop();

        assert_eq!(
            derive_task_state_digest(&removed),
            derive_task_state_digest(&without),
            "a tombstone and an absence are the same converged state"
        );
    }

    #[test]
    fn field_boundaries_cannot_be_shifted_by_content() {
        // Text is hashed rather than embedded, so no criterion's prose can
        // impersonate a separator and collide two different tasks.
        let mut a = task();
        a.criteria[0].text = "one\u{1f}two".into();
        let mut b = task();
        b.criteria[0].text = "one".into();
        b.criteria[0].ordinal = 1;
        assert_ne!(derive_task_state_digest(&a), derive_task_state_digest(&b));
    }

    #[test]
    fn progress_counts_satisfied_but_unverified_separately() {
        let criteria = vec![
            CriterionFacts {
                verification: CriterionVerification::Verified,
                ..criterion(1, 1, CriterionState::Satisfied)
            },
            criterion(2, 2, CriterionState::Satisfied),
            criterion(3, 3, CriterionState::Blocked),
            criterion(4, 4, CriterionState::Pending),
            criterion(5, 5, CriterionState::Waived),
        ];
        let p = progress(&criteria);
        assert_eq!(p.verified, 1);
        assert_eq!(p.satisfied_unverified, 1);
        assert_eq!(p.blocked, 1);
        assert_eq!(p.pending, 1);
        assert_eq!(p.waived, 1);
        assert_eq!(p.total, 5);
    }

    #[test]
    fn readiness_needs_every_criterion_verified_and_no_open_blocker() {
        let verified = |n: u128, o: i64| CriterionFacts {
            verification: CriterionVerification::Verified,
            ..criterion(n, o, CriterionState::Satisfied)
        };

        assert_eq!(
            completion_readiness(&[verified(1, 1), verified(2, 2)], &[]),
            CompletionReadiness::Ready
        );

        assert_eq!(
            completion_readiness(
                &[verified(1, 1), criterion(2, 2, CriterionState::Satisfied)],
                &[]
            ),
            CompletionReadiness::ReadyUnverified
        );

        assert_eq!(
            completion_readiness(
                &[verified(1, 1), criterion(2, 2, CriterionState::Pending)],
                &[]
            ),
            CompletionReadiness::NotReady
        );

        assert_eq!(
            completion_readiness(
                &[verified(1, 1)],
                &[BlockerFacts {
                    id: id(10),
                    state: BlockerState::Open,
                    deleted: false
                }]
            ),
            CompletionReadiness::NotReady,
            "an open blocker is not ready, however verified the criteria are"
        );

        assert_eq!(
            completion_readiness(
                &[verified(1, 1)],
                &[BlockerFacts {
                    id: id(10),
                    state: BlockerState::Cleared,
                    deleted: false
                }]
            ),
            CompletionReadiness::Ready
        );
    }

    #[test]
    fn a_waived_criterion_does_not_hold_readiness_back() {
        let verified = CriterionFacts {
            verification: CriterionVerification::Verified,
            ..criterion(1, 1, CriterionState::Satisfied)
        };
        let waived = criterion(2, 2, CriterionState::Waived);
        assert_eq!(
            completion_readiness(&[verified, waived], &[]),
            CompletionReadiness::Ready
        );
    }

    #[test]
    fn an_empty_task_is_ready() {
        // No criteria and no blockers: vacuously satisfied. Readiness is
        // derived, and Cairn still changes no status on the basis of it.
        assert_eq!(completion_readiness(&[], &[]), CompletionReadiness::Ready);
    }

    #[test]
    fn action_order_leads_with_what_stops_progress() {
        let criteria = vec![
            criterion(1, 1, CriterionState::Waived),
            CriterionFacts {
                verification: CriterionVerification::Verified,
                ..criterion(2, 2, CriterionState::Satisfied)
            },
            criterion(3, 3, CriterionState::Pending),
            criterion(4, 4, CriterionState::Satisfied),
            criterion(5, 5, CriterionState::Blocked),
        ];
        let ordered: Vec<i64> = action_order(&criteria).iter().map(|c| c.ordinal).collect();
        assert_eq!(ordered, vec![5, 4, 3, 2, 1]);
    }

    #[test]
    fn action_order_breaks_ties_by_ascending_ordinal() {
        let criteria = vec![
            criterion(1, 3, CriterionState::Pending),
            criterion(2, 1, CriterionState::Pending),
            criterion(3, 2, CriterionState::Pending),
        ];
        let ordered: Vec<i64> = action_order(&criteria).iter().map(|c| c.ordinal).collect();
        assert_eq!(ordered, vec![1, 2, 3]);
    }

    #[test]
    fn the_projection_is_the_ordinal_ordered_text() {
        let criteria = vec![
            CriterionFacts {
                text: "second".into(),
                ..criterion(1, 2, CriterionState::Pending)
            },
            CriterionFacts {
                text: "first".into(),
                ..criterion(2, 1, CriterionState::Pending)
            },
            CriterionFacts {
                text: "gone".into(),
                deleted: true,
                ..criterion(3, 3, CriterionState::Pending)
            },
        ];
        assert_eq!(criteria_projection(&criteria), vec!["first", "second"]);
    }

    #[test]
    fn a_label_is_derived_from_the_ordinal_and_never_renumbered() {
        assert_eq!(criterion_label(1), "AC-1");
        assert_eq!(criterion_label(3), "AC-3");
        // Removing AC-2 leaves AC-1 and AC-3; AC-3 keeps its name, which is
        // what a handoff or a checkpoint naming it depends on.
        let remaining = [
            criterion(1, 1, CriterionState::Pending),
            criterion(3, 3, CriterionState::Pending),
        ];
        let labels: Vec<String> = remaining
            .iter()
            .map(|c| criterion_label(c.ordinal))
            .collect();
        assert_eq!(labels, vec!["AC-1", "AC-3"]);
    }
}
