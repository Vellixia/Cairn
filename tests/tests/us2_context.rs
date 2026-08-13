//! T034 — the next session opens informed, and always within budget (SC-003).

use cairn_e2e::Sandbox;
use serde_json::json;

fn seed_a_finished_session(s: &Sandbox) {
    s.hook(
        "SessionStart",
        json!({ "session_id": "first", "source": "startup" }),
    );
    s.write_file("src/lib.rs", "pub fn work() {}\n");
    s.hook(
        "PostToolUse",
        json!({ "session_id": "first", "tool_name": "Edit", "tool_input": { "file_path": "src/lib.rs" } }),
    );
    s.hook(
        "PostToolUse",
        json!({
            "session_id": "first", "tool_name": "Bash",
            "tool_input": { "command": "cargo test" }, "tool_response": { "exit_code": 1 }
        }),
    );
    // End through the CLI rather than the hook. The hook is fail-soft by design
    // and legitimately drops work when it misses its deadline under parallel
    // test load — which would make this fixture flaky rather than wrong. The
    // hook path itself is covered by `us1_capture_handoff`.
    s.json(&[
        "session",
        "end",
        "--status",
        "completed",
        "--reason",
        "clear",
    ]);
    assert!(
        s.handoff_after_close(&[])["next_step"].is_string(),
        "the fixture must leave a handoff behind"
    );
}

#[test]
fn a_new_session_receives_the_previous_handoff_and_repository_state() {
    let s = Sandbox::new();
    seed_a_finished_session(&s);

    let ctx = s.json(&["context"]);
    let b = &ctx["briefing"];
    assert_eq!(b["repository"]["branch"], "main");
    assert!(b["repository"]["commit_sha"].is_string());

    let previous = &b["previous_handoff"];
    assert!(previous["next_step"]
        .as_str()
        .unwrap_or_default()
        .to_lowercase()
        .contains("fix"));
    assert!(!previous["remaining_work"].as_array().unwrap().is_empty());
    assert!(!b["known_failures"].as_array().unwrap().is_empty());
    assert_eq!(b["no_prior_history"], false);
}

#[test]
fn the_budget_is_never_exceeded_however_much_memory_exists() {
    let s = Sandbox::new();
    seed_a_finished_session(&s);
    for i in 0..40 {
        s.must(&[
            "memory",
            "add",
            "--type",
            "fact",
            "--scope",
            "project",
            &format!("project fact number {i} with a good deal of explanatory prose attached"),
        ]);
    }

    // Compliance is a property of the assembly loop, not a statistic (FR-029).
    for budget in ["300", "600", "1200", "2000", "3000", "4000"] {
        let ctx = s.json(&["context", "--budget", budget]);
        let limit: u64 = budget.parse().unwrap();
        let spent = ctx["estimated_tokens"].as_u64().unwrap();
        assert!(
            spent <= limit,
            "briefing spent {spent} of a {limit} budget (FR-029)"
        );
        assert_eq!(ctx["budget"], limit);
    }

    // And when it genuinely cannot fit, it says so and names what went.
    let tight = s.json(&["context", "--budget", "300"]);
    assert_eq!(tight["truncated"], true);
    assert!(
        !tight["omitted_sections"].as_array().unwrap().is_empty(),
        "truncation must name what was omitted"
    );
}

/// The briefing degrades in a defined priority order rather than by truncation,
/// so what survives a tight budget is the part the agent needs most (FR-030).
#[test]
fn high_priority_sections_survive_a_tight_budget() {
    // SC-003's second clause: a normal start keeps task, repository and handoff.
    let s = Sandbox::new();
    seed_a_finished_session(&s);
    for i in 0..50 {
        s.must(&[
            "memory",
            "add",
            "--type",
            "fact",
            "--scope",
            "project",
            &format!("fact {i}"),
        ]);
    }
    let ctx = s.json(&["context", "--budget", "1500"]);
    let omitted = ctx["omitted_sections"].as_array().unwrap().clone();
    for high in ["repository", "previous_handoff", "task"] {
        assert!(
            !omitted.contains(&json!(high)),
            "{high} was dropped: {omitted:?}"
        );
    }
}

#[test]
fn a_repository_with_no_history_still_gets_a_briefing() {
    let s = Sandbox::new();
    let ctx = s.json(&["context"]);
    assert_eq!(ctx["briefing"]["no_prior_history"], true);
    assert_eq!(ctx["truncated"], false);
}

#[test]
fn switching_branch_changes_what_the_briefing_prioritises() {
    let s = Sandbox::new();
    seed_a_finished_session(&s);
    s.must(&[
        "memory",
        "add",
        "--type",
        "failure",
        "--scope",
        "branch",
        "main-only fact",
    ]);

    s.git(&["switch", "-c", "feature/x"]);
    s.must(&[
        "memory",
        "add",
        "--type",
        "failure",
        "--scope",
        "branch",
        "feature-only fact",
    ]);

    let ctx = s.json(&["context"]);
    assert_eq!(ctx["briefing"]["repository"]["branch"], "feature/x");
    let branch_memory = ctx["briefing"]["memory"]["branch"]
        .as_array()
        .unwrap()
        .clone();
    let text = serde_json::to_string(&branch_memory).unwrap();
    assert!(text.contains("feature-only"), "{text}");
    assert!(
        !text.contains("main-only"),
        "another branch's memory leaked in"
    );
}

#[test]
fn session_start_still_starts_when_cairn_cannot_answer_in_time() {
    // FR-046: reduced context, never a blocked agent.
    let s = Sandbox::new();
    seed_a_finished_session(&s);

    let out = s.hook(
        "SessionStart",
        json!({ "session_id": "second", "source": "startup" }),
    );
    assert_eq!(out.code, 0);
    let emitted: serde_json::Value = serde_json::from_str(out.stdout.trim()).expect("hook output");
    let context = emitted["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap();
    assert!(context.contains("Cairn context"));
    assert!(
        context.contains("Previous session"),
        "the briefing should carry the handoff"
    );
}

#[test]
fn session_start_reports_reduced_context_when_the_deadline_cannot_be_met() {
    // M7 / FR-046: the agent session starts either way, and Cairn says so.
    let s = Sandbox::new();
    seed_a_finished_session(&s);

    // A context deadline of 1 ms cannot be met by any real assembly, so the
    // fallback is the only path left.
    let config = s.home.path().join("config.json");
    let raw = std::fs::read_to_string(&config).unwrap_or_else(|_| "{}".to_string());
    let mut parsed: serde_json::Value = serde_json::from_str(&raw).unwrap_or(serde_json::json!({}));
    parsed["context_deadline_ms"] = serde_json::json!(1);
    std::fs::write(&config, parsed.to_string()).expect("write config");

    let out = s.hook(
        "SessionStart",
        json!({ "session_id": "degraded", "source": "startup" }),
    );

    assert_eq!(out.code, 0, "a hook always exits 0");
    let emitted: serde_json::Value =
        serde_json::from_str(out.stdout.trim()).expect("hook still emits context JSON");
    let context = emitted["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .expect("additionalContext");
    assert!(
        context.contains("Reduced context"),
        "the fallback must say so: {context}"
    );
    assert!(
        context.contains("cairn context"),
        "and point at the full briefing: {context}"
    );
    assert!(
        out.stderr.is_empty(),
        "a missed deadline must not surface as an agent-visible error"
    );
}

#[test]
fn decisions_and_known_failures_appear_in_the_briefing() {
    let s = Sandbox::new();
    seed_a_finished_session(&s);

    s.must(&[
        "memory",
        "add",
        "--type",
        "decision",
        "--scope",
        "project",
        "chose a token bucket for rate limiting",
    ]);
    s.must(&[
        "memory",
        "add",
        "--type",
        "failure",
        "--scope",
        "project",
        "cargo test failed on first attempt",
    ]);

    let ctx = s.json(&["context"]);
    let briefing = &ctx["briefing"];
    let project_memory = briefing["memory"]["project"].as_array().unwrap();
    let text = serde_json::to_string(project_memory).unwrap();
    assert!(
        text.contains("token bucket"),
        "decisions should appear: {text}"
    );
    assert!(
        text.contains("cargo test failed"),
        "failures should appear: {text}"
    );
}

#[test]
fn a_task_with_no_acceptance_criteria_still_produces_a_briefing() {
    let s = Sandbox::new();
    let created = s.json(&[
        "task",
        "new",
        "--title",
        "No criteria",
        "--goal",
        "Just a goal",
    ]);
    let id = created["task"]["id"].as_str().unwrap().to_string();

    s.hook(
        "SessionStart",
        json!({ "session_id": "nc-ctx", "source": "startup" }),
    );
    s.must(&["session", "start", "--key", "nc-ctx", "--task", &id]);

    let ctx = s.json(&["context"]);
    let task = &ctx["briefing"]["task"];
    assert_eq!(task["goal"], "Just a goal");
    assert!(
        task["acceptance_criteria"]
            .as_array()
            .is_some_and(|c| c.is_empty()),
        "empty criteria should not break the briefing"
    );
}
