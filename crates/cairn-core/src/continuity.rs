//! Checkpoint staleness: what moved beneath a session while it was compacting
//! (`contracts/continuity-context.md` §Restoration and staleness).
//!
//! Pure. The worktree reads that produce a current fingerprint happen in
//! `cairnd`; the comparison and its classification happen here, so the rule
//! that a stale checkpoint must never read as current is testable without a
//! repository.
//!
//! # Why fingerprints rather than observations
//!
//! An earlier design detected a path change by looking for a `file_changed`
//! observation from another session. That misses everything Cairn did not see:
//! a human editing in an editor, a formatter, `git apply`, an IDE refactor,
//! another process — all of which leave the commit unmoved and produce no
//! observation at all. So the checkpoint records what each relevant path *was*,
//! and restoration recomputes it (D79, FR-432).

use crate::domain::{CheckpointState, DivergenceKind, FingerprintClass, PathOutcome};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// What one relevant path looked like.
///
/// `exists` is carried alongside `class` because `class = unknown` alone
/// conflates three different situations — privacy-excluded, unreadable, and
/// simply absent — and `added`/`removed` are only answerable if absence is
/// recorded as itself. The field is local: checkpoints never synchronize
/// (FR-503), so this costs nothing on the wire.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PathFingerprint {
    pub path: String,
    pub class: FingerprintClass,
    /// The digest, or the byte length, or nothing when the class is `unknown`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    pub exists: bool,
}

impl PathFingerprint {
    pub fn digest(path: impl Into<String>, digest: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            class: FingerprintClass::Digest,
            value: Some(digest.into()),
            exists: true,
        }
    }

    /// Used above `payload_cap_bytes`, where a digest would be unbounded work.
    ///
    /// Weaker than a digest — a same-length edit reads as unchanged — and that
    /// is stated rather than hidden. It applies only above the cap, where
    /// source edits are rare.
    pub fn size(path: impl Into<String>, bytes: u64) -> Self {
        Self {
            path: path.into(),
            class: FingerprintClass::Size,
            value: Some(bytes.to_string()),
            exists: true,
        }
    }

    /// Privacy-excluded or unreadable: nothing comparable.
    pub fn unknown(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            class: FingerprintClass::Unknown,
            value: None,
            exists: true,
        }
    }

    /// The path was not there.
    pub fn absent(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            class: FingerprintClass::Unknown,
            value: None,
            exists: false,
        }
    }
}

/// Compare one recorded fingerprint against what the worktree holds now.
///
/// `not_fingerprintable` is returned as itself and **never** collapses into
/// `unchanged`. "I could not look" and "nothing moved" are different answers,
/// and conflating them is exactly how a stale checkpoint would read as current
/// (FR-432, metric 15b).
pub fn compare_path_fingerprint(
    recorded: &PathFingerprint,
    current: &PathFingerprint,
) -> PathOutcome {
    match (recorded.exists, current.exists) {
        (false, false) => PathOutcome::Unchanged,
        (false, true) => PathOutcome::Added,
        (true, false) => PathOutcome::Removed,
        (true, true) => {
            if recorded.class == FingerprintClass::Unknown
                || current.class == FingerprintClass::Unknown
            {
                // Either end could not be fingerprinted. Nothing comparable
                // exists, so nothing is claimed.
                return PathOutcome::NotFingerprintable;
            }
            if recorded.class != current.class {
                // A file that grew past the payload cap between the checkpoint
                // and the restore is compared by different measures at each
                // end. Reporting `unchanged` would be a guess.
                return PathOutcome::NotFingerprintable;
            }
            match (&recorded.value, &current.value) {
                (Some(a), Some(b)) if a == b => PathOutcome::Unchanged,
                (Some(_), Some(_)) => PathOutcome::Changed,
                _ => PathOutcome::NotFingerprintable,
            }
        }
    }
}

/// The state a checkpoint was taken under (FR-424).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Assumptions {
    pub branch: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<Uuid>,
    /// The derived cross-device task state identity at that instant — not a
    /// counter, so it means the same thing on any machine (D80).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_state_digest: Option<String>,
    /// At most 32 repository-relative paths, each with one bounded fingerprint.
    #[serde(default)]
    pub path_fingerprints: Vec<PathFingerprint>,
}

/// What is true now, gathered by the caller from Git, the store and bounded
/// worktree reads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CurrentState {
    pub branch: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    /// False when the assumed task no longer exists.
    pub task_exists: bool,
    /// False when the worktree the checkpoint was taken in no longer exists.
    pub worktree_exists: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_state_digest: Option<String>,
    /// Recomputed for exactly the paths the checkpoint named, and no others.
    #[serde(default)]
    pub path_fingerprints: Vec<PathFingerprint>,
}

/// One class of difference, with both sides named so the report can state them
/// (FR-433).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Divergence {
    pub kind: DivergenceKind,
    pub recorded: String,
    pub current: String,
}

/// The per-path result, kept beside the divergence list because a path that
/// could not be fingerprinted is reported without being counted as a change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PathComparison {
    pub path: String,
    pub outcome: PathOutcome,
    pub recorded_class: FingerprintClass,
    pub current_class: FingerprintClass,
}

/// The whole comparison.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckpointClassification {
    pub state: CheckpointState,
    pub divergences: Vec<Divergence>,
    pub paths: Vec<PathComparison>,
}

impl CheckpointClassification {
    /// Whether the recorded next action may be emitted as the action to take.
    ///
    /// Only a `current` checkpoint may. Anything else emits
    /// `previous_next_action`, labelled, because presenting a stale
    /// instruction as live is the failure mode US6 #2 names (FR-434).
    pub fn next_action_is_live(&self) -> bool {
        self.state == CheckpointState::Current
    }

    pub fn has(&self, kind: DivergenceKind) -> bool {
        self.divergences.iter().any(|d| d.kind == kind)
    }

    /// Paths that could not be compared. Reported separately from changes, so
    /// "I could not look" never reads as either answer.
    pub fn not_fingerprintable(&self) -> Vec<&PathComparison> {
        self.paths
            .iter()
            .filter(|p| p.outcome == PathOutcome::NotFingerprintable)
            .collect()
    }
}

/// Classify a checkpoint against current state (FR-431).
///
/// ```text
/// current       no divergence
/// diverged      one or more divergences
/// unresolvable  the assumed task or the worktree no longer exists
/// ```
///
/// An `unresolvable` checkpoint still reports every divergence it *can*
/// compute, because the continuity fields that do not depend on the missing
/// state are still delivered (FR-435).
pub fn classify_checkpoint(
    assumed: &Assumptions,
    current: &CurrentState,
) -> CheckpointClassification {
    let mut divergences = Vec::new();

    if assumed.branch != current.branch {
        divergences.push(Divergence {
            kind: DivergenceKind::Branch,
            recorded: assumed.branch.clone(),
            current: current.branch.clone(),
        });
    }

    if assumed.commit != current.commit {
        divergences.push(Divergence {
            kind: DivergenceKind::Commit,
            recorded: assumed.commit.clone().unwrap_or_else(|| "none".into()),
            current: current.commit.clone().unwrap_or_else(|| "none".into()),
        });
    }

    // A task divergence is decided by the derived digest, never by a counter:
    // two machines can each advance a counter from 5 to 6 and mean entirely
    // different things (D80).
    if assumed.task_id.is_some()
        && current.task_exists
        && assumed.task_state_digest.is_some()
        && current.task_state_digest.is_some()
        && assumed.task_state_digest != current.task_state_digest
    {
        divergences.push(Divergence {
            kind: DivergenceKind::Task,
            recorded: assumed.task_state_digest.clone().unwrap_or_default(),
            current: current.task_state_digest.clone().unwrap_or_default(),
        });
    }

    // Bounded to the paths the checkpoint already names. No globbing, no
    // directory walk, no repository scan (FR-471's discipline preserved).
    let mut paths = Vec::new();
    let mut any_path_changed = false;
    for recorded in &assumed.path_fingerprints {
        let current_fp = current
            .path_fingerprints
            .iter()
            .find(|p| p.path == recorded.path);
        let (outcome, current_class) = match current_fp {
            Some(c) => (compare_path_fingerprint(recorded, c), c.class),
            // The caller did not recompute this path at all. That is not the
            // same as the path being gone, and it is not the same as unchanged.
            None => (PathOutcome::NotFingerprintable, FingerprintClass::Unknown),
        };
        if matches!(
            outcome,
            PathOutcome::Changed | PathOutcome::Removed | PathOutcome::Added
        ) {
            any_path_changed = true;
        }
        paths.push(PathComparison {
            path: recorded.path.clone(),
            outcome,
            recorded_class: recorded.class,
            current_class,
        });
    }
    if any_path_changed {
        let changed: Vec<&str> = paths
            .iter()
            .filter(|p| {
                matches!(
                    p.outcome,
                    PathOutcome::Changed | PathOutcome::Removed | PathOutcome::Added
                )
            })
            .map(|p| p.path.as_str())
            .collect();
        divergences.push(Divergence {
            kind: DivergenceKind::Files,
            recorded: format!("{} fingerprinted", assumed.path_fingerprints.len()),
            current: changed.join(", "),
        });
    }

    let unresolvable =
        !current.worktree_exists || (assumed.task_id.is_some() && !current.task_exists);

    let state = if unresolvable {
        CheckpointState::Unresolvable
    } else if divergences.is_empty() {
        CheckpointState::Current
    } else {
        CheckpointState::Diverged
    };

    CheckpointClassification {
        state,
        divergences,
        paths,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assumed() -> Assumptions {
        Assumptions {
            branch: "main".into(),
            commit: Some("abc123".into()),
            task_id: Some(Uuid::from_u128(1)),
            task_state_digest: Some("d0".into()),
            path_fingerprints: vec![PathFingerprint::digest("src/config.rs", "aaa")],
        }
    }

    fn current() -> CurrentState {
        CurrentState {
            branch: "main".into(),
            commit: Some("abc123".into()),
            task_exists: true,
            worktree_exists: true,
            task_state_digest: Some("d0".into()),
            path_fingerprints: vec![PathFingerprint::digest("src/config.rs", "aaa")],
        }
    }

    #[test]
    fn nothing_moved_is_current() {
        let c = classify_checkpoint(&assumed(), &current());
        assert_eq!(c.state, CheckpointState::Current);
        assert!(c.divergences.is_empty());
        assert!(c.next_action_is_live());
    }

    #[test]
    fn every_divergence_class_is_detected() {
        // SC-311: each class, alone.
        let mut branch = current();
        branch.branch = "feature/x".into();
        let c = classify_checkpoint(&assumed(), &branch);
        assert!(c.has(DivergenceKind::Branch));
        assert_eq!(c.state, CheckpointState::Diverged);

        let mut commit = current();
        commit.commit = Some("def456".into());
        assert!(classify_checkpoint(&assumed(), &commit).has(DivergenceKind::Commit));

        let mut task = current();
        task.task_state_digest = Some("d1".into());
        assert!(classify_checkpoint(&assumed(), &task).has(DivergenceKind::Task));

        let mut files = current();
        files.path_fingerprints = vec![PathFingerprint::digest("src/config.rs", "bbb")];
        assert!(classify_checkpoint(&assumed(), &files).has(DivergenceKind::Files));
    }

    #[test]
    fn divergence_classes_combine() {
        let mut c = current();
        c.branch = "feature/x".into();
        c.commit = Some("def456".into());
        c.task_state_digest = Some("d1".into());
        c.path_fingerprints = vec![PathFingerprint::digest("src/config.rs", "bbb")];
        let result = classify_checkpoint(&assumed(), &c);
        assert_eq!(result.divergences.len(), 4);
        for kind in [
            DivergenceKind::Branch,
            DivergenceKind::Commit,
            DivergenceKind::Task,
            DivergenceKind::Files,
        ] {
            assert!(result.has(kind), "{kind} missing");
        }
    }

    #[test]
    fn a_diverged_checkpoint_never_offers_a_live_next_action() {
        // FR-434, metric 16. Asserted for every class, because one class
        // slipping through is enough to hand an agent a stale instruction.
        for mutate in [
            (|c: &mut CurrentState| c.branch = "other".into()) as fn(&mut CurrentState),
            |c: &mut CurrentState| c.commit = Some("def456".into()),
            |c: &mut CurrentState| c.task_state_digest = Some("d1".into()),
            |c: &mut CurrentState| {
                c.path_fingerprints = vec![PathFingerprint::digest("src/config.rs", "bbb")]
            },
        ] {
            let mut cur = current();
            mutate(&mut cur);
            assert!(!classify_checkpoint(&assumed(), &cur).next_action_is_live());
        }
    }

    #[test]
    fn an_external_edit_is_detected_with_the_commit_unmoved() {
        // Metric 15a: a human editor, a formatter, `git apply` or an IDE
        // refactor. No session recorded anything and the commit did not move —
        // the recorded fingerprint is what notices.
        let mut cur = current();
        cur.path_fingerprints = vec![PathFingerprint::digest("src/config.rs", "edited-by-nobody")];
        let c = classify_checkpoint(&assumed(), &cur);
        assert_eq!(c.state, CheckpointState::Diverged);
        assert!(c.has(DivergenceKind::Files));
        assert_eq!(c.paths[0].outcome, PathOutcome::Changed);
    }

    #[test]
    fn a_path_that_cannot_be_fingerprinted_is_never_reported_unchanged() {
        // Metric 15b. Every way of failing to look must produce the same
        // honest answer.
        let recorded = PathFingerprint::digest("vendor/large.bin", "aaa");

        for current_fp in [
            PathFingerprint::unknown("vendor/large.bin"),
            PathFingerprint::size("vendor/large.bin", 4096),
        ] {
            assert_eq!(
                compare_path_fingerprint(&recorded, &current_fp),
                PathOutcome::NotFingerprintable
            );
        }

        // And the reverse: excluded when the checkpoint was taken.
        assert_eq!(
            compare_path_fingerprint(
                &PathFingerprint::unknown("secrets/prod.env"),
                &PathFingerprint::digest("secrets/prod.env", "aaa")
            ),
            PathOutcome::NotFingerprintable
        );
    }

    #[test]
    fn a_path_the_caller_did_not_recompute_is_not_unchanged() {
        // The quiet failure this guards: a restore that skipped a read and
        // reported the path as fine.
        let mut cur = current();
        cur.path_fingerprints.clear();
        let c = classify_checkpoint(&assumed(), &cur);
        assert_eq!(c.paths[0].outcome, PathOutcome::NotFingerprintable);
        assert_eq!(
            c.state,
            CheckpointState::Current,
            "not fingerprintable is reported, but it is not a change"
        );
        assert_eq!(c.not_fingerprintable().len(), 1);
    }

    #[test]
    fn removal_and_addition_are_told_apart_from_a_failed_read() {
        let present = PathFingerprint::digest("src/retry.rs", "aaa");
        let absent = PathFingerprint::absent("src/retry.rs");

        assert_eq!(
            compare_path_fingerprint(&present, &absent),
            PathOutcome::Removed
        );
        assert_eq!(
            compare_path_fingerprint(&absent, &present),
            PathOutcome::Added
        );
        assert_eq!(
            compare_path_fingerprint(&absent, &absent),
            PathOutcome::Unchanged,
            "a path that was not there and still is not has not moved"
        );
    }

    #[test]
    fn a_size_fingerprint_matches_only_on_equal_length() {
        let recorded = PathFingerprint::size("vendor/large.bin", 1024);
        assert_eq!(
            compare_path_fingerprint(&recorded, &PathFingerprint::size("vendor/large.bin", 1024)),
            PathOutcome::Unchanged,
            "weaker than a digest, and documented as such"
        );
        assert_eq!(
            compare_path_fingerprint(&recorded, &PathFingerprint::size("vendor/large.bin", 2048)),
            PathOutcome::Changed
        );
    }

    #[test]
    fn a_missing_task_is_unresolvable_and_still_reports_what_it_can() {
        let mut cur = current();
        cur.task_exists = false;
        cur.commit = Some("def456".into());
        let c = classify_checkpoint(&assumed(), &cur);
        assert_eq!(c.state, CheckpointState::Unresolvable);
        assert!(
            c.has(DivergenceKind::Commit),
            "the fields that do not depend on the missing state are still delivered"
        );
        assert!(!c.next_action_is_live());
    }

    #[test]
    fn a_missing_worktree_is_unresolvable() {
        let mut cur = current();
        cur.worktree_exists = false;
        assert_eq!(
            classify_checkpoint(&assumed(), &cur).state,
            CheckpointState::Unresolvable
        );
    }

    #[test]
    fn a_checkpoint_with_no_task_never_reports_a_task_divergence() {
        // Compaction with no bound task still carries repository state,
        // decisions, failures and a next action (spec Edge Cases).
        let mut a = assumed();
        a.task_id = None;
        a.task_state_digest = None;
        let mut cur = current();
        cur.task_exists = false;
        cur.task_state_digest = Some("d1".into());
        let c = classify_checkpoint(&a, &cur);
        assert!(!c.has(DivergenceKind::Task));
        assert_eq!(c.state, CheckpointState::Current);
    }

    #[test]
    fn a_session_bound_before_this_feature_reports_no_false_divergence() {
        // `task_snapshot_at_bind` is NULL for a session that bound before the
        // migration. Synthesizing one would produce a false divergence report,
        // so the absence means unknown and nothing is claimed (migration.md
        // §Step 4).
        let mut a = assumed();
        a.task_state_digest = None;
        let mut cur = current();
        cur.task_state_digest = Some("d1".into());
        let c = classify_checkpoint(&a, &cur);
        assert!(!c.has(DivergenceKind::Task));
    }
}
