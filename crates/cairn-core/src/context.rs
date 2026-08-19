//! Briefing assembly (FR-027 – FR-031, D8; FR-441 – FR-448).
//!
//! Sections are admitted in a fixed priority order, each measured with the
//! Cairn token estimator *before* it is emitted. That makes budget compliance a
//! property of this loop rather than a statistic: the output can never exceed
//! the budget.
//!
//! # Three levels, and why Level 0 has two tiers
//!
//! ```text
//! LEVEL 0  minimum safe continuity   ── a reserved share the lower levels cannot take
//! LEVEL 1  relevant current knowledge ── the remaining budget, ranked
//! LEVEL 2  history and evidence       ── never automatic; explicit request only
//! ```
//!
//! A budget is finite. Criterion text, blocker descriptions and warning detail
//! are not. Guaranteeing that all of them fit would be a promise Cairn cannot
//! keep, so Level 0 splits:
//!
//! * **Tier 0a** — the guaranteed work state. Every item is O(1) in the size of
//!   the project and the task, so the tier has a bounded worst case that fits
//!   the documented minimum budget. After any number of compactions the agent
//!   still knows what it is doing, how far along it is, what is blocking it and
//!   that something is wrong (FR-443).
//! * **Tier 0b** — bounded detail, admitted in a documented order until the
//!   budget binds, with whatever does not fit counted by kind and given a
//!   retrieval path. Omission is never silent (FR-448).
//!
//! The reserve is a **cap on the lower levels, not a floor Level 0 must spend**.
//! Unspent reserve returns to the general pool, which is why a project with no
//! task, no warnings and no pins delivers exactly what it delivered before this
//! feature existed (FR-442).

use crate::budget::{estimate, Budget};
use crate::domain::*;
use crate::tasks::{self, BlockerFacts, CriterionFacts};
use crate::wire::*;

/// Sections, highest priority first (D8).
pub const SECTION_ORDER: &[&str] = &[
    "task",
    "repository",
    "previous_handoff",
    "known_failures",
    "decisions",
    "task_memory",
    "branch_memory",
    "project_memory",
    // Last, deliberately. A prior pattern from another project is the least
    // authoritative thing in a briefing, so it is the first thing a tight
    // budget drops (FR-398).
    "patterns",
];

/// Sections whose loss means the briefing is materially degraded (SC-003).
pub const HIGH_PRIORITY_SECTIONS: &[&str] = &["task", "repository", "previous_handoff"];

/// Everything the briefing is assembled from. All recorded state.
pub struct ContextInputs<'a> {
    pub project: &'a Project,
    pub repository: RepositoryState,
    pub task: Option<&'a Task>,
    pub previous_handoff: Option<&'a Handoff>,
    pub decisions: &'a [String],
    pub known_failures: &'a [String],
    pub task_memory: &'a [String],
    pub branch_memory: &'a [String],
    pub project_memory: &'a [String],
    /// Signal-matched prior patterns, already capped and ordered by the caller.
    pub patterns: &'a [BriefingPattern],
    /// False for a project Cairn has never seen before (FR-031).
    pub has_history: bool,
    /// True when assembly ran out of time or storage was unavailable (FR-046).
    pub degraded: bool,
    /// Everything Feature 003 adds. Defaulted, so a caller that has none of it
    /// gets Feature 001's briefing unchanged.
    pub level0: Level0<'a>,
}

/// The Level 0 inputs. All plain data — this crate never reaches the store.
#[derive(Default)]
pub struct Level0<'a> {
    pub criteria: &'a [CriterionFacts],
    pub blockers: &'a [BlockerFacts],
    /// Blocker descriptions by id, so the most actionable one can be named.
    pub blocker_text: &'a [(uuid::Uuid, String)],
    pub warnings: &'a [ContextWarning],
    pub pins: &'a [PinnedConstraint],
    /// The recorded next action of a diverged checkpoint. Phase 9 supplies it;
    /// until then there are no checkpoints and it is absent, costing nothing.
    pub previous_next_action: Option<&'a str>,
    /// Emit the selection diagnostics (FR-461, FR-463).
    pub explain: bool,
    pub caps: Caps,
}

/// The bounds Level 0 admits within.
#[derive(Debug, Clone, Copy)]
pub struct Caps {
    pub goal_max_tokens: usize,
    pub warnings_in_context_max: usize,
    pub pins_in_context_max: usize,
    /// `floor(limit * min_safe_context_fraction)` is computed by the caller from
    /// config; this is the fraction already applied.
    pub reserve_fraction: f64,
}

impl Default for Caps {
    fn default() -> Self {
        Self {
            goal_max_tokens: 60,
            warnings_in_context_max: 5,
            pins_in_context_max: 4,
            reserve_fraction: 0.40,
        }
    }
}

/// Assemble a briefing that fits `budget_tokens` estimated tokens.
pub fn assemble(input: &ContextInputs<'_>, budget_tokens: usize) -> ContextPayload {
    let reserve = (budget_tokens as f64 * input.level0.caps.reserve_fraction).floor() as usize;
    let mut budget = Budget::with_reserve(budget_tokens, reserve);
    let mut omitted: Vec<String> = Vec::new();
    let mut selection = Selection {
        budget: budget_tokens,
        reserve,
        ..Default::default()
    };

    // The project header is the frame everything else hangs on; it is charged
    // first and is small enough that dropping it would mean a useless briefing.
    // Charged against the reserve, so withholding one cannot make the frame fail
    // where it previously fit.
    let header_cost = estimate(&input.project.name) + 8;
    budget.try_spend_reserved(header_cost);

    let mut briefing = Briefing {
        project: ProjectSummary::from(input.project),
        repository: RepositoryState::default(),
        task: None,
        previous_handoff: None,
        decisions: Vec::new(),
        known_failures: Vec::new(),
        memory: BriefingMemory::default(),
        no_prior_history: !input.has_history,
        warnings: Vec::new(),
        constraints: Vec::new(),
        previous_next_action: None,
        patterns: Vec::new(),
    };

    // ---- Level 0 -----------------------------------------------------------
    //
    // Tier 0a first, then Tier 0b, both drawing on the reserve before the
    // general pool. Tier 0b can never displace Tier 0a because it is admitted
    // after it.
    if !admit_tier_0a(&mut briefing, input, &mut budget, &mut selection) {
        omitted.push("task".into());
    }
    admit_tier_0b(&mut briefing, input, &mut budget, &mut selection);

    // Whatever Level 0 did not spend goes back. This single call is what makes
    // the no-regression property true (FR-442).
    budget.release_reserve();
    selection.reserve_used = budget.reserve_used();
    selection.reserve_released = budget.reserve_released();

    // ---- Level 1 -----------------------------------------------------------
    for section in SECTION_ORDER {
        let admitted = match *section {
            // Admitted above, as Tier 0a.
            "task" | "repository" => continue,
            "previous_handoff" => admit_handoff(&mut briefing, input, &mut budget),
            "known_failures" => {
                briefing.known_failures = budget
                    .take_while_fits(input.known_failures.iter().cloned(), |s| estimate(s) + 1);
                briefing.known_failures.len() == input.known_failures.len()
            }
            "decisions" => {
                briefing.decisions =
                    budget.take_while_fits(input.decisions.iter().cloned(), |s| estimate(s) + 1);
                briefing.decisions.len() == input.decisions.len()
            }
            "task_memory" => {
                briefing.memory.task =
                    budget.take_while_fits(input.task_memory.iter().cloned(), |s| estimate(s) + 1);
                briefing.memory.task.len() == input.task_memory.len()
            }
            "branch_memory" => {
                briefing.memory.branch = budget
                    .take_while_fits(input.branch_memory.iter().cloned(), |s| estimate(s) + 1);
                briefing.memory.branch.len() == input.branch_memory.len()
            }
            "project_memory" => {
                briefing.memory.project = budget
                    .take_while_fits(input.project_memory.iter().cloned(), |s| estimate(s) + 1);
                briefing.memory.project.len() == input.project_memory.len()
            }
            "patterns" => {
                briefing.patterns =
                    budget.take_while_fits(input.patterns.iter().cloned(), pattern_cost);
                briefing.patterns.len() == input.patterns.len()
            }
            _ => true,
        };
        if !admitted {
            omitted.push((*section).to_string());
        }
    }

    ContextPayload {
        estimated_tokens: budget.spent(),
        budget: budget.limit(),
        truncated: !omitted.is_empty(),
        omitted_sections: omitted,
        degraded: input.degraded,
        briefing,
        selection: input.level0.explain.then_some(selection),
    }
}

/// What a suggestion costs, including the labelling that makes it honest.
///
/// The label is not optional and so is not free: a pattern rendered without
/// "unverified in this project" is a different, worse thing than a pattern that
/// did not fit.
fn pattern_cost(p: &BriefingPattern) -> usize {
    estimate(&p.title)
        + estimate(&p.approach)
        + p.applicability.iter().map(|a| estimate(a)).sum::<usize>()
        + p.constraints.iter().map(|c| estimate(c)).sum::<usize>()
        + p.alternative_cause.as_deref().map(estimate).unwrap_or(0)
        + p.check_this_first.as_deref().map(estimate).unwrap_or(0)
        // The label, the trust word and the field names.
        + 12
}

// ---------------------------------------------------------------------------
// Tier 0a — the guaranteed work state
// ---------------------------------------------------------------------------

/// Admit the O(1) work state: repository, then the task's identity, bounded
/// goal, status, derived counts, readiness and the single most actionable
/// blocker, then the warning **kinds** with counts.
///
/// Returns false only when the task itself did not fit, which at any budget at
/// or above the documented minimum cannot happen — the tier's worst case is
/// bounded precisely so that it cannot.
fn admit_tier_0a(
    b: &mut Briefing,
    input: &ContextInputs<'_>,
    budget: &mut Budget,
    sel: &mut Selection,
) -> bool {
    // Repository state — fixed shape, Tier 0a item 7.
    let r = &input.repository;
    let repo_cost = estimate(&r.branch) + estimate(r.commit_sha.as_deref().unwrap_or("")) + 12;
    if budget.try_spend_reserved(repo_cost) {
        b.repository = r.clone();
        sel.included.push(SelectedItem {
            level: ContextLevel::MinimumSafe,
            kind: "repository".into(),
            id: r.branch.clone(),
            reasons: vec![SelectionReason::ScopeMatch],
            cost: repo_cost,
        });
    }

    // The warning kinds, with counts. Detail is Tier 0b; the *fact* that
    // something is wrong is guaranteed.
    if !input.level0.warnings.is_empty() {
        let kinds = warning_kind_counts(input.level0.warnings);
        let cost = estimate(&kinds) + 1;
        if budget.try_spend_reserved(cost) {
            b.warnings.push(ContextWarning {
                kind: "summary".into(),
                subject: kinds,
                detail: String::new(),
            });
            sel.included.push(SelectedItem {
                level: ContextLevel::MinimumSafe,
                kind: "warning".into(),
                id: "summary".into(),
                reasons: vec![SelectionReason::DriftWarning],
                cost,
            });
        }
    }

    // A diverged checkpoint's recorded action, always labelled as previous.
    if let Some(previous) = input.level0.previous_next_action {
        let cost = estimate(previous) + 2;
        if budget.try_spend_reserved(cost) {
            b.previous_next_action = Some(previous.to_string());
            sel.included.push(SelectedItem {
                level: ContextLevel::MinimumSafe,
                kind: "previous_next_action".into(),
                id: String::new(),
                reasons: vec![SelectionReason::CheckpointAssumption],
                cost,
            });
        }
    }

    let Some(t) = input.task else { return true };

    // The goal is bounded rather than dropped: the tier stays O(1) by
    // truncating, never by omitting (FR-443).
    let (goal, goal_truncated) = truncate_to_tokens(&t.goal, input.level0.caps.goal_max_tokens);
    let progress = tasks::progress(input.level0.criteria);
    let readiness = tasks::completion_readiness(input.level0.criteria, input.level0.blockers);
    let open_blockers = input
        .level0
        .blockers
        .iter()
        .filter(|x| !x.deleted && x.state == BlockerState::Open)
        .count();
    let blocker = most_actionable_blocker(input);

    // A fixed shape, so the cost does not grow with the project or the task.
    let cost =
        estimate(&t.title) + estimate(&goal) + estimate(blocker.as_deref().unwrap_or("")) + 24;
    if !budget.try_spend_reserved(cost) {
        return false;
    }

    b.task = Some(BriefingTask {
        id: t.id,
        title: t.title.clone(),
        goal,
        // Feature 001's array stays exactly as it was for its five readers; the
        // Level 0 rendering uses `criteria` below.
        acceptance_criteria: t.acceptance_criteria.clone(),
        status: t.status,
        progress: Some(progress),
        completion_readiness: Some(readiness),
        open_blockers: Some(open_blockers),
        blocker,
        goal_truncated,
        criteria: Vec::new(),
        criteria_omitted: None,
    });
    sel.included.push(SelectedItem {
        level: ContextLevel::MinimumSafe,
        kind: "task".into(),
        id: t.id.to_string(),
        reasons: vec![SelectionReason::TaskBinding],
        cost,
    });
    true
}

// ---------------------------------------------------------------------------
// Tier 0b — bounded detail
// ---------------------------------------------------------------------------

/// Warning detail, then pins, then criterion text in action order, then further
/// blockers — each until its cap or the budget binds (FR-444, FR-446, FR-448).
fn admit_tier_0b(
    b: &mut Briefing,
    input: &ContextInputs<'_>,
    budget: &mut Budget,
    sel: &mut Selection,
) {
    let caps = input.level0.caps;

    // 8 — warning detail, highest precedence first.
    let mut ordered: Vec<&ContextWarning> = input.level0.warnings.iter().collect();
    ordered.sort_by_key(|w| warning_precedence(&w.kind));
    let mut warnings_shown = 0usize;
    for w in ordered.iter().take(caps.warnings_in_context_max) {
        let cost = estimate(&w.subject) + estimate(&w.detail) + 2;
        if !budget.try_spend_reserved(cost) {
            break;
        }
        b.warnings.push((*w).clone());
        warnings_shown += 1;
        sel.included.push(SelectedItem {
            level: ContextLevel::MinimumSafe,
            kind: "warning".into(),
            id: format!("{}:{}", w.kind, w.subject),
            reasons: vec![warning_reason(&w.kind)],
            cost,
        });
    }
    note_omission(
        sel,
        "warning",
        input.level0.warnings.len().saturating_sub(warnings_shown),
        if warnings_shown >= caps.warnings_in_context_max {
            OmissionReason::CapReached
        } else {
            OmissionReason::BudgetExhausted
        },
        "",
    );

    // 9 — pinned constraints in force.
    let mut pins_shown = 0usize;
    for p in input.level0.pins.iter().take(caps.pins_in_context_max) {
        let cost = estimate(&p.text) + 2;
        if !budget.try_spend_reserved(cost) {
            break;
        }
        b.constraints.push(p.clone());
        pins_shown += 1;
        sel.included.push(SelectedItem {
            level: ContextLevel::MinimumSafe,
            kind: "constraint".into(),
            id: p.id.to_string(),
            reasons: vec![SelectionReason::Pinned],
            cost,
        });
    }
    note_omission(
        sel,
        "constraint",
        input.level0.pins.len().saturating_sub(pins_shown),
        if pins_shown >= caps.pins_in_context_max {
            OmissionReason::PinBudget
        } else {
            OmissionReason::BudgetExhausted
        },
        "",
    );

    // 10 — criterion text, in action order: blocked, then satisfied but
    // unverified, then pending, then verified, then waived. The ones an agent
    // must act on arrive first.
    let ordered = tasks::action_order(input.level0.criteria);
    let total = ordered.len();
    let mut shown = Vec::new();
    for c in ordered {
        let label = tasks::criterion_label(c.ordinal);
        let cost = estimate(&label) + estimate(&c.text) + 3;
        if !budget.try_spend_reserved(cost) {
            break;
        }
        sel.included.push(SelectedItem {
            level: ContextLevel::MinimumSafe,
            kind: "criterion".into(),
            id: c.id.to_string(),
            reasons: vec![SelectionReason::TaskBinding],
            cost,
        });
        shown.push(BriefingCriterion {
            label,
            text: c.text.clone(),
            state: c.state,
            verification: c.verification,
        });
    }
    let dropped = total.saturating_sub(shown.len());
    if let Some(task) = b.task.as_mut() {
        task.criteria = shown;
        task.criteria_omitted = (dropped > 0).then_some(dropped);
    }
    note_omission(
        sel,
        "criterion",
        dropped,
        OmissionReason::BudgetExhausted,
        "cairn task get <id>",
    );
}

/// `⚠ 1 conflict · 1 drift · checkpoint diverged` — the kinds and their counts,
/// which is what Tier 0a guarantees whatever the budget.
fn warning_kind_counts(warnings: &[ContextWarning]) -> String {
    let mut counts: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
    for w in warnings {
        *counts.entry(w.kind.as_str()).or_default() += 1;
    }
    let parts: Vec<String> = counts
        .iter()
        // "2 conflict" reads as a truncated string rather than a count. The
        // kinds are single English nouns, so the plural is the suffix.
        .map(|(kind, n)| {
            if *n == 1 {
                format!("{n} {kind}")
            } else {
                format!("{n} {kind}s")
            }
        })
        .collect();
    parts.join(" · ")
}

/// divergence → task → conflict → drift (`contracts/continuity-context.md`).
fn warning_precedence(kind: &str) -> usize {
    match kind {
        "task_divergence" | "checkpoint" => 0,
        "task" => 1,
        "conflict" => 2,
        "drift" => 3,
        _ => 4,
    }
}

fn warning_reason(kind: &str) -> SelectionReason {
    match kind {
        "conflict" => SelectionReason::ConflictWarning,
        "task_divergence" | "checkpoint" => SelectionReason::CheckpointAssumption,
        "task" => SelectionReason::TaskBinding,
        _ => SelectionReason::DriftWarning,
    }
}

fn note_omission(
    sel: &mut Selection,
    kind: &str,
    count: usize,
    reason: OmissionReason,
    retrieval: &str,
) {
    if count == 0 {
        return;
    }
    sel.omitted.push(OmittedItem {
        kind: kind.to_string(),
        count,
        reason,
        retrieval: retrieval.to_string(),
    });
}

/// The blocker an agent should act on: the oldest still open.
///
/// One bounded line. Which one is "most actionable" is not a judgement Cairn can
/// make from a description, so it uses the one that has been in force longest —
/// deterministic, and explainable.
fn most_actionable_blocker(input: &ContextInputs<'_>) -> Option<String> {
    let open = input
        .level0
        .blockers
        .iter()
        .find(|b| !b.deleted && b.state == BlockerState::Open)?;
    let text = input
        .level0
        .blocker_text
        .iter()
        .find(|(id, _)| *id == open.id)
        .map(|(_, t)| t.as_str())
        .unwrap_or("");
    let (bounded, _) = truncate_to_tokens(text, 24);
    Some(bounded)
}

/// Truncate to at most `max_tokens` estimated tokens, on a character boundary.
///
/// Returns whether anything was dropped, so the briefing can say so rather than
/// presenting a cut sentence as the whole goal.
fn truncate_to_tokens(text: &str, max_tokens: usize) -> (String, bool) {
    if estimate(text) <= max_tokens {
        return (text.to_string(), false);
    }
    let max_chars = (max_tokens as f64 * crate::budget::CHARS_PER_TOKEN) as usize;
    let mut out: String = text.chars().take(max_chars.saturating_sub(1)).collect();
    out.push('…');
    (out, true)
}

fn admit_handoff(b: &mut Briefing, input: &ContextInputs<'_>, budget: &mut Budget) -> bool {
    let Some(h) = input.previous_handoff else {
        return true;
    };

    // Remaining work and next step first: they are the part a successor
    // session actually acts on (D8).
    let head_cost = estimate(&h.next_step) + 4;
    if !budget.try_spend(head_cost) {
        return false;
    }
    let remaining = budget.take_while_fits(h.remaining_work.iter().cloned(), |s| estimate(s) + 1);
    let complete_remaining = remaining.len() == h.remaining_work.len();
    let changed = budget.take_while_fits(h.changed_files.iter().cloned(), |s| estimate(s) + 1);
    let complete_changed = changed.len() == h.changed_files.len();

    b.previous_handoff = Some(BriefingHandoff {
        session_id: h.session_id,
        next_step: h.next_step.clone(),
        remaining_work: remaining,
        changed_files: changed,
    });
    complete_remaining && complete_changed
}

/// True when nothing in `omitted` is a high-priority section (SC-003).
pub fn kept_all_high_priority(omitted: &[String]) -> bool {
    !omitted
        .iter()
        .any(|s| HIGH_PRIORITY_SECTIONS.contains(&s.as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn project() -> Project {
        Project {
            id: new_id(),
            name: "cairn".into(),
            git_common_dir: "/tmp/repo/.git".into(),
            repository_remote: None,
            linked: false,
            server_project_id: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            deleted_at: None,
        }
    }

    fn inputs<'a>(p: &'a Project, mem: &'a [String]) -> ContextInputs<'a> {
        ContextInputs {
            project: p,
            repository: RepositoryState {
                branch: "main".into(),
                commit_sha: Some("abc1234".into()),
                staged: 1,
                unstaged: 2,
                untracked: 0,
            },
            task: None,
            previous_handoff: None,
            decisions: &[],
            known_failures: &[],
            task_memory: &[],
            branch_memory: &[],
            project_memory: mem,
            patterns: &[],
            has_history: true,
            degraded: false,
            level0: Level0::default(),
        }
    }

    #[test]
    fn never_exceeds_the_budget_even_with_far_more_memory_than_fits() {
        let p = project();
        let mem: Vec<String> = (0..5000)
            .map(|i| format!("project memory item number {i} with some prose"))
            .collect();
        for budget in [200usize, 800, 3000, 4000] {
            let out = assemble(&inputs(&p, &mem), budget);
            assert!(
                out.estimated_tokens <= budget,
                "spent {} over budget {budget}",
                out.estimated_tokens
            );
            assert!(out.truncated);
            assert!(out.omitted_sections.contains(&"project_memory".to_string()));
        }
    }

    #[test]
    fn reports_no_prior_history_and_still_succeeds() {
        let p = project();
        let mut i = inputs(&p, &[]);
        i.has_history = false;
        let out = assemble(&i, 3000);
        assert!(out.briefing.no_prior_history);
        assert!(!out.truncated);
    }

    #[test]
    fn repository_state_survives_a_tight_budget_before_memory_does() {
        let p = project();
        let mem: Vec<String> = (0..200).map(|i| format!("memory {i}")).collect();
        let out = assemble(&inputs(&p, &mem), 60);
        assert_eq!(out.briefing.repository.branch, "main");
        assert!(out.omitted_sections.contains(&"project_memory".to_string()));
        assert!(kept_all_high_priority(&out.omitted_sections));
    }

    #[test]
    fn task_goal_and_criteria_lead_the_briefing() {
        let p = project();
        let task = Task {
            id: new_id(),
            project_id: p.id,
            title: "Rate limiting".into(),
            goal: "Requests over the limit get 429".into(),
            acceptance_criteria: vec!["429 above threshold".into()],
            status: TaskStatus::InProgress,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            deleted_at: None,
        };
        let mut i = inputs(&p, &[]);
        i.task = Some(&task);
        let out = assemble(&i, 3000);
        let bt = out.briefing.task.expect("task section");
        assert_eq!(bt.goal, "Requests over the limit get 429");
        assert_eq!(bt.acceptance_criteria.len(), 1);
    }

    #[test]
    fn previous_handoff_next_step_and_remaining_work_are_carried() {
        let p = project();
        let h = Handoff {
            id: new_id(),
            session_id: new_id(),
            trigger: HandoffTrigger::SessionEnd,
            goal: "g".into(),
            progress: "p".into(),
            completed_work: vec![],
            remaining_work: vec!["Open failure: cargo test".into()],
            changed_files: vec!["src/lib.rs".into()],
            decisions: vec![],
            failures: vec![],
            tests_executed: vec![],
            repository_state: RepositoryState::default(),
            next_step: "Fix the open failure".into(),
            agent_note: None,
            evidence: vec![],
            created_at: Utc::now(),
            deleted_at: None,
        };
        let mut i = inputs(&p, &[]);
        i.previous_handoff = Some(&h);
        let out = assemble(&i, 3000);
        let ph = out.briefing.previous_handoff.expect("handoff section");
        assert_eq!(ph.next_step, "Fix the open failure");
        assert_eq!(ph.remaining_work.len(), 1);
        assert_eq!(ph.changed_files, vec!["src/lib.rs".to_string()]);
    }

    #[test]
    fn degraded_flag_passes_through() {
        let p = project();
        let mut i = inputs(&p, &[]);
        i.degraded = true;
        assert!(assemble(&i, 3000).degraded);
    }
}
