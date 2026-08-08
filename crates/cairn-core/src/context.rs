//! Briefing assembly (FR-027 – FR-031, D8).
//!
//! Sections are admitted in a fixed priority order, each measured with the
//! Cairn token estimator *before* it is emitted. That makes budget compliance a
//! property of this loop rather than a statistic: the output can never exceed
//! the budget.

use crate::budget::{estimate, estimate_lines, Budget};
use crate::domain::*;
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
    /// False for a project Cairn has never seen before (FR-031).
    pub has_history: bool,
    /// True when assembly ran out of time or storage was unavailable (FR-046).
    pub degraded: bool,
}

/// Assemble a briefing that fits `budget_tokens` estimated tokens.
pub fn assemble(input: &ContextInputs<'_>, budget_tokens: usize) -> ContextPayload {
    let mut budget = Budget::new(budget_tokens);
    let mut omitted: Vec<String> = Vec::new();

    // The project header is the frame everything else hangs on; it is charged
    // first and is small enough that dropping it would mean a useless briefing.
    let header_cost = estimate(&input.project.name) + 8;
    budget.try_spend(header_cost);

    let mut briefing = Briefing {
        project: ProjectSummary::from(input.project),
        repository: RepositoryState::default(),
        task: None,
        previous_handoff: None,
        decisions: Vec::new(),
        known_failures: Vec::new(),
        memory: BriefingMemory::default(),
        no_prior_history: !input.has_history,
    };

    for section in SECTION_ORDER {
        let admitted = match *section {
            "task" => admit_task(&mut briefing, input, &mut budget),
            "repository" => admit_repository(&mut briefing, input, &mut budget),
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
    }
}

fn admit_task(b: &mut Briefing, input: &ContextInputs<'_>, budget: &mut Budget) -> bool {
    let Some(t) = input.task else { return true };
    let cost = estimate(&t.title) + estimate(&t.goal) + estimate_lines(&t.acceptance_criteria) + 4;
    if !budget.try_spend(cost) {
        return false;
    }
    b.task = Some(BriefingTask {
        id: t.id,
        title: t.title.clone(),
        goal: t.goal.clone(),
        acceptance_criteria: t.acceptance_criteria.clone(),
        status: t.status,
    });
    true
}

fn admit_repository(b: &mut Briefing, input: &ContextInputs<'_>, budget: &mut Budget) -> bool {
    let r = &input.repository;
    let cost = estimate(&r.branch) + estimate(r.commit_sha.as_deref().unwrap_or("")) + 12;
    if !budget.try_spend(cost) {
        return false;
    }
    b.repository = r.clone();
    true
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
            has_history: true,
            degraded: false,
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
