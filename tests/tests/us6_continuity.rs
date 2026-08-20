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

/// The post-compaction **hook** restores the checkpoint, with nobody asking
/// (FR-426, T148).
///
/// Every other test in this file restores by calling
/// `cairn context --reason post_compaction` itself. That is what an
/// `agent_initiated` agent does — and testing only that way is how an agent
/// deriving `automatic` came to promise a restoration it never performed: the
/// `PostCompact` hook asked the daemon for a `continuation`, which builds an
/// ordinary briefing and never touches the checkpoint. Written, never read.
///
/// Found by driving a real compaction in Claude Code against a real store: the
/// `context_compacting` checkpoint was there and its `restore_count` was 0.
/// So this test fires the hooks and asks nothing.
#[test]
fn the_post_compaction_hook_restores_without_being_asked() {
    let s = Sandbox::new();
    let session = session_with_checkpoint(&s, "compacted", "src/retry.rs");

    let before = s.query_column(&format!(
        "SELECT CAST(COALESCE(SUM(restore_count), 0) AS TEXT) FROM continuity_checkpoints
          WHERE session_id = '{session}'"
    ));
    assert_eq!(before, vec!["0".to_string()], "nothing has restored yet");

    // Exactly what the agent's adapter sends, and nothing else.
    s.hook(
        "PreCompact",
        json!({ "session_id": "compacted", "trigger": "auto" }),
    );
    s.hook(
        "PostCompact",
        json!({ "session_id": "compacted", "trigger": "auto" }),
    );

    // `context_compacted` is capture class, so the hook returns before the
    // daemon has finished with it. Poll rather than sleep a fixed time.
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
        "the post-compaction hook did not restore the checkpoint: an agent whose mode is \
         `automatic` was promised a rehydration it never got"
    );
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

/// Each agent's mode is what the rule produces from its capabilities (T130,
/// FR-426, FR-427).
///
/// Asserted against the **derivation**, not against a list. The values below
/// are what Claude Code, Codex, OpenCode and a generic MCP client happen to
/// come out as today; if one of them gained or lost a compaction event, this
/// test should change because the *agent* changed — never because someone
/// edited a table to make the output look right.
///
/// A mode that over-claims is a defect, not a note: an agent reported as
/// `automatic` that is never called back loses the session's continuity
/// silently, which is the failure this whole slice exists to prevent.
#[test]
fn each_agents_mode_is_the_rule_applied_to_its_capabilities() {
    use cairn_core::domain::ContinuityMode;
    use cairn_integrate::capability::{Availability, Capability, CapabilityProfile};
    use cairn_integrate::model::AgentId;

    for (agent, expected) in [
        // Claude Code reported `automatic` until a real compaction was driven
        // against a real store (T148). The checkpoint was written and its
        // `restore_count` stayed 0: `PostCompact` fires, but the vendor does
        // not support `additionalContext` on it, so there is no channel to hand
        // the checkpoint back. `agent_initiated` is what it actually delivers —
        // Cairn writes the checkpoint before compaction, and the agent asks for
        // it with `cairn_context(reason=post_compaction)`.
        (AgentId::ClaudeCode, ContinuityMode::AgentInitiated),
        // Codex reported `automatic` on the strength of registering both
        // compaction hooks. Registration is not re-delivery: `ContextCompacted`
        // is capture class, so the hook sends it one-way and returns without
        // emitting anything back to the session (`delivers_context` in
        // `crates/cairn/src/hook.rs` is `SessionOpened` alone). The checkpoint
        // is restored, and the agent still has to ask for it -- which is what
        // `agent_initiated` means. Held unverified for T148 until it was read
        // against the delivery path instead of the vendor's hook list.
        (AgentId::Codex, ContinuityMode::AgentInitiated),
        (AgentId::Opencode, ContinuityMode::AgentInitiated),
        (AgentId::GenericMcp, ContinuityMode::UnavailableAutomatic),
    ] {
        let profile = CapabilityProfile::base(agent);
        let derived = profile.continuity_mode();
        assert_eq!(
            derived,
            expected,
            "{} derives {derived:?}, expected {expected:?} — pre-compaction is {:?}, \
             post-compaction is {:?}",
            agent.as_str(),
            profile.get(Capability::LifecyclePreCompaction).availability,
            profile
                .get(Capability::LifecyclePostCompaction)
                .availability,
        );

        // And the derivation is the rule, not a lookup: the answer follows the
        // two capabilities it reads.
        let pre = profile.get(Capability::LifecyclePreCompaction).availability;
        let post = profile
            .get(Capability::LifecyclePostCompaction)
            .availability;
        let by_rule = if matches!(pre, Availability::Absent | Availability::PendingActivation) {
            // Nothing warns Cairn: the agent must checkpoint for itself.
            ContinuityMode::UnavailableAutomatic
        } else if pre == Availability::Guaranteed && post == Availability::Guaranteed {
            // `automatic` promises Cairn is called back on both sides, and only
            // two guarantees can keep that promise. A conditional warning is
            // one an agent cannot plan around.
            ContinuityMode::Automatic
        } else {
            ContinuityMode::AgentInitiated
        };
        assert_eq!(
            derived,
            by_rule,
            "{} does not follow the documented rule",
            agent.as_str()
        );
    }
}

/// No agent may report `automatic` while Cairn has no way to re-deliver
/// context after a compaction (FR-426).
///
/// `automatic` is a promise that the developer does not have to act: Cairn is
/// called back on both sides and the context comes back on its own. The second
/// half does not exist. `ContextCompacted` is capture class, so `cairn hook`
/// sends it one-way and returns before reaching `emit_context` -- the only
/// event that delivers context is `SessionOpened`. Whatever a vendor's hook
/// list says, nothing is handed back to the session, so the honest mode is
/// `agent_initiated`: Cairn restores the checkpoint and the agent asks for it.
///
/// This is the invariant, not the four values above. Both agents that once
/// reported `automatic` did so from their hook registrations rather than from
/// the delivery path -- Claude Code until a real compaction disproved it (F10),
/// Codex until the path was read (F12). A future agent added with both
/// compaction hooks `guaranteed` would silently claim `automatic` again, and
/// this test is what stops it.
///
/// **If this test fails because Cairn gained a post-compaction delivery path,
/// it is the test that is wrong.** Delete it, and let the agents whose vendor
/// channel genuinely carries the context back report `automatic` again.
#[test]
fn no_agent_claims_automatic_while_nothing_is_redelivered() {
    use cairn_core::domain::ContinuityMode;
    use cairn_integrate::capability::CapabilityProfile;
    use cairn_integrate::model::AgentId;

    for agent in AgentId::ALL {
        let mode = CapabilityProfile::base(agent).continuity_mode();
        assert_ne!(
            mode,
            ContinuityMode::Automatic,
            "{} reports `automatic`, which promises context comes back without being \
             asked for; no agent can keep that promise while `delivers_context` in \
             crates/cairn/src/hook.rs is SessionOpened alone",
            agent.as_str(),
        );
    }
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
