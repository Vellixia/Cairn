//! Human-readable rendering used by hook and MCP adapters.

use cairn_core::wire::ContextPayload;

pub fn briefing(payload: &ContextPayload) -> String {
    let briefing = &payload.briefing;
    let mut out = String::from("# Cairn context\n\n");

    if briefing.no_prior_history {
        out.push_str("Cairn has no prior history for this project yet.\n\n");
    }
    if payload.degraded {
        out.push_str("_Reduced context: Cairn could not assemble the full briefing in time._\n\n");
    }

    out.push_str(&format!("**Project**: {}\n", briefing.project.name));
    let repository = &briefing.repository;
    out.push_str(&format!(
        "**Repository**: branch `{}`, commit `{}`, working tree {}\n",
        repository.branch,
        repository.commit_sha.as_deref().unwrap_or("(none)"),
        if repository.is_clean() {
            "clean".to_string()
        } else {
            format!(
                "{} staged, {} unstaged, {} untracked",
                repository.staged, repository.unstaged, repository.untracked
            )
        }
    ));

    if !briefing.warnings.is_empty() {
        out.push_str("\n## Warnings\n");
        for warning in &briefing.warnings {
            if warning.kind == "summary" {
                out.push_str(&format!("{}\n", warning.subject));
                continue;
            }
            out.push_str(&format!(
                "⚠ {} {}",
                warning.kind.to_uppercase(),
                warning.subject
            ));
            if !warning.detail.is_empty() {
                out.push_str(&format!(" — {}", warning.detail));
            }
            out.push('\n');
        }
    }

    if !briefing.constraints.is_empty() {
        out.push_str("\n## Constraints\n");
        for constraint in &briefing.constraints {
            out.push_str(&format!("- {}", constraint.text));
            if constraint.drifted {
                out.push_str(" _(the evidence for this has drifted)_");
            }
            out.push('\n');
        }
    }

    if let Some(handoff) = &briefing.previous_handoff {
        out.push_str("\n## Previous session\n");
        out.push_str(&format!("Next step: {}\n", handoff.next_step));
        if !handoff.remaining_work.is_empty() {
            out.push_str("\nRemaining work:\n");
            for item in &handoff.remaining_work {
                out.push_str(&format!("- {item}\n"));
            }
        }
        if !handoff.changed_files.is_empty() {
            out.push_str(&format!(
                "\nChanged files: {}\n",
                handoff.changed_files.join(", ")
            ));
        }
    }

    section(&mut out, "Known failures", &briefing.known_failures);
    section(&mut out, "Decisions", &briefing.decisions);
    section(&mut out, "Branch memory", &briefing.memory.branch);
    section(&mut out, "Project memory", &briefing.memory.project);

    if !briefing.patterns.is_empty() {
        out.push_str("\n## Patterns from other projects (unverified here)\n");
        for pattern in &briefing.patterns {
            let matched = match pattern.signal_overlap {
                Some(1) => " (1 signal matched)".to_string(),
                Some(count) => format!(" ({count} signals matched)"),
                None => String::new(),
            };
            out.push_str(&format!(
                "- **{}** ({}{}): {}\n",
                pattern.title, pattern.trust, matched, pattern.approach
            ));
            if let Some(cause) = &pattern.alternative_cause {
                out.push_str(&format!("  - another cause found behind this: {cause}\n"));
            }
            if let Some(first) = &pattern.check_this_first {
                out.push_str(&format!("  - check first: {first}\n"));
            }
        }
    }

    section(&mut out, "Personal notes", &briefing.personal_notes);
    section(&mut out, "Team guidance", &briefing.team_guidance);

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
    for item in items {
        out.push_str(&format!("- {item}\n"));
    }
}

pub fn continuity(value: &serde_json::Value) -> String {
    let checkpoint = &value["checkpoint"];
    let mut out = String::new();
    let state = checkpoint["classification"]["state"].as_str().unwrap_or("");

    if state == "diverged" {
        out.push_str("⚠ CHECKPOINT DIVERGED\n");
        for divergence in checkpoint["classification"]["divergences"]
            .as_array()
            .into_iter()
            .flatten()
        {
            let kind = divergence["kind"].as_str().unwrap_or("?");
            let recorded = divergence["recorded"].as_str().unwrap_or("?");
            let current = divergence["current"].as_str().unwrap_or("?");
            match kind {
                "commit" => out.push_str(&format!(
                    "    recorded at {}\n    now at      {}\n",
                    short(recorded),
                    short(current)
                )),
                "files" => out.push_str(&format!("    files changed: {current}\n")),
                other => out.push_str(&format!("    {other}: {recorded} → {current}\n")),
            }
        }
        for path in checkpoint["classification"]["paths"]
            .as_array()
            .into_iter()
            .flatten()
        {
            if path["outcome"].as_str().unwrap_or("") != "unchanged" {
                out.push_str(&format!(
                    "      {}  ({}, {})\n",
                    path["path"].as_str().unwrap_or("?"),
                    path["outcome"].as_str().unwrap_or("?"),
                    path["current_class"].as_str().unwrap_or("?")
                ));
            }
        }
    }

    if state == "diverged" {
        if let Some(previous) = value["briefing"]["previous_next_action"]
            .as_str()
            .or_else(|| checkpoint["previous_next_action"].as_str())
        {
            out.push_str(&format!(
                "    previous next action (may be stale):\n        \"{previous}\"\n"
            ));
        }
    }

    if !out.is_empty() {
        out.push('\n');
    }
    out
}

fn short(sha: &str) -> String {
    sha.chars().take(12).collect()
}
