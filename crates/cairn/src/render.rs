//! Human-readable rendering of briefings and handoffs.
//!
//! The briefing text is what a `SessionStart` hook injects as context, so it is
//! shaped for an agent to read at a glance rather than for a terminal.

use cairn_core::domain::Handoff;
use cairn_core::wire::{ContextPayload, StatusPayload};

pub fn briefing(payload: &ContextPayload) -> String {
    let b = &payload.briefing;
    let mut out = String::new();
    out.push_str("# Cairn context\n\n");

    if b.no_prior_history {
        out.push_str("Cairn has no prior history for this project yet.\n\n");
    }
    if payload.degraded {
        out.push_str("_Reduced context: Cairn could not assemble the full briefing in time._\n\n");
    }

    out.push_str(&format!("**Project**: {}\n", b.project.name));
    let r = &b.repository;
    out.push_str(&format!(
        "**Repository**: branch `{}`, commit `{}`, working tree {}\n",
        r.branch,
        r.commit_sha.as_deref().unwrap_or("(none)"),
        if r.is_clean() {
            "clean".to_string()
        } else {
            format!(
                "{} staged, {} unstaged, {} untracked",
                r.staged, r.unstaged, r.untracked
            )
        }
    ));

    if let Some(t) = &b.task {
        out.push_str(&format!(
            "\n## Task: {} ({})\n{}\n",
            t.title, t.status, t.goal
        ));
        if !t.acceptance_criteria.is_empty() {
            out.push_str("\nAcceptance criteria:\n");
            for c in &t.acceptance_criteria {
                out.push_str(&format!("- {c}\n"));
            }
        }
    }

    if let Some(h) = &b.previous_handoff {
        out.push_str("\n## Previous session\n");
        out.push_str(&format!("Next step: {}\n", h.next_step));
        if !h.remaining_work.is_empty() {
            out.push_str("\nRemaining work:\n");
            for w in &h.remaining_work {
                out.push_str(&format!("- {w}\n"));
            }
        }
        if !h.changed_files.is_empty() {
            out.push_str(&format!(
                "\nChanged files: {}\n",
                h.changed_files.join(", ")
            ));
        }
    }

    section(&mut out, "Known failures", &b.known_failures);
    section(&mut out, "Decisions", &b.decisions);
    section(&mut out, "Task memory", &b.memory.task);
    section(&mut out, "Branch memory", &b.memory.branch);
    section(&mut out, "Project memory", &b.memory.project);

    out.push_str(&format!(
        "\n---\n{} of {} estimated tokens",
        payload.estimated_tokens, payload.budget
    ));
    if payload.truncated {
        out.push_str(&format!(
            "; omitted: {}",
            payload.omitted_sections.join(", ")
        ));
    }
    out.push('\n');
    out
}

fn section(out: &mut String, title: &str, items: &[String]) {
    if items.is_empty() {
        return;
    }
    out.push_str(&format!("\n## {title}\n"));
    for i in items {
        out.push_str(&format!("- {i}\n"));
    }
}

pub fn handoff(h: &Handoff) -> String {
    let mut out = String::new();
    out.push_str(&format!("# Handoff ({})\n\n", h.trigger));
    out.push_str(&format!("**Goal**: {}\n", h.goal));
    out.push_str(&format!("**Progress**: {}\n", h.progress));
    out.push_str(&format!("**Next step**: {}\n", h.next_step));

    list(&mut out, "Completed", &h.completed_work);
    list(&mut out, "Remaining", &h.remaining_work);
    list(&mut out, "Changed files", &h.changed_files);
    list(&mut out, "Decisions", &h.decisions);
    list(&mut out, "Failures", &h.failures);

    if !h.tests_executed.is_empty() {
        out.push_str("\n## Tests executed\n");
        for t in &h.tests_executed {
            out.push_str(&format!("- {} — {}\n", t.command, t.outcome));
        }
    }

    let r = &h.repository_state;
    out.push_str(&format!(
        "\n## Repository\nbranch `{}`, commit `{}`, {} staged / {} unstaged / {} untracked\n",
        r.branch,
        r.commit_sha.as_deref().unwrap_or("(none)"),
        r.staged,
        r.unstaged,
        r.untracked
    ));

    if let Some(note) = &h.agent_note {
        out.push_str(&format!("\n## Agent note (unverified)\n{note}\n"));
    }
    out.push_str(&format!(
        "\n_{} supporting observation(s)_\n",
        h.evidence.len()
    ));
    out
}

fn list(out: &mut String, title: &str, items: &[String]) {
    if items.is_empty() {
        return;
    }
    out.push_str(&format!("\n## {title}\n"));
    for i in items {
        out.push_str(&format!("- {i}\n"));
    }
}

pub fn status(s: &StatusPayload) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "Project      {} ({})\n",
        s.project.name, s.project.id
    ));
    // "linked" alone never said *to what*, so learning where work goes took
    // three commands.
    out.push_str(&format!(
        "Sharing      {}\n",
        match (s.project.linked, s.server_url.as_deref()) {
            (true, Some(url)) => format!("linked to {url}"),
            (true, None) => "linked, but no server is configured".to_string(),
            (false, _) => "local only".to_string(),
        }
    ));
    if s.project.linked && !s.authenticated {
        out.push_str("             no API token stored — run `cairn auth token set`\n");
    }
    out.push_str(&format!("Worktree     {}\n", s.worktree_path));
    out.push_str(&format!(
        "Branch       {} @ {}\n",
        s.repository.branch,
        s.repository.commit_sha.as_deref().unwrap_or("(no commits)")
    ));
    out.push_str(&format!(
        "Working tree {} staged, {} unstaged, {} untracked\n",
        s.repository.staged, s.repository.unstaged, s.repository.untracked
    ));
    out.push_str(&format!("Integration  {}\n", s.integration_mode));
    out.push_str(&format!("Daemon       {}\n", s.daemon));
    out.push_str(&format!(
        "Recorded     {} observations, {} memories\n",
        s.observation_count, s.memory_count
    ));

    if s.sessions.is_empty() {
        out.push_str("Sessions     none active\n");
    } else {
        out.push_str(&format!("Sessions     {} active\n", s.sessions.len()));
        for session in &s.sessions {
            out.push_str(&format!(
                "  {}  {}  idle {}s\n",
                session.id, session.agent, session.idle_seconds
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_core::domain::{HandoffTrigger, RepositoryState};
    use cairn_core::wire::{
        Briefing, BriefingMemory, ContextPayload, ProjectSummary, StatusPayload,
    };
    use uuid::Uuid;

    fn project_summary() -> ProjectSummary {
        ProjectSummary {
            id: Uuid::nil(),
            name: "demo".to_string(),
            linked: false,
            server_project_id: None,
        }
    }

    fn clean_repo() -> RepositoryState {
        RepositoryState {
            branch: "main".to_string(),
            commit_sha: Some("abc".to_string()),
            staged: 0,
            unstaged: 0,
            untracked: 0,
        }
    }

    #[test]
    fn briefing_reports_no_prior_history_when_set() {
        let p = ContextPayload {
            briefing: Briefing {
                project: project_summary(),
                repository: clean_repo(),
                task: None,
                previous_handoff: None,
                decisions: vec![],
                known_failures: vec![],
                memory: BriefingMemory::default(),
                no_prior_history: true,
            },
            estimated_tokens: 100,
            budget: 1000,
            truncated: false,
            omitted_sections: vec![],
            degraded: false,
        };
        let text = briefing(&p);
        assert!(text.contains("no prior history"), "missing no-history line");
        assert!(text.contains("Project**: demo"));
    }

    #[test]
    fn briefing_notes_truncated_omitted_sections() {
        let p = ContextPayload {
            briefing: Briefing {
                project: project_summary(),
                repository: clean_repo(),
                task: None,
                previous_handoff: None,
                decisions: vec![],
                known_failures: vec![],
                memory: BriefingMemory::default(),
                no_prior_history: false,
            },
            estimated_tokens: 999,
            budget: 1000,
            truncated: true,
            omitted_sections: vec!["decisions".to_string()],
            degraded: false,
        };
        let text = briefing(&p);
        assert!(
            text.contains("omitted: decisions"),
            "missing omitted sections"
        );
    }

    #[test]
    fn handoff_shows_trigger_and_next_step() {
        let h = Handoff {
            id: Uuid::nil(),
            session_id: Uuid::nil(),
            trigger: HandoffTrigger::SessionEnd,
            goal: "ship".to_string(),
            progress: "halfway".to_string(),
            completed_work: vec![],
            remaining_work: vec![],
            changed_files: vec![],
            decisions: vec![],
            failures: vec![],
            tests_executed: vec![],
            repository_state: clean_repo(),
            next_step: "run tests".to_string(),
            agent_note: None,
            evidence: vec![],
            created_at: chrono::Utc::now(),
            deleted_at: None,
        };
        let text = handoff(&h);
        assert!(text.contains("session_end"), "missing trigger");
        assert!(text.contains("Next step**: run tests"));
        assert!(text.contains("0 supporting observation(s)"));
    }

    #[test]
    fn status_local_only_sharing_line() {
        let s = StatusPayload {
            project: project_summary(),
            repository: clean_repo(),
            worktree_path: "/tmp/wt".to_string(),
            sessions: vec![],
            integration_mode: "manual".to_string(),
            daemon: "running".to_string(),
            observation_count: 0,
            memory_count: 0,
            server_url: None,
            authenticated: false,
            version: None,
            local_schema_version: 0,
            sessions_awaiting_handoff: 0,
            handoff_synthesis_failures: vec![],
        };
        let text = status(&s);
        assert!(text.contains("Sharing      local only"));
        assert!(text.contains("Sessions     none active"));
    }
}
