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

    // Level 0 first, and before the task: it is the tier defined as the content
    // that is never dropped, and this text is what a `SessionStart` hook injects
    // as the agent's context. A warning that reaches the payload and not the
    // rendering has not reached the agent (FR-464, US3).
    if !b.warnings.is_empty() {
        out.push_str("\n## Warnings\n");
        for w in &b.warnings {
            // The summary warning carries the counts and names no subject of its
            // own, so it reads as the lead line rather than as an item.
            if w.kind == "summary" {
                out.push_str(&format!("{}\n", w.subject));
                continue;
            }
            out.push_str(&format!("⚠ {} {}", w.kind.to_uppercase(), w.subject));
            if !w.detail.is_empty() {
                out.push_str(&format!(" — {}", w.detail));
            }
            out.push('\n');
        }
    }

    if !b.constraints.is_empty() {
        out.push_str("\n## Constraints\n");
        for c in &b.constraints {
            out.push_str(&format!("- {}", c.text));
            // A pin whose claim no longer holds keeps its pin and says so
            // (FR-456).
            if c.drifted {
                out.push_str(" _(the evidence for this has drifted)_");
            }
            out.push('\n');
        }
    }

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

    // Last, and under their own heading. A pattern comes from another project
    // and is offered rather than asserted, so it must not be readable as this
    // project's own memory — which is why it is a separate array on the wire and
    // stays a separate section here.
    if !b.patterns.is_empty() {
        out.push_str("\n## Patterns from other projects (unverified here)\n");
        for p in &b.patterns {
            out.push_str(&format!(
                "- **{}** ({}, {} signal{} matched): {}\n",
                p.title,
                p.trust,
                p.signal_overlap,
                if p.signal_overlap == 1 { "" } else { "s" },
                p.approach
            ));
            if let Some(cause) = &p.alternative_cause {
                out.push_str(&format!("  - another cause found behind this: {cause}\n"));
            }
            if let Some(first) = &p.check_this_first {
                out.push_str(&format!("  - check first: {first}\n"));
            }
        }
    }

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

    // How far subject identity actually reaches here, and what the mechanism
    // is currently reporting (FR-499). Visible in every project, so nobody has
    // to run an evaluation to find out whether any of this is being used.
    if let Some(k) = &s.knowledge {
        if let Some(share) = k.subject_share_percent {
            out.push_str(&format!(
                "Subjects     {share}% of project memory ({} of {})\n",
                k.with_subject, k.project_memories
            ));
        }
        let attention = [
            (k.conflicted_subjects, "conflicted"),
            (k.needs_recheck, "needs recheck"),
            (k.drifted, "drifted"),
        ]
        .into_iter()
        .filter(|(n, _)| *n > 0)
        .map(|(n, label)| format!("{n} {label}"))
        .collect::<Vec<_>>();
        if !attention.is_empty() {
            out.push_str(&format!("Attention    {}\n", attention.join(" · ")));
        }
        if let Some(d) = &k.sync_degradation {
            out.push_str(&format!(
                "Retained     {} item(s) waiting for: {}\n",
                d.blocked,
                d.missing_capabilities.join(", ")
            ));
        }
    }

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

/// Render a subject: its answer or answers, and why (FR-307).
///
/// The reconciliation state leads, because it is the thing a reader needs
/// first — a conflicted subject has no single answer, and presenting one of its
/// members as the answer would be the silent winner this feature exists to
/// prevent.
pub fn subject(v: &serde_json::Value) -> String {
    let s = &v["subject"];
    let mut out = String::new();
    let state = s["reconciliation"].as_str().unwrap_or("unknown");
    out.push_str(&format!(
        "# {} ({}:{})\n\n",
        s["topic_key"].as_str().unwrap_or("?"),
        s["scope"].as_str().unwrap_or("?"),
        s["scope_key"].as_str().unwrap_or("?")
    ));
    out.push_str(&format!("**Reconciliation**: {state}\n"));

    let answers = s["answers"].as_array().cloned().unwrap_or_default();
    match state {
        "historical" => out.push_str("\nNo current answer: every member is history.\n"),
        "conflicted" => out.push_str(&format!(
            "\n⚠ {} competing answers, and no winner. Resolve by superseding one, \
             narrowing its scope, or attaching verification that distinguishes them.\n",
            answers.len()
        )),
        "corroborated" => out.push_str(&format!(
            "\nThe value is agreed and the statements are several: {} distinct statements.\n",
            answers.len()
        )),
        _ => {}
    }

    if !answers.is_empty() {
        out.push_str("\n## Answers\n");
        let accounting = s["accounting"].as_array().cloned().unwrap_or_default();
        for (i, a) in answers.iter().enumerate() {
            let id = a.as_str().unwrap_or("?");
            let member = s["members"]
                .as_array()
                .and_then(|ms| ms.iter().find(|m| m["id"].as_str() == Some(id)));
            let verification = member
                .and_then(|m| m["verification"].as_str())
                .unwrap_or("unverified");
            let authority = member
                .and_then(|m| m["verification_authority"].as_str())
                .map(|a| format!(" ({a})"))
                .unwrap_or_default();
            out.push_str(&format!("- `{id}` — {verification}{authority}"));
            if let Some(acc) = accounting.get(i) {
                let origins = acc["distinct_origins"].as_i64().unwrap_or(1);
                let dupes = acc["duplicates"].as_array().map(|d| d.len()).unwrap_or(0);
                if dupes > 0 {
                    // Never presented as a number of independent verifications
                    // (FR-406).
                    out.push_str(&format!(
                        " · {dupes} duplicate statements · {origins} distinct origins"
                    ));
                }
            }
            out.push('\n');
        }
    }

    let narrowed = s["narrowed_by"].as_array().cloned().unwrap_or_default();
    if !narrowed.is_empty() {
        out.push_str("\n## Narrowed by\n");
        for n in &narrowed {
            out.push_str(&format!("- `{}`\n", n.as_str().unwrap_or("?")));
        }
    }

    let decisions = s["decisions"].as_array().cloned().unwrap_or_default();
    if !decisions.is_empty() {
        out.push_str("\n## Decisions\n");
        for d in &decisions {
            out.push_str(&format!(
                "- {} `{}` → `{}` ({})\n",
                d["kind"].as_str().unwrap_or("?"),
                d["from"].as_str().unwrap_or("?"),
                d["to"].as_str().unwrap_or("?"),
                d["basis"].as_str().unwrap_or("?")
            ));
        }
    }

    let elevation = s["elevation_candidates"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    if !elevation.is_empty() {
        out.push_str("\n## Elevation candidates\n");
        out.push_str(
            "Reported, never applied. A merge does not make branch knowledge project \
             knowledge; that takes an explicit decision.\n",
        );
        for c in &elevation {
            out.push_str(&format!(
                "- `{}` from `{}`\n",
                c["memory_id"].as_str().unwrap_or("?"),
                c["branch"].as_str().unwrap_or("?")
            ));
        }
    }

    if s["degraded"].as_bool().unwrap_or(false) {
        out.push_str("\n⚠ The subject read hit its bound; some members were not examined.\n");
    }
    out
}

/// Render a verification result.
///
/// **Never bare.** Every line that shows a verification state shows its
/// authority, because collapsing the two is exactly what lets an agent's own
/// assertion wear a deterministic check's badge (FR-370).
pub fn verification(v: &serde_json::Value) -> String {
    if let Some(state) = v["verification"].as_str() {
        let authority = v["authority"].as_str();
        let mut out = format!("{}\n", authority_line(state, authority));
        if let Some(runs) = v["runs"].as_array() {
            out.push_str("\n## Runs\n");
            for r in runs {
                out.push_str(&format!(
                    "- {} → {} at {} ({})\n",
                    r["verifier"].as_str().unwrap_or("?"),
                    r["result"].as_str().unwrap_or("?"),
                    r["checked_at"].as_str().unwrap_or("?"),
                    r["triggered_by"].as_str().unwrap_or("?"),
                ));
                if let Some(detail) = r["detail"].as_str() {
                    out.push_str(&format!("  {detail}\n"));
                }
            }
        }
        return out;
    }

    // A whole pass.
    let mut out = format!(
        "Examined {} facts · {} runs recorded · {} memories updated\n",
        v["facts_examined"].as_i64().unwrap_or(0),
        v["runs_recorded"].as_i64().unwrap_or(0),
        v["memories_updated"].as_i64().unwrap_or(0),
    );
    if v["notes"]
        .as_array()
        .map(|n| n.iter().any(|x| x.as_str() == Some("verify_pass_yielded")))
        .unwrap_or(false)
    {
        out.push_str("The pass hit a cap and yielded; the rest is queued for the next tick.\n");
    }
    out
}

/// The four renderings a verification state can have, and they are four rather
/// than one on purpose (`contracts/evidence-verification.md` §How it is
/// reported).
pub fn authority_line(state: &str, authority: Option<&str>) -> String {
    match (state, authority) {
        ("verified", Some("cairn")) => "✓ verified                      (authority: cairn)".into(),
        ("verified", Some("attested")) => {
            "✓ verified (attested)           (authority: attested)".into()
        }
        ("verified", Some("remote_cairn")) => {
            "✓ verified elsewhere            (authority: remote_cairn)".into()
        }
        ("verified", Some("remote_attested")) => {
            "✓ verified elsewhere (attested) (authority: remote_attested)".into()
        }
        ("verified", None) => "✓ verified                      (authority: unknown)".into(),
        (other, _) => format!("· {other}"),
    }
}

/// Render evidence facts. A deleted fact says so rather than vanishing.
pub fn evidence_list(v: &serde_json::Value) -> String {
    let items = match v["evidence"].as_array() {
        Some(a) => a.clone(),
        None => vec![v["evidence"].clone()],
    };
    if items.is_empty() {
        return "No evidence.\n".to_string();
    }
    let mut out = String::new();
    for e in &items {
        if e["deleted"].as_bool().unwrap_or(false) {
            out.push_str(&format!(
                "- `{}` {} — evidence deleted\n",
                e["id"].as_str().unwrap_or("?"),
                e["kind"].as_str().unwrap_or("?"),
            ));
            continue;
        }
        out.push_str(&format!(
            "- `{}` {} [{}] {} = {}\n  at {}\n",
            e["id"].as_str().unwrap_or("?"),
            e["kind"].as_str().unwrap_or("?"),
            e["collector"].as_str().unwrap_or("?"),
            e["subject"].as_str().unwrap_or("?"),
            e["observed_value"].as_str().unwrap_or("—"),
            e["source_locator"].as_str().unwrap_or("—"),
        ));
        if let Some(role) = e["role"].as_str() {
            out.push_str(&format!("  role: {role}\n"));
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Task work state (`contracts/task-model.md` §Surfaces)
// ---------------------------------------------------------------------------

/// One criterion, with both axes named.
///
/// `satisfied` and `unverified` are printed side by side rather than folded
/// into one word, because "the agent says it is done and nothing has checked"
/// is exactly what a reader needs to be able to see (FR-483).
pub fn criterion_line(c: &serde_json::Value) -> String {
    let label = c["label"].as_str().unwrap_or("?");
    let state = c["state"].as_str().unwrap_or("?");
    let verification = c["verification"].as_str().unwrap_or("?");
    let text = c["text"].as_str().unwrap_or("");
    let revision = c["revision"].as_i64().unwrap_or(0);
    format!("{label}  {state} · {verification}  (rev {revision})  {text}\n")
}

/// The derived counts, the open blockers and the readiness.
pub fn readiness(v: &serde_json::Value) -> String {
    let p = &v["progress"];
    let n = |k: &str| p[k].as_u64().unwrap_or(0);
    let mut out = format!(
        "PROGRESS  {} verified · {} satisfied but unverified · {} blocked · {} pending",
        n("verified"),
        n("satisfied_unverified"),
        n("blocked"),
        n("pending")
    );
    if n("waived") > 0 {
        out.push_str(&format!(" · {} waived", n("waived")));
    }
    out.push('\n');
    let open = v["open_blockers"].as_u64().unwrap_or(0);
    if open > 0 {
        out.push_str(&format!("BLOCKERS  {open} open\n"));
    }
    out.push_str(&format!(
        "READINESS {}\n",
        v["completion_readiness"].as_str().unwrap_or("?")
    ));
    out
}

/// `cairn task get`'s work-state block.
pub fn task_work_state(v: &serde_json::Value) -> String {
    let mut out = String::new();
    if let Some(rev) = v["local_revision"].as_i64() {
        out.push_str(&format!("Revision: {rev} (local)\n"));
    }
    if let Some(d) = v["state_digest"].as_str() {
        out.push_str(&format!("State:    {}\n", &d[..d.len().min(16)]));
    }
    if let Some(cs) = v["criteria"].as_array() {
        if !cs.is_empty() {
            out.push_str("Acceptance criteria:\n");
            for c in cs {
                out.push_str("  ");
                out.push_str(&criterion_line(c));
            }
        }
    }
    if let Some(bs) = v["blockers"].as_array() {
        for b in bs.iter().filter(|b| b["state"] == "open") {
            out.push_str(&format!(
                "BLOCKER   {}\n",
                b["description"].as_str().unwrap_or("")
            ));
        }
    }
    if v["progress"].is_object() {
        out.push_str(&readiness(v));
    }
    out
}

/// The change log, including the blind writes.
pub fn task_history(v: &serde_json::Value) -> String {
    let Some(changes) = v["changes"].as_array() else {
        return "No changes recorded.\n".into();
    };
    if changes.is_empty() {
        return "No changes recorded.\n".into();
    }
    let mut out = String::new();
    for c in changes {
        let blind = if c["blind_write"].as_bool().unwrap_or(false) {
            "  [blind write — no expected_revision supplied]"
        } else {
            ""
        };
        out.push_str(&format!(
            "r{}  {}  {} → {}{}\n",
            c["local_revision"].as_i64().unwrap_or(0),
            c["kind"].as_str().unwrap_or("?"),
            c["prior_value"].as_str().unwrap_or("—"),
            c["new_value"].as_str().unwrap_or("—"),
            blind
        ));
    }
    out
}

/// The selection table — the answer to "why did Cairn tell the agent this?"
/// (FR-462).
pub fn selection(s: &cairn_core::wire::Selection) -> String {
    let mut out = format!(
        "\nbudget {} · reserve {} · reserve used {} · released {}\n",
        s.budget, s.reserve, s.reserve_used, s.reserve_released
    );
    if !s.included.is_empty() {
        out.push_str("INCLUDED\n");
        for item in &s.included {
            let reasons: Vec<&str> = item.reasons.iter().map(|r| r.as_str()).collect();
            out.push_str(&format!(
                "  {:<13} {:<10} {:<36} {}\n",
                item.level.as_str(),
                item.kind,
                reasons.join(" "),
                item.cost
            ));
        }
    }
    if !s.omitted.is_empty() {
        out.push_str("OMITTED\n");
        for item in &s.omitted {
            let retrieval = if item.retrieval.is_empty() {
                String::new()
            } else {
                format!("  — `{}`", item.retrieval)
            };
            out.push_str(&format!(
                "  {} ×{:<8} {}{}\n",
                item.kind,
                item.count,
                item.reason.as_str(),
                retrieval
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

    /// Level 0 reaches the rendered text, not just the payload (FR-464, US3).
    ///
    /// This text is what a `SessionStart` hook injects, so it *is* the agent's
    /// context on that path. The renderer used to walk straight from the
    /// repository line to the task, dropping every warning and every pinned
    /// constraint on the floor — the tier defined as the one that is never
    /// dropped.
    #[test]
    fn briefing_renders_level0_warnings_and_constraints() {
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
                warnings: vec![
                    cairn_core::wire::ContextWarning {
                        kind: "summary".to_string(),
                        subject: "1 conflict · 1 drift".to_string(),
                        detail: String::new(),
                    },
                    cairn_core::wire::ContextWarning {
                        kind: "conflict".to_string(),
                        subject: "deploy.queue_backend".to_string(),
                        detail: "2 competing answers (rabbitmq, sqs)".to_string(),
                    },
                    cairn_core::wire::ContextWarning {
                        kind: "drift".to_string(),
                        subject: "service.api_port".to_string(),
                        detail: "drifted".to_string(),
                    },
                ],
                constraints: vec![cairn_core::wire::PinnedConstraint {
                    id: Uuid::nil(),
                    text: "never log request bodies".to_string(),
                    drifted: true,
                }],
                previous_next_action: None,
                patterns: Vec::new(),
            },
            estimated_tokens: 100,
            budget: 1000,
            truncated: false,
            omitted_sections: vec![],
            degraded: false,
            selection: None,
        };

        let text = briefing(&p);
        // The line the quickstart documents, verbatim in shape.
        assert!(
            text.contains("⚠ CONFLICT deploy.queue_backend"),
            "the conflict warning is not in the rendered briefing:\n{text}"
        );
        assert!(text.contains("⚠ DRIFT service.api_port"), "{text}");
        assert!(text.contains("1 conflict · 1 drift"), "{text}");
        assert!(text.contains("never log request bodies"), "{text}");
        assert!(
            text.contains("drifted"),
            "a pin whose evidence moved must say so: {text}"
        );
    }

    /// A project with no Level 0 content renders exactly what it always did —
    /// the no-regression property, on the rendering side (FR-442).
    #[test]
    fn a_briefing_with_no_level0_content_gains_no_sections() {
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
                warnings: Vec::new(),
                constraints: Vec::new(),
                previous_next_action: None,
                patterns: Vec::new(),
            },
            estimated_tokens: 100,
            budget: 1000,
            truncated: false,
            omitted_sections: vec![],
            degraded: false,
            selection: None,
        };
        let text = briefing(&p);
        assert!(!text.contains("## Warnings"), "{text}");
        assert!(!text.contains("## Constraints"), "{text}");
        assert!(!text.contains("## Patterns"), "{text}");
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
                warnings: Vec::new(),
                constraints: Vec::new(),
                previous_next_action: None,
                patterns: Vec::new(),
            },
            estimated_tokens: 100,
            budget: 1000,
            truncated: false,
            omitted_sections: vec![],
            degraded: false,
            selection: None,
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
                warnings: Vec::new(),
                constraints: Vec::new(),
                previous_next_action: None,
                patterns: Vec::new(),
            },
            estimated_tokens: 999,
            budget: 1000,
            truncated: true,
            omitted_sections: vec!["decisions".to_string()],
            degraded: false,
            selection: None,
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
            knowledge: None,
            handoff_synthesis_failures: vec![],
        };
        let text = status(&s);
        assert!(text.contains("Sharing      local only"));
        assert!(text.contains("Sessions     none active"));
    }
}
