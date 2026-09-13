//! The event model as a contract, from outside the crate (T014).
//!
//! Unit tests inside `event.rs` and `eventid.rs` check the pieces. This file
//! checks the properties the rest of the system depends on and that no single
//! piece can establish alone:
//!
//! - the union really is closed at twenty-one, and nothing quietly widened it;
//! - the seven lifecycle events still map across without loss, so handoffs and
//!   context delivery keep working;
//! - content and kind cannot be mixed;
//! - every bound refuses rather than truncates;
//! - identity survives a retry and never reads a clock;
//! - a reference encodes its whole domain.

use cairn_core::domain::{KnowledgeDomain, KnowledgeRef, PatternRef, RecordType, Reference};
use cairn_core::event::*;
use cairn_core::eventid;
use cairn_core::lifecycle::CanonicalEvent;
use uuid::Uuid;

fn envelope(kind: EventKind, content: Option<EventContent>) -> SafeCanonicalEvent {
    let session = Uuid::now_v7();
    SafeCanonicalEvent {
        event_id: eventid::event_id(session, 1),
        contract_version: CONTRACT_VERSION,
        kind,
        agent: EventAgent::ClaudeCode,
        vendor_event: Some("PostToolUse".into()),
        session_id: session,
        session_seq: 1,
        occurred_at: chrono::Utc::now(),
        content,
    }
}

/// One legal content value for each of the twenty-one kinds.
fn legal_content(kind: EventKind) -> Option<EventContent> {
    Some(match kind {
        EventKind::SessionOpened | EventKind::SessionResumed => EventContent::SessionOpen {
            open_trigger: OpenTrigger::Startup,
        },
        EventKind::SessionClosed => EventContent::SessionClose {
            close_reason: "ended".into(),
        },
        EventKind::ContextCompacting | EventKind::ContextCompacted => EventContent::Compaction {
            compaction_trigger: CompactionTrigger::Auto,
        },
        EventKind::SubagentStarted | EventKind::SubagentCompleted => EventContent::Subagent {
            subagent_ref: "sub-1".into(),
            subagent_kind: "explore".into(),
            parent_session_seq: 3,
        },
        EventKind::ToolStarted | EventKind::ToolSucceeded => EventContent::Tool {
            vendor_tool: "Bash".into(),
            tool_class: ToolClass::Execute,
        },
        EventKind::ToolFailed => EventContent::ToolFailure {
            vendor_tool: "Bash".into(),
            tool_class: ToolClass::Execute,
            failure_kind: FailureKind::NonZeroExit,
            failure_note: Some("exit 1".into()),
            exit_status: Some(1),
        },
        EventKind::FileRead => EventContent::File {
            repo_file: Some("crates/cairnd/src/sync.rs".into()),
            repo_file_from: None,
            change_kind: None,
            file_identity: FileIdentity::Present,
        },
        EventKind::FileChanged => EventContent::File {
            repo_file: Some("crates/cairnd/src/sync.rs".into()),
            repo_file_from: None,
            change_kind: Some(ChangeKind::Modified),
            file_identity: FileIdentity::Present,
        },
        EventKind::CommandExecuted => EventContent::Command {
            command_line: "cargo test".into(),
            exit_status: Some(0),
        },
        EventKind::TestExecuted => EventContent::TestInvocation {
            test_command: "cargo test -p cairn-core".into(),
        },
        EventKind::TestResult => EventContent::TestVerdict {
            test_outcome: TestOutcome::Failed,
            exit_status: Some(101),
            tests_total: Some(12),
            tests_failed: Some(1),
        },
        EventKind::ResearchActivity => EventContent::Research {
            resource_kind: ResourceKind::Docs,
        },
        EventKind::UserInstructionSignal => EventContent::Instruction {
            instruction_kind: InstructionKind::Require,
            subject_token: VocabToken::subject("deploy.images").unwrap(),
            object_token: VocabToken::object("signed").unwrap(),
            justified_by_seq: Some(2),
            lexicon_version: 1,
        },
        EventKind::DecisionSignal => EventContent::Decision {
            decision_kind: DecisionKind::Adopt,
            subject_token: VocabToken::subject("storage.authority").unwrap(),
            object_token: VocabToken::object("server").unwrap(),
            justified_by_seq: None,
            lexicon_version: 1,
        },
        EventKind::CaptureDeclined | EventKind::CaptureFailed => EventContent::CaptureOutcome {
            disposition: Disposition::DeclinedByPolicy,
            stage: PipelineStage::EventParsed,
            decline_reason: DeclineReason::PolicyExcluded,
        },
        EventKind::AgentQuiesced => return None,
    })
}

// ---------------------------------------------------------------------------
// The union
// ---------------------------------------------------------------------------

#[test]
fn the_union_is_exactly_the_twenty_one_kinds_the_data_model_names() {
    assert_eq!(EventKind::ALL.len(), 21);
    let names: Vec<&str> = EventKind::ALL.iter().map(|k| k.as_str()).collect();
    assert_eq!(
        names,
        vec![
            "session_opened",
            "session_closed",
            "context_compacting",
            "context_compacted",
            "subagent_started",
            "subagent_completed",
            "tool_started",
            "tool_succeeded",
            "tool_failed",
            "file_read",
            "file_changed",
            "command_executed",
            "test_executed",
            "test_result",
            "research_activity",
            "user_instruction_signal",
            "decision_signal",
            "capture_declined",
            "capture_failed",
            "session_resumed",
            "agent_quiesced",
        ]
    );
}

#[test]
fn a_kind_outside_the_union_does_not_parse() {
    use std::str::FromStr;
    for outside in [
        "prompt_submitted",
        "message_sent",
        "assistant_response",
        "SESSION_OPENED",
        "",
    ] {
        assert!(
            EventKind::from_str(outside).is_err(),
            "{outside:?} parsed as a canonical event kind"
        );
    }
}

#[test]
fn every_kind_has_a_legal_content_shape_and_accepts_it() {
    for kind in EventKind::ALL {
        let event = envelope(*kind, legal_content(*kind));
        event
            .validate()
            .unwrap_or_else(|e| panic!("{kind} rejected its own legal content: {e}"));
    }
}

#[test]
fn content_may_not_be_worn_by_a_kind_it_does_not_belong_to() {
    // Every mismatched pairing, not a sample: content that leaks across kinds
    // is how a field reaches a JSONB column the type system was supposed to
    // guard.
    for kind in EventKind::ALL {
        for other in EventKind::ALL {
            let Some(content) = legal_content(*other) else {
                continue;
            };
            let legal = content.is_legal_for(*kind);
            let outcome = envelope(*kind, Some(content)).validate();
            assert_eq!(
                outcome.is_ok(),
                legal,
                "{other}'s content on a {kind} event was {} when it should not have been",
                if outcome.is_ok() {
                    "accepted"
                } else {
                    "refused"
                }
            );
        }
    }
}

#[test]
fn only_the_kind_that_carries_nothing_may_omit_its_content() {
    for kind in EventKind::ALL {
        let outcome = envelope(*kind, None).validate();
        assert_eq!(
            outcome.is_ok(),
            *kind == EventKind::AgentQuiesced,
            "{kind} with no content was handled wrongly"
        );
    }
    // Omitted and explicitly-none are the same statement, so both are accepted.
    assert!(envelope(EventKind::AgentQuiesced, Some(EventContent::None))
        .validate()
        .is_ok());
}

// ---------------------------------------------------------------------------
// The seven lifecycle events (FR-744)
// ---------------------------------------------------------------------------

#[test]
fn all_seven_lifecycle_events_still_cross_without_loss() {
    let mapped: Vec<EventKind> = CanonicalEvent::ALL
        .iter()
        .map(|e| e.safe_event_kind())
        .collect();
    assert_eq!(mapped.len(), 7);
    // Distinct: two lifecycle events collapsing onto one kind would be the
    // loss FR-744 forbids.
    let unique: std::collections::BTreeSet<_> = mapped.iter().collect();
    assert_eq!(unique.len(), 7, "two lifecycle events map to one kind");
    // And each keeps its name, so nothing downstream has to learn a synonym.
    for event in CanonicalEvent::ALL {
        assert_eq!(event.as_str(), event.safe_event_kind().as_str());
    }
}

#[test]
fn the_boundary_class_of_a_lifecycle_event_survives_the_mapping() {
    // The capacity policy sheds capture-class rows and never boundary-class
    // ones, so a lifecycle event that lost its class on the way across would
    // become droppable.
    for event in CanonicalEvent::ALL {
        if event.is_boundary_class() {
            assert!(
                event.safe_event_kind().is_boundary_class(),
                "{event:?} stopped being boundary class"
            );
        }
    }
    // The event model classes more kinds than the lifecycle vocabulary does:
    // `session_resumed` and `context_compacted` also establish the session
    // structure every other event is read against.
    assert!(EventKind::SessionResumed.is_boundary_class());
    assert!(EventKind::ContextCompacted.is_boundary_class());
    assert!(!EventKind::FileRead.is_boundary_class());
    assert_eq!(EventKind::SessionOpened.boundary_class(), 1);
    assert_eq!(EventKind::FileRead.boundary_class(), 0);
}

// ---------------------------------------------------------------------------
// Bounds — refused, never truncated
// ---------------------------------------------------------------------------

#[test]
fn an_over_bound_value_is_refused_rather_than_shortened() {
    let long_path = format!("{}.rs", "a".repeat(REPO_FILE_MAX_BYTES));
    let event = envelope(
        EventKind::FileRead,
        Some(EventContent::File {
            repo_file: Some(long_path.clone()),
            repo_file_from: None,
            change_kind: None,
            file_identity: FileIdentity::Present,
        }),
    );
    let err = event
        .validate()
        .expect_err("an over-long path was accepted");
    assert!(matches!(
        err,
        EventRefusal::TooLong {
            field: "repo_file",
            ..
        }
    ));
    // Truncating instead would be worse than refusing: a shortened path can
    // land inside the repository when the original pointed outside it.
    assert_eq!(long_path.len(), REPO_FILE_MAX_BYTES + 3);
}

#[test]
fn a_path_with_too_many_segments_is_refused_even_when_it_is_short() {
    let deep = vec!["a"; REPO_FILE_MAX_SEGMENTS + 1].join("/");
    assert!(
        deep.len() < REPO_FILE_MAX_BYTES,
        "the byte bound must not be what refuses this"
    );
    let err = envelope(
        EventKind::FileRead,
        Some(EventContent::File {
            repo_file: Some(deep),
            repo_file_from: None,
            change_kind: None,
            file_identity: FileIdentity::Present,
        }),
    )
    .validate()
    .expect_err("a 65-segment path was accepted");
    assert!(matches!(err, EventRefusal::TooManySegments { .. }));
}

#[test]
fn every_free_text_field_is_bounded_at_five_hundred_and_twelve_bytes() {
    let over = "x".repeat(FREE_TEXT_MAX_BYTES + 1);
    let cases = [
        (
            EventKind::CommandExecuted,
            EventContent::Command {
                command_line: over.clone(),
                exit_status: None,
            },
        ),
        (
            EventKind::TestExecuted,
            EventContent::TestInvocation {
                test_command: over.clone(),
            },
        ),
        (
            EventKind::ToolFailed,
            EventContent::ToolFailure {
                vendor_tool: "Bash".into(),
                tool_class: ToolClass::Execute,
                failure_kind: FailureKind::Unknown,
                failure_note: Some(over.clone()),
                exit_status: None,
            },
        ),
    ];
    for (kind, content) in cases {
        assert!(
            envelope(kind, Some(content)).validate().is_err(),
            "{kind} accepted an over-long free-text field"
        );
    }
}

#[test]
fn vendor_provenance_fields_are_bounded_at_sixty_four_characters() {
    let over = "v".repeat(VENDOR_TOKEN_MAX_CHARS + 1);
    let mut event = envelope(EventKind::AgentQuiesced, None);
    event.vendor_event = Some(over.clone());
    assert!(event.validate().is_err(), "vendor_event was not bounded");

    assert!(envelope(
        EventKind::ToolStarted,
        Some(EventContent::Tool {
            vendor_tool: over.clone(),
            tool_class: ToolClass::Other,
        })
    )
    .validate()
    .is_err());

    assert!(envelope(
        EventKind::SubagentStarted,
        Some(EventContent::Subagent {
            subagent_ref: "r".into(),
            subagent_kind: over,
            parent_session_seq: 0,
        })
    )
    .validate()
    .is_err());

    // `subagent_ref` gets the longer bound, and one character over it fails.
    assert!(envelope(
        EventKind::SubagentStarted,
        Some(EventContent::Subagent {
            subagent_ref: "r".repeat(SUBAGENT_REF_MAX_CHARS + 1),
            subagent_kind: "explore".into(),
            parent_session_seq: 0,
        })
    )
    .validate()
    .is_err());
}

#[test]
fn a_file_event_cannot_disagree_with_itself_about_whether_it_has_a_path() {
    // `present` with nothing to be present.
    assert!(envelope(
        EventKind::FileRead,
        Some(EventContent::File {
            repo_file: None,
            repo_file_from: None,
            change_kind: None,
            file_identity: FileIdentity::Present,
        })
    )
    .validate()
    .is_err());

    // Out of the repository, yet somehow carrying a repository-relative path.
    for identity in [
        FileIdentity::OutOfRepository,
        FileIdentity::UnavailableFromVendor,
    ] {
        assert!(
            envelope(
                EventKind::FileRead,
                Some(EventContent::File {
                    repo_file: Some("src/main.rs".into()),
                    repo_file_from: None,
                    change_kind: None,
                    file_identity: identity,
                })
            )
            .validate()
            .is_err(),
            "{identity} carried a repo_file"
        );
    }

    // The two honest absences are accepted, and stay distinguishable from each
    // other — a file the vendor never named and a file outside the repository
    // are different facts.
    for identity in [
        FileIdentity::OutOfRepository,
        FileIdentity::UnavailableFromVendor,
    ] {
        assert!(envelope(
            EventKind::FileRead,
            Some(EventContent::File {
                repo_file: None,
                repo_file_from: None,
                change_kind: None,
                file_identity: identity,
            })
        )
        .validate()
        .is_ok());
    }
}

#[test]
fn a_rename_source_requires_a_rename() {
    let with_source = |change: Option<ChangeKind>| {
        envelope(
            EventKind::FileChanged,
            Some(EventContent::File {
                repo_file: Some("b.rs".into()),
                repo_file_from: Some("a.rs".into()),
                change_kind: change,
                file_identity: FileIdentity::Present,
            }),
        )
        .validate()
    };
    assert!(with_source(Some(ChangeKind::Renamed)).is_ok());
    assert!(with_source(Some(ChangeKind::Modified)).is_err());
    assert!(with_source(None).is_err());
}

#[test]
fn a_token_is_key_shaped_or_it_is_refused() {
    for good in [
        "deploy",
        "deploy.images",
        "a_b.c1",
        "x".repeat(128).as_str(),
    ] {
        assert!(
            VocabToken::subject(good).is_ok(),
            "{good:?} should be a legal subject token"
        );
    }
    for bad in [
        "",
        "Deploy",        // a sentence's capitalization
        "deploy images", // a space is a sentence, not a key
        "deploy-images", // the separator normalization repairs elsewhere
        ".deploy",
        "deploy.",
        "deploy..images",
        "deploy/images",
        "password=hunter2", // the shape a credential arrives in
        "x".repeat(129).as_str(),
    ] {
        assert!(
            VocabToken::subject(bad).is_err(),
            "{bad:?} was accepted as a vocabulary token"
        );
    }
    // The object token has the tighter bound.
    assert!(VocabToken::object(&"o".repeat(64)).is_ok());
    assert!(VocabToken::object(&"o".repeat(65)).is_err());
}

#[test]
fn a_refusal_names_the_field_and_never_the_value() {
    // The property is structural: no variant of the refusal type holds a
    // value, so neither rendering can leak one however it is formatted.
    let secret = "ghp_thisisnotarealtokenbutlooksenough".repeat(20);
    let err = envelope(
        EventKind::CommandExecuted,
        Some(EventContent::Command {
            command_line: secret.clone(),
            exit_status: None,
        }),
    )
    .validate()
    .expect_err("an over-long command was accepted");

    for rendering in [format!("{err}"), format!("{err:?}")] {
        assert!(
            !rendering.contains("ghp_"),
            "a refusal rendered the value that caused it: {rendering}"
        );
        assert!(!rendering.contains(&secret));
    }
    assert_eq!(err.code(), "bound_exceeded");
}

#[test]
fn an_unsupported_contract_version_is_refused_by_its_own_code() {
    let mut event = envelope(EventKind::AgentQuiesced, None);
    event.contract_version = 0;
    let err = event.validate().expect_err("version 0 was accepted");
    assert_eq!(
        err.code(),
        "contract_version_unsupported",
        "a version refusal must be recognisable, so a client can defer rather than retry"
    );
}

// ---------------------------------------------------------------------------
// Identity
// ---------------------------------------------------------------------------

#[test]
fn a_retried_delivery_carries_the_identity_it_was_spooled_with() {
    let session = Uuid::now_v7();
    let first = eventid::event_id(session, 7);
    // Five deliveries of the same spooled row, minutes apart, across a daemon
    // restart: one identity, so the server's primary key collapses them into
    // one event rather than five (FR-770).
    for _ in 0..5 {
        assert_eq!(eventid::event_id(session, 7), first);
    }
}

#[test]
fn identity_does_not_move_when_the_clock_does() {
    let session = Uuid::now_v7();
    let a = envelope(EventKind::AgentQuiesced, None);
    let b = SafeCanonicalEvent {
        occurred_at: "1999-01-01T00:00:00Z".parse().unwrap(),
        event_id: eventid::event_id(a.session_id, a.session_seq),
        ..a.clone()
    };
    // The clock moved twenty-seven years and the identity did not (FR-780).
    assert_ne!(a.occurred_at, b.occurred_at);
    assert_eq!(a.event_id, b.event_id);
    // And the derivation genuinely depends on its inputs, so the equality
    // above is a property rather than a constant.
    assert_ne!(a.event_id, eventid::event_id(session, a.session_seq));
}

#[test]
fn the_server_can_re_derive_what_the_client_sent() {
    // Idempotency must not be client-controlled: the server recomputes the id
    // and refuses a mismatch, so a client cannot submit a colliding id, be told
    // `duplicate`, and suppress a genuine event.
    let event = envelope(EventKind::AgentQuiesced, None);
    assert_eq!(
        event.event_id,
        eventid::event_id(event.session_id, event.session_seq)
    );

    let forged = SafeCanonicalEvent {
        event_id: Uuid::now_v7(),
        ..event.clone()
    };
    assert_ne!(
        forged.event_id,
        eventid::event_id(forged.session_id, forged.session_seq),
        "a forged id must be detectable by re-derivation"
    );
}

// ---------------------------------------------------------------------------
// References
// ---------------------------------------------------------------------------

#[test]
fn a_reference_carries_its_whole_domain_across_the_boundary() {
    let id = Uuid::now_v7();
    let refs = [
        Reference::Knowledge(KnowledgeRef::project(id)),
        Reference::Knowledge(KnowledgeRef::personal(id)),
        Reference::Knowledge(KnowledgeRef::team(id)),
        Reference::Pattern(PatternRef(id)),
    ];
    // Round-tripping through JSON is the boundary crossing that matters: a
    // serialization that dropped the domain would leave four references
    // indistinguishable on the wire even though they are distinguishable in
    // memory.
    for original in refs {
        let json = serde_json::to_string(&original).expect("serializes");
        let back: Reference = serde_json::from_str(&json).expect("deserializes");
        assert_eq!(back, original);
        assert_eq!(back.reference_key(), original.reference_key());
    }
    let keys: std::collections::BTreeSet<String> = refs.iter().map(|r| r.reference_key()).collect();
    assert_eq!(keys.len(), 4);
}

#[test]
fn a_pattern_stays_a_personal_record_however_it_is_referenced() {
    let p = Reference::Pattern(PatternRef(Uuid::now_v7()));
    assert_eq!(p.domain_slot(), None);
    assert_eq!(p.canonical_domain(), KnowledgeDomain::Personal);
    assert!(RecordType::Pattern.allows(KnowledgeDomain::Personal));
    assert!(!RecordType::Pattern.allows(KnowledgeDomain::Team));
}
