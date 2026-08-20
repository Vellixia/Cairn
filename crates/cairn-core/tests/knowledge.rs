//! Tier 2 — the deterministic corpus (`contracts/evaluation.md` §Tiers).
//!
//! `cargo test -p cairn-core --test knowledge`
//!
//! No database, no daemon, no repository. Everything here is a pure function
//! over JSON fixtures in `tests/knowledge/`, which is why this tier runs in
//! milliseconds and why most of Feature 003's correctness lives in it.
//!
//! Suites are added as their derivations land. This file starts by proving the
//! loader itself: every fixture in the tree parses, and every directory the
//! corpus contract names exists.

use cairn_core::corpus::{self, Case, MemoryCase};
use cairn_core::knowledge::{
    classify_proposal, content_norm_digest, derive_subject, normalize_topic_key,
    normalize_value_key, MemoryFacts, ProposalOutcome, Relation,
};
use cairn_core::{
    Importance, MemoryScope, MemoryState, RelationBasis, RelationKind, VerificationAuthority,
    VerificationState,
};
use std::str::FromStr;
use uuid::Uuid;

/// A deterministic identifier for a session label.
///
/// Distinct-origin accounting counts distinct sessions, so a fixture needs
/// distinct labels to become distinct identifiers — and the same label in two
/// cases must not accidentally collide with a member id.
fn session_id(label: &str) -> Uuid {
    let d = cairn_core::digest(&format!("corpus-session:{label}"));
    let bytes: [u8; 16] = hex_prefix(&d);
    Uuid::from_bytes(bytes)
}

fn hex_prefix(hex: &str) -> [u8; 16] {
    let mut out = [0u8; 16];
    for (i, slot) in out.iter_mut().enumerate() {
        *slot = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).unwrap_or(0);
    }
    out
}

/// Turn a fixture's memory into the facts the derivation reads.
///
/// Keys are normalized here because that is what storage does before the
/// derivation ever sees them: a fixture writes what an agent would write, not
/// what the column holds.
fn facts(case: &Case, m: &MemoryCase) -> MemoryFacts {
    MemoryFacts {
        id: case.id(&m.label),
        state: MemoryState::from_str(&m.state)
            .unwrap_or_else(|e| panic!("{}", case.context(e.to_string()))),
        scope: MemoryScope::from_str(&m.scope)
            .unwrap_or_else(|e| panic!("{}", case.context(e.to_string()))),
        scope_key: m.scope_key.clone(),
        topic_key: m.topic_key.as_deref().and_then(normalize_topic_key),
        value_key: m.value_key.as_deref().and_then(normalize_value_key),
        content_norm_digest: (!m.content.is_empty()).then(|| content_norm_digest(&m.content)),
        verification: VerificationState::from_str(&m.verification)
            .unwrap_or_else(|e| panic!("{}", case.context(e.to_string()))),
        verification_authority: m
            .verification_authority
            .as_deref()
            .map(|a| VerificationAuthority::from_str(a).expect("authority")),
        evidence_fact_count: m.evidence_fact_count,
        pinned: m.pinned,
        importance: Importance::from_str(&m.importance).expect("importance"),
        origin_session_id: session_id(&m.origin_session),
    }
}

fn relations(case: &Case) -> Vec<Relation> {
    case.input
        .relations
        .iter()
        .map(|r| {
            Relation::new(
                RelationKind::from_str(&r.kind)
                    .unwrap_or_else(|e| panic!("{}", case.context(e.to_string()))),
                case.id(&r.from),
                case.id(&r.to),
                RelationBasis::from_str(&r.basis).expect("basis"),
            )
        })
        .collect()
}

fn labels(case: &Case, ids: &[Uuid]) -> Vec<String> {
    ids.iter().map(|id| case.label(*id)).collect()
}

/// Every directory `contracts/evaluation.md` §The corpus names.
const REQUIRED_GROUPS: &[&str] = &[
    "reconciliation/equivalent",
    "reconciliation/distinct",
    "reconciliation/coarse_value_key",
    "reconciliation/duplicate_content",
    "reconciliation/free_form",
    "conflict/real",
    "conflict/scope_exception",
    "conflict/disjoint",
    "supersession",
    "merge/symmetric_relation",
    "merge/task_divergence",
    "merge/blocked_recovery",
    "verification/authority",
    "drift",
    "budget/oversized_task",
    "continuity",
    "staleness/external_edit",
    "patterns/promote",
    "patterns/refuse",
    "patterns/independence",
    "patterns/counterexample",
    "privacy",
    "tasks",
];

#[test]
fn every_contract_named_group_exists() {
    let root = corpus::root();
    for group in REQUIRED_GROUPS {
        let dir = root.join(group);
        assert!(
            dir.is_dir(),
            "the corpus contract names {group}, which does not exist at {}",
            dir.display()
        );
        assert!(
            dir.join("README.md").is_file(),
            "{group} has no README.md stating the rule its cases follow"
        );
    }
}

#[test]
fn every_fixture_in_the_tree_parses() {
    // A parse failure names its file — that is the loader's contract, and it
    // is what makes a red corpus run point at a case rather than at a loop.
    let cases = corpus::load_all(&corpus::root()).expect("every corpus fixture parses");
    for case in &cases {
        assert!(
            !case.group.is_empty(),
            "{}: case has no group",
            case.path.display()
        );
    }
}

/// The seeded privacy corpus has to stay adversarial.
///
/// Its values are deliberately synthetic — an upstream secret scanner blocks a
/// push carrying anything that looks genuine, and a corpus nobody can commit is
/// no corpus at all. Synthetic is not the same as *toothless*: every value
/// seeded as a secret must still be something `redact.rs` recognizes, or the
/// case would pass the promotion gate for the wrong reason and SC-315 would be
/// measuring nothing.
#[test]
fn every_seeded_secret_is_still_recognizable_as_one() {
    let cases = corpus::load_group(&corpus::root(), "privacy").expect("privacy corpus loads");
    assert!(!cases.is_empty(), "the privacy corpus is empty");

    let mut secrets = 0;
    for case in &cases {
        let refusals = &case.expect.refusals;
        if !refusals.iter().any(|r| r == "possible_secret") {
            continue;
        }
        secrets += 1;
        let seeded = case.input.extra["seeded_value"]
            .as_str()
            .unwrap_or_else(|| panic!("{}", case.context("no seeded_value")));

        let redacted = cairn_core::redact::redact(seeded);
        assert!(
            redacted != seeded || cairn_core::redact::contains_secret(seeded),
            "{}",
            case.context(format!(
                "the seeded value is not recognized by the redaction pattern set, \
                 so this case would pass gate check 7 for the wrong reason \
                 (class {})",
                case.input.extra["seeded_class"]
            ))
        );
    }
    assert!(
        secrets >= 15,
        "only {secrets} secret-class cases; the corpus contract names more"
    );
}

/// The complement: a case seeded as project-identifying must **not** be caught
/// by redaction, or it would be refused by check 7 and never exercise check 8.
#[test]
fn project_identifying_seeds_are_not_secret_shaped() {
    let cases = corpus::load_group(&corpus::root(), "privacy").expect("privacy corpus loads");
    for case in &cases {
        if !case
            .expect
            .refusals
            .iter()
            .any(|r| r == "project_identifying")
        {
            continue;
        }
        let seeded = case.input.extra["seeded_value"]
            .as_str()
            .unwrap_or_else(|| panic!("{}", case.context("no seeded_value")));
        assert_eq!(
            cairn_core::redact::redact(seeded),
            seeded,
            "{}",
            case.context(
                "this value is secret-shaped, so the gate's fixed order would report \
                 possible_secret and check 8 would never run"
            )
        );
    }
}

#[test]
fn every_group_loads_independently() {
    let root = corpus::root();
    for group in REQUIRED_GROUPS {
        corpus::load_group(&root, group)
            .unwrap_or_else(|e| panic!("group {group} failed to load: {e}"));
    }
}

// ---------------------------------------------------------------------------
// The reconciliation corpus (SC-301, SC-302, SC-305)
// ---------------------------------------------------------------------------

/// Directories whose cases are derivation cases.
const DERIVATION_GROUPS: &[&str] = &[
    "reconciliation/equivalent",
    "reconciliation/distinct",
    "reconciliation/coarse_value_key",
    "reconciliation/duplicate_content",
    "reconciliation/free_form",
    "conflict/real",
    "conflict/scope_exception",
    "conflict/disjoint",
    "supersession",
];

fn derivation_cases() -> Vec<Case> {
    let root = corpus::root();
    let mut all = Vec::new();
    for group in DERIVATION_GROUPS {
        all.extend(
            corpus::load_group(&root, group)
                .unwrap_or_else(|e| panic!("group {group} failed to load: {e}")),
        );
    }
    all
}

/// Every derivation case produces exactly the `SubjectView` it states.
#[test]
fn the_corpus_derives_what_it_says() {
    let cases = derivation_cases();
    assert!(!cases.is_empty(), "the derivation corpus is empty");

    for case in &cases {
        if case.expect.reconciliation.is_none() && case.expect.answers.is_empty() {
            continue;
        }
        let members: Vec<MemoryFacts> =
            case.input.memories.iter().map(|m| facts(case, m)).collect();
        let view = derive_subject(&members, &relations(case));

        if let Some(expected_state) = case.expect.reconciliation.as_deref() {
            assert_eq!(
                view.reconciliation.as_str(),
                expected_state,
                "{}",
                case.context(format!(
                    "expected {expected_state}, derived {} with answers {:?}",
                    view.reconciliation,
                    labels(case, &view.answers)
                ))
            );
        }

        if !case.expect.answers.is_empty() {
            assert_eq!(
                labels(case, &view.answers),
                case.expect.answers,
                "{}",
                case.context("answers differ")
            );
        }
        if !case.expect.narrowed_by.is_empty() {
            assert_eq!(
                labels(case, &view.narrowed_by),
                case.expect.narrowed_by,
                "{}",
                case.context("narrowed_by differs")
            );
        }
    }
}

/// Metric 2 — zero false merges across the paired negative corpus.
///
/// A `distinct/`, `free_form/` or `coarse_value_key/` case that produced one
/// answer for two materially different statements would suppress a claim, which
/// is the failure this corpus exists to make measurable.
#[test]
fn no_case_that_must_not_merge_ever_merges() {
    let root = corpus::root();
    let mut false_merges = Vec::new();

    for group in [
        "reconciliation/distinct",
        "reconciliation/free_form",
        "reconciliation/coarse_value_key",
    ] {
        for case in corpus::load_group(&root, group).expect("group loads") {
            let members: Vec<MemoryFacts> = case
                .input
                .memories
                .iter()
                .map(|m| facts(&case, m))
                .collect();
            let active = members
                .iter()
                .filter(|m| m.state == MemoryState::Active)
                .count();
            let view = derive_subject(&members, &relations(&case));

            if active > 1 && view.answers.len() < active {
                // Some collapse is legitimate — two members of a `distinct/`
                // case may genuinely duplicate a third. The corpus states the
                // expected count, so a shortfall against *that* is the defect.
                if view.answers.len() != case.expect.answers.len() {
                    false_merges.push(format!(
                        "{}: {active} active members collapsed to {} answers",
                        case.path.display(),
                        view.answers.len()
                    ));
                }
            }
        }
    }
    assert!(false_merges.is_empty(), "false merges: {false_merges:#?}");
}

/// Metric 2a — no automatic path writes a `reinforces` relation, ever.
///
/// `reinforces` was demoted to explicit-only when the coarse-value-key
/// false-merge path was closed. Nothing but an explicit act may record one.
#[test]
fn no_automatic_reinforcement() {
    for case in derivation_cases() {
        let members: Vec<MemoryFacts> = case
            .input
            .memories
            .iter()
            .map(|m| facts(&case, m))
            .collect();
        for (i, proposal) in members.iter().enumerate() {
            let existing: Vec<MemoryFacts> = members
                .iter()
                .enumerate()
                .filter(|(j, _)| *j != i)
                .map(|(_, m)| m.clone())
                .collect();
            let (_, recorded) = classify_proposal(proposal, &existing, 64);
            for r in &recorded {
                assert_ne!(
                    r.kind,
                    RelationKind::Reinforces,
                    "{}",
                    case.context("an automatic path recorded a reinforcement")
                );
            }
        }
    }
}

/// Metric 2b — every `coarse_value_key/` case corroborates, records nothing,
/// and retains every statement.
#[test]
fn corroboration() {
    let cases = corpus::load_group(&corpus::root(), "reconciliation/coarse_value_key")
        .expect("the coarse-value-key corpus loads");
    assert!(
        cases.len() >= 15,
        "the corpus contract asks for at least 15 adversarial cases, found {}",
        cases.len()
    );

    for case in &cases {
        let members: Vec<MemoryFacts> =
            case.input.memories.iter().map(|m| facts(case, m)).collect();
        let view = derive_subject(&members, &relations(case));

        assert_eq!(
            view.reconciliation.as_str(),
            "corroborated",
            "{}",
            case.context("a shared value key with differing content must corroborate")
        );
        assert_eq!(
            view.answers.len(),
            members.len(),
            "{}",
            case.context("a statement was dropped from the answer set")
        );

        // And the write path records nothing at all for it.
        let (outcome, recorded) = classify_proposal(
            members.last().expect("members"),
            &members[..members.len() - 1],
            64,
        );
        assert!(
            matches!(outcome, ProposalOutcome::Corroborating { .. }),
            "{}",
            case.context(format!("expected corroboration, got {outcome:?}"))
        );
        assert!(
            recorded.is_empty(),
            "{}",
            case.context("corroboration recorded a relation")
        );
    }
}

/// Metric 3 and 4 — every real conflict is visible, and neither negative
/// directory ever produces one.
#[test]
fn conflicts_are_real_and_the_negatives_are_not() {
    let root = corpus::root();

    let real = corpus::load_group(&root, "conflict/real").expect("loads");
    assert!(real.len() >= 15, "conflict/real has {} cases", real.len());
    for case in &real {
        let members: Vec<MemoryFacts> =
            case.input.memories.iter().map(|m| facts(case, m)).collect();
        let view = derive_subject(&members, &relations(case));
        assert_eq!(
            view.reconciliation.as_str(),
            "conflicted",
            "{}",
            case.context("a real conflict did not surface")
        );
        assert!(
            view.answers.len() >= 2,
            "{}",
            case.context("a conflict emitted a single answer — a silent winner")
        );
        // Every member stays active. Nothing is marked superseded to resolve it.
        assert!(
            members.iter().all(|m| m.state == MemoryState::Active),
            "{}",
            case.context("a conflict case seeded a non-active member")
        );
    }

    for group in ["conflict/scope_exception", "conflict/disjoint"] {
        let cases = corpus::load_group(&root, group).expect("loads");
        assert!(cases.len() >= 10, "{group} has {} cases", cases.len());
        for case in &cases {
            let members: Vec<MemoryFacts> =
                case.input.memories.iter().map(|m| facts(case, m)).collect();
            // The two are not members of one subject at all: the scope key
            // differs, so a single working context never selects both. Asserted
            // through the write path, which is where a false conflict would be
            // recorded.
            let (outcome, recorded) = classify_proposal(
                members.last().expect("members"),
                &members[..members.len() - 1],
                64,
            );
            assert!(
                !matches!(outcome, ProposalOutcome::ConflictDetected { .. }),
                "{}",
                case.context(format!("false conflict: {outcome:?}"))
            );
            assert!(
                recorded.is_empty(),
                "{}",
                case.context("a relation was recorded between memories that never interact")
            );
        }
    }
}

/// The corpus meets the sizes the contract states.
///
/// A corpus that quietly shrank would make every metric above pass while
/// measuring less, which is the failure mode a count catches and an assertion
/// does not.
#[test]
fn the_corpus_is_at_least_the_size_the_contract_asks_for() {
    let root = corpus::root();
    for (group, minimum) in [
        ("reconciliation/equivalent", 20),
        ("reconciliation/distinct", 20),
        ("reconciliation/coarse_value_key", 15),
        ("reconciliation/duplicate_content", 20),
        ("reconciliation/free_form", 20),
        ("conflict/real", 15),
        ("conflict/scope_exception", 10),
        ("conflict/disjoint", 10),
        ("supersession", 5),
    ] {
        let count = corpus::load_group(&root, group).expect("loads").len();
        assert!(
            count >= minimum,
            "{group}: {count} cases, contract asks for at least {minimum}"
        );
    }
}

// ---------------------------------------------------------------------------
// Verification authority (T044, SC-329)
// ---------------------------------------------------------------------------

/// Every authority case derives what it says, through the real
/// `derive_authority`.
#[test]
fn the_authority_corpus_derives_what_it_says() {
    use cairn_core::verify::{derive_authority, RunFacts};
    use cairn_core::{EvidenceCollector, VerifierKind, VerifyResult};

    let cases = corpus::load_group(&corpus::root(), "verification/authority")
        .expect("the authority corpus loads");
    assert!(cases.len() >= 12, "{} cases", cases.len());

    let mut exercised = 0usize;
    for case in &cases {
        let Some(runs) = case.input.extra.get("runs").and_then(|r| r.as_array()) else {
            continue;
        };
        // An imported case has no local runs, and the wire case asserts a
        // different property. Both are checked in their own tests.
        if case.expect.extra.contains_key("imported_from")
            || case.expect.extra.contains_key("on_the_wire")
        {
            continue;
        }
        exercised += 1;

        let facts: Vec<RunFacts> = runs
            .iter()
            .map(|r| RunFacts {
                verifier: VerifierKind::from_str(r["verifier"].as_str().unwrap_or("file_digest"))
                    .expect("verifier"),
                result: VerifyResult::from_str(r["result"].as_str().expect("result"))
                    .expect("result"),
                evidence_collector: r["collector"]
                    .as_str()
                    .map(|c| EvidenceCollector::from_str(c).expect("collector")),
            })
            .collect();

        let state = VerificationState::from_str(
            case.expect
                .extra
                .get("state")
                .and_then(|s| s.as_str())
                .unwrap_or("verified"),
        )
        .expect("state");

        let expected = case
            .expect
            .extra
            .get("authority")
            .and_then(|a| a.as_str())
            .map(|a| VerificationAuthority::from_str(a).expect("authority"));

        assert_eq!(
            derive_authority(state, &facts),
            expected,
            "{}",
            case.context("the derived authority differs")
        );
    }
    assert!(exercised >= 10, "only {exercised} local cases exercised");
}

/// Only a deterministic check this machine ran satisfies the two strict
/// consumers — a criterion's verification and cross-project promotion.
#[test]
fn the_authority_corpus_agrees_about_the_strict_consumers() {
    use cairn_core::verify::{deterministic_refusal_code, satisfies_deterministic_requirement};

    for case in corpus::load_group(&corpus::root(), "verification/authority").expect("loads") {
        let Some(expected) = case
            .expect
            .extra
            .get("satisfies_deterministic_requirement")
            .and_then(|v| v.as_bool())
        else {
            continue;
        };
        let authority = case
            .expect
            .extra
            .get("authority")
            .and_then(|a| a.as_str())
            .map(|a| VerificationAuthority::from_str(a).expect("authority"));

        assert_eq!(
            satisfies_deterministic_requirement(authority),
            expected,
            "{}",
            case.context("a strict consumer disagreed with the corpus")
        );

        if let Some(refusal) = case.expect.extra.get("refusal").and_then(|r| r.as_str()) {
            assert_eq!(
                deterministic_refusal_code(authority),
                Some(refusal),
                "{}",
                case.context("the refusal code differs")
            );
        }
    }
}

/// The wire carries only the two local values, and an import maps them back.
#[test]
fn an_authority_never_crosses_a_boundary_wearing_the_wrong_badge() {
    let case = corpus::load_group(&corpus::root(), "verification/authority")
        .expect("loads")
        .into_iter()
        .find(|c| c.expect.extra.contains_key("on_the_wire"))
        .expect("the wire case");

    for (sent, expected) in case.expect.extra["on_the_wire"]
        .as_object()
        .expect("object")
    {
        let a = VerificationAuthority::from_str(sent).expect("authority");
        assert_eq!(
            a.on_the_wire().as_str(),
            expected.as_str().expect("value"),
            "{}",
            case.context(format!("{sent} went onto the wire wrong"))
        );
    }

    for (received, expected) in case.expect.extra["on_import"].as_object().expect("object") {
        let a = VerificationAuthority::from_str(received).expect("authority");
        assert_eq!(
            VerificationAuthority::imported(a).as_str(),
            expected.as_str().expect("value"),
            "{}",
            case.context(format!("{received} was imported wrong"))
        );
    }
}

// ---------------------------------------------------------------------------
// Drift (T060, SC-307)
// ---------------------------------------------------------------------------

/// Every drift case names a `(state, trigger)` pair and the state it produces —
/// or `null` where the contract documents **no** transition at all.
///
/// The exhaustive proof that the machine is total lives in `cairn-core::verify`
/// and enumerates the whole product. This corpus is the readable form: each
/// case says, in words, why the transition is what it is.
#[test]
fn the_drift_corpus_matches_the_state_machine() {
    use cairn_core::verify::{transition, VerificationTrigger};
    use cairn_core::VerifyResult;

    let cases = corpus::load_group(&corpus::root(), "drift").expect("the drift corpus loads");
    assert!(cases.len() >= 10, "{} cases", cases.len());

    for case in &cases {
        let from = VerificationState::from_str(case.input.extra["from"].as_str().expect("from"))
            .expect("state");

        let trigger = match case.input.extra["trigger"].as_str().expect("trigger") {
            "fingerprint_changed" => VerificationTrigger::FingerprintChanged,
            "run_verified" => VerificationTrigger::Run(VerifyResult::Verified),
            "run_drifted" => VerificationTrigger::Run(VerifyResult::Drifted),
            "run_inconclusive" => VerificationTrigger::Run(VerifyResult::Inconclusive),
            "last_supporting_evidence_deleted" => {
                VerificationTrigger::LastSupportingEvidenceDeleted
            }
            "contradicting_evidence_attached" => VerificationTrigger::ContradictingEvidenceAttached,
            "contradicting_evidence_removed" => VerificationTrigger::ContradictingEvidenceRemoved,
            "superseded" => VerificationTrigger::Superseded,
            "marked_stale" => VerificationTrigger::MarkedStale,
            "imported" => VerificationTrigger::Imported,
            other => panic!("{}", case.context(format!("unknown trigger {other}"))),
        };

        let expected = case.expect.extra["to"]
            .as_str()
            .map(|t| VerificationState::from_str(t).expect("state"));

        assert_eq!(
            transition(from, trigger),
            expected,
            "{}",
            case.context("the transition differs from what the case states")
        );
    }
}

/// Every task case names a set of criteria and blockers, and the progress,
/// readiness and action order the contract derives from them.
///
/// Three families in one group: the criterion **state × verification** matrix,
/// derived **readiness**, and the **action order** Level 0 admits. What the
/// matrix exists to hold down is that the two axes never collapse —
/// `satisfied` + `unverified` is counted separately from `verified`, because it
/// is the honest description of "the agent says it is done and nothing has
/// checked" (FR-482, FR-483, FR-486, FR-487).
#[test]
fn the_tasks_corpus_matches_the_derivations() {
    use cairn_core::tasks::{
        action_order, completion_readiness, progress, BlockerFacts, CriterionFacts,
    };
    use cairn_core::{BlockerState, CompletionReadiness, CriterionState, CriterionVerification};

    let cases = corpus::load_group(&corpus::root(), "tasks").expect("the tasks corpus loads");
    assert!(cases.len() >= 20, "{} cases", cases.len());

    for case in &cases {
        let criteria: Vec<CriterionFacts> = case.input.extra["criteria"]
            .as_array()
            .unwrap_or_else(|| panic!("{}", case.context("input.extra.criteria must be an array")))
            .iter()
            .map(|c| {
                let ordinal = c["ordinal"].as_i64().expect("ordinal");
                CriterionFacts {
                    // Ordinals are distinct within a case, so a tie never falls
                    // through to the id and the expected order is fully
                    // determined by the contract rather than by this mapping.
                    id: Uuid::from_u128(ordinal as u128),
                    ordinal,
                    text: c["text"].as_str().expect("text").to_string(),
                    state: CriterionState::from_str(c["state"].as_str().expect("state"))
                        .expect("criterion state"),
                    verification: CriterionVerification::from_str(
                        c["verification"].as_str().expect("verification"),
                    )
                    .expect("criterion verification"),
                    deleted: c["deleted"].as_bool().unwrap_or(false),
                }
            })
            .collect();

        let blockers: Vec<BlockerFacts> = case.input.extra["blockers"]
            .as_array()
            .unwrap_or_else(|| panic!("{}", case.context("input.extra.blockers must be an array")))
            .iter()
            .enumerate()
            .map(|(i, b)| BlockerFacts {
                id: Uuid::from_u128(i as u128 + 1),
                state: BlockerState::from_str(b["state"].as_str().expect("state"))
                    .expect("blocker state"),
                deleted: b["deleted"].as_bool().unwrap_or(false),
            })
            .collect();

        // Progress — counts by state, and never a percentage. Compared field by
        // field through the serialized form so a new bucket cannot be added
        // without a case naming it.
        let expected_progress = &case.expect.extra["progress"];
        let actual = serde_json::to_value(progress(&criteria)).expect("progress serializes");
        assert_eq!(
            &actual,
            expected_progress,
            "{}",
            case.context("derived progress differs from what the case states")
        );

        // Readiness, where the case states one. `null` means the case is about
        // progress or ordering and does not constrain readiness.
        if let Some(want) = case.expect.extra["readiness"].as_str() {
            let want = CompletionReadiness::from_str(want).expect("readiness");
            assert_eq!(
                completion_readiness(&criteria, &blockers),
                want,
                "{}",
                case.context("derived readiness differs from what the case states")
            );
        }

        // The action order Level 0 consumes, named by label so the expectation
        // reads the way the briefing does.
        if let Some(want) = case.expect.extra["action_order"].as_array() {
            let want: Vec<String> = want
                .iter()
                .map(|v| v.as_str().expect("label").to_string())
                .collect();
            let got: Vec<String> = action_order(&criteria)
                .into_iter()
                .map(|c| cairn_core::tasks::criterion_label(c.ordinal))
                .collect();
            assert_eq!(
                got,
                want,
                "{}",
                case.context("the action order differs from what the case states")
            );
        }
    }
}

/// The corpus covers the whole criterion `state × verification` product.
///
/// Twelve combinations, all twelve present. An absence assertion written the
/// only way that holds: enumerate the product from the enums themselves, so a
/// value added to either axis fails this test until a case names it.
#[test]
fn the_tasks_corpus_covers_the_whole_matrix() {
    use cairn_core::{CriterionState, CriterionVerification};

    let cases = corpus::load_group(&corpus::root(), "tasks").expect("the tasks corpus loads");
    let seen: std::collections::BTreeSet<(String, String)> = cases
        .iter()
        .filter(|c| c.name.contains("matrix_"))
        .filter_map(|c| {
            let first = c.input.extra["criteria"].as_array()?.first()?;
            Some((
                first["state"].as_str()?.to_string(),
                first["verification"].as_str()?.to_string(),
            ))
        })
        .collect();

    for state in CriterionState::ALL {
        for verification in CriterionVerification::ALL {
            let pair = (
                state.as_str().to_string(),
                verification.as_str().to_string(),
            );
            assert!(
                seen.contains(&pair),
                "the corpus names no case for {} + {}; the matrix must be exhaustive",
                pair.0,
                pair.1
            );
        }
    }
}

/// Every staleness case classifies as the contract says, and a diverged
/// checkpoint never presents its recorded action as the one to take
/// (FR-431, FR-432, FR-434, SC-311).
#[test]
fn the_staleness_corpus_classifies_as_it_says() {
    use cairn_core::continuity::{classify_checkpoint, Assumptions, CurrentState};
    use cairn_core::DivergenceKind;

    let root = corpus::root();
    let mut cases = corpus::load_group(&root, "staleness").expect("the staleness corpus loads");
    cases
        .extend(corpus::load_group(&root, "staleness/external_edit").expect("external edits load"));
    assert!(cases.len() >= 18, "{} cases", cases.len());

    for case in &cases {
        let assumed: Assumptions =
            serde_json::from_value(case.input.extra["assumed"].clone()).expect("assumptions");
        let current: CurrentState =
            serde_json::from_value(case.input.extra["current"].clone()).expect("current state");
        let got = classify_checkpoint(&assumed, &current);

        if let Some(want) = case.expect.extra.get("state").and_then(|v| v.as_str()) {
            assert_eq!(
                got.state.as_str(),
                want,
                "{}",
                case.context("the classification differs from what the case states")
            );
        }

        if let Some(want) = case
            .expect
            .extra
            .get("divergences")
            .and_then(|v| v.as_array())
        {
            for kind in want {
                let kind = DivergenceKind::from_str(kind.as_str().expect("a kind"))
                    .expect("divergence kind");
                assert!(
                    got.has(kind),
                    "{}",
                    case.context(format!("{} was not detected", kind.as_str()))
                );
            }
            assert_eq!(
                got.divergences.len(),
                want.len(),
                "{}",
                case.context("a divergence was reported that the case does not name")
            );
        }

        // The whole point of the phase: a stale action is never the live one.
        if let Some(live) = case
            .expect
            .extra
            .get("next_action_is_live")
            .and_then(|v| v.as_bool())
        {
            assert_eq!(
                got.next_action_is_live(),
                live,
                "{}",
                case.context(
                    "a diverged checkpoint must emit previous_next_action, never next_action"
                )
            );
        }

        if let Some(outcomes) = case
            .expect
            .extra
            .get("path_outcomes")
            .and_then(|v| v.as_object())
        {
            for (path, want) in outcomes {
                let found = got
                    .paths
                    .iter()
                    .find(|p| &p.path == path)
                    .unwrap_or_else(|| panic!("{}", case.context(format!("no result for {path}"))));
                assert_eq!(
                    found.outcome.as_str(),
                    want.as_str().expect("an outcome"),
                    "{}",
                    case.context(format!("{path} compared differently"))
                );
            }
        }
    }
}
