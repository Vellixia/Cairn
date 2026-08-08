//! Handoff synthesis (FR-033, FR-034, D7).
//!
//! Every field is *derived* from recorded state. An agent-supplied narrative is
//! accepted only as a bounded, clearly attributed `agent_note` — never as the
//! source of record, because a summary the agent writes about itself is exactly
//! the thing that drifts.

use crate::domain::*;
use chrono::Utc;

/// Everything handoff synthesis reads. All of it is recorded state.
pub struct HandoffInputs<'a> {
    pub session: &'a Session,
    pub task: Option<&'a Task>,
    /// Session observations, oldest first.
    pub observations: &'a [Observation],
    /// Decision-typed memories produced during this session.
    pub decision_memories: &'a [Memory],
    pub repository_state: RepositoryState,
    /// Paths Git currently reports as changed, for reconciliation.
    pub git_changed_files: &'a [String],
    pub agent_note: Option<String>,
}

/// Build a handoff for `trigger`.
pub fn synthesize(input: &HandoffInputs<'_>, trigger: HandoffTrigger) -> Handoff {
    let obs = input.observations;

    let changed_files = derive_changed_files(obs, input.git_changed_files);
    let tests_executed = derive_tests(obs);
    let failures = derive_failures(obs);
    let decisions = derive_decisions(obs, input.decision_memories);
    let discoveries: Vec<String> = obs
        .iter()
        .filter(|o| o.kind == ObservationType::Discovery)
        .map(|o| o.summary.clone())
        .collect();

    let goal = derive_goal(input);
    let completed_work = derive_completed(&changed_files, &tests_executed, &discoveries);
    let remaining_work = derive_remaining(input, &failures, &tests_executed);
    let progress = derive_progress(&changed_files, &tests_executed, &failures);
    let next_step = derive_next_step(&failures, &remaining_work, &changed_files);

    Handoff {
        id: new_id(),
        session_id: input.session.id,
        trigger,
        goal,
        progress,
        completed_work,
        remaining_work,
        changed_files,
        decisions,
        failures,
        tests_executed,
        repository_state: input.repository_state.clone(),
        next_step,
        agent_note: input.agent_note.clone(),
        // Identifiers only. Never their content (FR-055).
        evidence: obs.iter().map(|o| o.id).collect(),
        created_at: Utc::now(),
        deleted_at: None,
    }
}

fn derive_goal(input: &HandoffInputs<'_>) -> String {
    if let Some(t) = input.task {
        return t.goal.clone();
    }
    // No task bound: the earliest user instruction is the best recorded proxy.
    if let Some(o) = input
        .observations
        .iter()
        .find(|o| o.kind == ObservationType::UserInstruction)
    {
        return o.summary.clone();
    }
    format!("Unbound session on branch {}", input.session.branch)
}

fn derive_changed_files(obs: &[Observation], git_changed: &[String]) -> Vec<String> {
    let mut files: Vec<String> = obs
        .iter()
        .filter(|o| o.kind == ObservationType::FileChanged)
        .filter_map(|o| o.path.clone())
        .chain(git_changed.iter().cloned())
        .collect();
    files.sort();
    files.dedup();
    files
}

fn derive_tests(obs: &[Observation]) -> Vec<TestRunRecord> {
    obs.iter()
        .filter(|o| o.kind == ObservationType::TestRun)
        .map(|o| TestRunRecord {
            command: o.command.clone().unwrap_or_else(|| o.summary.clone()),
            outcome: o.outcome.clone().unwrap_or_else(|| "unknown".to_string()),
            occurred_at: o.occurred_at,
        })
        .collect()
}

fn derive_failures(obs: &[Observation]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for o in obs {
        match o.kind {
            ObservationType::Error => out.push(o.summary.clone()),
            ObservationType::TestRun if o.outcome.as_deref() == Some("failed") => {
                let cmd = o.command.clone().unwrap_or_else(|| o.summary.clone());
                out.push(format!("Test failed: {cmd}"));
            }
            _ => {}
        }
    }
    out.dedup();
    out
}

fn derive_decisions(obs: &[Observation], memories: &[Memory]) -> Vec<String> {
    let mut out: Vec<String> = obs
        .iter()
        .filter(|o| o.kind == ObservationType::Decision)
        .map(|o| o.summary.clone())
        .collect();
    out.extend(
        memories
            .iter()
            .filter(|m| m.kind == MemoryType::Decision && m.deleted_at.is_none())
            .map(|m| m.content.clone()),
    );
    out.dedup();
    out
}

fn derive_completed(
    changed_files: &[String],
    tests: &[TestRunRecord],
    discoveries: &[String],
) -> Vec<String> {
    let mut out = Vec::new();
    if !changed_files.is_empty() {
        let shown: Vec<&str> = changed_files.iter().take(10).map(String::as_str).collect();
        let more = changed_files.len().saturating_sub(shown.len());
        let suffix = if more > 0 {
            format!(" (+{more} more)")
        } else {
            String::new()
        };
        out.push(format!(
            "Changed {} file(s): {}{}",
            changed_files.len(),
            shown.join(", "),
            suffix
        ));
    }
    if !tests.is_empty() {
        let passed = tests.iter().filter(|t| t.outcome == "passed").count();
        let failed = tests.iter().filter(|t| t.outcome == "failed").count();
        out.push(format!(
            "Ran {} test command(s): {passed} passed, {failed} failed",
            tests.len()
        ));
    }
    out.extend(discoveries.iter().cloned());
    out
}

fn derive_remaining(
    input: &HandoffInputs<'_>,
    failures: &[String],
    tests: &[TestRunRecord],
) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(t) = input.task {
        if t.status != TaskStatus::Done {
            // An acceptance criterion counts as satisfied only when a test
            // command passed and nothing failed. Anything less is remaining.
            let all_green = !tests.is_empty()
                && tests.iter().all(|r| r.outcome == "passed")
                && failures.is_empty();
            if !all_green {
                out.extend(
                    t.acceptance_criteria
                        .iter()
                        .map(|c| format!("Acceptance criterion: {c}")),
                );
            }
        }
    }
    out.extend(failures.iter().map(|f| format!("Open failure: {f}")));
    if out.is_empty() && input.task.is_none() {
        out.push("No task bound; remaining work not tracked".to_string());
    }
    out
}

fn derive_progress(
    changed_files: &[String],
    tests: &[TestRunRecord],
    failures: &[String],
) -> String {
    let failed = failures.len();
    format!(
        "{} file(s) changed, {} test command(s) run, {} failure(s) open",
        changed_files.len(),
        tests.len(),
        failed
    )
}

fn derive_next_step(failures: &[String], remaining: &[String], changed_files: &[String]) -> String {
    if let Some(first) = failures.first() {
        return format!("Fix the open failure: {first}");
    }
    if let Some(first) = remaining.first() {
        return format!("Continue with: {first}");
    }
    if !changed_files.is_empty() {
        return "Review the changed files and decide whether to commit".to_string();
    }
    "Pick up the task goal and start the first acceptance criterion".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn session() -> Session {
        Session {
            id: new_id(),
            project_id: new_id(),
            task_id: None,
            user_id: new_id(),
            agent: "claude-code".into(),
            branch: "main".into(),
            commit_sha: Some("abc123".into()),
            worktree_path: "/tmp/repo".into(),
            agent_session_key: "sess-1".into(),
            previous_session_id: None,
            status: SessionStatus::Active,
            started_at: Utc::now(),
            ended_at: None,
            last_event_at: Utc::now(),
            last_turn_ended_at: None,
            daemon_run_id: new_id(),
            end_reason: None,
            deleted_at: None,
        }
    }

    fn obs(kind: ObservationType, summary: &str) -> Observation {
        Observation {
            id: new_id(),
            session_id: new_id(),
            kind,
            occurred_at: Utc::now(),
            branch: "main".into(),
            commit_sha: None,
            path: None,
            command: None,
            exit_code: None,
            outcome: None,
            summary: summary.into(),
            details: None,
            payload_bytes: summary.len() as i64,
            truncated: false,
            deleted_at: None,
        }
    }

    #[test]
    fn names_changed_file_failing_test_and_next_step() {
        let s = session();
        let mut changed = obs(ObservationType::FileChanged, "edited src/lib.rs");
        changed.path = Some("src/lib.rs".into());
        let mut test = obs(ObservationType::TestRun, "cargo test");
        test.command = Some("cargo test".into());
        test.outcome = Some("failed".into());

        let observations = vec![changed, test];
        let h = synthesize(
            &HandoffInputs {
                session: &s,
                task: None,
                observations: &observations,
                decision_memories: &[],
                repository_state: RepositoryState::default(),
                git_changed_files: &[],
                agent_note: None,
            },
            HandoffTrigger::SessionEnd,
        );

        assert!(h.changed_files.contains(&"src/lib.rs".to_string()));
        assert_eq!(h.tests_executed.len(), 1);
        assert!(h.failures.iter().any(|f| f.contains("cargo test")));
        assert!(h.next_step.contains("cargo test"), "{}", h.next_step);
        assert_eq!(h.trigger, HandoffTrigger::SessionEnd);
    }

    #[test]
    fn reconciles_git_status_with_observations() {
        let s = session();
        let mut changed = obs(ObservationType::FileChanged, "edited a");
        changed.path = Some("a.rs".into());
        let observations = vec![changed];
        let git = vec!["b.rs".to_string(), "a.rs".to_string()];
        let h = synthesize(
            &HandoffInputs {
                session: &s,
                task: None,
                observations: &observations,
                decision_memories: &[],
                repository_state: RepositoryState::default(),
                git_changed_files: &git,
                agent_note: None,
            },
            // `Stop` is not a handoff trigger (FR-032); compaction is.
            HandoffTrigger::PreCompact,
        );
        assert_eq!(
            h.changed_files,
            vec!["a.rs".to_string(), "b.rs".to_string()]
        );
    }

    #[test]
    fn agent_note_is_attributed_and_cannot_replace_derived_fields() {
        let s = session();
        let observations: Vec<Observation> = vec![];
        let h = synthesize(
            &HandoffInputs {
                session: &s,
                task: None,
                observations: &observations,
                decision_memories: &[],
                repository_state: RepositoryState::default(),
                git_changed_files: &[],
                agent_note: Some("I fixed everything".into()),
            },
            HandoffTrigger::SessionEnd,
        );
        assert_eq!(h.agent_note.as_deref(), Some("I fixed everything"));
        // The derived record disagrees with the narrative, and wins.
        assert!(h.changed_files.is_empty());
        assert!(h.progress.contains("0 file(s) changed"));
    }

    #[test]
    fn unmet_acceptance_criteria_become_remaining_work() {
        let s = session();
        let task = Task {
            id: new_id(),
            project_id: s.project_id,
            title: "Rate limiting".into(),
            goal: "Requests over the limit get 429".into(),
            acceptance_criteria: vec!["429 above threshold".into(), "Limit configurable".into()],
            status: TaskStatus::InProgress,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            deleted_at: None,
        };
        let observations: Vec<Observation> = vec![];
        let h = synthesize(
            &HandoffInputs {
                session: &s,
                task: Some(&task),
                observations: &observations,
                decision_memories: &[],
                repository_state: RepositoryState::default(),
                git_changed_files: &[],
                agent_note: None,
            },
            HandoffTrigger::SessionEnd,
        );
        assert_eq!(h.goal, "Requests over the limit get 429");
        assert_eq!(h.remaining_work.len(), 2);
    }

    #[test]
    fn evidence_is_identifiers_only() {
        let s = session();
        let o = obs(ObservationType::FileRead, "read src/lib.rs");
        let id = o.id;
        let observations = vec![o];
        let h = synthesize(
            &HandoffInputs {
                session: &s,
                task: None,
                observations: &observations,
                decision_memories: &[],
                repository_state: RepositoryState::default(),
                git_changed_files: &[],
                agent_note: None,
            },
            HandoffTrigger::Recovered,
        );
        assert_eq!(h.evidence, vec![id]);
    }
}
