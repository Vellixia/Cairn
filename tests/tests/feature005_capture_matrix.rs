//! The per-agent capture matrix, provenance and file-identity dispositions
//! (T040, `contracts/extraction.md` §13.3/§13.7/§13.10, `data-model.md`
//! §1.2/§1.3/§2).
//!
//! Everything here drives the **local capture layer** directly —
//! `cairn_integrate::capture` and the declared capability matrix — with no
//! daemon, no socket and no PostgreSQL. That is a deliberate boundary: the
//! matrix is a claim about what one machine's adapters can establish before
//! anything crosses the wire, and `feature005_extraction.rs` already covers
//! what happens to an event once it does.
//!
//! # What "no cells silently absent" means, mechanically
//!
//! `MatrixCapability::all()` names 25 cells. `declared_matrix(agent)` must
//! answer every one of them, and every answer must be coherent with its own
//! evidence rule (`MatrixCell::is_coherent`). Where the declared answer is
//! `no_evidence` and the adapter actually implements the cell, driving a
//! realistic vendor payload through `capture` and finding the canonical kind
//! in the output is the evidence the matrix is waiting for — a `no_evidence`
//! cell is honest only until it is proven otherwise, and this file is what
//! proves it (SC-706).

use cairn_core::event::{
    ChangeKind, EventAgent, EventContent, EventKind, FileIdentity, SafeEventDraft,
};
use cairn_core::vocabulary::SessionVocabulary;
use cairn_integrate::agents;
use cairn_integrate::capability::{declared_matrix, MatrixCapability, MatrixStatus};
use cairn_integrate::{adapter_for, capture, carries_semantic_material, AgentId, RawPayload};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// A vocabulary built the way the daemon actually builds one: from one prior
/// `file_changed` event's path, at `session_seq` 1 (`crates/cairnd/src/
/// capture.rs::session_vocabulary`). `module` ranks above `object` because a
/// directory component outranks a file's final component (§13.5).
fn vocab(module: &str, object: &str) -> (SessionVocabulary, BTreeMap<String, String>) {
    let mut vocabulary = SessionVocabulary::new();
    let content = EventContent::File {
        repo_file: Some(format!("{module}/{object}.rs")),
        repo_file_from: None,
        change_kind: Some(ChangeKind::Modified),
        file_identity: FileIdentity::Present,
    };
    vocabulary.observe_at(Some(1), EventKind::FileChanged, Some(&content));
    (vocabulary, BTreeMap::new())
}

/// Every produced draft's provenance is bounded to two fields and nothing
/// vendor-shaped survives beside them (FR-724).
fn assert_provenance_only(draft: &SafeEventDraft, expected_agent: EventAgent, label: &str) {
    assert_eq!(
        draft.agent, expected_agent,
        "{label}: wrong agent on the draft"
    );
    let vendor_event = draft
        .vendor_event
        .as_deref()
        .unwrap_or_else(|| panic!("{label}: no sanitized vendor_event carried as provenance"));
    assert!(
        vendor_event.chars().count() <= cairn_core::event::VENDOR_TOKEN_MAX_CHARS,
        "{label}: vendor_event exceeds its bound"
    );
    assert!(
        vendor_event
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-')),
        "{label}: vendor_event {vendor_event:?} is not sanitized"
    );
    let value = serde_json::to_value(draft).expect("a SafeEventDraft always serializes");
    let obj = value
        .as_object()
        .expect("a SafeEventDraft serializes to an object");
    for key in obj.keys() {
        assert!(
            matches!(key.as_str(), "kind" | "agent" | "vendor_event" | "content"),
            "{label}: {key} is a field a safe event draft must not carry"
        );
    }
}

fn event_agent_of(agent: AgentId) -> EventAgent {
    match agent {
        AgentId::ClaudeCode => EventAgent::ClaudeCode,
        AgentId::Codex => EventAgent::Codex,
        AgentId::Opencode => EventAgent::OpenCode,
        AgentId::GenericMcp => unreachable!("generic MCP is not part of the capture population"),
    }
}

/// One realistic vendor fixture: an event name, its payload, and the
/// canonical kinds a correct implementation must produce from it.
struct StructuralFixture {
    label: &'static str,
    event: &'static str,
    payload: fn() -> Value,
    expected_kinds: &'static [EventKind],
}

fn run_structural_fixtures(agent: AgentId, fixtures: &[StructuralFixture]) {
    let expected_agent = event_agent_of(agent);
    for fixture in fixtures {
        let payload = RawPayload::new((fixture.payload)(), ".");
        let out = capture(
            agent,
            fixture.event,
            &payload,
            &agents::CaptureEnv::default(),
        );
        for &expected in fixture.expected_kinds {
            let draft = out
                .events
                .iter()
                .find(|d| d.kind == expected)
                .unwrap_or_else(|| {
                    panic!(
                        "{agent:?} / {}: {expected:?} was not produced; got {:?}",
                        fixture.label, out
                    )
                });
            assert_provenance_only(draft, expected_agent, fixture.label);
        }
    }
}

// ---------------------------------------------------------------------------
// Part 1 — the matrix has no blank cells, for every agent (SC-706)
// ---------------------------------------------------------------------------

#[test]
fn every_declared_matrix_cell_is_present_and_coherent_for_every_agent() {
    // SC-706: the matrix is fixed before implementation and is the population
    // under test — a cell that is not there cannot be "silently absent", it
    // has to be found missing.
    let all = MatrixCapability::all();
    assert_eq!(
        all.len(),
        25,
        "the capability population itself has drifted"
    );

    for agent in ["claude_code", "codex", "opencode"] {
        let declared = declared_matrix(agent);
        assert_eq!(
            declared.len(),
            all.len(),
            "{agent}: the declared matrix does not have one cell per capability"
        );
        let seen: std::collections::BTreeSet<&str> =
            declared.iter().map(|c| c.capability.as_str()).collect();
        for capability in &all {
            assert!(
                seen.contains(capability.key().as_str()),
                "{agent}: {} is silently absent from the declared matrix",
                capability.key()
            );
        }
        for cell in &declared {
            assert!(
                cell.is_coherent(),
                "{agent}: {} declared {:?} with evidence that contradicts the claim",
                cell.capability,
                cell.status
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Part 2 — driving `no_evidence` cells to real evidence, per agent
// ---------------------------------------------------------------------------

const CLAUDE_CODE_FIXTURES: &[StructuralFixture] = &[
    StructuralFixture {
        label: "a fresh session start",
        event: "SessionStart",
        payload: || json!({"session_id": "s1", "source": "startup"}),
        expected_kinds: &[EventKind::SessionOpened],
    },
    StructuralFixture {
        label: "a resumed session start",
        event: "SessionStart",
        payload: || json!({"session_id": "s1", "source": "resume"}),
        expected_kinds: &[EventKind::SessionResumed],
    },
    StructuralFixture {
        label: "a session end",
        event: "SessionEnd",
        payload: || json!({"session_id": "s1", "reason": "exit"}),
        expected_kinds: &[EventKind::SessionClosed],
    },
    StructuralFixture {
        label: "a manual pre-compaction",
        event: "PreCompact",
        payload: || json!({"session_id": "s1", "trigger": "manual"}),
        expected_kinds: &[EventKind::ContextCompacting],
    },
    StructuralFixture {
        label: "an automatic post-compaction",
        event: "PostCompact",
        payload: || json!({"session_id": "s1", "trigger": "auto"}),
        expected_kinds: &[EventKind::ContextCompacted],
    },
    StructuralFixture {
        label: "a turn boundary with no settled text",
        event: "Stop",
        payload: || json!({"session_id": "s1"}),
        expected_kinds: &[EventKind::AgentQuiesced],
    },
    StructuralFixture {
        label: "a subagent completion",
        event: "SubagentStop",
        payload: || json!({"session_id": "s1", "agent_id": "sub-1", "agent_type": "reviewer"}),
        expected_kinds: &[EventKind::SubagentCompleted],
    },
    StructuralFixture {
        label: "a tool about to run",
        event: "PreToolUse",
        payload: || json!({"session_id": "s1", "tool_name": "Bash", "tool_input": {"command": "ls"}}),
        expected_kinds: &[EventKind::ToolStarted],
    },
    StructuralFixture {
        label: "a successful shell command",
        event: "PostToolUse",
        payload: || {
            json!({
                "session_id": "s1", "tool_name": "Bash",
                "tool_input": {"command": "echo hi"}, "tool_response": {"exit_code": 0},
            })
        },
        expected_kinds: &[EventKind::ToolSucceeded, EventKind::CommandExecuted],
    },
    StructuralFixture {
        label: "a failed shell command",
        event: "PostToolUseFailure",
        payload: || {
            json!({
                "session_id": "s1", "tool_name": "Bash",
                "tool_input": {"command": "ls /nope"}, "tool_response": {"error": "no such file"},
            })
        },
        expected_kinds: &[EventKind::ToolFailed],
    },
    StructuralFixture {
        label: "a file read",
        event: "PostToolUse",
        payload: || {
            json!({
                "session_id": "s1", "tool_name": "Read",
                "tool_input": {"file_path": "/repo/src/lib.rs"}, "tool_response": {"exit_code": 0},
            })
        },
        expected_kinds: &[EventKind::FileRead],
    },
    StructuralFixture {
        label: "a file edit",
        event: "PostToolUse",
        payload: || {
            json!({
                "session_id": "s1", "tool_name": "Edit",
                "tool_input": {"file_path": "/repo/src/lib.rs"}, "tool_response": {"exit_code": 0},
            })
        },
        expected_kinds: &[EventKind::FileChanged],
    },
    StructuralFixture {
        label: "a passing test run",
        event: "PostToolUse",
        payload: || {
            json!({
                "session_id": "s1", "tool_name": "Bash",
                "tool_input": {"command": "cargo test"}, "tool_response": {"exit_code": 0},
            })
        },
        expected_kinds: &[EventKind::TestExecuted, EventKind::TestResult],
    },
    StructuralFixture {
        label: "research activity",
        event: "PostToolUse",
        payload: || json!({"session_id": "s1", "tool_name": "WebFetch", "tool_response": {"exit_code": 0}}),
        expected_kinds: &[EventKind::ResearchActivity],
    },
];

#[test]
fn claude_code_produces_the_canonical_event_for_every_structural_no_evidence_capability_it_implements(
) {
    run_structural_fixtures(AgentId::ClaudeCode, CLAUDE_CODE_FIXTURES);
}

const CODEX_FIXTURES: &[StructuralFixture] = &[
    StructuralFixture {
        label: "a fresh session start",
        event: "SessionStart",
        payload: || json!({"session_id": "s1", "source": "startup"}),
        expected_kinds: &[EventKind::SessionOpened],
    },
    StructuralFixture {
        label: "a resumed session start",
        event: "SessionStart",
        payload: || json!({"session_id": "s1", "source": "resume"}),
        expected_kinds: &[EventKind::SessionResumed],
    },
    StructuralFixture {
        label: "a session end",
        event: "SessionEnd",
        payload: || json!({"session_id": "s1", "reason": "exit"}),
        expected_kinds: &[EventKind::SessionClosed],
    },
    StructuralFixture {
        label: "a manual pre-compaction",
        event: "PreCompact",
        payload: || json!({"session_id": "s1", "trigger": "manual"}),
        expected_kinds: &[EventKind::ContextCompacting],
    },
    StructuralFixture {
        label: "an automatic post-compaction",
        event: "PostCompact",
        payload: || json!({"session_id": "s1", "trigger": "auto"}),
        expected_kinds: &[EventKind::ContextCompacted],
    },
    StructuralFixture {
        label: "a turn boundary with no settled text",
        event: "Stop",
        payload: || json!({"session_id": "s1"}),
        expected_kinds: &[EventKind::AgentQuiesced],
    },
    StructuralFixture {
        label: "a subagent completion",
        event: "SubagentStop",
        payload: || json!({"session_id": "s1", "agent_id": "sub-1", "agent_type": "reviewer"}),
        expected_kinds: &[EventKind::SubagentCompleted],
    },
    StructuralFixture {
        label: "a tool about to run",
        event: "PreToolUse",
        payload: || json!({"session_id": "s1", "tool_name": "shell", "tool_input": {"command": "ls"}}),
        expected_kinds: &[EventKind::ToolStarted],
    },
    StructuralFixture {
        label: "a successful shell command",
        event: "PostToolUse",
        payload: || {
            json!({
                "session_id": "s1", "tool_name": "shell",
                "tool_input": {"command": "echo hi"}, "tool_response": {"exit_code": 0},
            })
        },
        expected_kinds: &[EventKind::ToolSucceeded, EventKind::CommandExecuted],
    },
    StructuralFixture {
        // Codex has no dedicated failure hook: one `PostToolUse` carries both
        // outcomes and the payload decides (D31).
        label: "a failed shell command, on the one tool event codex has",
        event: "PostToolUse",
        payload: || {
            json!({
                "session_id": "s1", "tool_name": "shell",
                "tool_input": {"command": "ls /nope"}, "tool_response": {"exit_code": 1},
            })
        },
        expected_kinds: &[EventKind::ToolFailed],
    },
    StructuralFixture {
        label: "a file read",
        event: "PostToolUse",
        payload: || {
            json!({
                "session_id": "s1", "tool_name": "read",
                "tool_input": {"file_path": "/repo/src/lib.rs"}, "tool_response": {"exit_code": 0},
            })
        },
        expected_kinds: &[EventKind::FileRead],
    },
    StructuralFixture {
        label: "a file edit via apply_patch",
        event: "PostToolUse",
        payload: || {
            json!({
                "session_id": "s1", "tool_name": "apply_patch",
                "tool_input": {"file_path": "/repo/src/lib.rs"}, "tool_response": {"exit_code": 0},
            })
        },
        expected_kinds: &[EventKind::FileChanged],
    },
    StructuralFixture {
        label: "a passing test run",
        event: "PostToolUse",
        payload: || {
            json!({
                "session_id": "s1", "tool_name": "shell",
                "tool_input": {"command": "cargo test"}, "tool_response": {"exit_code": 0},
            })
        },
        expected_kinds: &[EventKind::TestExecuted, EventKind::TestResult],
    },
    StructuralFixture {
        label: "research activity",
        event: "PostToolUse",
        payload: || json!({"session_id": "s1", "tool_name": "web_search", "tool_response": {"exit_code": 0}}),
        expected_kinds: &[EventKind::ResearchActivity],
    },
];

#[test]
fn codex_produces_the_canonical_event_for_every_structural_no_evidence_capability_it_implements() {
    run_structural_fixtures(AgentId::Codex, CODEX_FIXTURES);
}

const OPENCODE_FIXTURES: &[StructuralFixture] = &[
    StructuralFixture {
        label: "a session start",
        event: "session.created",
        payload: || json!({"sessionID": "s1", "source": "startup"}),
        expected_kinds: &[EventKind::SessionOpened],
    },
    StructuralFixture {
        label: "a manual pre-compaction",
        event: "experimental.session.compacting",
        payload: || json!({"sessionID": "s1", "trigger": "manual"}),
        expected_kinds: &[EventKind::ContextCompacting],
    },
    StructuralFixture {
        label: "an automatic post-compaction",
        event: "session.compacted",
        payload: || json!({"sessionID": "s1", "trigger": "auto"}),
        expected_kinds: &[EventKind::ContextCompacted],
    },
    StructuralFixture {
        // `session.idle` means the agent went quiet; it is never
        // `session_closed` (FR-116) — see the dedicated test below.
        label: "the agent going idle",
        event: "session.idle",
        payload: || json!({"sessionID": "s1"}),
        expected_kinds: &[EventKind::AgentQuiesced],
    },
    StructuralFixture {
        label: "a successful shell command",
        event: "tool.execute.after",
        payload: || {
            json!({
                "sessionID": "s1", "tool": "bash",
                "args": {"command": "echo hi"}, "output": {"exit_code": 0},
            })
        },
        expected_kinds: &[EventKind::ToolSucceeded, EventKind::CommandExecuted],
    },
    StructuralFixture {
        label: "a failed shell command",
        event: "tool.execute.after",
        payload: || {
            json!({
                "sessionID": "s1", "tool": "bash",
                "args": {"command": "ls /nope"}, "output": {"exit_code": 1},
            })
        },
        expected_kinds: &[EventKind::ToolFailed],
    },
    StructuralFixture {
        label: "a file read",
        event: "tool.execute.after",
        payload: || {
            json!({
                "sessionID": "s1", "tool": "read",
                "args": {"filePath": "/repo/src/lib.rs"}, "output": {"exit_code": 0},
            })
        },
        expected_kinds: &[EventKind::FileRead],
    },
    StructuralFixture {
        label: "a file edit",
        event: "tool.execute.after",
        payload: || {
            json!({
                "sessionID": "s1", "tool": "edit",
                "args": {"filePath": "/repo/src/lib.rs"}, "output": {"exit_code": 0},
            })
        },
        expected_kinds: &[EventKind::FileChanged],
    },
    StructuralFixture {
        label: "a passing test run",
        event: "tool.execute.after",
        payload: || {
            json!({
                "sessionID": "s1", "tool": "bash",
                "args": {"command": "cargo test"}, "output": {"exit_code": 0},
            })
        },
        expected_kinds: &[EventKind::TestExecuted, EventKind::TestResult],
    },
    StructuralFixture {
        label: "research activity",
        event: "tool.execute.after",
        payload: || json!({"sessionID": "s1", "tool": "webfetch", "output": {"exit_code": 0}}),
        expected_kinds: &[EventKind::ResearchActivity],
    },
];

#[test]
fn opencode_produces_the_canonical_event_for_every_structural_no_evidence_capability_it_implements()
{
    run_structural_fixtures(AgentId::Opencode, OPENCODE_FIXTURES);
}

// ---------------------------------------------------------------------------
// Part 3 — the semantic-source cells (FR-838a, §13.10)
// ---------------------------------------------------------------------------

#[test]
fn claude_code_and_codex_map_the_user_prompt_field_to_an_instruction_and_the_settled_assistant_field_to_a_decision(
) {
    // §13.10: the event kind follows from which vendor field the material
    // came from, never from a reading of the text. `UserPromptSubmit.prompt`
    // emits `user_instruction_signal`; `Stop`/`SubagentStop`'s settled
    // assistant field emits `decision_signal`.
    for agent in [AgentId::ClaudeCode, AgentId::Codex] {
        let (vocabulary, established_values) = vocab("routing", "middleware");
        let env = agents::CaptureEnv {
            repo_root: None,
            vocabulary: &vocabulary,
            established_values: &established_values,
        };

        let prompt = json!({"session_id": "s1", "prompt": "please require middleware for routing"});
        let out = capture(
            agent,
            "UserPromptSubmit",
            &RawPayload::new(prompt, "."),
            &env,
        );
        assert!(
            out.events
                .iter()
                .any(|d| d.kind == EventKind::UserInstructionSignal),
            "{agent:?}: the prompt field did not map to an instruction ({out:?})"
        );

        let stop = json!({"session_id": "s1", "last_assistant_message": "we should use middleware for routing"});
        let out = capture(agent, "Stop", &RawPayload::new(stop, "."), &env);
        assert!(
            out.events
                .iter()
                .any(|d| d.kind == EventKind::DecisionSignal),
            "{agent:?}: Stop's settled assistant field did not map to a decision ({out:?})"
        );

        let subagent_stop = json!({
            "session_id": "s1", "agent_id": "sub-1", "agent_type": "reviewer",
            "last_assistant_message": "we should use middleware for routing",
        });
        let out = capture(
            agent,
            "SubagentStop",
            &RawPayload::new(subagent_stop, "."),
            &env,
        );
        assert!(
            out.events
                .iter()
                .any(|d| d.kind == EventKind::DecisionSignal),
            "{agent:?}: SubagentStop's settled assistant field did not map to a decision ({out:?})"
        );
    }
}

#[test]
fn opencodes_semantic_cells_are_declined_by_cairn_and_it_emits_no_semantic_signal_for_any_registered_event(
) {
    // FR-838b, §13.10: OpenCode's semantic cells are `declined_by_cairn`
    // structurally — there is no field the adapter reads — and no payload,
    // however prompt-shaped, changes that.
    let declared = declared_matrix("opencode");
    for capability in [
        MatrixCapability::Event(EventKind::UserInstructionSignal),
        MatrixCapability::Event(EventKind::DecisionSignal),
    ] {
        let cell = declared
            .iter()
            .find(|c| c.capability == capability.key())
            .expect("every declared capability has a cell");
        assert_eq!(
            cell.status,
            MatrixStatus::DeclinedByCairn,
            "{}",
            capability.key()
        );
    }

    let vocabulary = SessionVocabulary::new();
    let established_values = BTreeMap::new();
    let env = agents::CaptureEnv {
        repo_root: None,
        vocabulary: &vocabulary,
        established_values: &established_values,
    };
    // Every plausible name a prompt or assistant-text field might carry,
    // across vendors and API generations, on one payload.
    let prompt_shaped = json!({
        "sessionID": "s1",
        "session_id": "s1",
        "prompt": "use redis for storage",
        "text": "use redis for storage",
        "message": "use redis for storage",
        "input": "use redis for storage",
        "last_assistant_message": "use redis for storage",
        "event": {"prompt": {"text": "use redis for storage"}},
    });
    for &event in adapter_for(AgentId::Opencode).registered_events() {
        assert!(
            !carries_semantic_material(AgentId::Opencode, event),
            "{event}: OpenCode's adapter unexpectedly claims to carry semantic material"
        );
        let out = capture(
            AgentId::Opencode,
            event,
            &RawPayload::new(prompt_shaped.clone(), "."),
            &env,
        );
        assert!(
            !out.events.iter().any(|d| matches!(
                d.kind,
                EventKind::UserInstructionSignal | EventKind::DecisionSignal
            )),
            "{event}: produced a semantic signal despite OpenCode's decline ({out:?})"
        );
    }
}

// ---------------------------------------------------------------------------
// Part 4 — subagent attribution
// ---------------------------------------------------------------------------

#[test]
fn subagent_completion_is_attributed_on_claude_code_and_codex_and_absent_from_opencode() {
    for agent in [AgentId::ClaudeCode, AgentId::Codex] {
        let payload = json!({"session_id": "s1", "agent_id": "sub-42", "agent_type": "reviewer"});
        let out = capture(
            agent,
            "SubagentStop",
            &RawPayload::new(payload, "."),
            &agents::CaptureEnv::default(),
        );
        let draft = out
            .events
            .iter()
            .find(|d| d.kind == EventKind::SubagentCompleted)
            .unwrap_or_else(|| panic!("{agent:?} did not attribute the subagent completion"));
        match &draft.content {
            Some(EventContent::Subagent {
                subagent_ref,
                subagent_kind,
                ..
            }) => {
                assert_eq!(subagent_ref, "sub-42");
                assert_eq!(subagent_kind, "reviewer");
            }
            other => panic!("{agent:?}: unexpected content for a subagent completion: {other:?}"),
        }
    }

    // OpenCode registers no subagent-shaped event at all — both cells are a
    // fact about the vendor, declared as such.
    assert!(!adapter_for(AgentId::Opencode)
        .registered_events()
        .contains(&"SubagentStop"));
    let declared = declared_matrix("opencode");
    for capability in [
        MatrixCapability::Event(EventKind::SubagentStarted),
        MatrixCapability::Event(EventKind::SubagentCompleted),
    ] {
        let cell = declared
            .iter()
            .find(|c| c.capability == capability.key())
            .expect("every declared capability has a cell");
        assert_eq!(
            cell.status,
            MatrixStatus::UnsupportedByVendor,
            "{}",
            capability.key()
        );
    }
}

// ---------------------------------------------------------------------------
// Part 5 — a decline becomes its own `capture_declined` record
// ---------------------------------------------------------------------------

#[test]
fn a_decline_becomes_its_own_capture_declined_event_for_every_agent() {
    // The `capture_declined` cell's evidence: any decline can be turned into
    // its own record by `CaptureDecline::as_event`, deterministically, for
    // every agent.
    let cases: [(AgentId, &str, Value); 3] = [
        (
            AgentId::ClaudeCode,
            "PostToolUse",
            json!({"session_id": "s1"}),
        ),
        (AgentId::Codex, "PostToolUse", json!({"session_id": "s1"})),
        (
            AgentId::Opencode,
            "tool.execute.after",
            json!({"sessionID": "s1"}),
        ),
    ];
    for (agent, event, payload) in cases {
        let out = capture(
            agent,
            event,
            &RawPayload::new(payload, "."),
            &agents::CaptureEnv::default(),
        );
        let decline = out.declines.first().unwrap_or_else(|| {
            panic!("{agent:?}: a tool event with no tool name produced no decline")
        });
        let as_event = decline.as_event(event_agent_of(agent), Some(event.to_string()));
        assert_eq!(as_event.kind, EventKind::CaptureDeclined);
    }
}

// ---------------------------------------------------------------------------
// Part 6 — an unregistered vendor event
// ---------------------------------------------------------------------------

#[test]
fn an_unregistered_vendor_event_produces_an_empty_capture_output_for_every_agent() {
    // FR-115: an event Cairn does not map is the normal case, not a failure.
    let env = agents::CaptureEnv::default();
    for agent in [AgentId::ClaudeCode, AgentId::Codex, AgentId::Opencode] {
        let payload = json!({"session_id": "s1", "sessionID": "s1"});
        let out = capture(
            agent,
            "NotARealVendorEvent",
            &RawPayload::new(payload, "."),
            &env,
        );
        assert!(
            out.is_empty(),
            "{agent:?} produced something for an unregistered event: {out:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// Part 7 — file identity dispositions (SC-707, SC-744)
// ---------------------------------------------------------------------------

/// Build the vendor event and payload one agent's edit-class tool would send
/// for a given raw `file_path` (or its absence).
fn edit_fixture(agent: AgentId, path_value: Option<&str>) -> (&'static str, Value) {
    match agent {
        AgentId::ClaudeCode => (
            "PostToolUse",
            match path_value {
                Some(p) => json!({
                    "session_id": "s1", "tool_name": "Edit",
                    "tool_input": {"file_path": p}, "tool_response": {"exit_code": 0},
                }),
                None => json!({
                    "session_id": "s1", "tool_name": "Edit",
                    "tool_input": {}, "tool_response": {"exit_code": 0},
                }),
            },
        ),
        AgentId::Codex => (
            "PostToolUse",
            match path_value {
                Some(p) => json!({
                    "session_id": "s1", "tool_name": "apply_patch",
                    "tool_input": {"file_path": p}, "tool_response": {"exit_code": 0},
                }),
                None => json!({
                    "session_id": "s1", "tool_name": "apply_patch",
                    "tool_input": {}, "tool_response": {"exit_code": 0},
                }),
            },
        ),
        AgentId::Opencode => (
            "tool.execute.after",
            match path_value {
                Some(p) => json!({
                    "sessionID": "s1", "tool": "edit",
                    "args": {"filePath": p}, "output": {"exit_code": 0},
                }),
                None => json!({
                    "sessionID": "s1", "tool": "edit",
                    "args": {}, "output": {"exit_code": 0},
                }),
            },
        ),
        AgentId::GenericMcp => unreachable!(),
    }
}

#[test]
fn a_file_changing_tool_never_synthesizes_an_identity_and_always_reports_one_of_the_three_dispositions(
) {
    // SC-707, SC-744: `present` with a repository-relative path, or an
    // explicit `out_of_repository` / `unavailable_from_vendor` — and never a
    // fabricated value, a cwd substitute, or a degrade to `command_executed`
    // — for every supported agent and five path shapes.
    let repo = TempDir::new().expect("a real repo root");
    let outside = TempDir::new().expect("a real directory outside the repo");
    let inside_abs = repo.path().join("src/lib.rs");
    let outside_abs = outside.path().join("intruder.rs");

    let cases: [(&str, Option<String>, FileIdentity, Option<&str>); 5] = [
        (
            "an absolute path inside the repository",
            Some(inside_abs.to_string_lossy().into_owned()),
            FileIdentity::Present,
            Some("src/lib.rs"),
        ),
        (
            "an absolute path outside the repository",
            Some(outside_abs.to_string_lossy().into_owned()),
            FileIdentity::OutOfRepository,
            None,
        ),
        (
            "a relative path",
            Some("src/lib.rs".to_string()),
            FileIdentity::Present,
            Some("src/lib.rs"),
        ),
        (
            "a traversing path",
            Some("../../etc/passwd".to_string()),
            FileIdentity::OutOfRepository,
            None,
        ),
        (
            "a missing path",
            None,
            FileIdentity::UnavailableFromVendor,
            None,
        ),
    ];

    for agent in [AgentId::ClaudeCode, AgentId::Codex, AgentId::Opencode] {
        for (label, path_value, expected_identity, expected_repo_file) in &cases {
            let (event, payload) = edit_fixture(agent, path_value.as_deref());
            let env = agents::CaptureEnv {
                repo_root: Some(repo.path()),
                vocabulary: &SessionVocabulary::new(),
                established_values: &BTreeMap::new(),
            };
            let out = capture(agent, event, &RawPayload::new(payload, "."), &env);

            let file_draft = out
                .events
                .iter()
                .find(|d| d.kind == EventKind::FileChanged)
                .unwrap_or_else(|| {
                    panic!("{agent:?} / {label}: no file_changed event was produced ({out:?})")
                });
            match &file_draft.content {
                Some(EventContent::File {
                    file_identity,
                    repo_file,
                    ..
                }) => {
                    assert_eq!(
                        file_identity, expected_identity,
                        "{agent:?} / {label}: wrong file identity disposition"
                    );
                    assert_eq!(
                        repo_file.as_deref(),
                        *expected_repo_file,
                        "{agent:?} / {label}: wrong (or fabricated) repo_file"
                    );
                }
                other => {
                    panic!("{agent:?} / {label}: file_changed carried unexpected content {other:?}")
                }
            }
            assert!(
                !out.events
                    .iter()
                    .any(|d| d.kind == EventKind::CommandExecuted),
                "{agent:?} / {label}: degraded into a generic command_executed"
            );
        }
    }
}
