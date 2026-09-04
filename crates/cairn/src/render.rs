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
        // Both axes, never collapsed: what somebody *asserted* about a
        // criterion and what Cairn *checked* are different claims, and printing
        // one of them would be the collapse FR-483 exists to prevent. The
        // payload has carried these since Tier 0a landed; nothing rendered
        // them, so the agent reading the briefing saw a list of sentences and
        // could not tell which were done.
        if !t.criteria.is_empty() {
            out.push_str("\nAcceptance criteria:\n");
            for c in &t.criteria {
                out.push_str(&format!(
                    "- {} {} · {} — {}\n",
                    c.label, c.state, c.verification, c.text
                ));
            }
            if let Some(omitted) = t.criteria_omitted {
                out.push_str(&format!(
                    "  (+{omitted} more — `cairn task show {}`)\n",
                    t.id
                ));
            }
        } else if !t.acceptance_criteria.is_empty() {
            // A Feature 001 payload, which carries the text and nothing else.
            out.push_str("\nAcceptance criteria:\n");
            for c in &t.acceptance_criteria {
                out.push_str(&format!("- {c}\n"));
            }
        }

        // Counts, never a percentage: there is no field for one (FR-486).
        if let Some(p) = &t.progress {
            out.push_str(&format!(
                "\nProgress: {} verified · {} satisfied but unverified · {} blocked · {} pending",
                p.verified, p.satisfied_unverified, p.blocked, p.pending
            ));
            if p.waived > 0 {
                out.push_str(&format!(" · {} waived", p.waived));
            }
            out.push('\n');
        }
        if let Some(r) = &t.completion_readiness {
            out.push_str(&format!("Readiness: {r}\n"));
        }
        if let Some(blocker) = &t.blocker {
            let more = match t.open_blockers {
                Some(n) if n > 1 => format!(" (+{} more open)", n - 1),
                _ => String::new(),
            };
            out.push_str(&format!("Blocked by: {blocker}{more}\n"));
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

    // Personal, then team — the last two sections in `SECTION_ORDER`, and the
    // last two rendered here for the same reason: project truth outranks
    // knowledge that was never observed against this project at all
    // (`contracts/recall-composition.md` §4). Plain lines, exactly like
    // `known_failures`/`decisions` above — there is no field for a selection
    // reason to occupy (FR-478, D451).
    section(&mut out, "Personal notes", &b.personal_notes);
    section(&mut out, "Team guidance", &b.team_guidance);

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
            out.push_str(&format!("- {} — {}\n", t.runner, t.outcome));
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

/// A stored instant as an age a person can read.
///
/// Coarse on purpose. The question is "is this minutes or days", and a
/// spurious-precision "3h07m12s" invites someone to read a difference that
/// means nothing. An unparsable value is shown as-is rather than hidden: a
/// timestamp the daemon could not produce properly is itself worth seeing.
fn age_of(rfc3339: &str) -> String {
    let Ok(then) = chrono::DateTime::parse_from_rfc3339(rfc3339) else {
        return rfc3339.to_string();
    };
    let secs = (chrono::Utc::now() - then.with_timezone(&chrono::Utc)).num_seconds();
    match secs {
        s if s < 0 => "just now".to_string(),
        s if s < 90 => format!("{s}s ago"),
        s if s < 5400 => format!("{}m ago", s / 60),
        s if s < 172_800 => format!("{}h ago", s / 3600),
        s => format!("{}d ago", s / 86_400),
    }
}

/// The closed `blocked_reason` vocabulary, said in words.
///
/// The wire carries a token so a caller can branch on it; a person reads this.
/// An unknown token is passed through rather than dropped — a newer daemon
/// talking to an older CLI should still say *something*, and inventing
/// "unknown" would hide the very state that was added.
fn blocked_phrase(reason: &str) -> String {
    match reason {
        "saturated" => "the queue is full and new work is being refused".to_string(),
        "retry_exhausted" => "some rows ran out of attempts and will not retry".to_string(),
        "refused_by_server" => "the server refused some rows permanently".to_string(),
        "awaiting_capability" => "waiting for a server that can accept them".to_string(),
        "backing_off" => "retrying after a failure".to_string(),
        "no_account" => "nobody is signed in, so nothing can be delivered".to_string(),
        other => other.to_string(),
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

    // The spool, when it holds anything (FR-792).
    //
    // Three things and all three are required: the depth, the age of the oldest
    // entry, and why delivery is not progressing. The depth alone is the number
    // people already had and it is the least useful of them — fifty rows a
    // second old is a busy minute, one row a week old is an outage nobody
    // noticed, and only the age tells them apart. The reason is what turns
    // noticing into acting.
    //
    // Silent when nothing is queued, because a line reading "0 waiting" on every
    // healthy machine is a line people stop reading.
    if let Some(c) = &s.capture {
        for (label, spool) in [("Events", &c.events), ("Commands", &c.commands)] {
            if spool.undelivered == 0 && spool.terminal == 0 {
                continue;
            }
            let mut line = format!("{} undelivered", spool.undelivered);
            if let Some(oldest) = &spool.oldest_at {
                line.push_str(&format!(", oldest {}", age_of(oldest)));
            }
            if spool.terminal > 0 {
                line.push_str(&format!(", {} given up on", spool.terminal));
            }
            if let Some(reason) = &spool.blocked_reason {
                line.push_str(&format!(" — {}", blocked_phrase(reason)));
            }
            out.push_str(&format!("Queue        {label}: {line}\n"));
        }
    }

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

/// What a write did to the subject it joined, and the one call that would
/// settle it (FR-327, D2).
///
/// The daemon has always returned this; nothing rendered it, so the prompt
/// FR-327 depends on reached the JSON and never a human. A corroborating write
/// merges nothing on purpose — the value agrees and the statements differ, and
/// only a reader can say whether they are one claim. That is a decision Cairn
/// deliberately declines, so it has to be *asked for*, in the place the writer
/// is already looking.
///
/// `created` with no subject renders nothing: a free-form memory took part in
/// no reconciliation, and saying "created" would imply it did.
pub fn reconciliation(v: &serde_json::Value) -> String {
    let r = &v["reconciliation"];
    let outcome = match r["outcome"].as_str() {
        Some(o) => o,
        None => return String::new(),
    };
    let subject = r["subject"].as_str();
    if outcome == "created" && subject.is_none() {
        return String::new();
    }

    let short = |value: &serde_json::Value| -> String { value.as_str().unwrap_or("?").to_string() };
    let matched = short(&r["matched_memory_id"]);
    let mut out = format!("  reconciliation: {outcome}\n");
    match outcome {
        "duplicate" => {
            out.push_str(&format!(
                "  identical to memory {matched} after normalization — recorded as a duplicate\n"
            ));
        }
        "corroborating" => {
            let value = r["matched_value_key"].as_str().unwrap_or("?");
            out.push_str(&format!(
                "  agrees on value `{value}` with memory {matched}, but the wording differs\n"
            ));
            // The complete command, not an abbreviation of one: `--from` names
            // the statement doing the confirming, which is the memory just
            // written.
            out.push_str(&format!(
                "  → if this is the same claim: cairn memory reinforce {matched} --from {}\n",
                short(&v["memory"]["id"])
            ));
        }
        "conflict_detected" => {
            let competing = r["competing_memory_ids"].as_array().map_or(0, Vec::len);
            out.push_str(&format!(
                "  subject {} now has {} competing answers\n",
                subject.unwrap_or("?"),
                competing + 1
            ));
            out.push_str(
                "  → both stand until somebody decides: cairn memory reconcile --from <id> --to <id> --relation supersedes\n",
            );
        }
        "deferred" => {
            out.push_str(
                "  the subject exceeds reconcile_members_max; the decision runs at the next maintenance tick\n",
            );
        }
        _ => {}
    }
    for note in v["notes"].as_array().into_iter().flatten() {
        if let Some(note) = note.as_str() {
            // `corroborating_member` and `reconciliation_deferred` are already
            // said above, in words. The rest are why a key did not survive.
            let reason = match note {
                "invalid_topic_key" => "the topic key could not be normalized; stored free-form",
                "value_without_topic" => "a value key needs a topic key; the value key was dropped",
                _ => continue,
            };
            out.push_str(&format!("  note: {reason}\n"));
        }
    }
    out
}

/// One search result, with what a caller needs to judge it.
///
/// The lifecycle state, the value it asserts and whether it still stands are
/// the difference between a current answer and a historical one. Printing the
/// content alone made a superseded memory and a verified one look identical,
/// which is exactly the mistake `--as-of` exists to prevent.
pub fn search_result(r: &cairn_core::wire::MemoryResult) -> String {
    let mut line = format!("{}  [{}/{}] {}\n    ", r.id, r.kind, r.scope, r.content);
    line.push_str(&format!("{}", r.state));
    if let Some(v) = &r.value_key {
        line.push_str(&format!(" · value: {v}"));
    }
    if r.verification.state != cairn_core::VerificationState::Unverified {
        line.push_str(&format!(
            " · {}",
            authority_word(r.verification.state, r.verification.authority)
        ));
    }
    if r.pinned {
        line.push_str(" · pinned");
    }
    line.push_str(&format!(
        "\n    from {} session {} · {} evidence\n",
        r.provenance.agent.as_deref().unwrap_or("unknown"),
        r.provenance.session_id,
        r.provenance.evidence_count
    ));
    line
}

/// `verified (cairn)` — the state, and what established it (FR-370).
fn authority_word(
    state: cairn_core::VerificationState,
    authority: Option<cairn_core::VerificationAuthority>,
) -> String {
    match authority {
        Some(a) => format!("{state} ({a})"),
        None => state.to_string(),
    }
}

/// One memory, read on its own.
///
/// `cairn memory show` printed the raw JSON body, so the question it is
/// actually asked — *does this still hold?* — was answered by making the reader
/// parse a document. The lifecycle state, the verification and the subject
/// position lead, because those are the answer.
pub fn memory_detail(m: &cairn_core::wire::MemoryResult) -> String {
    let mut out = String::new();
    out.push_str(&format!("content       {}\n", m.content));
    out.push_str(&format!("id            {}\n", m.id));
    out.push_str(&format!("type          {} · {}\n", m.kind, m.scope));
    out.push_str(&format!("state         {}\n", m.state));
    if let Some(by) = m.superseded_by_id {
        out.push_str(&format!("superseded by {by}\n"));
    }
    out.push_str(&format!(
        "verification  {}\n",
        authority_word(m.verification.state, m.verification.authority)
    ));
    if m.verification.fact_count > 0 {
        out.push_str(&format!(
            "evidence      {} fact(s){}\n",
            m.verification.fact_count,
            if m.verification.basis.is_empty() {
                String::new()
            } else {
                format!(
                    " · {}",
                    m.verification
                        .basis
                        .iter()
                        .map(|b| b.to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            }
        ));
    }
    if let Some(topic) = &m.topic_key {
        out.push_str(&format!("subject       {topic}"));
        if let Some(v) = &m.value_key {
            out.push_str(&format!(" = {v}"));
        }
        out.push('\n');
        if let Some(s) = &m.subject {
            out.push_str(&format!(
                "              {} · {}\n",
                s.reconciliation,
                if s.is_canonical_answer {
                    "a canonical answer"
                } else {
                    "not a canonical answer"
                }
            ));
            if !s.competing_answers.is_empty() {
                out.push_str(&format!(
                    "              {} competing answer(s)\n",
                    s.competing_answers.len()
                ));
            }
            if !s.corroborating_answers.is_empty() {
                out.push_str(&format!(
                    "              {} corroborating statement(s)\n",
                    s.corroborating_answers.len()
                ));
            }
        }
    }
    if m.pinned {
        out.push_str("pinned        yes\n");
    }
    if m.importance != cairn_core::Importance::Normal {
        out.push_str(&format!("importance    {}\n", m.importance));
    }
    // Never labelled as verifications (FR-406).
    if m.reinforcement.count > 0 {
        out.push_str(&format!(
            "reinforced    {} time(s) · {} distinct origin(s)\n",
            m.reinforcement.count, m.reinforcement.distinct_origins
        ));
    }
    out.push_str(&format!(
        "from          {} session {}\n",
        m.provenance.agent.as_deref().unwrap_or("unknown"),
        m.provenance.session_id
    ));
    out
}

/// The identifier of a JSON object, unquoted.
///
/// `Display` on a `serde_json::Value` prints a string *with its quotes*, which
/// is right for JSON and wrong for a line a person reads.
pub fn id_of(v: &serde_json::Value) -> String {
    v["id"].as_str().unwrap_or("?").to_string()
}

/// `n thing` or `n things`. A count read back as "1 duplicate statements" reads
/// as a bug in the thing being counted.
fn plural(n: usize, thing: &str) -> String {
    if n == 1 {
        format!("{n} {thing}")
    } else {
        format!("{n} {thing}s")
    }
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
    // A project subject is identified by its scope; a personal or team subject
    // has none, and printing `(?:?)` for it would look like missing data rather
    // than a category that does not apply.
    match s["domain"].as_str() {
        Some(domain) => out.push_str(&format!(
            "# {} ({domain})\n\n",
            s["topic_key"].as_str().unwrap_or("?")
        )),
        None => out.push_str(&format!(
            "# {} ({}:{})\n\n",
            s["topic_key"].as_str().unwrap_or("?"),
            s["scope"].as_str().unwrap_or("?"),
            s["scope_key"].as_str().unwrap_or("?")
        )),
    }
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
            // The value it asserts and what it actually says, because those are
            // the two things a reader came for. An answer rendered as a bare
            // identifier does not answer the question (FR-307).
            let value = member
                .and_then(|m| m["value_key"].as_str())
                .map(|v| format!("{v} — "))
                .unwrap_or_default();
            let content = member
                .and_then(|m| m["content"].as_str())
                .map(|c| format!("\"{c}\" "))
                .unwrap_or_default();
            out.push_str(&format!(
                "- {value}{content}`{id}` — {verification}{authority}"
            ));
            if let Some(acc) = accounting.get(i) {
                let origins = acc["distinct_origins"].as_i64().unwrap_or(1);
                let dupes = acc["duplicates"].as_array().map(|d| d.len()).unwrap_or(0);
                if dupes > 0 {
                    // Never presented as a number of independent verifications
                    // (FR-406).
                    out.push_str(&format!(
                        " · {} · {origins} distinct origin{}",
                        plural(dupes, "duplicate statement"),
                        if origins == 1 { "" } else { "s" }
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
        // The reason, when there is one and the caller did not ask for the
        // whole history. "unverified" on its own is a fact with no next step.
        if let Some(last) = v["last_run"].as_object() {
            out.push_str(&format!(
                "  last run: {} → {}\n",
                last.get("verifier").and_then(|x| x.as_str()).unwrap_or("?"),
                last.get("result").and_then(|x| x.as_str()).unwrap_or("?"),
            ));
            if let Some(detail) = last.get("detail").and_then(|x| x.as_str()) {
                out.push_str(&format!("  {detail}\n"));
            }
        }
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

/// What a post-compaction read must be told before anything else (FR-426,
/// FR-434).
///
/// The daemon has always returned the checkpoint's classification — the commit
/// it was recorded at, the task digest, every relevant path whose fingerprint
/// moved — and the mode the integration actually delivers. None of it was
/// rendered, so an agent resuming after a compaction read a briefing that
/// looked exactly like a fresh one and carried on from a commit that had moved.
///
/// It leads, because a stale next action acted on is worse than no next action
/// at all. And the recorded action is labelled **previous**, never `next`: it
/// was written against a state that no longer holds, and presenting it as the
/// thing to do would be the confident wrong answer this whole tier exists to
/// prevent.
pub fn continuity(v: &serde_json::Value) -> String {
    let checkpoint = &v["checkpoint"];
    let mut out = String::new();

    let state = checkpoint["classification"]["state"].as_str().unwrap_or("");
    if state == "diverged" {
        out.push_str("⚠ CHECKPOINT DIVERGED\n");
        for d in checkpoint["classification"]["divergences"]
            .as_array()
            .into_iter()
            .flatten()
        {
            let kind = d["kind"].as_str().unwrap_or("?");
            let recorded = d["recorded"].as_str().unwrap_or("?");
            let current = d["current"].as_str().unwrap_or("?");
            match kind {
                "commit" => out.push_str(&format!(
                    "    recorded at {}\n    now at      {}\n",
                    short(recorded),
                    short(current)
                )),
                "task" => out.push_str("    the task changed since the checkpoint\n"),
                "files" => out.push_str(&format!("    files changed: {current}\n")),
                other => out.push_str(&format!("    {other}: {recorded} → {current}\n")),
            }
        }
        for p in checkpoint["classification"]["paths"]
            .as_array()
            .into_iter()
            .flatten()
        {
            // A path Cairn could not fingerprint says so rather than pretending
            // it was unchanged.
            let path = p["path"].as_str().unwrap_or("?");
            let outcome = p["outcome"].as_str().unwrap_or("?");
            let class = p["current_class"].as_str().unwrap_or("?");
            if outcome != "unchanged" {
                out.push_str(&format!("      {path}  ({outcome}, {class})\n"));
            }
        }
    }

    if let Some(previous) = v["briefing"]["previous_next_action"]
        .as_str()
        .or_else(|| checkpoint["previous_next_action"].as_str())
    {
        if state == "diverged" {
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

/// The continuity a session's integration actually delivers, and the count that
/// says whether it has ever been exercised here (FR-426).
pub fn continuity_footer(v: &serde_json::Value) -> String {
    let Some(mode) = v["continuity_mode"].as_str() else {
        return String::new();
    };
    let restores = v["checkpoint"]["restore_count"].as_i64();
    let mut line = format!("\n---\ncontinuity {mode}");
    if let Some(n) = restores {
        line.push_str(&format!(" · checkpoint restored {n} time(s)"));
    }
    line.push('\n');
    line
}

/// The first twelve characters of an object name, which is what a person reads.
fn short(sha: &str) -> String {
    sha.chars().take(12).collect()
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
                personal_notes: Vec::new(),
                team_guidance: Vec::new(),
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
                personal_notes: Vec::new(),
                team_guidance: Vec::new(),
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
                personal_notes: Vec::new(),
                team_guidance: Vec::new(),
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
                personal_notes: Vec::new(),
                team_guidance: Vec::new(),
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
            capture: None,
        };
        let text = status(&s);
        assert!(text.contains("Sharing      local only"));
        assert!(text.contains("Sessions     none active"));
    }
}
