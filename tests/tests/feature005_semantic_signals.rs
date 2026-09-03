//! Vocabulary-justified semantic signals, as a contract (T041,
//! `contracts/extraction.md` §13.3, §13.7, §13.10; `spec.md` SC-701a,
//! SC-701b).
//!
//! Every scenario here is driven through the **real** per-vendor field map
//! and routing table — `cairn_integrate::capture` — exactly as a hook
//! invocation would drive it: a vendor, a source role, the session's prior
//! events (which is what builds the vocabulary a signal's tokens must be
//! justified against), and the transient text a `UserPromptSubmit` or `Stop`
//! payload would actually carry. No PostgreSQL, no daemon.
//!
//! The scenario set is fixed in [`SCENARIOS`] below, *before* any assertion
//! reads it, so the population SC-701a and SC-701b are measured against
//! cannot be narrowed after the fact.

use cairn_core::event::{
    ChangeKind, DecisionKind, DeclineReason, EventContent, EventKind, FileIdentity, InstructionKind,
};
use cairn_core::lexicon::{SourceRole, LEXICON_VERSION};
use cairn_core::vocabulary::SessionVocabulary;
use cairn_integrate::agents::CaptureEnv;
use cairn_integrate::{capture, AgentId, RawPayload};
use serde_json::json;
use std::collections::BTreeMap;

// ---------------------------------------------------------------------------
// The pre-registered scenario set
// ---------------------------------------------------------------------------

/// What a scenario declares it expects: a produced signal with exact subject
/// and object tokens (and its granular `decision_kind`/`instruction_kind`),
/// or a decline with a named reason.
#[derive(Debug, Clone, Copy)]
enum Expect {
    Decision {
        subject: &'static str,
        object: &'static str,
        decision_kind: DecisionKind,
    },
    Instruction {
        subject: &'static str,
        object: &'static str,
        instruction_kind: InstructionKind,
    },
    Decline(DeclineReason),
}

/// One session: a vendor, a source role, the file this session touched
/// before the turn under test (which is what seeds the vocabulary), the
/// transient text, and the declared expectation.
#[derive(Debug, Clone, Copy)]
struct Scenario {
    name: &'static str,
    vendor: AgentId,
    role: SourceRole,
    /// The prior event's `repo_file` is `"{module}/{object}.rs"` — a
    /// directory component and a file component, ranked `Module` and `File`
    /// respectively (§13.5), which is what makes `module` the subject and
    /// `object` the object whenever both survive step 4.
    module: &'static str,
    object: &'static str,
    text: &'static str,
    expect: Expect,
}

/// Twenty pre-registered sessions: sixteen in which a decision or a standing
/// instruction is expressed and then acted on (the population SC-701a counts
/// against), and four adversarial ones exercising the three decline reasons
/// `contracts/extraction.md` §13.7 step 6 distinguishes. Claude Code and
/// Codex CLI only — OpenCode emits no semantic signals at all (FR-727e) and
/// is covered instead in `feature005_capture_matrix.rs`.
const SCENARIOS: &[Scenario] = &[
    // -- decisions (the settled assistant field) -----------------------------
    Scenario {
        name: "claude code adopts postgresql for storage",
        vendor: AgentId::ClaudeCode,
        role: SourceRole::AssistantMessage,
        module: "storage",
        object: "postgresql",
        text: "we should use postgresql for storage",
        expect: Expect::Decision {
            subject: "storage",
            object: "postgresql",
            decision_kind: DecisionKind::Adopt,
        },
    },
    Scenario {
        name: "codex adopts redis for caching",
        vendor: AgentId::Codex,
        role: SourceRole::AssistantMessage,
        module: "caching",
        object: "redis",
        text: "let's adopt redis for caching",
        expect: Expect::Decision {
            subject: "caching",
            object: "redis",
            decision_kind: DecisionKind::Adopt,
        },
    },
    Scenario {
        name: "claude code rejects syslog for logging",
        vendor: AgentId::ClaudeCode,
        role: SourceRole::AssistantMessage,
        module: "logging",
        object: "syslog",
        text: "let's drop syslog for logging",
        expect: Expect::Decision {
            subject: "logging",
            object: "syslog",
            decision_kind: DecisionKind::Reject,
        },
    },
    Scenario {
        name: "codex rejects grpc for transport",
        vendor: AgentId::Codex,
        role: SourceRole::AssistantMessage,
        module: "transport",
        object: "grpc",
        text: "we reject grpc for transport",
        expect: Expect::Decision {
            subject: "transport",
            object: "grpc",
            decision_kind: DecisionKind::Reject,
        },
    },
    Scenario {
        name: "claude code defers kubernetes for deployment",
        vendor: AgentId::ClaudeCode,
        role: SourceRole::AssistantMessage,
        module: "deployment",
        object: "kubernetes",
        text: "let's defer kubernetes for deployment",
        expect: Expect::Decision {
            subject: "deployment",
            object: "kubernetes",
            decision_kind: DecisionKind::Defer,
        },
    },
    Scenario {
        name: "codex defers cron for scheduling",
        vendor: AgentId::Codex,
        role: SourceRole::AssistantMessage,
        module: "scheduling",
        object: "cron",
        text: "let's postpone cron for scheduling",
        expect: Expect::Decision {
            subject: "scheduling",
            object: "cron",
            decision_kind: DecisionKind::Defer,
        },
    },
    Scenario {
        name: "claude code constrains schema for validation",
        vendor: AgentId::ClaudeCode,
        role: SourceRole::AssistantMessage,
        module: "validation",
        object: "schema",
        text: "we must apply schema for validation",
        expect: Expect::Decision {
            subject: "validation",
            object: "schema",
            decision_kind: DecisionKind::Constrain,
        },
    },
    Scenario {
        name: "codex reverts certificates for security",
        vendor: AgentId::Codex,
        role: SourceRole::AssistantMessage,
        module: "security",
        object: "certificates",
        text: "we never trust certificates for security",
        expect: Expect::Decision {
            subject: "security",
            object: "certificates",
            decision_kind: DecisionKind::Revert,
        },
    },
    // -- standing instructions (the user-prompt field) -----------------------
    Scenario {
        name: "claude code requires middleware for routing",
        vendor: AgentId::ClaudeCode,
        role: SourceRole::UserPrompt,
        module: "routing",
        object: "middleware",
        text: "please require middleware for routing",
        expect: Expect::Instruction {
            subject: "routing",
            object: "middleware",
            instruction_kind: InstructionKind::Require,
        },
    },
    Scenario {
        name: "codex requires metrics for monitoring",
        vendor: AgentId::Codex,
        role: SourceRole::UserPrompt,
        module: "monitoring",
        object: "metrics",
        text: "please enforce metrics for monitoring",
        expect: Expect::Instruction {
            subject: "monitoring",
            object: "metrics",
            instruction_kind: InstructionKind::Require,
        },
    },
    Scenario {
        name: "claude code forbids keys for encryption",
        vendor: AgentId::ClaudeCode,
        role: SourceRole::UserPrompt,
        module: "encryption",
        object: "keys",
        text: "please forbid keys for encryption",
        expect: Expect::Instruction {
            subject: "encryption",
            object: "keys",
            instruction_kind: InstructionKind::Forbid,
        },
    },
    Scenario {
        name: "codex forbids cookies for sessions",
        vendor: AgentId::Codex,
        role: SourceRole::UserPrompt,
        module: "sessions",
        object: "cookies",
        text: "don't touch cookies in sessions",
        expect: Expect::Instruction {
            subject: "sessions",
            object: "cookies",
            instruction_kind: InstructionKind::Forbid,
        },
    },
    Scenario {
        name: "claude code prefers templates for rendering",
        vendor: AgentId::ClaudeCode,
        role: SourceRole::UserPrompt,
        module: "rendering",
        object: "templates",
        text: "please prefer templates for rendering",
        expect: Expect::Instruction {
            subject: "rendering",
            object: "templates",
            instruction_kind: InstructionKind::Prefer,
        },
    },
    Scenario {
        name: "codex scopes invoices to billing",
        vendor: AgentId::Codex,
        role: SourceRole::UserPrompt,
        module: "billing",
        object: "invoices",
        text: "please scope to invoices for billing",
        expect: Expect::Instruction {
            subject: "billing",
            object: "invoices",
            instruction_kind: InstructionKind::Scope,
        },
    },
    Scenario {
        name: "claude code scopes index to search",
        vendor: AgentId::ClaudeCode,
        role: SourceRole::UserPrompt,
        module: "search",
        object: "index",
        text: "only in search for index",
        expect: Expect::Instruction {
            subject: "search",
            object: "index",
            instruction_kind: InstructionKind::Scope,
        },
    },
    Scenario {
        name: "codex corrects queue for notifications",
        vendor: AgentId::Codex,
        role: SourceRole::UserPrompt,
        module: "notifications",
        object: "queue",
        text: "actually configure queue for notifications",
        expect: Expect::Instruction {
            subject: "notifications",
            object: "queue",
            instruction_kind: InstructionKind::Correct,
        },
    },
    // -- adversarial: each decline reason at least once ----------------------
    Scenario {
        name: "no marker at all names nothing to decide",
        vendor: AgentId::ClaudeCode,
        role: SourceRole::AssistantMessage,
        module: "reporting",
        object: "dashboards",
        text: "the tests are passing now",
        expect: Expect::Decline(DeclineReason::NoSafeSemanticMapping),
    },
    Scenario {
        name: "a marker with nothing this session established",
        vendor: AgentId::Codex,
        role: SourceRole::UserPrompt,
        module: "billing",
        object: "receipts",
        text: "please require compliance for auditing",
        expect: Expect::Decline(DeclineReason::InsufficientVocabulary),
    },
    Scenario {
        name: "two markers of different kinds in one assistant turn",
        vendor: AgentId::ClaudeCode,
        role: SourceRole::AssistantMessage,
        module: "storage",
        object: "redis",
        text: "use redis, but never storage",
        expect: Expect::Decline(DeclineReason::AmbiguousClassification),
    },
    Scenario {
        name: "a decision-only marker in a user prompt has no counterpart",
        vendor: AgentId::Codex,
        role: SourceRole::UserPrompt,
        module: "orchestration",
        object: "kubernetes",
        text: "adopt kubernetes for orchestration",
        expect: Expect::Decline(DeclineReason::AmbiguousClassification),
    },
];

// ---------------------------------------------------------------------------
// Running one scenario through the real capture path
// ---------------------------------------------------------------------------

/// The vocabulary a scenario's prior events establish — one `file_changed`
/// at `session_seq` 1, built the way `cairnd::capture::session_vocabulary`
/// builds one from a session's accepted events.
fn vocabulary_for(module: &str, object: &str) -> SessionVocabulary {
    let mut vocabulary = SessionVocabulary::new();
    let content = EventContent::File {
        repo_file: Some(format!("{module}/{object}.rs")),
        repo_file_from: None,
        change_kind: Some(ChangeKind::Modified),
        file_identity: FileIdentity::Present,
    };
    vocabulary.observe_at(Some(1), EventKind::FileChanged, Some(&content));
    vocabulary
}

/// Drive one piece of transient text through the real vendor field map, for
/// an arbitrary (possibly runtime-built) text — used both by scenarios and by
/// the credential-injection test, which needs to vary `text` independently of
/// the fixed table.
fn run_text(
    vendor: AgentId,
    role: SourceRole,
    module: &str,
    object: &str,
    text: &str,
) -> cairn_core::event::CaptureOutput {
    let vocabulary = vocabulary_for(module, object);
    let established_values: BTreeMap<String, String> = BTreeMap::new();
    let env = CaptureEnv {
        repo_root: None,
        vocabulary: &vocabulary,
        established_values: &established_values,
    };
    let (event, field) = match role {
        SourceRole::UserPrompt => ("UserPromptSubmit", "prompt"),
        SourceRole::AssistantMessage => ("Stop", "last_assistant_message"),
    };
    let mut payload = json!({"session_id": "scenario-session"});
    payload[field] = json!(text);
    capture(vendor, event, &RawPayload::new(payload, "."), &env)
}

fn run(s: &Scenario) -> cairn_core::event::CaptureOutput {
    run_text(s.vendor, s.role, s.module, s.object, s.text)
}

/// The signal content this scenario's role would have produced, if any.
fn produced(out: &cairn_core::event::CaptureOutput, role: SourceRole) -> Option<&EventContent> {
    let kind = role.event_kind();
    out.events
        .iter()
        .find(|d| d.kind == kind)
        .and_then(|d| d.content.as_ref())
}

/// The decline reason recorded for this scenario's role, if any.
fn declined(out: &cairn_core::event::CaptureOutput, role: SourceRole) -> Option<DeclineReason> {
    let kind = role.event_kind();
    out.declines
        .iter()
        .find(|d| d.kind == kind)
        .map(|d| d.reason)
}

// ---------------------------------------------------------------------------
// The set itself
// ---------------------------------------------------------------------------

#[test]
fn the_pre_registered_scenario_set_has_at_least_twenty_sessions_drawn_only_from_claude_code_and_codex(
) {
    // SC-701a's population, and FR-727e's exclusion: OpenCode emits no
    // semantic signals, so it has no place in this table.
    assert!(
        SCENARIOS.len() >= 20,
        "the pre-registered set must have at least twenty sessions"
    );
    for s in SCENARIOS {
        assert!(
            matches!(s.vendor, AgentId::ClaudeCode | AgentId::Codex),
            "{}: used a vendor outside the semantic-signal population",
            s.name
        );
    }
}

// ---------------------------------------------------------------------------
// SC-701a
// ---------------------------------------------------------------------------

#[test]
fn at_least_fourteen_of_the_twenty_scenarios_produce_a_record_matching_its_declared_expectation() {
    // SC-701a. Counted independently of whether a scenario's *own* row
    // expected a match or a decline: the criterion is about the population as
    // a whole, not about this table grading its own homework.
    let mut matched = 0usize;
    let mut shortfalls: Vec<&'static str> = Vec::new();

    for s in SCENARIOS {
        let out = run(s);
        let is_match = match (produced(&out, s.role), s.expect) {
            (
                Some(EventContent::Decision {
                    decision_kind,
                    subject_token,
                    object_token,
                    ..
                }),
                Expect::Decision {
                    subject,
                    object,
                    decision_kind: expected_kind,
                },
            ) => {
                subject_token.as_str() == subject
                    && object_token.as_str() == object
                    && *decision_kind == expected_kind
            }
            (
                Some(EventContent::Instruction {
                    instruction_kind,
                    subject_token,
                    object_token,
                    ..
                }),
                Expect::Instruction {
                    subject,
                    object,
                    instruction_kind: expected_kind,
                },
            ) => {
                subject_token.as_str() == subject
                    && object_token.as_str() == object
                    && *instruction_kind == expected_kind
            }
            _ => false,
        };
        if is_match {
            matched += 1;
        } else {
            shortfalls.push(s.name);
        }
    }

    assert!(
        matched >= 14,
        "only {matched}/{} scenarios produced a matching decision or instruction; \
         scenarios that fell short: {shortfalls:?}",
        SCENARIOS.len()
    );
}

/// A finer-grained companion to the count above: every scenario resolves
/// *exactly* to what it declared, so a shortfall is diagnosable by name
/// rather than only by count.
#[test]
fn every_scenario_resolves_to_its_own_declared_expectation() {
    let mut failures: Vec<String> = Vec::new();

    for s in SCENARIOS {
        let out = run(s);
        match s.expect {
            Expect::Decision {
                subject,
                object,
                decision_kind,
            } => match produced(&out, s.role) {
                Some(EventContent::Decision {
                    subject_token,
                    object_token,
                    decision_kind: actual,
                    lexicon_version,
                    ..
                }) => {
                    if subject_token.as_str() != subject
                        || object_token.as_str() != object
                        || *actual != decision_kind
                    {
                        failures.push(format!(
                            "{}: expected Decision({decision_kind:?}, {subject}, {object}), got Decision({actual:?}, {subject_token}, {object_token})",
                            s.name
                        ));
                    }
                    if *lexicon_version != LEXICON_VERSION {
                        failures.push(format!(
                            "{}: lexicon_version {lexicon_version} != {LEXICON_VERSION}",
                            s.name
                        ));
                    }
                }
                other => failures.push(format!(
                    "{}: expected a decision, got {other:?} (declines: {:?})",
                    s.name, out.declines
                )),
            },
            Expect::Instruction {
                subject,
                object,
                instruction_kind,
            } => match produced(&out, s.role) {
                Some(EventContent::Instruction {
                    subject_token,
                    object_token,
                    instruction_kind: actual,
                    lexicon_version,
                    ..
                }) => {
                    if subject_token.as_str() != subject
                        || object_token.as_str() != object
                        || *actual != instruction_kind
                    {
                        failures.push(format!(
                            "{}: expected Instruction({instruction_kind:?}, {subject}, {object}), got Instruction({actual:?}, {subject_token}, {object_token})",
                            s.name
                        ));
                    }
                    if *lexicon_version != LEXICON_VERSION {
                        failures.push(format!(
                            "{}: lexicon_version {lexicon_version} != {LEXICON_VERSION}",
                            s.name
                        ));
                    }
                }
                other => failures.push(format!(
                    "{}: expected an instruction, got {other:?} (declines: {:?})",
                    s.name, out.declines
                )),
            },
            Expect::Decline(reason) => match declined(&out, s.role) {
                Some(actual) if actual == reason => {}
                Some(actual) => failures.push(format!(
                    "{}: expected decline {reason:?}, got {actual:?}",
                    s.name
                )),
                None => failures.push(format!(
                    "{}: expected a decline ({reason:?}) but a signal was produced: {:?}",
                    s.name,
                    produced(&out, s.role)
                )),
            },
        }
    }

    assert!(
        failures.is_empty(),
        "scenario mismatches:\n{}",
        failures.join("\n")
    );
}

// ---------------------------------------------------------------------------
// SC-701b — the falsifiable form of "reasoning does not cross the boundary"
// ---------------------------------------------------------------------------

/// Case-folded words of a source text, split the same way a human would read
/// it apart — not the lexicon's own tokenizer, so this check cannot pass
/// merely by agreeing with the code under test.
fn words_in(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| w.to_ascii_lowercase())
        .collect()
}

#[test]
fn no_emitted_token_contains_a_segment_that_is_not_independently_in_the_sessions_vocabulary() {
    // SC-701b. For every scenario that actually produced a signal: split its
    // source text into words, and for every segment of every emitted token,
    // require (a) the segment is a word that appeared in the source text, and
    // (b) the segment is independently present in the session's own derived
    // vocabulary — not merely quoted from the prose. Either check failing
    // would mean a word crossed the boundary that the vocabulary never
    // established.
    for s in SCENARIOS {
        let vocabulary = vocabulary_for(s.module, s.object);
        let out = run(s);
        let Some(content) = produced(&out, s.role) else {
            continue;
        };
        let source_words = words_in(s.text);

        let tokens: Vec<&str> = match content {
            EventContent::Decision {
                subject_token,
                object_token,
                ..
            }
            | EventContent::Instruction {
                subject_token,
                object_token,
                ..
            } => {
                vec![subject_token.as_str(), object_token.as_str()]
            }
            other => panic!("{}: unexpected content {other:?}", s.name),
        };

        for token in tokens {
            for segment in token.split('.') {
                assert!(
                    vocabulary.contains(segment),
                    "{}: token segment {segment:?} (from {token:?}) is not in the session's own vocabulary",
                    s.name
                );
                assert!(
                    source_words.iter().any(|w| w == segment),
                    "{}: token segment {segment:?} does not even appear as a word in the source text {:?}",
                    s.name, s.text
                );
            }
        }
    }
}

#[test]
fn a_scenario_whose_text_names_nothing_the_session_established_declines_without_emitting_a_token() {
    // The vocabulary-only guarantee, isolated: every scenario declared
    // `InsufficientVocabulary` in the pre-registered set must decline, and
    // must never emit a token built from prose alone.
    let cases: Vec<&Scenario> = SCENARIOS
        .iter()
        .filter(|s| {
            matches!(
                s.expect,
                Expect::Decline(DeclineReason::InsufficientVocabulary)
            )
        })
        .collect();
    assert!(
        !cases.is_empty(),
        "the table must exercise this decline reason at least once"
    );

    for s in cases {
        let out = run(s);
        assert!(
            produced(&out, s.role).is_none(),
            "{}: emitted a token despite an empty vocabulary intersection",
            s.name
        );
        assert_eq!(
            declined(&out, s.role),
            Some(DeclineReason::InsufficientVocabulary),
            "{}",
            s.name
        );
    }
}

// ---------------------------------------------------------------------------
// Credentials never cross, and their presence never changes the outcome
// ---------------------------------------------------------------------------

#[test]
fn a_credential_in_the_source_text_never_appears_in_a_token_and_does_not_change_the_outcome() {
    // Redaction runs before vocabulary derivation and before classification
    // (`contracts/extraction.md` §13.3): appending a credential to an
    // otherwise-identical decision must neither leak it into a token nor
    // change whether — or what — a decision is recorded.
    let base = &SCENARIOS[0];
    assert!(matches!(base.expect, Expect::Decision { .. }));
    let credential = "ghp_abcdefghijklmnopqrstuvwxyz0123456789";
    let with_credential = format!("{} {credential}", base.text);

    let clean = run(base);
    let dirty = run_text(
        base.vendor,
        base.role,
        base.module,
        base.object,
        &with_credential,
    );

    let extract =
        |out: &cairn_core::event::CaptureOutput| -> Option<(String, DecisionKind, String)> {
            match produced(out, base.role) {
                Some(EventContent::Decision {
                    subject_token,
                    decision_kind,
                    object_token,
                    ..
                }) => Some((
                    subject_token.as_str().to_string(),
                    *decision_kind,
                    object_token.as_str().to_string(),
                )),
                _ => None,
            }
        };
    let clean_result = extract(&clean);
    let dirty_result = extract(&dirty);
    assert!(
        clean_result.is_some(),
        "the base scenario itself must produce a decision"
    );
    assert_eq!(
        clean_result, dirty_result,
        "a credential in the source text changed the outcome of an otherwise identical decision"
    );

    for draft in dirty.events.iter().chain(clean.events.iter()) {
        let serialized = serde_json::to_string(draft).expect("serializes");
        assert!(
            !serialized.contains("ghp_") && !serialized.contains(credential),
            "a credential leaked into an emitted event: {serialized}"
        );
    }
}
