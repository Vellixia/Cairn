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

use cairn_core::corpus;

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
