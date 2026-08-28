//! The promotion gate: eight checks, fixed order, fail-closed
//! (D415, D416, revised by D433 and D446).
//!
//! Promotion turns a project memory into personal or team knowledge, and it is
//! the highest-risk path in this feature because it is the one that moves
//! something *out* of a project. The gate is a pure function: no database
//! handle, no clock, no network. Every input arrives by value, so the same
//! inputs always give the same answer and the whole thing is testable against a
//! seeded adversarial corpus with no store behind it.
//!
//! **Eight checks, not nine.** The original check 4, `project_identifying`, is
//! not dropped — it moved into [`crate::validate::validate_global_content`],
//! which runs at all five entry points rather than only on the promotion path
//! (D446). Check 1 below is where a promotion inherits it, and the gate must
//! **not** re-implement it: FR-579 keeps exactly one implementation of every
//! rejection class, because a second one is a second place for the two to
//! drift.
//!
//! Two checks never refuse. `verification_reset` (5) and `origin_computation`
//! (7) hold their positions anyway, because D416 numbers checks positionally
//! and the reported reason must stay stable across releases. `verification_reset`
//! in particular resets *nothing* — a personal or team record has no
//! verification field of any kind (D452, FR-513) — and it keeps its slot so the
//! absence is visible at the moment promotion happens rather than inferred from
//! a schema.

use crate::domain::{ApplicabilityKind, MemoryState, PromotionTarget};
use crate::validate::{validate_global_content, ProjectIdentity};

/// What a passing promotion yields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromotionApproval {
    /// Normalized applicability, as the validator accepted it.
    pub sanitized_applicability: Vec<(ApplicabilityKind, String)>,
    /// The salted, machine-local digest of the source project (FR-516).
    /// Never transmitted (FR-551).
    pub origin_digest: String,
}

/// Why a promotion was refused.
///
/// Neither field can hold offending text: `check` is the fixed name of the
/// check that stopped it, and `class` — set only for content failures — is one
/// of the validator's fixed class names. The type has no `String` field at all,
/// so a caller cannot log the rejected content through it even carelessly
/// (FR-507, FR-510, FR-520).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PromotionRejection {
    pub check: &'static str,
    pub class: Option<&'static str>,
}

impl PromotionRejection {
    fn at(check: &'static str) -> Self {
        Self { check, class: None }
    }
}

/// The eight checks, in the order they run. Named once so a test can assert the
/// order and the count rather than trusting a comment.
pub const CHECKS: &[&str] = &[
    "shared_content_validation",
    "source_not_active",
    "no_subject",
    "evidence_leak",
    "verification_reset",
    "not_a_member",
    "origin_computation",
    "evaluation_incomplete",
];

/// Evaluate a promotion (FR-506–FR-520).
///
/// `project_identities` is the source project's **whole** identity: its name and
/// every token its git remote contributes. Not the name alone — a project called
/// `internal-tooling` behind `git@host:acme/widgets.git` is named by "acme" and
/// by "widgets" just as surely, and a screen that only knew the name would let
/// both through. Direct creation has always passed the full set
/// (`current_project_identities`); this path passing a narrower one made the
/// promotion entry point weaker than the creation entry point for the same
/// content, which is precisely the asymmetry FR-545 exists to rule out.
///
/// It feeds check 1's delegation to the validator, and check 7's origin digest
/// needs `project_id`, which is why an **empty** set makes check 1 unanswerable
/// and the gate refuses with `evaluation_incomplete` rather than proceeding
/// (FR-518). Empty here does not mean what it means to the validator: a project
/// always has an identity, so a caller that supplied none did not establish that
/// there is none — it failed to look. The validator's documented fail-open for an
/// empty set (FR-580) covers a *creation made outside any project*, a situation
/// that cannot arise for a promotion, which by definition has a source project.
///
/// `machine_salt` is passed in rather than read, because reading it would be
/// I/O and this function is pure. The caller that has a filesystem does that.
#[allow(clippy::too_many_arguments)]
pub fn evaluate_promotion(
    content: &str,
    topic_key: Option<&str>,
    value_key: Option<&str>,
    proposed_applicability: &[(ApplicabilityKind, String)],
    project_identities: &[ProjectIdentity],
    project_id: Option<uuid::Uuid>,
    machine_salt: Option<&str>,
    target: PromotionTarget,
    promoter_is_project_member: bool,
    source_state: Option<MemoryState>,
) -> Result<PromotionApproval, PromotionRejection> {
    // Check 8's condition, evaluated first for the inputs it governs.
    //
    // Fail-closed means the gate refuses when it *cannot answer*, not when it
    // gets an inconvenient answer. An empty `project_identities` leaves check 1
    // unable to screen for the source project at all; an absent `project_id` or
    // salt leaves check 7 unable to compute a digest; an absent `source_state`
    // leaves check 2 unanswerable. Skipping an unanswerable check and proceeding
    // is the one behavior this gate must never have (FR-518).
    if project_identities.is_empty() {
        return Err(PromotionRejection::at("evaluation_incomplete"));
    }
    let (Some(project_id), Some(salt)) = (project_id, machine_salt) else {
        return Err(PromotionRejection::at("evaluation_incomplete"));
    };
    let Some(source_state) = source_state else {
        return Err(PromotionRejection::at("evaluation_incomplete"));
    };

    // 1. shared_content_validation — delegated, never re-implemented (FR-579).
    let facts: Vec<crate::domain::ApplicabilityFact> = proposed_applicability
        .iter()
        .map(|(kind, value)| crate::domain::ApplicabilityFact {
            kind: *kind,
            value: value.clone(),
        })
        .collect();
    if let Err(rejection) =
        validate_global_content(content, topic_key, value_key, &facts, project_identities)
    {
        return Err(PromotionRejection {
            check: "shared_content_validation",
            class: Some(rejection.class),
        });
    }

    // 2. source_not_active — a superseded memory is no longer this project's
    //    answer, so promoting it would export a claim the project has retracted.
    if source_state != MemoryState::Active {
        return Err(PromotionRejection::at("source_not_active"));
    }

    // 3. no_subject — without a subject key the record cannot participate in
    //    reconciliation at the far end, so it would arrive unreconcilable.
    if topic_key.map(str::trim).unwrap_or("").is_empty() {
        return Err(PromotionRejection::at("no_subject"));
    }

    // 4. evidence_leak — a *count* of supporting evidence may travel; the
    //    identifiers may not. An identifier is a handle into local-only
    //    observation storage, and it means nothing anywhere else while naming
    //    something on the machine that produced it.
    if carries_evidence_identifier(content) {
        return Err(PromotionRejection::at("evidence_leak"));
    }

    // 5. verification_reset — never refuses, and resets nothing.
    //
    //    There is no verification field on a personal or team record to reset
    //    (D452, FR-513, FR-517). The slot is held so that the absence is
    //    visible here, where a reader of the gate would otherwise wonder what
    //    became of the source's verification state. The answer is that it does
    //    not travel, in any form, including as a value.

    // 6. not_a_member — a team promotion by someone with no membership in the
    //    source project would let an administrator export knowledge from a
    //    project they were never part of.
    if target == PromotionTarget::Team && !promoter_is_project_member {
        return Err(PromotionRejection::at("not_a_member"));
    }

    // 7. origin_computation — never refuses.
    let origin_digest = crate::global::origin_digest(salt, project_id);

    // 8. evaluation_incomplete — its condition was evaluated above, before the
    //    checks that depend on the inputs it guards. Reaching here means every
    //    required input was present.

    Ok(PromotionApproval {
        sanitized_applicability: proposed_applicability
            .iter()
            .map(|(kind, value)| {
                (
                    *kind,
                    crate::applicability::normalize_applicability_value(value)
                        .unwrap_or_else(|_| value.clone()),
                )
            })
            .collect(),
        origin_digest,
    })
}

/// Does `content` name an evidence or observation identifier?
///
/// A bare count ("verified by 2 configuration checks") is fine and useful. What
/// must not travel is a handle — a UUID, or the hex prefix of one, presented as
/// a reference.
fn carries_evidence_identifier(content: &str) -> bool {
    let lowered = content.to_ascii_lowercase();
    let names_evidence = [
        "evidence",
        "observation",
        "obs id",
        "evidence_id",
        "observation_id",
    ]
    .iter()
    .any(|w| lowered.contains(w));
    if !names_evidence {
        return false;
    }
    // A hex run long enough to be an identifier prefix, next to the word.
    content
        .split(|c: char| !c.is_ascii_alphanumeric())
        .any(|t| {
            t.len() >= 8
                && t.chars().all(|c| c.is_ascii_hexdigit())
                && t.chars().any(|c| c.is_ascii_digit())
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    struct Case {
        content: String,
        topic_key: Option<String>,
        applicability: Vec<(ApplicabilityKind, String)>,
        /// The source project's whole identity set. Empty models the caller
        /// that could not establish one, which is unanswerable rather than
        /// vacuous — see [`evaluate_promotion`].
        identity: Vec<ProjectIdentity>,
        project_id: Option<Uuid>,
        salt: Option<String>,
        target: PromotionTarget,
        member: bool,
        state: Option<MemoryState>,
    }

    impl Case {
        /// A promotion that passes every check, so each test can break exactly
        /// one thing. A fixture that started invalid would prove nothing about
        /// which check fired.
        fn passing() -> Self {
            Self {
                content: "Clear the build cache when a stale artifact is suspected".into(),
                topic_key: Some("build.cache_dir".into()),
                applicability: vec![(ApplicabilityKind::Tool, "cargo".into())],
                identity: vec![ProjectIdentity("acme-widgets".into())],
                project_id: Some(Uuid::now_v7()),
                salt: Some("machine-salt".into()),
                target: PromotionTarget::Personal,
                member: true,
                state: Some(MemoryState::Active),
            }
        }

        fn run(&self) -> Result<PromotionApproval, PromotionRejection> {
            evaluate_promotion(
                &self.content,
                self.topic_key.as_deref(),
                None,
                &self.applicability,
                self.identity.as_slice(),
                self.project_id,
                self.salt.as_deref(),
                self.target,
                self.member,
                self.state,
            )
        }
    }

    #[test]
    fn the_baseline_case_passes_so_every_other_test_isolates_one_check() {
        let approved = Case::passing().run().expect("baseline should pass");
        assert!(!approved.origin_digest.is_empty());
        assert_eq!(
            approved.sanitized_applicability,
            vec![(ApplicabilityKind::Tool, "cargo".to_string())]
        );
    }

    #[test]
    fn the_check_order_is_fixed_and_eight_long() {
        assert_eq!(CHECKS.len(), 8);
        assert_eq!(CHECKS[0], "shared_content_validation");
        assert_eq!(CHECKS[7], "evaluation_incomplete");
        // The ninth check is gone, and gone by name: `project_identifying` is
        // the validator's class now, never a gate check (D446).
        assert!(!CHECKS.contains(&"project_identifying"));
    }

    #[test]
    fn check_1_delegates_content_validation_and_reports_the_class() {
        let mut case = Case::passing();
        case.content = "Scratch files live at /Users/alice/tmp".into();
        let err = case.run().unwrap_err();
        assert_eq!(err.check, "shared_content_validation");
        assert_eq!(err.class, Some("absolute_path"));
    }

    /// The gate inherits the project-name screen through check 1 rather than
    /// implementing it. If the gate re-implemented it, this would report a gate
    /// check name instead of the delegation.
    #[test]
    fn check_1_inherits_the_project_identity_screen_rather_than_owning_it() {
        let mut case = Case::passing();
        case.content = "The acme-widgets CI is slow".into();
        let err = case.run().unwrap_err();
        assert_eq!(err.check, "shared_content_validation");
        assert_eq!(err.class, Some("project_identifying"));
    }

    #[test]
    fn check_2_refuses_a_source_that_is_not_active() {
        for state in [MemoryState::Superseded, MemoryState::Stale] {
            let mut case = Case::passing();
            case.state = Some(state);
            assert_eq!(
                case.run().unwrap_err().check,
                "source_not_active",
                "{state}"
            );
        }
    }

    #[test]
    fn check_3_refuses_a_source_with_no_subject() {
        for missing in [None, Some(String::new()), Some("   ".into())] {
            let mut case = Case::passing();
            case.topic_key = missing.clone();
            assert_eq!(case.run().unwrap_err().check, "no_subject", "{missing:?}");
        }
    }

    #[test]
    fn check_4_refuses_an_evidence_identifier_but_admits_a_count() {
        let mut leaking = Case::passing();
        leaking.content = "See evidence 4f2a1c9e for the config value".into();
        assert_eq!(leaking.run().unwrap_err().check, "evidence_leak");

        let mut counting = Case::passing();
        counting.content = "Verified by 2 configuration checks".into();
        assert!(
            counting.run().is_ok(),
            "a bare count must be allowed to travel"
        );
    }

    /// Check 5 never refuses, and there is nothing for it to reset. Asserted by
    /// the baseline passing with a `PromotionApproval` that has no verification
    /// field to inspect — the absence is in the type (D452).
    #[test]
    fn check_5_never_refuses_and_the_approval_has_no_verification_field() {
        let approved = Case::passing().run().unwrap();
        let rendered = format!("{approved:?}");
        assert!(
            !rendered.to_ascii_lowercase().contains("verif"),
            "PromotionApproval gained a verification field: {rendered}"
        );
    }

    #[test]
    fn check_6_refuses_a_team_promotion_by_a_non_member_only() {
        let mut team_outsider = Case::passing();
        team_outsider.target = PromotionTarget::Team;
        team_outsider.member = false;
        assert_eq!(team_outsider.run().unwrap_err().check, "not_a_member");

        let mut team_member = Case::passing();
        team_member.target = PromotionTarget::Team;
        team_member.member = true;
        assert!(team_member.run().is_ok());

        // Personal promotion does not consult membership at all: there is no
        // project on the far side to be a member of.
        let mut personal_outsider = Case::passing();
        personal_outsider.member = false;
        assert!(personal_outsider.run().is_ok());
    }

    #[test]
    fn check_7_computes_a_digest_and_never_refuses() {
        let case = Case::passing();
        let first = case.run().unwrap().origin_digest;
        let second = case.run().unwrap().origin_digest;
        assert_eq!(first, second, "the same inputs must give the same digest");
    }

    /// FR-518 — a check that cannot be evaluated refuses. One case per missing
    /// input, so a partial implementation fails on the one it forgot.
    #[test]
    fn check_8_refuses_when_a_required_input_is_missing() {
        let mut no_identity = Case::passing();
        no_identity.identity = Vec::new();
        assert_eq!(
            no_identity.run().unwrap_err().check,
            "evaluation_incomplete"
        );

        let mut no_project = Case::passing();
        no_project.project_id = None;
        assert_eq!(no_project.run().unwrap_err().check, "evaluation_incomplete");

        let mut no_salt = Case::passing();
        no_salt.salt = None;
        assert_eq!(no_salt.run().unwrap_err().check, "evaluation_incomplete");

        let mut no_state = Case::passing();
        no_state.state = None;
        assert_eq!(no_state.run().unwrap_err().check, "evaluation_incomplete");
    }

    /// The first failing check is the one reported, and later checks do not run.
    /// A gate that collected all failures would make the reported reason depend
    /// on how many things were wrong.
    #[test]
    fn the_first_failing_check_is_the_one_reported() {
        let mut everything_wrong = Case::passing();
        everything_wrong.content = "/etc/passwd is the file".into(); // check 1
        everything_wrong.state = Some(MemoryState::Superseded); // check 2
        everything_wrong.topic_key = None; // check 3
                                           // Check 8's inputs are all present, so evaluation reaches check 1 first.
        assert_eq!(
            everything_wrong.run().unwrap_err().check,
            "shared_content_validation"
        );
    }

    /// FR-514 — an applicability value outside the closed vocabulary's format is
    /// refused on promotion, not silently dropped. The rejection arrives through
    /// check 1, because the validator owns that check too.
    #[test]
    fn an_unrepresentable_applicability_value_is_refused_on_promotion() {
        let mut case = Case::passing();
        case.applicability = vec![(ApplicabilityKind::Tool, "not a value".into())];
        let err = case.run().unwrap_err();
        assert_eq!(err.check, "shared_content_validation");
        assert_eq!(err.class, Some(crate::validate::INVALID_APPLICABILITY));
    }

    /// The rejection type has nowhere to put offending text, asserted across
    /// every check that can fire (FR-510).
    #[test]
    fn a_rejection_never_carries_offending_text() {
        let secret = "/Users/alice/very-secret-path";
        let mut case = Case::passing();
        case.content = format!("{secret} is where it lives");
        let err = case.run().unwrap_err();
        let rendered = format!("{err:?}");
        assert!(!rendered.contains("alice"), "{rendered}");
        assert!(!rendered.contains("secret"), "{rendered}");
    }
}
