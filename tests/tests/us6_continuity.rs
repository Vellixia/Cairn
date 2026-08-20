//! US6 — compression-safe continuity (`contracts/continuity-context.md`
//! Part 1).
//!
//! After any number of compactions the agent still knows the goal, the state,
//! what is blocking it and what to do next — from Cairn, not from a summariser.
//! Nothing is carried in the conversation, so nothing degrades with each pass.
//!
//! The two negatives this file exists for:
//!
//! * a change nobody told Cairn about is **still detected**, because the
//!   checkpoint compares the fingerprint it recorded rather than looking for
//!   another session's observation (D79);
//! * a stale action is **never** presented as the action to take (FR-434).

use cairn_e2e::Sandbox;
use serde_json::{json, Value};

/// A session with a checkpoint over one relevant path, returning
/// `(session_id, path)`.
fn session_with_checkpoint(s: &Sandbox, key: &str, path: &str) -> String {
    s.write_file(path, "pub fn retry() {}\n");
    s.git(&["add", "."]);
    s.git(&["commit", "-m", "work", "--no-gpg-sign"]);

    s.hook(
        "SessionStart",
        json!({ "session_id": key, "source": "startup" }),
    );
    let started = s.json(&["session", "start", "--key", key]);
    let id = started["session"]["id"].as_str().expect("id").to_string();

    // An observation, so the path becomes a relevant path of this session.
    s.hook(
        "PostToolUse",
        json!({
            "session_id": key,
            "tool_name": "Edit",
            "tool_input": { "file_path": path }
        }),
    );
    s.settle_observations(1);

    let out = s.cairn(&["session", "checkpoint", "--session", &id]);
    assert!(out.ok(), "cairn session checkpoint failed: {}", out.stderr);
    id
}

fn restored(s: &Sandbox, session: &str) -> Value {
    let v = s.json(&[
        "context",
        "--session",
        session,
        "--reason",
        "post_compaction",
        "--json",
    ]);
    v["checkpoint"].clone()
}

/// The compaction lifecycle restores the checkpoint with nobody asking, at the
/// boundary where context can actually reach the model (FR-426, T148, F14).
///
/// Every other test in this file restores by calling
/// `cairn context --reason post_compaction` itself. That is what an
/// `agent_initiated` agent does, and testing only that way is how an agent
/// deriving `automatic` came to promise a restoration it never performed.
///
/// This test fires the hooks and asks nothing. It fires all three, in the order
/// the vendor sends them, because `PostCompact` is capture class: `cairn hook`
/// sends it one-way and throws the reply away, so nothing can be delivered from
/// it for any agent. The boundary that can deliver is the session the agent
/// opens next -- Claude Code names it with source `compact`. An earlier version
/// of this test fired only `PreCompact` and `PostCompact` and asserted a restore,
/// which is why `context_compacted` used to ask for a briefing it could not
/// return: it consumed the checkpoint the session open needed and emitted
/// nothing.
#[test]
fn the_compaction_lifecycle_restores_without_being_asked() {
    let s = Sandbox::new();
    let session = session_with_checkpoint(&s, "compacted", "src/retry.rs");

    let before = s.query_column(&format!(
        "SELECT CAST(COALESCE(SUM(restore_count), 0) AS TEXT) FROM continuity_checkpoints
          WHERE session_id = '{session}'"
    ));
    assert_eq!(before, vec!["0".to_string()], "nothing has restored yet");

    // Exactly what the agent's adapter sends, in order, and nothing else.
    s.hook(
        "PreCompact",
        json!({ "session_id": "compacted", "trigger": "auto" }),
    );
    s.hook(
        "PostCompact",
        json!({ "session_id": "compacted", "trigger": "auto" }),
    );
    s.hook(
        "SessionStart",
        json!({ "session_id": "compacted", "source": "compact" }),
    );

    // Capture-class hooks return before the daemon has finished, so poll.
    let restores = |s: &Sandbox| {
        s.query_column(&format!(
            "SELECT CAST(COALESCE(SUM(restore_count), 0) AS TEXT) FROM continuity_checkpoints
              WHERE session_id = '{session}'"
        ))
        .first()
        .cloned()
        .unwrap_or_default()
    };
    for _ in 0..60 {
        if restores(&s) != "0" {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    assert_ne!(
        restores(&s),
        "0",
        "the compaction lifecycle did not restore the checkpoint: an agent whose mode is \
         `automatic` was promised a rehydration it never got"
    );

    // The matching `context_after_compaction` evidence row is deliberately not
    // asserted here: `capability_evidence.agent` is a foreign key into
    // `agent_integrations`, so evidence can only be recorded for an agent that
    // has actually been connected, and this sandbox connects none. That the
    // delivery is recorded as *delivery* is covered by the derivation
    // regressions above and by the live T148 evidence in the release record,
    // where both Claude Code and Codex establish it against a real store.
}

// ---------------------------------------------------------------------------
// T087 — detection that does not depend on Cairn having been watching
// ---------------------------------------------------------------------------

/// A relevant path modified with **no Cairn session involved** and the commit
/// unmoved is still reported changed (FR-432, D79, SC-311).
///
/// This is the case the earlier design missed entirely. It looked for a
/// `file_changed` observation from another session, which finds nothing when a
/// developer edits in an editor, a formatter rewrites on save, `git apply` lands
/// a patch, or an IDE refactors — all of which leave the commit where it was.
#[test]
fn external_edit() {
    let s = Sandbox::new();
    let session = session_with_checkpoint(&s, "s1", "src/retry.rs");

    let before = restored(&s, &session);
    assert_eq!(
        before["classification"]["state"], "current",
        "nothing has moved yet: {before}"
    );

    // The edit nobody told Cairn about. No hook, no observation, no commit.
    s.write_file("src/retry.rs", "pub fn retry() { /* patched */ }\n");

    let after = restored(&s, &session);
    assert_eq!(
        after["classification"]["state"], "diverged",
        "an external edit must still be detected: {after}"
    );
    let kinds: Vec<&str> = after["classification"]["divergences"]
        .as_array()
        .expect("divergences")
        .iter()
        .filter_map(|d| d["kind"].as_str())
        .collect();
    assert!(
        kinds.contains(&"files"),
        "the change must be reported as a file divergence: {kinds:?}"
    );

    let paths = after["classification"]["paths"]
        .as_array()
        .expect("path results");
    let entry = paths
        .iter()
        .find(|p| p["path"] == "src/retry.rs")
        .expect("the relevant path is reported");
    assert_eq!(entry["outcome"], "changed");
}

/// A path that cannot be fingerprinted is reported as such and **never** as
/// `unchanged` (FR-432, SC-311 metric 15b).
///
/// "I could not look" and "nothing moved" are different answers, and conflating
/// them is exactly how a stale checkpoint reads as current.
#[test]
fn not_fingerprintable() {
    let s = Sandbox::new();
    // A path Cairn could read at checkpoint time, so it is genuinely a relevant
    // path with a recorded digest. An excluded path never becomes one at all:
    // capture drops it, which is a different guarantee.
    let session = session_with_checkpoint(&s, "s2", "src/retry.rs");

    // Now Cairn is told not to look at it. At restoration there is nothing
    // comparable — not because the file is gone, but because it may not be read.
    s.must(&["privacy", "exclude", "--path", "src/**"]);

    let v = restored(&s, &session);
    let entry = v["classification"]["paths"]
        .as_array()
        .expect("path results")
        .iter()
        .find(|p| p["path"] == "src/retry.rs")
        .expect("the relevant path is still reported")
        .clone();

    assert_eq!(
        entry["outcome"], "not_fingerprintable",
        "a path that cannot be read must be reported as itself: {entry}"
    );
    assert_ne!(
        entry["outcome"], "unchanged",
        "'I could not look' must never read as 'nothing moved' — that is exactly \
         how a stale checkpoint would read as current"
    );

    // And it does not manufacture a file divergence either: nothing was
    // compared, so nothing is claimed in the divergence list.
    let kinds: Vec<&str> = v["classification"]["divergences"]
        .as_array()
        .expect("divergences")
        .iter()
        .filter_map(|d| d["kind"].as_str())
        .collect();
    assert!(
        !kinds.contains(&"files"),
        "an uncomparable path is not evidence of a change: {kinds:?}"
    );
}

// ---------------------------------------------------------------------------
// T088 — a stale action is labelled
// ---------------------------------------------------------------------------

/// A diverged checkpoint emits `previous_next_action` and **never**
/// `next_action`, for every divergence class (FR-434, SC-311).
///
/// The recorded action still appears, because throwing it away loses
/// information. It appears *labelled*, because presenting it as the instruction
/// is the failure mode US6 #2 names.
#[test]
fn stale_action_is_labelled() {
    // One divergence class per sandbox, so each is proved on its own.
    for class in ["commit", "branch", "files"] {
        let s = Sandbox::new();
        let session = session_with_checkpoint(&s, "s1", "src/retry.rs");

        match class {
            "commit" => {
                s.write_file("src/other.rs", "pub fn other() {}\n");
                s.git(&["add", "."]);
                s.git(&["commit", "-m", "move the head", "--no-gpg-sign"]);
            }
            "branch" => {
                s.git(&["checkout", "-b", "feature/retry"]);
            }
            "files" => {
                s.write_file("src/retry.rs", "pub fn retry() { /* changed */ }\n");
            }
            _ => unreachable!(),
        }

        let v = restored(&s, &session);
        assert_eq!(
            v["classification"]["state"], "diverged",
            "{class}: the checkpoint must be diverged: {v}"
        );
        assert!(
            v["next_action"].is_null(),
            "{class}: a diverged checkpoint must not emit next_action: {v}"
        );
        assert!(
            v["previous_next_action"].is_string(),
            "{class}: the recorded action must still be delivered, labelled: {v}"
        );
    }
}

// ---------------------------------------------------------------------------
// T095 — ten compaction cycles
// ---------------------------------------------------------------------------

/// Ten consecutive cycles, asserting after **each** one that the continuity
/// fields present in recorded state are delivered (SC-310, FR-428).
///
/// Every field is derived from the store on each pass rather than copied forward
/// from the previous checkpoint, which is why the tenth is as complete as the
/// first. Nothing is carried in the conversation, so nothing degrades.
#[test]
fn ten_compaction_cycles() {
    let s = Sandbox::new();

    let created = s.json(&[
        "task",
        "new",
        "--title",
        "Retry backoff",
        "--goal",
        "transient failures retry with jitter",
        "--criterion",
        "backoff is configurable",
    ]);
    let task = created["task"]["id"].as_str().expect("id").to_string();

    s.write_file("src/retry.rs", "pub fn retry() {}\n");
    s.git(&["add", "."]);
    s.git(&["commit", "-m", "work", "--no-gpg-sign"]);

    s.hook(
        "SessionStart",
        json!({ "session_id": "long", "source": "startup" }),
    );
    let started = s.json(&["session", "start", "--key", "long", "--task", &task]);
    let session = started["session"]["id"].as_str().expect("id").to_string();

    s.hook(
        "PostToolUse",
        json!({
            "session_id": "long",
            "tool_name": "Edit",
            "tool_input": { "file_path": "src/retry.rs" }
        }),
    );
    s.settle_observations(1);

    for cycle in 1..=10 {
        let out = s.cairn(&["session", "checkpoint", "--session", &session]);
        assert!(out.ok(), "cycle {cycle}: checkpoint failed: {}", out.stderr);

        let v = restored(&s, &session);
        assert!(
            v["checkpoint_id"].is_string(),
            "cycle {cycle}: no checkpoint was restored: {v}"
        );

        // The assumption set is complete on every cycle, not only the first.
        let checkpoints = s.query_column(
            "SELECT assumed_branch || '|' || COALESCE(assumed_task_id, '') || '|' ||
                    COALESCE(assumed_task_state_digest, '')
               FROM continuity_checkpoints ORDER BY created_at DESC LIMIT 1",
        );
        let recorded = checkpoints.first().expect("a checkpoint row");
        let parts: Vec<&str> = recorded.split('|').collect();
        assert_eq!(parts[0], "main", "cycle {cycle}: branch not recorded");
        assert!(!parts[1].is_empty(), "cycle {cycle}: task not recorded");
        assert!(
            !parts[2].is_empty(),
            "cycle {cycle}: task state digest not recorded"
        );

        // The briefing still leads with the work state.
        let context = s.json(&["context", "--session", &session, "--json"]);
        assert_eq!(
            context["briefing"]["task"]["id"], task,
            "cycle {cycle}: the task was lost from the briefing"
        );
        assert!(
            context["briefing"]["task"]["progress"].is_object(),
            "cycle {cycle}: derived progress was lost"
        );
    }

    // Ten cycles leave ten records: checkpoints are append-only, and the tenth
    // restoration reads the tenth rather than a rewritten first.
    let count = s.query_column("SELECT CAST(COUNT(*) AS TEXT) FROM continuity_checkpoints");
    assert_eq!(
        count.first().map(String::as_str),
        Some("10"),
        "each boundary must write its own checkpoint"
    );
}

/// Cairn reports the continuity mode it can honestly promise, and never claims
/// a rehydration guarantee an adapter cannot provide (FR-426, FR-427).
#[test]
fn continuity_mode_is_derived_not_claimed() {
    let s = Sandbox::new();
    s.hook(
        "SessionStart",
        json!({ "session_id": "m1", "source": "startup" }),
    );
    let started = s.json(&["session", "start", "--key", "m1", "--agent", "claude-code"]);
    let id = started["session"]["id"].as_str().expect("id").to_string();

    let v = s.json(&["context", "--session", &id, "--json"]);
    let mode = v["continuity_mode"].as_str();
    assert!(
        matches!(
            mode,
            Some("automatic") | Some("agent_initiated") | Some("unavailable_automatic")
        ),
        "the mode must be one of the three derived values, not absent or invented: {v}"
    );
}

/// Every agent's mode is the rule applied to its own capabilities, and no
/// agent's table is asserted (T130, FR-426, FR-427).
///
/// The previous version of this test listed the four expected values. That is
/// what made it possible to "fix" a wrong mode by editing the list, which is
/// twice what actually happened. So it asserts the rule and two properties that
/// hold whatever the agents do, and it names no agent at all.
#[test]
fn every_agents_mode_is_the_rule_applied_to_its_capabilities() {
    use cairn_core::domain::ContinuityMode;
    use cairn_integrate::capability::{Availability, Capability, CapabilityProfile};
    use cairn_integrate::model::AgentId;

    for agent in AgentId::ALL {
        let profile = CapabilityProfile::base(agent);
        let derived = profile.continuity_mode();

        let capture = profile.get(Capability::LifecyclePreCompaction);
        let delivery = profile.get(Capability::ContextAfterCompaction);

        let by_rule = if matches!(
            capture.availability,
            Availability::Absent | Availability::PendingActivation
        ) {
            // Nothing warns Cairn: the agent must checkpoint for itself.
            ContinuityMode::UnavailableAutomatic
        } else if capture.availability != Availability::Guaranteed {
            // A warning that may not arrive is one an agent cannot plan around.
            ContinuityMode::AgentInitiated
        } else if delivery.established() {
            // Observed delivery is the only thing that keeps the promise.
            ContinuityMode::Automatic
        } else {
            ContinuityMode::AgentInitiated
        };

        assert_eq!(
            derived,
            by_rule,
            "{} does not follow the documented rule -- capture {:?}/{:?}, delivery {:?}/{:?}",
            agent.as_str(),
            capture.availability,
            capture.confidence,
            delivery.availability,
            delivery.confidence,
        );

        // The delivery capability is what the mode turns on. Whatever the
        // capture side says, a profile with no *observed* delivery cannot be
        // `automatic` -- which is the property the old list could not express.
        if !delivery.established() {
            assert_ne!(
                derived,
                ContinuityMode::Automatic,
                "{} claims automatic without an observed post-compaction delivery",
                agent.as_str()
            );
        }
    }
}

/// A freshly installed agent never claims `automatic` (FR-426).
///
/// `base()` is what a vendor documents, and nothing in it is evidence that this
/// installation works. Until Cairn has watched a compaction hand context back
/// here, the honest answer is the one that tells the agent to ask. This is the
/// property that makes a wrong entry in `base()` harmless.
#[test]
fn a_profile_with_no_observations_never_claims_automatic() {
    use cairn_core::domain::ContinuityMode;
    use cairn_integrate::capability::CapabilityProfile;
    use cairn_integrate::model::AgentId;

    for agent in AgentId::ALL {
        let mode = CapabilityProfile::base(agent).continuity_mode();
        assert_ne!(
            mode,
            ContinuityMode::Automatic,
            "{} claims automatic from vendor documentation alone",
            agent.as_str()
        );
    }
}

/// Capture is not delivery, and only delivery can make a mode `automatic`
/// (FR-426).
///
/// This is the rule the whole slice turns on, so it is asserted on a profile
/// built by hand rather than on any agent's table. An agent that reports a
/// compaction has told Cairn the boundary happened; it has said nothing about
/// whether anything can be handed back. Reading the capture capability as if it
/// were the delivery one is what let two agents in turn be given a mode the code
/// could not keep -- once too generously, once too meanly.
#[test]
fn capture_of_a_compaction_never_implies_delivery_after_it() {
    use cairn_core::domain::ContinuityMode;
    use cairn_integrate::capability::{
        Availability, Capability, CapabilityProfile, CapabilityState, Confidence,
    };
    use cairn_integrate::model::AgentId;

    let state = |availability, confidence| CapabilityState {
        availability,
        confidence,
        depends_on: None,
    };

    // Perfect capture on both sides, verified by observation, and no delivery.
    let mut p = CapabilityProfile::base(AgentId::ClaudeCode);
    p.capabilities.insert(
        Capability::LifecyclePreCompaction,
        state(Availability::Guaranteed, Confidence::Verified),
    );
    p.capabilities.insert(
        Capability::LifecyclePostCompaction,
        state(Availability::Guaranteed, Confidence::Verified),
    );
    p.capabilities.insert(
        Capability::ContextAfterCompaction,
        state(Availability::Absent, Confidence::Expected),
    );
    assert_eq!(
        p.continuity_mode(),
        ContinuityMode::AgentInitiated,
        "a compaction Cairn was merely told about must not read as re-delivery"
    );

    // The delivery capability is what moves it, and nothing else needs to change.
    p.capabilities.insert(
        Capability::ContextAfterCompaction,
        state(Availability::Guaranteed, Confidence::Verified),
    );
    assert_eq!(
        p.continuity_mode(),
        ContinuityMode::Automatic,
        "an observed delivery after compaction is exactly what `automatic` means"
    );
}

/// `automatic` requires a delivery Cairn has **seen**, not one a vendor
/// documents (FR-426).
///
/// `base()` records what a vendor offers, and a vendor fact can be wrong, stale,
/// or true only on some builds. Gating on `established()` -- guaranteed *and*
/// verified -- means a mistake in that table can only ever under-promise, which
/// is the direction FR-426 permits.
#[test]
fn a_documented_delivery_is_not_enough_for_automatic() {
    use cairn_core::domain::ContinuityMode;
    use cairn_integrate::capability::{
        Availability, Capability, CapabilityProfile, CapabilityState, Confidence,
    };
    use cairn_integrate::model::AgentId;

    let mut p = CapabilityProfile::base(AgentId::ClaudeCode);
    p.capabilities.insert(
        Capability::LifecyclePreCompaction,
        CapabilityState {
            availability: Availability::Guaranteed,
            confidence: Confidence::Verified,
            depends_on: None,
        },
    );

    for (confidence, expected) in [
        (Confidence::Expected, ContinuityMode::AgentInitiated),
        (Confidence::Verified, ContinuityMode::Automatic),
    ] {
        p.capabilities.insert(
            Capability::ContextAfterCompaction,
            CapabilityState {
                availability: Availability::Guaranteed,
                confidence,
                depends_on: None,
            },
        );
        assert_eq!(
            p.continuity_mode(),
            expected,
            "confidence {confidence:?} should derive {expected:?}"
        );
    }
}

/// A warning an agent cannot rely on is worth no promise at all, however good
/// the delivery side looks (FR-426).
#[test]
fn a_conditional_capture_is_never_automatic() {
    use cairn_core::domain::ContinuityMode;
    use cairn_integrate::capability::{
        Availability, Capability, CapabilityProfile, CapabilityState, Confidence,
    };
    use cairn_integrate::model::AgentId;

    let mut p = CapabilityProfile::base(AgentId::ClaudeCode);
    p.capabilities.insert(
        Capability::ContextAfterCompaction,
        CapabilityState {
            availability: Availability::Guaranteed,
            confidence: Confidence::Verified,
            depends_on: None,
        },
    );

    for (capture, expected) in [
        (Availability::Conditional, ContinuityMode::AgentInitiated),
        (Availability::Absent, ContinuityMode::UnavailableAutomatic),
        (
            Availability::PendingActivation,
            ContinuityMode::UnavailableAutomatic,
        ),
        (Availability::Guaranteed, ContinuityMode::Automatic),
    ] {
        p.capabilities.insert(
            Capability::LifecyclePreCompaction,
            CapabilityState {
                availability: capture,
                confidence: Confidence::Verified,
                depends_on: None,
            },
        );
        assert_eq!(
            p.continuity_mode(),
            expected,
            "capture {capture:?} with an established delivery should derive {expected:?}"
        );
    }
}

/// A duplicate post-compaction session open does not restore twice (F14).
///
/// Delivery is recognised by an *unrestored* compaction checkpoint, so the first
/// session open consumes it and any repeat finds nothing to do. This matters
/// because an agent may legitimately emit more than one session open around a
/// compaction -- Codex was observed sending two for one compaction -- and because
/// `restore_count` is the evidence that a delivery happened, it has to keep
/// meaning "delivered once".
#[test]
fn a_repeated_post_compaction_session_open_restores_once() {
    let s = Sandbox::new();
    let session = session_with_checkpoint(&s, "twice", "src/retry.rs");

    let restores = |s: &Sandbox| {
        s.query_column(&format!(
            "SELECT CAST(COALESCE(SUM(restore_count), 0) AS TEXT) FROM continuity_checkpoints
              WHERE session_id = '{session}'"
        ))
        .first()
        .cloned()
        .unwrap_or_default()
    };

    s.hook(
        "PreCompact",
        json!({ "session_id": "twice", "trigger": "auto" }),
    );
    s.hook(
        "PostCompact",
        json!({ "session_id": "twice", "trigger": "auto" }),
    );

    // Two session opens naming the compaction, exactly as a vendor that
    // re-emits the boundary would send them.
    for _ in 0..2 {
        s.hook(
            "SessionStart",
            json!({ "session_id": "twice", "source": "compact" }),
        );
    }

    for _ in 0..60 {
        if restores(&s) != "0" {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    // Give a second restore, if the gate allowed one, time to land.
    std::thread::sleep(std::time::Duration::from_millis(500));

    assert_eq!(
        restores(&s),
        "1",
        "two post-compaction session opens restored more than once: `restore_count` \
         no longer means the checkpoint was delivered exactly one time"
    );
}

/// Divergence detection, per class and in combination — and a diverged
/// checkpoint never emits a live next action (metrics 15 and 16, FR-431,
/// FR-434).
///
/// Every class on its own, so a missed one cannot hide behind another; then
/// all of them at once, so a classifier that reported only the first still
/// fails. The combination case is the one that matters in practice: a session
/// resumed on a different branch, at a different commit, with the task moved
/// and files edited, is the ordinary shape of coming back to work.
#[test]
fn staleness() {
    use cairn_core::continuity::{classify_checkpoint, Assumptions, CurrentState, PathFingerprint};
    use cairn_core::domain::{CheckpointState, DivergenceKind};
    use uuid::Uuid;

    let task = Uuid::now_v7();
    let assumed = Assumptions {
        branch: "feature/retry".into(),
        commit: Some("abc123".into()),
        task_id: Some(task),
        task_state_digest: Some("3f9c".into()),
        path_fingerprints: vec![PathFingerprint::digest("src/config.rs", "d1")],
    };
    let unchanged = CurrentState {
        branch: "feature/retry".into(),
        commit: Some("abc123".into()),
        task_exists: true,
        worktree_exists: true,
        task_state_digest: Some("3f9c".into()),
        path_fingerprints: vec![PathFingerprint::digest("src/config.rs", "d1")],
    };

    // Nothing moved: current, and the recorded next action is live.
    let same = classify_checkpoint(&assumed, &unchanged);
    assert_eq!(same.state, CheckpointState::Current, "{same:?}");
    assert!(same.divergences.is_empty(), "{same:?}");
    assert!(
        same.next_action_is_live(),
        "an unchanged checkpoint's next action is still the action to take"
    );

    // --- One class at a time.
    let cases: Vec<(&str, DivergenceKind, CurrentState)> = vec![
        (
            "the branch moved",
            DivergenceKind::Branch,
            CurrentState {
                branch: "main".into(),
                ..unchanged.clone()
            },
        ),
        (
            "the commit moved",
            DivergenceKind::Commit,
            CurrentState {
                commit: Some("def456".into()),
                ..unchanged.clone()
            },
        ),
        (
            "the task state moved",
            DivergenceKind::Task,
            CurrentState {
                task_state_digest: Some("8b21".into()),
                ..unchanged.clone()
            },
        ),
        (
            "a relevant file changed",
            DivergenceKind::Files,
            CurrentState {
                path_fingerprints: vec![PathFingerprint::digest("src/config.rs", "d2")],
                ..unchanged.clone()
            },
        ),
    ];

    for (what, kind, current) in &cases {
        let c = classify_checkpoint(&assumed, current);
        assert!(
            c.has(*kind),
            "{what}: {kind:?} was not detected — {:?}",
            c.divergences
        );
        assert_eq!(
            c.state,
            CheckpointState::Diverged,
            "{what}: a difference must make the checkpoint diverged"
        );
        // Metric 16. A stale next action presented as the action to take is
        // worse than no next action at all: the agent acts on it.
        assert!(
            !c.next_action_is_live(),
            "{what}: a diverged checkpoint emitted a live next action"
        );
    }

    // --- All of them at once.
    let all = classify_checkpoint(
        &assumed,
        &CurrentState {
            branch: "main".into(),
            commit: Some("def456".into()),
            task_state_digest: Some("8b21".into()),
            path_fingerprints: vec![PathFingerprint::digest("src/config.rs", "d2")],
            ..unchanged.clone()
        },
    );
    for kind in [
        DivergenceKind::Branch,
        DivergenceKind::Commit,
        DivergenceKind::Task,
        DivergenceKind::Files,
    ] {
        assert!(
            all.has(kind),
            "{kind:?} was lost when four classes diverged at once: {:?}",
            all.divergences
        );
    }
    assert_eq!(all.state, CheckpointState::Diverged);
    assert!(!all.next_action_is_live());

    // --- And the two states that are not divergence at all.
    let gone = classify_checkpoint(
        &assumed,
        &CurrentState {
            task_exists: false,
            ..unchanged.clone()
        },
    );
    assert_eq!(
        gone.state,
        CheckpointState::Unresolvable,
        "a checkpoint anchored to a task that no longer exists cannot be restored"
    );
    assert!(!gone.next_action_is_live());

    let no_worktree = classify_checkpoint(
        &assumed,
        &CurrentState {
            worktree_exists: false,
            ..unchanged
        },
    );
    assert_eq!(no_worktree.state, CheckpointState::Unresolvable);
}
