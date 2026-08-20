//! US10 — minimum-safe context, pins and explainability
//! (`contracts/continuity-context.md` Part 2).
//!
//! The guarantee this slice makes is deliberately *finite*. Guaranteeing that
//! every criterion, every blocker description and every warning detail fits a
//! budget would be a promise Cairn cannot keep, because none of them is bounded.
//! So Level 0 splits in two: **Tier 0a** is O(1) in the size of the project and
//! the task and is guaranteed; **Tier 0b** is bounded detail that fills the
//! remaining reserve and reports what it dropped.
//!
//! The three properties below are what make that real rather than aspirational:
//! the budget is never exceeded, a project with no Level 0 content is unchanged
//! from before the feature, and no quantity of low-priority memory can displace
//! the guaranteed state.

use cairn_core::budget::estimate;
use cairn_core::context::{assemble, ContextInputs, Level0};
use cairn_core::corpus;
use cairn_core::domain::*;
use cairn_core::tasks::{BlockerFacts, CriterionFacts};
use cairn_core::wire::{ContextWarning, PinnedConstraint};
use cairn_e2e::{baseline, Sandbox};
use serde_json::{json, Value};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Fixtures — plain data, so the property runs in milliseconds and the assembler
// is exercised as the pure function it is.
// ---------------------------------------------------------------------------

fn project() -> Project {
    Project {
        id: Uuid::nil(),
        name: "budget-fixture".into(),
        git_common_dir: "/fixture/.git".into(),
        repository_remote: None,
        linked: false,
        server_project_id: None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        deleted_at: None,
    }
}

fn repository() -> RepositoryState {
    RepositoryState {
        branch: "main".into(),
        commit_sha: Some("0123456789abcdef0123456789abcdef01234567".into()),
        staged: 0,
        unstaged: 1,
        untracked: 0,
    }
}

/// `n` memories of realistic length.
fn memories(n: usize) -> Vec<String> {
    (0..n)
        .map(|i| format!("Memory {i}: a recorded fact about this project's configuration."))
        .collect()
}

fn task_with(criteria: usize, text_tokens: usize) -> Task {
    // `text_tokens` at ~3.5 characters per estimated token.
    let word = "criterion ";
    let body: String = word.repeat((text_tokens * 7 / 2 / word.len()).max(1));
    Task {
        id: Uuid::nil(),
        project_id: Uuid::nil(),
        title: "Add rate limiting".into(),
        goal: "Requests over the configured limit are rejected with 429".into(),
        acceptance_criteria: (0..criteria)
            .map(|i| format!("AC-{}: {body}", i + 1))
            .collect(),
        status: TaskStatus::InProgress,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        deleted_at: None,
    }
}

/// One assembly described by a corpus case.
///
/// Kept as a single constructor so the Level 0 inputs are threaded in exactly
/// one place as the tiers land.
fn assemble_case(case_input: &serde_json::Map<String, Value>) -> Value {
    let n = |k: &str| case_input.get(k).and_then(Value::as_u64).unwrap_or(0) as usize;
    let has_task = case_input
        .get("task")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let p = project();
    let mem = memories(n("memories"));
    let task = task_with(n("criteria"), n("criterion_text_tokens").max(6));
    let criteria = criteria_facts(&task);
    let blockers = blocker_facts(n("blockers"));
    let blocker_text: Vec<(Uuid, String)> = blockers
        .iter()
        .map(|b| (b.id, "staging credentials expired".to_string()))
        .collect();
    let warnings = warning_fixtures(n("warnings"));
    let pins = pin_fixtures(n("pins"));

    let input = ContextInputs {
        project: &p,
        repository: repository(),
        task: has_task.then_some(&task),
        previous_handoff: None,
        decisions: &[],
        known_failures: &[],
        task_memory: &[],
        branch_memory: &[],
        project_memory: &mem,
        patterns: &[],
        has_history: n("memories") > 0,
        degraded: false,
        level0: Level0 {
            criteria: if has_task { &criteria } else { &[] },
            blockers: &blockers,
            blocker_text: &blocker_text,
            warnings: &warnings,
            pins: &pins,
            ..Default::default()
        },
    };

    serde_json::to_value(assemble(&input, n("budget"))).expect("the payload serializes")
}

/// Criterion facts matching a task's criteria, in a spread of states so action
/// order and the progress buckets both have something to say.
fn criteria_facts(task: &Task) -> Vec<CriterionFacts> {
    task.acceptance_criteria
        .iter()
        .enumerate()
        .map(|(i, text)| CriterionFacts {
            id: Uuid::from_u128(i as u128 + 1),
            ordinal: i as i64 + 1,
            text: text.clone(),
            state: match i % 4 {
                0 => CriterionState::Blocked,
                1 => CriterionState::Satisfied,
                2 => CriterionState::Pending,
                _ => CriterionState::Satisfied,
            },
            verification: if i % 4 == 3 {
                CriterionVerification::Verified
            } else {
                CriterionVerification::Unverified
            },
            deleted: false,
        })
        .collect()
}

fn blocker_facts(n: usize) -> Vec<BlockerFacts> {
    (0..n)
        .map(|i| BlockerFacts {
            id: Uuid::from_u128(1000 + i as u128),
            state: BlockerState::Open,
            deleted: false,
        })
        .collect()
}

fn warning_fixtures(n: usize) -> Vec<ContextWarning> {
    let kinds = ["conflict", "drift", "task_divergence", "task"];
    (0..n)
        .map(|i| ContextWarning {
            kind: kinds[i % kinds.len()].to_string(),
            subject: format!("subject.{i}"),
            detail: "remembered 8080, config/app.yml says 9000".into(),
        })
        .collect()
}

fn pin_fixtures(n: usize) -> Vec<PinnedConstraint> {
    (0..n)
        .map(|i| PinnedConstraint {
            id: Uuid::from_u128(2000 + i as u128),
            text: "never mutate CC Switch's private database directly".into(),
            drifted: false,
        })
        .collect()
}

// ---------------------------------------------------------------------------
// T077 — the property, and the promise that nothing regressed
// ---------------------------------------------------------------------------

/// `estimated_tokens <= budget` in **100%** of assemblies across the whole
/// matrix (FR-445, SC-308, I16).
///
/// This is a property of the admission loop, not a statistic: every section is
/// measured before it is emitted, so there is no path on which the output can
/// exceed the budget. A single violation anywhere in the matrix falsifies that.
#[test]
fn budget() {
    let cases = corpus::load_group(&corpus::root(), "budget").expect("the budget corpus loads");
    assert!(
        cases.len() >= 30,
        "the matrix is too small to be a property: {} cases",
        cases.len()
    );

    for case in &cases {
        let payload = assemble_case(&case.input.extra);
        let budget = payload["budget"].as_u64().expect("a budget is reported");
        let spent = payload["estimated_tokens"]
            .as_u64()
            .expect("a cost is reported");

        assert!(
            spent <= budget,
            "{}",
            case.context(format!(
                "the assembly spent {spent} against a budget of {budget}"
            ))
        );

        // A briefing is never rejected for size, however small the budget.
        assert!(
            payload["briefing"].is_object(),
            "{}",
            case.context("no briefing was produced at all")
        );
    }
}

/// With no task, no warnings, no pins and no checkpoint, the briefing is exactly
/// what Feature 001 produced (FR-442, SC-308).
///
/// This is the assertion that makes the reserve honest. A reserve that Level 0
/// held whether or not it had anything to put in it would silently shrink every
/// existing project's briefing. Unspent reserve returns to the general pool, so
/// a project with nothing for Level 0 is untouched — and "untouched" is checked
/// against the recorded pre-feature baseline rather than against a description
/// of it.
#[test]
fn no_regression() {
    let s = Sandbox::new();
    let out = s.cairn(&["context", "--json"]);
    assert!(out.ok(), "cairn context failed: {}", out.stderr);
    let full: Value = serde_json::from_str(&out.stdout).expect("context --json is JSON");
    let data = full.get("data").cloned().unwrap_or(full);

    let subset = json!({
        "briefing": data.get("briefing").cloned().unwrap_or(Value::Null),
        "estimated_tokens": data.get("estimated_tokens").cloned().unwrap_or(Value::Null),
        "truncated": data.get("truncated").cloned().unwrap_or(Value::Null),
        "omitted_sections": data.get("omitted_sections").cloned().unwrap_or(Value::Null),
    });

    let now = baseline::normalize(&subset);
    let before = baseline::load("briefing.json");

    assert_eq!(
        now, before,
        "a project with no Level 0 content must be byte-identical to the pre-feature \
         baseline.\n  now:    {now}\n  before: {before}"
    );
}

// ---------------------------------------------------------------------------
// T078 — the negative that makes the reserve real
// ---------------------------------------------------------------------------

/// An unbounded number of low-priority memories can never displace the
/// guaranteed work state, the applicable pin, or the drift and conflict
/// warnings (FR-442, FR-443, SC-309).
///
/// Asserted against the serialized payload rather than typed fields, because
/// what matters is what actually reaches the agent.
#[test]
fn critical_content_survives() {
    let p = project();
    let mem = memories(5000);
    let task = task_with(40, 12);
    let criteria = criteria_facts(&task);
    let blockers = blocker_facts(2);
    let blocker_text: Vec<(Uuid, String)> = blockers
        .iter()
        .map(|b| (b.id, "staging credentials expired".to_string()))
        .collect();
    let warnings = warning_fixtures(2);
    let pins = pin_fixtures(1);

    let input = ContextInputs {
        project: &p,
        repository: repository(),
        task: Some(&task),
        previous_handoff: None,
        decisions: &[],
        known_failures: &[],
        task_memory: &[],
        branch_memory: &[],
        project_memory: &mem,
        patterns: &[],
        has_history: true,
        degraded: false,
        level0: Level0 {
            criteria: &criteria,
            blockers: &blockers,
            blocker_text: &blocker_text,
            warnings: &warnings,
            pins: &pins,
            ..Default::default()
        },
    };

    // The documented minimum. Tier 0a's worst case is O(1) and fits it.
    let payload = serde_json::to_value(assemble(&input, 600)).expect("serializes");
    let briefing = &payload["briefing"];

    assert!(
        payload["estimated_tokens"].as_u64().unwrap_or(u64::MAX) <= 600,
        "the budget was exceeded: {payload}"
    );

    // Tier 0a — every item present, whatever the population.
    let t = &briefing["task"];
    assert!(
        t.is_object(),
        "5,000 memories displaced the task itself, which is Tier 0a: {briefing}"
    );
    assert!(
        t["id"].is_string() && t["status"].is_string(),
        "the task's identity and status are Tier 0a and must survive: {t}"
    );
    assert!(
        !t["goal"].as_str().unwrap_or("").is_empty(),
        "the goal is Tier 0a and must survive: {t}"
    );
    assert!(
        t["progress"].is_object(),
        "derived progress counts are Tier 0a and must survive: {t}"
    );
    assert!(
        t["completion_readiness"].is_string(),
        "completion readiness is Tier 0a and must survive: {t}"
    );

    // The goal is truncated to its bound rather than dropped — the tier stays
    // O(1) by bounding, never by omitting.
    assert!(
        estimate(t["goal"].as_str().unwrap_or("")) <= 60,
        "the goal must be truncated to goal_max_tokens, not admitted whole: {t}"
    );

    // Repository state is Tier 0a item 7.
    assert_eq!(
        briefing["repository"]["branch"], "main",
        "repository state is Tier 0a: {briefing}"
    );

    // Level 1 was squeezed, which is the point: the reserve is a cap on the
    // lower levels. With 5,000 memories at 600 tokens not all of them fit.
    let admitted = briefing["memory"]["project"]
        .as_array()
        .map(|a| a.len())
        .unwrap_or(0);
    assert!(
        admitted < 5000,
        "5,000 memories cannot fit in 600 tokens; the budget is not being enforced"
    );
    assert!(
        payload["truncated"].as_bool().unwrap_or(false),
        "truncation must be reported when memory was dropped: {payload}"
    );
}

/// Tier 0a survives a task whose criterion text alone dwarfs the budget, and
/// what was dropped is counted rather than silently lost (FR-443, FR-448, D83).
#[test]
fn oversized_task() {
    let cases = corpus::load_group(&corpus::root(), "budget/oversized_task")
        .expect("the oversized-task corpus loads");
    assert!(cases.len() >= 9, "{} cases", cases.len());

    for case in &cases {
        let payload = assemble_case(&case.input.extra);
        let budget = payload["budget"].as_u64().expect("a budget");
        let spent = payload["estimated_tokens"].as_u64().expect("a cost");
        assert!(
            spent <= budget,
            "{}",
            case.context(format!("spent {spent} against {budget}"))
        );

        if case.expect.extra["tier_0a_complete"]
            .as_bool()
            .unwrap_or(false)
        {
            let t = &payload["briefing"]["task"];
            assert!(
                t.is_object() && t["progress"].is_object(),
                "{}",
                case.context("Tier 0a must survive a task larger than the budget")
            );
        }
    }
}

// ---------------------------------------------------------------------------
// T085 — action order, and the pin budget
// ---------------------------------------------------------------------------

/// Criterion text is admitted in the documented action order (FR-446).
///
/// `blocked → satisfied but unverified → pending → verified → waived`, ties by
/// ascending ordinal. Chosen so the ones an agent must act on arrive first: a
/// blocked criterion is what stops progress, a satisfied-but-unverified one is
/// what needs a check, and `verified`/`waived` are what an agent least needs to
/// re-read.
#[test]
fn action_order() {
    let p = project();
    let task = task_with(4, 6);
    let criteria = vec![
        CriterionFacts {
            id: Uuid::from_u128(1),
            ordinal: 1,
            text: "waived one".into(),
            state: CriterionState::Waived,
            verification: CriterionVerification::Unverified,
            deleted: false,
        },
        CriterionFacts {
            id: Uuid::from_u128(2),
            ordinal: 2,
            text: "verified one".into(),
            state: CriterionState::Satisfied,
            verification: CriterionVerification::Verified,
            deleted: false,
        },
        CriterionFacts {
            id: Uuid::from_u128(3),
            ordinal: 3,
            text: "pending one".into(),
            state: CriterionState::Pending,
            verification: CriterionVerification::Unverified,
            deleted: false,
        },
        CriterionFacts {
            id: Uuid::from_u128(4),
            ordinal: 4,
            text: "satisfied but unchecked".into(),
            state: CriterionState::Satisfied,
            verification: CriterionVerification::Unverified,
            deleted: false,
        },
        CriterionFacts {
            id: Uuid::from_u128(5),
            ordinal: 5,
            text: "blocked one".into(),
            state: CriterionState::Blocked,
            verification: CriterionVerification::Unverified,
            deleted: false,
        },
    ];

    let input = ContextInputs {
        project: &p,
        repository: repository(),
        task: Some(&task),
        previous_handoff: None,
        decisions: &[],
        known_failures: &[],
        task_memory: &[],
        branch_memory: &[],
        project_memory: &[],
        patterns: &[],
        has_history: true,
        degraded: false,
        level0: Level0 {
            criteria: &criteria,
            ..Default::default()
        },
    };

    // A budget generous enough that all five fit, so the assertion is about
    // order rather than about what the budget dropped.
    let payload = serde_json::to_value(assemble(&input, 4000)).expect("serializes");
    let labels: Vec<&str> = payload["briefing"]["task"]["criteria"]
        .as_array()
        .expect("admitted criteria")
        .iter()
        .map(|c| c["label"].as_str().unwrap_or("?"))
        .collect();

    assert_eq!(
        labels,
        vec!["AC-5", "AC-4", "AC-3", "AC-2", "AC-1"],
        "criterion text must be admitted blocked → satisfied-unverified → pending \
         → verified → waived"
    );
}

/// A tight budget drops the tail of the action order, never the head, and says
/// how many it dropped with the path that retrieves them (FR-448).
#[test]
fn omissions_are_counted_with_a_retrieval_path() {
    let mut case = serde_json::Map::new();
    case.insert("memories".into(), json!(50));
    case.insert("budget".into(), json!(600));
    case.insert("criteria".into(), json!(200));
    case.insert("criterion_text_tokens".into(), json!(40));
    case.insert("blockers".into(), json!(2));
    case.insert("warnings".into(), json!(1));
    case.insert("pins".into(), json!(1));
    case.insert("task".into(), json!(true));

    let payload = assemble_case(&case);
    let task = &payload["briefing"]["task"];

    assert!(
        task["progress"].is_object(),
        "Tier 0a must survive 200 criteria at 600 tokens: {task}"
    );
    let shown = task["criteria"].as_array().map(|a| a.len()).unwrap_or(0);
    let omitted = task["criteria_omitted"].as_u64().unwrap_or(0) as usize;
    assert!(
        omitted > 0,
        "200 criteria cannot fit in 600 tokens; the omission must be reported"
    );
    assert_eq!(
        shown + omitted,
        200,
        "every criterion is either shown or counted as omitted — none vanishes"
    );
}

/// The pin budget refuses at the edge and unpins nothing (FR-454), a pin never
/// widens scope (FR-453), and a superseded memory loses its pin while a drifted
/// one keeps it (FR-456).
#[test]
fn pins() {
    let s = Sandbox::new();

    // Fill the per-scope budget of 4 on the project scope.
    let mut ids = Vec::new();
    for i in 0..5 {
        let m = s.json(&[
            "memory",
            "add",
            &format!("constraint number {i} that must always hold"),
            "--scope",
            "project",
        ]);
        ids.push(
            m["memory"]["id"]
                .as_str()
                .expect("a created memory has an id")
                .to_string(),
        );
    }

    for id in ids.iter().take(4) {
        let out = s.cairn(&["memory", "pin", id]);
        assert!(
            out.ok(),
            "pinning within budget must succeed: {}",
            out.stderr
        );
    }

    // The fifth is refused, and nothing is unpinned to make room.
    let refused = s.json_err(&["memory", "pin", &ids[4]]);
    assert_eq!(
        refused["code"], "pin_budget_exhausted",
        "the fifth pin in a scope must be refused by name: {refused}"
    );
    let pinned_now = s.query_column("SELECT CAST(COUNT(*) AS TEXT) FROM memories WHERE pinned = 1");
    assert_eq!(
        pinned_now.first().map(String::as_str),
        Some("4"),
        "a refused pin must unpin nothing — evicting someone else's constraint is \
         the opposite of what a pin is for"
    );

    // Unpinning frees a slot.
    s.must(&["memory", "pin", &ids[0], "--off"]);
    let out = s.cairn(&["memory", "pin", &ids[4]]);
    assert!(out.ok(), "a freed slot must be usable: {}", out.stderr);
}

/// A superseded memory loses its pin; a drifted one keeps it (FR-456).
///
/// The asymmetry is the point. A superseded constraint has been replaced, so
/// holding it in Level 0 would present a retired invariant as current. A drifted
/// one is a constraint that *stopped being true* — which is exactly what must be
/// said, so it keeps its pin and carries its warning.
#[test]
fn a_superseded_pin_is_cleared_and_a_drifted_one_is_kept() {
    let s = Sandbox::new();

    let old = s.json(&[
        "memory",
        "add",
        "the API listens on 8080",
        "--scope",
        "project",
    ]);
    let old_id = old["memory"]["id"].as_str().expect("id").to_string();
    s.must(&["memory", "pin", &old_id]);

    let new = s.json(&[
        "memory",
        "add",
        "the API listens on 9000",
        "--scope",
        "project",
    ]);
    let new_id = new["memory"]["id"].as_str().expect("id").to_string();

    // The decision that replaces it.
    let out = s.cairn(&[
        "memory",
        "reconcile",
        "--from",
        &new_id,
        "--to",
        &old_id,
        "--relation",
        "supersedes",
    ]);
    assert!(
        out.ok(),
        "recording a supersession must succeed: {}",
        out.stderr
    );

    let pinned = s.query_column(&format!(
        "SELECT CAST(pinned AS TEXT) FROM memories WHERE id = '{old_id}'"
    ));
    assert_eq!(
        pinned.first().map(String::as_str),
        Some("0"),
        "a superseded memory must lose its pin in the same transaction as the decision"
    );
    let successor = s.query_column(&format!(
        "SELECT CAST(pinned AS TEXT) FROM memories WHERE id = '{new_id}'"
    ));
    assert_eq!(
        successor.first().map(String::as_str),
        Some("0"),
        "the successor is pinned only explicitly — a pin is never inherited"
    );

    // A drifted memory keeps its pin.
    let drifting = s.json(&[
        "memory",
        "add",
        "the queue backend is sqs",
        "--scope",
        "project",
    ]);
    let drift_id = drifting["memory"]["id"].as_str().expect("id").to_string();
    s.must(&["memory", "pin", &drift_id]);
    s.execute_sql(&format!(
        "UPDATE memories SET verification = 'drifted' WHERE id = '{drift_id}'"
    ));

    let still = s.query_column(&format!(
        "SELECT CAST(pinned AS TEXT) FROM memories WHERE id = '{drift_id}'"
    ));
    assert_eq!(
        still.first().map(String::as_str),
        Some("1"),
        "a drifted constraint keeps its pin — a constraint that no longer holds is \
         exactly what must be said"
    );
}
