//! Human-readable rendering used by hook and MCP adapters.

use cairn_core::wire::ContextPayload;

/// Render the canonical server envelope without inventing local history.
/// Unknown populated sections fail closed so transmission cannot overclaim.
pub fn context(value: &serde_json::Value) -> Result<String, String> {
    if value.get("briefing").is_some() {
        let payload = serde_json::from_value(value.clone()).map_err(|e| format!("{e}"))?;
        return Ok(format!("{}{}", continuity(value), briefing(&payload)));
    }
    let invalid = || "invalid server context; retry with cairn_context".to_string();
    let sections = value["sections"].as_object().ok_or_else(invalid)?;
    let titles = [
        ("session_memory", "Session memory"),
        ("branch_memory", "Branch memory"),
        ("project_memory", "Project memory"),
        ("patterns", "Patterns (unverified here)"),
        ("personal_notes", "Personal notes"),
        ("team_guidance", "Team guidance"),
    ];
    for (name, items) in sections {
        let items = items.as_array().ok_or_else(invalid)?;
        if !items.is_empty() && !titles.iter().any(|(key, _)| key == name) {
            return Err(invalid());
        }
    }
    let mut out = String::from("# Cairn context\n\n");
    if value["served_from_cache"] == true {
        let age = value["cache_age_seconds"].as_u64().ok_or_else(invalid)?;
        out.push_str(&format!(
            "_Cached context, {age}s old; the server is unavailable._\n\n"
        ));
    } else if value["fresh_knowledge_unavailable"] == true {
        out.push_str("_Fresh knowledge is unavailable; retry cairn_context with your own agent_session_key and cwd._\n");
        if !sections.is_empty() {
            return Err(invalid());
        }
        return Ok(out);
    }
    let tokens = value["budget"]["tokens"].as_u64().ok_or_else(invalid)?;
    let spent = value["budget"]["spent"].as_u64().ok_or_else(invalid)?;
    if spent > tokens {
        return Err(invalid());
    }
    if value["degradation_level"] != "full" {
        out.push_str("_Reduced context; some guidance may be unavailable._\n\n");
    }
    let mut count = 0;
    for (key, title) in titles {
        let Some(items) = sections.get(key) else {
            continue;
        };
        let texts: Vec<String> = items
            .as_array()
            .ok_or_else(invalid)?
            .iter()
            .map(|item| {
                item["content"]
                    .as_str()
                    .map(str::to_owned)
                    .ok_or_else(invalid)
            })
            .collect::<Result<_, _>>()?;
        count += texts.len();
        section(&mut out, title, &texts);
    }
    if count == 0 {
        out.push_str("No applicable memory was selected.\n");
    }
    out.push_str(&format!("\n---\n{spent} of {tokens} estimated tokens\n"));
    Ok(out)
}

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

#[cfg(test)]
mod context_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn canonical_context_renders_all_selected_sections_and_honest_fallback() {
        let mut payload = json!({
            "budget": {"tokens": 3000, "spent": 12}, "degradation_level": "full",
            "sections": {
                "session_memory": [{"content": "caller decision"}],
                "project_memory": [{"content": "supported procedure"}],
                "patterns": [{"content": "canonical pattern approach"}]
            }
        });
        let text = context(&payload).unwrap();
        for expected in [
            "caller decision",
            "supported procedure",
            "canonical pattern approach",
            "12 of 3000",
        ] {
            assert!(text.contains(expected));
        }
        payload["served_from_cache"] = json!(true);
        payload["cache_age_seconds"] = json!(24);
        assert!(context(&payload).unwrap().contains("24s old"));
        payload["sections"]["unknown"] = json!([{"content": "unrendered claim"}]);
        assert!(context(&payload).is_err());
        payload["sections"] = json!({"project_memory": [{"content": 7}]});
        assert!(context(&payload).is_err());
        assert!(
            context(&json!({"fresh_knowledge_unavailable":true,"sections":{}}))
                .unwrap()
                .contains("Fresh knowledge is unavailable")
        );
        assert!(context(
            &json!({"budget":{"tokens":3,"spent":0},"degradation_level":"full","sections":{}})
        )
        .unwrap()
        .contains("No applicable memory"));
    }
}
