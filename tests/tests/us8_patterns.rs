//! T124 — US8 and US9 end to end, across two projects in one store (SC-312,
//! SC-313).
//!
//! A verified procedure promotes, is offered in a second project labelled
//! unverified **there**, and a counterexample makes it contested without
//! decreasing anything or deleting it. Later suggestions carry the alternative
//! cause and what to check first.
//!
//! Two real repositories, one machine, the real CLI. `us9_counterexamples.rs`
//! proves the accounting against the store; this proves the developer actually
//! sees it.

use cairn_e2e::Sandbox;
use serde_json::{json, Value};

/// A verified, evidence-backed procedure in a project of its own.
///
/// The evidence is a real file Cairn reads for itself, so the verification
/// carries `cairn` authority — which the gate requires and an attestation
/// cannot supply.
fn promotable_memory(s: &Sandbox) -> String {
    // A scalar the configuration reader can actually extract. It reads a
    // line's value and stops at the first structural character, so a bare `[]`
    // would be read as `[` and the check would report drift against evidence
    // that never moved.
    s.write_file(
        "docker/daemon.json",
        "{\n  \"default-address-pools\": \"exhausted\"\n}\n",
    );
    let memory = s.json(&[
        "memory",
        "add",
        "When bridge creation fails, expand the daemon's default address pools and restart.",
        "--type",
        "procedure",
        "--scope",
        "project",
    ]);
    let id = memory["memory"]["id"]
        .as_str()
        .unwrap_or_else(|| panic!("a memory id: {memory}"))
        .to_string();

    let evidence = s.json(&[
        "evidence",
        "add",
        "--type",
        "configuration",
        "--subject",
        "docker default address pools",
        "--value",
        "exhausted",
        "--locator",
        "docker/daemon.json#default-address-pools",
        "--collector",
        "cairn",
        "--memory",
        &id,
    ]);
    assert!(evidence.get("error").is_none(), "{evidence}");

    let verified = s.json(&["verify", "--memory", &id]);
    assert_eq!(
        verified["authority"].as_str(),
        Some("cairn"),
        "the gate needs a deterministic check Cairn ran itself: {verified}"
    );
    id
}

fn promote(s: &Sandbox, memory: &str, extra: &[&str]) -> Value {
    let mut argv = vec![
        "pattern",
        "promote",
        "--memory",
        memory,
        "--signal",
        "could not find an available non-overlapping ipv4 address pool",
        "--signal",
        "docker bridge network create failure",
        "--title",
        "Docker cannot allocate a non-overlapping bridge network",
        "--problem",
        "Container creation fails because no bridge subnet is free.",
        "--root-cause",
        "The daemon's default address pools are fully allocated.",
        "--approach",
        "Expand default-address-pools and restart the daemon.",
        "--applies-when",
        "Docker bridge networking is in use",
        "--caveat",
        "existing networks are not migrated to the new pool",
    ];
    argv.extend_from_slice(extra);
    s.json(&argv)
}

/// A verified procedure promotes, and says so without claiming more.
#[test]
fn a_verified_procedure_promotes() {
    let s = Sandbox::new();
    s.must(&["init"]);
    let memory = promotable_memory(&s);

    // `--dry-run` first: the gate's value is that it explains, and a developer
    // should be able to ask before committing to the wording.
    let dry = promote(&s, &memory, &["--dry-run"]);
    assert!(dry.get("error").is_none(), "{dry}");
    assert_eq!(dry["dry_run"], true);
    assert_eq!(
        s.json(&["pattern", "list"])["total"],
        0,
        "a dry run must write nothing"
    );

    let promoted = promote(&s, &memory, &[]);
    assert!(promoted.get("error").is_none(), "{promoted}");
    let pattern = promoted["pattern"].clone();
    assert_eq!(
        pattern["trust"], "sanitized",
        "promotion sanitizes; it does not validate: {pattern}"
    );

    // No project identity anywhere in the record. Asserted against the actual
    // values, not against field names: `project_identifier_scan` legitimately
    // appears in the sanitization report, and a substring check on
    // `project_id` would fail on the name of the check that prevents the thing.
    let text = pattern.to_string();
    let status = s.json(&["status"]);
    let project_id = status["project"]["id"].as_str().unwrap_or("«none»");
    let repo_path = s.repo_path().to_string_lossy().to_string();
    for forbidden in [project_id, repo_path.as_str()] {
        assert!(
            !text.contains(forbidden),
            "a pattern must carry no project identity: {forbidden} in {text}"
        );
    }
    assert!(
        pattern["origin_ref"]
            .as_str()
            .is_some_and(|r| r.len() == 64 && r.chars().all(|c| c.is_ascii_hexdigit())),
        "the origin reference must be an opaque digest: {pattern}"
    );

    let listed = s.json(&["pattern", "list"]);
    assert_eq!(listed["total"], 1);
    let counts = listed["patterns"][0]["counts"].as_str().unwrap_or_default();
    assert_eq!(
        counts,
        "applications 0 · distinct projects 0 · independently validated in 0 · counterexamples 0",
        "counts are reported in the one permitted shape"
    );
    assert!(
        !counts.to_lowercase().contains("verif"),
        "no count is ever presented as a number of verifications: {counts}"
    );
}

/// The same machine, a second project: the pattern is offered, labelled
/// unverified **there** (SC-312).
#[test]
fn a_second_project_is_offered_it_labelled_unverified() {
    let a = Sandbox::new();
    a.must(&["init"]);
    let memory = promotable_memory(&a);
    let promoted = promote(&a, &memory, &[]);
    let pattern_id = promoted["pattern"]["id"]
        .as_str()
        .expect("a pattern id")
        .to_string();

    // A second repository, sharing this machine's Cairn home — which is what
    // makes the pattern reachable and the project boundary meaningful.
    let b = a.sibling_project("second-project");
    b.must(&["init"]);

    // The second project hits the symptom. The suggestion is matched on **its**
    // signals, not on the pattern's existence.
    b.hook(
        "SessionStart",
        json!({ "session_id": "b-1", "source": "startup" }),
    );
    b.settle_session_count(1);
    // `PostToolUseFailure` is what produces an `error` observation. A
    // `PostToolUse` with a non-zero exit would not: Cairn never infers a
    // failure from a success payload (D16, FR-117), which is exactly why the
    // signals a pattern matches on cannot be faked by a hopeful caller.
    for message in [
        "could not find an available non-overlapping ipv4 address pool",
        "docker bridge network create failure",
    ] {
        b.hook(
            "PostToolUseFailure",
            json!({
                "session_id": "b-1",
                "tool_name": "Bash",
                "tool_input": { "command": "docker network create demo" },
                "tool_response": { "exit_code": 1 },
                "error": { "message": message },
            }),
        );
    }
    b.settle("the failure observations to land", |b| {
        b.json(&["status"])["observation_count"]
            .as_i64()
            .unwrap_or(0)
            >= 2
    });

    let found = b.json(&["memory", "search", "--include-patterns"]);
    let suggested = found["patterns"].as_array().cloned().unwrap_or_default();
    assert!(
        suggested.iter().any(|p| p["id"] == pattern_id.as_str()),
        "the pattern should be offered where the symptom appears: {found}"
    );
    let offered = suggested
        .iter()
        .find(|p| p["id"] == pattern_id.as_str())
        .expect("the offered pattern");
    assert_eq!(
        offered["verified_in_this_project"], false,
        "a pattern is offered, never asserted, in the project receiving it"
    );
    assert!(
        !offered["applicability"]
            .as_array()
            .unwrap_or(&Vec::new())
            .is_empty(),
        "the applicability travels with it, so it can be ruled out cheaply: {offered}"
    );

    // And it is a separate array — never among this project's own results.
    let results = found["results"].as_array().cloned().unwrap_or_default();
    assert!(
        !results.iter().any(|r| r["id"] == pattern_id.as_str()),
        "a pattern must never appear among a project's own memories: {found}"
    );
}

/// A counterexample contests the pattern, decreases nothing, and deletes
/// nothing — and the next suggestion carries what to check first (SC-313).
#[test]
fn a_counterexample_makes_it_contested_and_travels() {
    let a = Sandbox::new();
    a.must(&["init"]);
    let memory = promotable_memory(&a);
    let pattern_id = promote(&a, &memory, &[])["pattern"]["id"]
        .as_str()
        .expect("a pattern id")
        .to_string();

    // Project B resolves it. A distinct, non-origin project with its own
    // evidence, which is what a validation requires.
    let b = a.sibling_project("resolved-here");
    b.must(&["init"]);
    let outcome_b = b.json(&[
        "pattern",
        "outcome",
        &pattern_id,
        "--outcome",
        "resolved",
        "--signal",
        "could not find an available non-overlapping ipv4 address pool",
        "--signal",
        "docker bridge network create failure",
    ]);
    assert!(outcome_b.get("error").is_none(), "{outcome_b}");
    assert_eq!(outcome_b["trust"], "validated", "{outcome_b}");

    // Project C saw the same symptom from a different cause.
    let c = a.sibling_project("different-cause-here");
    c.must(&["init"]);
    let outcome_c = c.json(&[
        "pattern",
        "outcome",
        &pattern_id,
        "--outcome",
        "not_applicable",
        "--signal",
        "could not find an available non-overlapping ipv4 address pool",
        "--signal",
        "docker bridge network create failure",
        "--alternative-cause",
        "A VPN route collision produced the same symptom.",
    ]);
    assert!(outcome_c.get("error").is_none(), "{outcome_c}");
    assert_eq!(
        outcome_c["trust"], "contested",
        "a counterexample contests, and contested is decided before validated"
    );

    let shown = c.json(&["pattern", "show", &pattern_id]);
    assert_eq!(
        shown["independently_validated_in"], 1,
        "the success is not decreased by the counterexample: {shown}"
    );
    assert_eq!(shown["counterexamples"], 1);
    assert!(
        shown["alternative_causes"]
            .as_array()
            .unwrap_or(&Vec::new())
            .iter()
            .any(|c| c.as_str().unwrap_or("").contains("VPN route collision")),
        "the alternative cause is retained and reported: {shown}"
    );
    // Retained, not deleted.
    assert_eq!(
        c.json(&["pattern", "list"])["total"],
        1,
        "a counterexample must never delete a pattern"
    );
}

/// Ten sessions in one project buy nothing (SC-314).
#[test]
fn repetition_in_one_project_does_not_advance_trust() {
    let a = Sandbox::new();
    a.must(&["init"]);
    let memory = promotable_memory(&a);
    let pattern_id = promote(&a, &memory, &[])["pattern"]["id"]
        .as_str()
        .expect("a pattern id")
        .to_string();

    let b = a.sibling_project("one-noisy-project");
    b.must(&["init"]);
    let record = |b: &Sandbox| {
        b.cairn(&[
            "--json",
            "pattern",
            "outcome",
            &pattern_id,
            "--outcome",
            "resolved",
            "--signal",
            "could not find an available non-overlapping ipv4 address pool",
            "--signal",
            "docker bridge network create failure",
        ])
    };

    let first = record(&b);
    assert!(first.ok(), "{}", first.stderr);
    for attempt in 2..=10 {
        let again = record(&b);
        assert!(
            !again.ok(),
            "attempt {attempt} recorded a second row for one incident: {}",
            again.stdout
        );
        assert!(
            again.stdout.contains("outcome_already_recorded")
                || again.stderr.contains("outcome_already_recorded"),
            "attempt {attempt} was refused for the wrong reason: {} {}",
            again.stdout,
            again.stderr
        );
    }

    let shown = b.json(&["pattern", "show", &pattern_id]);
    assert_eq!(shown["applications"], 1, "ten retellings, one incident");
    assert_eq!(shown["distinct_projects"], 1);
}

/// A project that **was shown** the pattern does not validate it by agreeing
/// (FR-403, SC-314).
///
/// The daemon decides `discovery`, never the caller: an agent cannot be asked
/// to report honestly on whether it was influenced by something it read. This
/// is that decision at the surface a real agent uses — the store-level test
/// supplies `discovery` directly, which is precisely the thing the agent must
/// not be able to do.
#[test]
fn a_project_that_was_shown_the_pattern_does_not_validate_it_by_agreeing() {
    let a = Sandbox::new();
    a.must(&["init"]);
    let memory = promotable_memory(&a);
    let pattern_id = promote(&a, &memory, &[])["pattern"]["id"]
        .as_str()
        .expect("a pattern id")
        .to_string();

    let b = a.sibling_project("was-shown-the-pattern");
    b.must(&["init"]);
    b.hook(
        "SessionStart",
        json!({ "session_id": "b-1", "source": "startup" }),
    );
    b.settle_session_count(1);

    // B hits the symptom, so B is a project Cairn would have suggested this
    // pattern to.
    for message in [
        "could not find an available non-overlapping ipv4 address pool",
        "docker bridge network create failure",
    ] {
        b.hook(
            "PostToolUseFailure",
            json!({
                "session_id": "b-1",
                "tool_name": "Bash",
                "tool_input": { "command": "docker network create demo" },
                "tool_response": { "exit_code": 1 },
                "error": { "message": message },
            }),
        );
    }
    b.settle("the failure observations to land", |b| {
        b.json(&["status"])["observation_count"]
            .as_i64()
            .unwrap_or(0)
            >= 2
    });

    // It agrees, with no evidence of its own.
    let agreed = b.json(&[
        "pattern",
        "outcome",
        &pattern_id,
        "--outcome",
        "resolved",
        "--signal",
        "could not find an available non-overlapping ipv4 address pool",
        "--signal",
        "docker bridge network create failure",
    ]);
    assert!(agreed.get("error").is_none(), "{agreed}");
    assert_eq!(
        agreed["discovery"], "cairn_suggested",
        "the daemon must recognise a project it would have suggested this to, \
         whatever the caller says: {agreed}"
    );
    assert_eq!(
        agreed["trust"], "sanitized",
        "an agent agreeing with Cairn's own suggestion is Cairn confirming \
         Cairn, and must not advance trust: {agreed}"
    );

    let shown = b.json(&["pattern", "show", &pattern_id]);
    assert_eq!(shown["applications"], 1, "it is still an application");
    assert_eq!(
        shown["independently_validated_in"], 0,
        "and not a validation: {shown}"
    );

    // The same project, having collected deterministic evidence of its own,
    // does validate it. Without this the assertion above would also pass
    // against an implementation that ignored suggested applications entirely.
    let c = a.sibling_project("confirmed-with-evidence");
    c.must(&["init"]);
    let evidence = c.json(&[
        "evidence",
        "add",
        "--type",
        "file",
        "--subject",
        "daemon configuration",
        "--value",
        "present",
        "--locator",
        "README.md",
        "--collector",
        "cairn",
    ]);
    let evidence_id = evidence["evidence"]["id"]
        .as_str()
        .unwrap_or_else(|| panic!("an evidence id: {evidence}"))
        .to_string();
    let confirmed = c.json(&[
        "pattern",
        "outcome",
        &pattern_id,
        "--outcome",
        "resolved",
        "--signal",
        "could not find an available non-overlapping ipv4 address pool",
        "--signal",
        "docker bridge network create failure",
        "--evidence",
        &evidence_id,
    ]);
    assert!(confirmed.get("error").is_none(), "{confirmed}");
    assert_eq!(
        confirmed["trust"], "validated",
        "a suggestion confirmed by local evidence is confirmation: {confirmed}"
    );
}

/// A memory whose own evidence contradicts it does not promote.
///
/// `VerificationState::Conflicted` had no writer, so this case was previously
/// unreachable: the source stayed `verified` and the promotion gate never saw
/// anything to refuse. Now that attaching a contradiction actually moves the
/// memory (`crates/cairn-store/src/evidence.rs`), the gate's own precondition —
/// `source.verification != Verified` refuses with `source_unverified` — refuses
/// it, with no change to `patterns.rs` itself.
#[test]
fn a_conflicted_source_does_not_promote() {
    let s = Sandbox::new();
    let id = promotable_memory(&s);

    // An agent attaches evidence that disagrees with the claim just verified.
    let contradiction = s.json(&[
        "evidence",
        "add",
        "--type",
        "configuration",
        "--subject",
        "docker default address pools",
        "--value",
        "plenty free",
        "--locator",
        "incident/2026-08-19.md",
        "--collector",
        "agent",
        "--role",
        "contradicts",
        "--memory",
        &id,
    ]);
    assert!(contradiction.get("error").is_none(), "{contradiction}");
    assert_eq!(
        contradiction["verification"]["state"], "conflicted",
        "attaching the contradiction did not move the memory: {contradiction}"
    );

    let shown = s.json(&["memory", "show", &id]);
    assert_eq!(
        shown["memory"]["verification"]["state"], "conflicted",
        "the memory itself does not report the conflict: {shown}"
    );

    // `promote()`'s own helper goes through `Sandbox::json`, which asserts
    // success — exactly the response this call must not produce, so the
    // refusal is read with `json_err` instead.
    let refused = s.json_err(&[
        "pattern",
        "promote",
        "--memory",
        &id,
        "--dry-run",
        "--signal",
        "could not find an available non-overlapping ipv4 address pool",
        "--signal",
        "docker bridge network create failure",
    ]);
    assert_eq!(
        refused["code"].as_str(),
        Some("source_unverified"),
        "a conflicted source was not refused: {refused}"
    );
}
