//! The verification state machine, authority derivation and fingerprints
//! (`contracts/evidence-verification.md`).
//!
//! Pure. Nothing here reads a file, resolves a ref or runs anything — the
//! verifiers that do live in `cairnd`, which is the only crate with worktree
//! and Git access. This module decides *what a result means*, which is the part
//! that has to be total, documented and testable without a repository.
//!
//! # The one thing this module exists to prevent
//!
//! A model's opinion is never verification (FR-361), and an agent's own
//! attestation must never become indistinguishable from a check Cairn performed
//! (FR-370). The state says what was established; the **authority** says what
//! established it. They are separate dimensions here, in storage, on the wire
//! and on every surface, because collapsing them anywhere is enough to lose the
//! distinction everywhere.

use crate::domain::{
    EvidenceCollector, VerificationAuthority, VerificationState, VerifierKind, VerifyResult,
};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// The state machine (FR-375, SC-306)
// ---------------------------------------------------------------------------

/// Everything that may move a memory's verification state.
///
/// The set is closed, which is what makes "every undocumented transition is
/// unreachable" a property rather than a review note: a trigger that does not
/// appear here cannot be applied, and a `(state, trigger)` pair the contract
/// does not document returns `None`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationTrigger {
    /// A verification run completed with this result.
    Run(VerifyResult),
    /// A supporting evidence fact's fingerprint differs from the recorded one.
    FingerprintChanged,
    /// An evidence fact was attached with role `contradicts`, or two runs of one
    /// verifier disagreed at the same commit.
    ContradictingEvidenceAttached,
    /// The contradiction was removed.
    ContradictingEvidenceRemoved,
    /// The last fact supporting this memory was deleted. A tombstone clears the
    /// fingerprint, so this is a fingerprint change with a name — kept distinct
    /// because the contract documents its behaviour from `needs_recheck`
    /// separately (`contracts/evidence-verification.md` §Deletion).
    LastSupportingEvidenceDeleted,
    /// The memory was superseded. **Not** a verification transition: a
    /// superseded memory keeps its last verification, which is what lets a
    /// historical query say what was verified then (D50).
    Superseded,
    /// The memory's scope key stopped resolving. Orthogonal to verification.
    MarkedStale,
    /// A peer's state arrived. The local state is unchanged; only the authority
    /// is rewritten, by [`VerificationAuthority::imported`].
    Imported,
}

/// Apply a trigger to a verification state.
///
/// Returns the resulting state for a **documented** transition — including the
/// rows where the state is unchanged, because "a run was inconclusive and the
/// claim stays where it was" is a real outcome and not the absence of one — and
/// `None` where the contract documents no transition at all.
///
/// The distinction matters for SC-306: `Some(same)` is a reachable documented
/// transition, `None` is an unreachable undocumented one.
pub fn transition(
    from: VerificationState,
    trigger: VerificationTrigger,
) -> Option<VerificationState> {
    use VerificationState::*;
    use VerificationTrigger::*;
    use VerifyResult as R;

    match (from, trigger) {
        // A claim nothing has checked.
        (Unverified, Run(R::Verified)) => Some(Verified),
        (Unverified, Run(R::Drifted)) => Some(Drifted),
        (Unverified, Run(R::Inconclusive)) => Some(Unverified),

        // Established, until its support moves.
        (Verified, FingerprintChanged) => Some(NeedsRecheck),
        (Verified, ContradictingEvidenceAttached) => Some(Conflicted),
        // §Deletion: the supported memory becomes `needs_recheck`, never stays
        // `verified`. A tombstone clears the fingerprint, so this is the same
        // mechanism as a fingerprint change.
        (Verified, LastSupportingEvidenceDeleted) => Some(NeedsRecheck),

        // Owed a check.
        (NeedsRecheck, Run(R::Verified)) => Some(Verified),
        (NeedsRecheck, Run(R::Drifted)) => Some(Drifted),
        (NeedsRecheck, Run(R::Inconclusive)) => Some(NeedsRecheck),
        (NeedsRecheck, LastSupportingEvidenceDeleted) => Some(NeedsRecheck),
        (NeedsRecheck, FingerprintChanged) => Some(NeedsRecheck),

        // The support moved and the claim no longer matches it. Cleared only by
        // evidence — never by assertion, and never by time.
        (Drifted, FingerprintChanged) => Some(NeedsRecheck),
        (Drifted, Run(R::Verified)) => Some(Verified),
        (Drifted, LastSupportingEvidenceDeleted) => Some(NeedsRecheck),

        // This memory's own evidence disagrees with itself.
        (Conflicted, FingerprintChanged) => Some(NeedsRecheck),
        (Conflicted, ContradictingEvidenceRemoved) => Some(NeedsRecheck),
        (Conflicted, LastSupportingEvidenceDeleted) => Some(NeedsRecheck),

        // Deliberately not transitions. Named so that adding one has to be a
        // decision rather than an oversight.
        (_, Superseded) => None,
        (_, MarkedStale) => None,
        (_, Imported) => None,

        _ => None,
    }
}

/// Every documented transition, for the exhaustiveness test and for
/// documentation generation.
///
/// SC-306 asks for two things: every documented transition reachable, and every
/// undocumented one unreachable. Both are checked against this table, so the
/// table and the function cannot drift apart without a failure.
pub fn documented_transitions() -> Vec<(VerificationState, VerificationTrigger, VerificationState)>
{
    use VerificationState::*;
    use VerificationTrigger::*;
    use VerifyResult as R;
    vec![
        (Unverified, Run(R::Verified), Verified),
        (Unverified, Run(R::Drifted), Drifted),
        (Unverified, Run(R::Inconclusive), Unverified),
        (Verified, FingerprintChanged, NeedsRecheck),
        (Verified, ContradictingEvidenceAttached, Conflicted),
        (Verified, LastSupportingEvidenceDeleted, NeedsRecheck),
        (NeedsRecheck, Run(R::Verified), Verified),
        (NeedsRecheck, Run(R::Drifted), Drifted),
        (NeedsRecheck, Run(R::Inconclusive), NeedsRecheck),
        (NeedsRecheck, LastSupportingEvidenceDeleted, NeedsRecheck),
        (NeedsRecheck, FingerprintChanged, NeedsRecheck),
        (Drifted, FingerprintChanged, NeedsRecheck),
        (Drifted, Run(R::Verified), Verified),
        (Drifted, LastSupportingEvidenceDeleted, NeedsRecheck),
        (Conflicted, FingerprintChanged, NeedsRecheck),
        (Conflicted, ContradictingEvidenceRemoved, NeedsRecheck),
        (Conflicted, LastSupportingEvidenceDeleted, NeedsRecheck),
    ]
}

/// Every trigger, for exhaustive enumeration.
pub fn all_triggers() -> Vec<VerificationTrigger> {
    use VerificationTrigger::*;
    let mut out = vec![
        FingerprintChanged,
        ContradictingEvidenceAttached,
        ContradictingEvidenceRemoved,
        LastSupportingEvidenceDeleted,
        Superseded,
        MarkedStale,
        Imported,
    ];
    for r in VerifyResult::ALL {
        out.push(Run(*r));
    }
    out
}

// ---------------------------------------------------------------------------
// Authority (FR-370, D76)
// ---------------------------------------------------------------------------

/// What one verification run consulted, as authority derivation sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunFacts {
    pub verifier: VerifierKind,
    pub result: VerifyResult,
    /// Who observed the evidence the run checked against. `None` when the run
    /// consulted no evidence fact — which can never establish anything.
    pub evidence_collector: Option<EvidenceCollector>,
}

/// Derive a memory's verification authority from the runs that established its
/// state.
///
/// ```text
/// cairn     ≥1 run with result `verified` consulted `collector = cairn` evidence
/// attested  the memory is verified and every establishing run consulted only
///           agent-attested evidence
/// —         the memory is not verified; authority is meaningless
/// ```
///
/// **The strongest basis wins.** A memory supported by both an attested fact and
/// a Cairn-read file digest, verified by the digest, has authority `cairn`: a
/// deterministic check did establish the claim, and saying otherwise would
/// understate what Cairn knows. The attested fact stays attached and stays
/// labelled.
///
/// The reverse cannot happen. Attaching an attested fact to a
/// Cairn-established verification does not degrade it, because the
/// deterministic run still stands.
///
/// Fails closed: a memory reported `verified` with no successful run has an
/// inconsistent cache, and `None` is the honest answer rather than a guess
/// (FR-478, FR-518).
pub fn derive_authority(
    state: VerificationState,
    runs: &[RunFacts],
) -> Option<VerificationAuthority> {
    if state != VerificationState::Verified {
        return None;
    }
    let successful: Vec<&RunFacts> = runs
        .iter()
        .filter(|r| r.result == VerifyResult::Verified)
        .collect();
    if successful.is_empty() {
        return None;
    }
    if successful
        .iter()
        .any(|r| r.evidence_collector == Some(EvidenceCollector::Cairn))
    {
        return Some(VerificationAuthority::Cairn);
    }
    if successful
        .iter()
        .any(|r| r.evidence_collector == Some(EvidenceCollector::Agent))
    {
        return Some(VerificationAuthority::Attested);
    }
    // A successful run that consulted no evidence establishes nothing.
    None
}

/// Whether an authority satisfies a consumer that requires a deterministic
/// check Cairn ran on this machine.
///
/// The two consumers with an incentive attached — a task criterion's
/// verification (FR-484) and cross-project promotion (FR-396) — both ask this
/// one question, so both ask it in one place.
pub fn satisfies_deterministic_requirement(authority: Option<VerificationAuthority>) -> bool {
    matches!(authority, Some(a) if a.is_local_deterministic())
}

/// Why a strict consumer refused, as a stable wire code.
pub fn deterministic_refusal_code(
    authority: Option<VerificationAuthority>,
) -> Option<&'static str> {
    match authority {
        Some(VerificationAuthority::Cairn) => None,
        Some(VerificationAuthority::Attested) => Some("attested_not_sufficient"),
        Some(VerificationAuthority::RemoteCairn) | Some(VerificationAuthority::RemoteAttested) => {
            Some("imported_not_sufficient")
        }
        None => Some("source_unverified"),
    }
}

// ---------------------------------------------------------------------------
// Fingerprints (`contracts/evidence-verification.md` §The verifier catalog)
// ---------------------------------------------------------------------------

/// What a verifier observed, in the shape its fingerprint is built from.
///
/// Constructing the fingerprint here rather than at each call site is what
/// keeps the recorded and the recomputed forms identical — a mismatch in
/// formatting would read as drift for every claim at once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Observed {
    /// `file_exists`
    FileExistence { exists: bool, size: u64 },
    /// `file_digest`, `configuration`, `schema_version`, `runtime_state` — a
    /// SHA-256 over the value.
    ValueDigest(String),
    /// `git_ref`, `git_commit` — a resolved object id.
    ObjectId(String),
    /// `test_outcome`
    TestOutcome {
        outcome: String,
        exit_code: i64,
        commit: Option<String>,
    },
    /// `command_outcome`
    CommandOutcome {
        exit_code: i64,
        commit: Option<String>,
    },
}

/// Render the fingerprint a verifier kind compares.
///
/// Returns `None` where the observation does not match the verifier — a
/// mismatch is a programming error at the call site, and producing a
/// well-formed but meaningless fingerprint would hide it behind spurious drift.
pub fn fingerprint(kind: VerifierKind, observed: &Observed) -> Option<String> {
    use VerifierKind as V;
    Some(match (kind, observed) {
        (V::FileExists, Observed::FileExistence { exists, size }) => {
            format!("exists:{}:{}", u8::from(*exists), size)
        }
        (V::FileDigest, Observed::ValueDigest(d))
        | (V::Configuration, Observed::ValueDigest(d))
        | (V::SchemaVersion, Observed::ValueDigest(d))
        | (V::RuntimeState, Observed::ValueDigest(d)) => d.clone(),
        (V::GitRef, Observed::ObjectId(id)) | (V::GitCommit, Observed::ObjectId(id)) => id.clone(),
        (
            V::TestOutcome,
            Observed::TestOutcome {
                outcome,
                exit_code,
                commit,
            },
        ) => format!(
            "{outcome}:{exit_code}:{}",
            commit.as_deref().unwrap_or("none")
        ),
        (V::CommandOutcome, Observed::CommandOutcome { exit_code, commit }) => {
            format!("{exit_code}:{}", commit.as_deref().unwrap_or("none"))
        }
        _ => return None,
    })
}

/// Whether a recorded fingerprint and a freshly computed one differ.
///
/// Exact equality, deliberately. There is no tolerance, no prefix comparison
/// and no similarity: a fingerprint is equal or it is not, which is what keeps
/// drift detection deterministic (FR-326's discipline applied to evidence).
pub fn fingerprint_changed(recorded: &str, observed: &str) -> bool {
    recorded != observed
}

/// Which collector a verifier's evidence must have come from.
///
/// `test_outcome` and `command_outcome` are reachable **both** ways — Cairn
/// reads a captured observation, or an agent submits the outcome — which is
/// exactly why `basis` alone could never carry authority and why the authority
/// is derived from the evidence's collector instead (D76, R11).
pub fn collector_for(kind: VerifierKind) -> Option<EvidenceCollector> {
    use VerifierKind as V;
    match kind {
        V::FileExists
        | V::FileDigest
        | V::GitRef
        | V::GitCommit
        | V::Configuration
        | V::SchemaVersion => Some(EvidenceCollector::Cairn),
        V::RuntimeState => Some(EvidenceCollector::Agent),
        V::TestOutcome | V::CommandOutcome => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn every_documented_transition_is_reachable() {
        for (from, trigger, to) in documented_transitions() {
            assert_eq!(
                transition(from, trigger),
                Some(to),
                "documented transition {from} --{trigger:?}--> {to} is not produced"
            );
        }
    }

    #[test]
    fn every_undocumented_transition_is_unreachable() {
        // SC-306's other half. Enumerate the whole product of states and
        // triggers; anything the contract does not document must return None.
        let documented: HashSet<(String, String)> = documented_transitions()
            .into_iter()
            .map(|(f, t, _)| (f.to_string(), format!("{t:?}")))
            .collect();

        for from in VerificationState::ALL {
            for trigger in all_triggers() {
                let key = (from.to_string(), format!("{trigger:?}"));
                if documented.contains(&key) {
                    continue;
                }
                assert_eq!(
                    transition(*from, trigger),
                    None,
                    "undocumented transition from {from} on {trigger:?} is reachable"
                );
            }
        }
    }

    #[test]
    fn supersession_and_staleness_change_no_verification_state() {
        // A superseded memory keeps its last verification, which is what lets a
        // historical query say what was verified then (D50). Staleness is scope,
        // and scope is orthogonal to evidence.
        for state in VerificationState::ALL {
            assert_eq!(transition(*state, VerificationTrigger::Superseded), None);
            assert_eq!(transition(*state, VerificationTrigger::MarkedStale), None);
        }
    }

    #[test]
    fn drift_is_never_cleared_without_a_run() {
        // The failure this guards: a `drifted` claim quietly becoming `verified`
        // because something other than evidence said so.
        use VerificationTrigger::*;
        for trigger in [
            FingerprintChanged,
            ContradictingEvidenceAttached,
            ContradictingEvidenceRemoved,
            LastSupportingEvidenceDeleted,
            Superseded,
            MarkedStale,
            Imported,
        ] {
            assert_ne!(
                transition(VerificationState::Drifted, trigger),
                Some(VerificationState::Verified),
                "{trigger:?} cleared drift without a run"
            );
        }
        assert_eq!(
            transition(
                VerificationState::Drifted,
                VerificationTrigger::Run(VerifyResult::Verified)
            ),
            Some(VerificationState::Verified),
            "a run is the one thing that clears it"
        );
    }

    #[test]
    fn an_inconclusive_run_establishes_nothing() {
        // FR-366: the memory becomes neither verified nor drifted.
        assert_eq!(
            transition(
                VerificationState::Unverified,
                VerificationTrigger::Run(VerifyResult::Inconclusive)
            ),
            Some(VerificationState::Unverified)
        );
        assert_eq!(
            transition(
                VerificationState::NeedsRecheck,
                VerificationTrigger::Run(VerifyResult::Inconclusive)
            ),
            Some(VerificationState::NeedsRecheck)
        );
        assert_eq!(
            transition(
                VerificationState::Verified,
                VerificationTrigger::Run(VerifyResult::Inconclusive)
            ),
            None,
            "an established claim is not disturbed by a check that could not look"
        );
    }

    #[test]
    fn importing_never_moves_the_local_state() {
        // FR-368: a peer's verification is reported as established elsewhere,
        // and it does not overwrite what this machine knows.
        for state in VerificationState::ALL {
            assert_eq!(transition(*state, VerificationTrigger::Imported), None);
        }
    }

    // -- authority ---------------------------------------------------------

    fn run(result: VerifyResult, collector: Option<EvidenceCollector>) -> RunFacts {
        RunFacts {
            verifier: VerifierKind::FileDigest,
            result,
            evidence_collector: collector,
        }
    }

    #[test]
    fn a_cairn_collected_check_establishes_cairn_authority() {
        assert_eq!(
            derive_authority(
                VerificationState::Verified,
                &[run(VerifyResult::Verified, Some(EvidenceCollector::Cairn))]
            ),
            Some(VerificationAuthority::Cairn)
        );
    }

    #[test]
    fn attested_evidence_establishes_attested_authority() {
        assert_eq!(
            derive_authority(
                VerificationState::Verified,
                &[run(VerifyResult::Verified, Some(EvidenceCollector::Agent))]
            ),
            Some(VerificationAuthority::Attested)
        );
    }

    #[test]
    fn the_deterministic_basis_wins_when_both_exist() {
        // Metric 25c. Order must not matter, so both orderings are asserted.
        let cairn = run(VerifyResult::Verified, Some(EvidenceCollector::Cairn));
        let agent = run(VerifyResult::Verified, Some(EvidenceCollector::Agent));
        assert_eq!(
            derive_authority(VerificationState::Verified, &[agent, cairn]),
            Some(VerificationAuthority::Cairn)
        );
        assert_eq!(
            derive_authority(VerificationState::Verified, &[cairn, agent]),
            Some(VerificationAuthority::Cairn)
        );
    }

    #[test]
    fn a_failed_run_never_establishes_authority() {
        for result in [VerifyResult::Drifted, VerifyResult::Inconclusive] {
            assert_eq!(
                derive_authority(
                    VerificationState::Verified,
                    &[run(result, Some(EvidenceCollector::Cairn))]
                ),
                None,
                "{result} established an authority"
            );
        }
    }

    #[test]
    fn authority_is_meaningless_unless_verified() {
        for state in VerificationState::ALL {
            if *state == VerificationState::Verified {
                continue;
            }
            assert_eq!(
                derive_authority(
                    *state,
                    &[run(VerifyResult::Verified, Some(EvidenceCollector::Cairn))]
                ),
                None,
                "{state} carried an authority"
            );
        }
    }

    #[test]
    fn a_verified_state_with_no_successful_run_fails_closed() {
        // FR-478/FR-518: a derived value that disagrees with its inputs is
        // reported unavailable, never guessed.
        assert_eq!(derive_authority(VerificationState::Verified, &[]), None);
        assert_eq!(
            derive_authority(
                VerificationState::Verified,
                &[run(VerifyResult::Verified, None)]
            ),
            None,
            "a run that consulted no evidence establishes nothing"
        );
    }

    #[test]
    fn only_a_local_deterministic_check_satisfies_the_strict_consumers() {
        // SC-328: a criterion never reaches verified on attested evidence
        // alone, nor on an imported verification of any authority.
        assert!(satisfies_deterministic_requirement(Some(
            VerificationAuthority::Cairn
        )));
        for weaker in [
            VerificationAuthority::Attested,
            VerificationAuthority::RemoteCairn,
            VerificationAuthority::RemoteAttested,
        ] {
            assert!(
                !satisfies_deterministic_requirement(Some(weaker)),
                "{weaker}"
            );
        }
        assert!(!satisfies_deterministic_requirement(None));

        assert_eq!(
            deterministic_refusal_code(Some(VerificationAuthority::Attested)),
            Some("attested_not_sufficient")
        );
        assert_eq!(
            deterministic_refusal_code(Some(VerificationAuthority::RemoteCairn)),
            Some("imported_not_sufficient")
        );
        assert_eq!(
            deterministic_refusal_code(Some(VerificationAuthority::RemoteAttested)),
            Some("imported_not_sufficient")
        );
        assert_eq!(deterministic_refusal_code(None), Some("source_unverified"));
        assert_eq!(
            deterministic_refusal_code(Some(VerificationAuthority::Cairn)),
            None
        );
    }

    // -- fingerprints -------------------------------------------------------

    #[test]
    fn fingerprints_match_the_verifier_catalog() {
        use VerifierKind as V;
        assert_eq!(
            fingerprint(
                V::FileExists,
                &Observed::FileExistence {
                    exists: true,
                    size: 42
                }
            )
            .as_deref(),
            Some("exists:1:42")
        );
        assert_eq!(
            fingerprint(
                V::FileExists,
                &Observed::FileExistence {
                    exists: false,
                    size: 0
                }
            )
            .as_deref(),
            Some("exists:0:0")
        );
        assert_eq!(
            fingerprint(V::FileDigest, &Observed::ValueDigest("9f2e".into())).as_deref(),
            Some("9f2e")
        );
        assert_eq!(
            fingerprint(V::GitRef, &Observed::ObjectId("abc123".into())).as_deref(),
            Some("abc123")
        );
        assert_eq!(
            fingerprint(
                V::TestOutcome,
                &Observed::TestOutcome {
                    outcome: "passed".into(),
                    exit_code: 0,
                    commit: Some("abc123".into())
                }
            )
            .as_deref(),
            Some("passed:0:abc123")
        );
        assert_eq!(
            fingerprint(
                V::CommandOutcome,
                &Observed::CommandOutcome {
                    exit_code: 1,
                    commit: None
                }
            )
            .as_deref(),
            Some("1:none")
        );
    }

    #[test]
    fn a_verifier_and_observation_mismatch_is_reported_not_papered_over() {
        assert_eq!(
            fingerprint(
                VerifierKind::FileDigest,
                &Observed::FileExistence {
                    exists: true,
                    size: 1
                }
            ),
            None
        );
    }

    #[test]
    fn fingerprint_comparison_is_exact() {
        assert!(!fingerprint_changed("9f2e", "9f2e"));
        assert!(fingerprint_changed("9f2e", "9f2f"));
        assert!(
            fingerprint_changed("passed:0:abc123", "passed:0:def456"),
            "the same outcome at a different commit is a different fact"
        );
    }

    #[test]
    fn outcome_verifiers_are_reachable_from_either_collector() {
        // The precise reason `basis` could not carry authority (R11).
        assert_eq!(collector_for(VerifierKind::TestOutcome), None);
        assert_eq!(collector_for(VerifierKind::CommandOutcome), None);
        assert_eq!(
            collector_for(VerifierKind::FileDigest),
            Some(EvidenceCollector::Cairn)
        );
        assert_eq!(
            collector_for(VerifierKind::RuntimeState),
            Some(EvidenceCollector::Agent)
        );
    }
}
