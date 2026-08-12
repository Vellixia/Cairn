//! T048, T064, T074 — the recorded vendor payload corpus (D40 tier 3).
//!
//! Every payload under `tests/integrations/<agent>/` is a realistic vendor
//! lifecycle event recorded from that vendor's own documentation or published
//! source, cited in the directory's `SOURCES.md`. Each one records what the
//! adapter must make of it, so a vendor change shows up as a diff in this
//! corpus rather than as a silent behavioral regression.
//!
//! Two halves are asserted for every adapter, and they are equally important:
//!
//! 1. every capability the profile claims **guaranteed** is proved by a
//!    payload that produces its canonical event;
//! 2. every capability the profile does **not** claim produces nothing, from
//!    any payload in the corpus under any event name.
//!
//! The second half is what keeps the capability report honest. A profile that
//! says `session_closed: absent` while some payload quietly produces one would
//! be a lie a developer could not detect (FR-115, FR-116, SC-110, SC-111).
//!
//! Hermetic by construction: no vendor binary, no authentication, no network,
//! no daemon (FR-204, SC-124).

use cairn_core::lifecycle::CanonicalEvent;
use cairn_integrate::adapter::RawPayload;
use cairn_integrate::capability::{Availability, Capability, CapabilityProfile};
use cairn_integrate::model::AgentId;
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::PathBuf;

/// One recorded payload and what the adapter must make of it.
struct Fixture {
    agent: AgentId,
    file: String,
    event: String,
    /// The canonical event this payload must produce, or `None` if the
    /// adapter must decline it.
    expect: Option<CanonicalEvent>,
    /// The capability the recorded expectation demonstrates, as written in
    /// the fixture. Cross-checked against `Capability::for_event`.
    capability: Option<String>,
    payload: Value,
}

fn corpus_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/integrations")
        .canonicalize()
        .expect("the recorded payload corpus is checked in")
}

fn dir_for(agent: AgentId) -> PathBuf {
    corpus_root().join(agent.as_str())
}

fn load(agent: AgentId) -> Vec<Fixture> {
    let dir = dir_for(agent);
    let mut out = Vec::new();
    let mut entries: Vec<_> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()))
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("json"))
        .collect();
    entries.sort();
    for path in entries {
        let file = path.file_name().unwrap().to_string_lossy().to_string();
        let text = std::fs::read_to_string(&path).expect("fixture readable");
        let v: Value = serde_json::from_str(&text)
            .unwrap_or_else(|e| panic!("{} is not valid JSON: {e}", path.display()));
        let event = v["event"]
            .as_str()
            .unwrap_or_else(|| panic!("{file} has no `event`"))
            .to_string();
        let expect = match &v["expect"] {
            Value::Null => None,
            Value::String(s) => Some(
                serde_json::from_value::<CanonicalEvent>(Value::String(s.clone()))
                    .unwrap_or_else(|e| panic!("{file} names an unknown canonical event: {e}")),
            ),
            other => panic!("{file} has a malformed `expect`: {other}"),
        };
        assert!(
            v.get("payload").is_some(),
            "{file} records no vendor payload"
        );
        out.push(Fixture {
            agent,
            file,
            event,
            expect,
            capability: v["capability"].as_str().map(str::to_string),
            payload: v["payload"].clone(),
        });
    }
    assert!(
        !out.is_empty(),
        "no recorded payloads for {}",
        agent.as_str()
    );
    out
}

/// Every adapter that has a lifecycle vocabulary at all.
const NATIVE: [AgentId; 3] = [AgentId::ClaudeCode, AgentId::Codex, AgentId::Opencode];

fn everything() -> Vec<Fixture> {
    NATIVE.into_iter().flat_map(load).collect()
}

fn normalize(f: &Fixture) -> Option<cairn_core::lifecycle::CanonicalLifecycleEvent> {
    cairn_integrate::normalize(
        f.agent,
        &f.event,
        &RawPayload::new(f.payload.clone(), "/home/dev/app"),
    )
}

// ---------------------------------------------------------------- corpus ---

#[test]
fn every_recorded_payload_normalizes_exactly_as_recorded() {
    for f in everything() {
        let produced = normalize(&f).map(|e| e.event);
        assert_eq!(
            produced,
            f.expect,
            "{}/{} produced {:?}, recorded expectation is {:?}",
            f.agent.as_str(),
            f.file,
            produced.map(|e| e.as_str()),
            f.expect.map(|e| e.as_str())
        );
    }
}

#[test]
fn every_produced_event_is_well_formed() {
    // Only tool events carry observations, quiescence never does, and nothing
    // reaches the daemon without a routing key (FR-231, FR-118).
    for f in everything() {
        let Some(event) = normalize(&f) else { continue };
        assert!(
            event.is_well_formed(),
            "{}/{} produced a malformed canonical event: {event:?}",
            f.agent.as_str(),
            f.file
        );
        assert_eq!(
            event.agent,
            f.agent.as_str(),
            "{}/{} attributed its event to another agent",
            f.agent.as_str(),
            f.file
        );
    }
}

#[test]
fn each_fixtures_recorded_capability_matches_the_event_it_produces() {
    for f in everything() {
        let Some(named) = &f.capability else {
            assert!(
                f.expect.is_none(),
                "{}/{} produces an event but names no capability",
                f.agent.as_str(),
                f.file
            );
            continue;
        };
        let expected = f
            .expect
            .map(Capability::for_event)
            .unwrap_or_else(|| panic!("{} names a capability but expects nothing", f.file));
        assert_eq!(
            named,
            expected.as_str(),
            "{}/{} names the wrong capability",
            f.agent.as_str(),
            f.file
        );
    }
}

#[test]
fn every_guaranteed_lifecycle_capability_is_proved_by_a_payload() {
    // The first half of D40 tier 3: a claim of `guaranteed` is backed by a
    // recorded payload that produces the canonical event, per agent.
    for agent in NATIVE {
        let profile = CapabilityProfile::base(agent);
        let proved: BTreeSet<Capability> = load(agent)
            .iter()
            .filter_map(normalize)
            .map(|e| Capability::for_event(e.event))
            .collect();
        for c in Capability::ALL {
            if !c.is_lifecycle() {
                continue;
            }
            if profile.get(c).availability != Availability::Guaranteed {
                continue;
            }
            assert!(
                proved.contains(&c),
                "{} claims {} guaranteed with no payload proving it",
                agent.as_str(),
                c.as_str()
            );
        }
    }
}

#[test]
fn no_payload_produces_a_capability_the_profile_does_not_claim() {
    // The second half, and the one that matters most: every payload in the
    // whole corpus, under every event name any adapter recognizes, against
    // every adapter's absent capabilities.
    let all = everything();
    let names: BTreeSet<&str> = all.iter().map(|f| f.event.as_str()).collect();

    for agent in NATIVE {
        let profile = CapabilityProfile::base(agent);
        let absent: BTreeSet<Capability> = Capability::ALL
            .into_iter()
            .filter(|c| c.is_lifecycle() && profile.get(*c).availability == Availability::Absent)
            .collect();
        if absent.is_empty() {
            continue;
        }
        for f in &all {
            for name in &names {
                let produced = cairn_integrate::normalize(
                    agent,
                    name,
                    &RawPayload::new(f.payload.clone(), "/home/dev/app"),
                );
                let Some(event) = produced else { continue };
                let c = Capability::for_event(event.event);
                assert!(
                    !absent.contains(&c),
                    "{} reports {} absent but produced it from {} under event `{}`",
                    agent.as_str(),
                    c.as_str(),
                    f.file,
                    name
                );
            }
        }
    }
}

#[test]
fn the_generic_client_maps_no_event_at_all() {
    // Its profile claims no lifecycle capability, so nothing in the corpus may
    // produce one (FR-110, US8 #3).
    for f in everything() {
        assert!(
            cairn_integrate::normalize(
                AgentId::GenericMcp,
                &f.event,
                &RawPayload::new(f.payload.clone(), "/home/dev/app"),
            )
            .is_none(),
            "the generic MCP path produced a lifecycle event from {}",
            f.file
        );
    }
}

#[test]
fn an_adapter_declines_every_event_it_does_not_register() {
    // One direction only: an unregistered event never produces anything. The
    // converse does not hold, and should not — a registered event whose
    // payload cannot name its session is declined too (FR-115, FR-118).
    let all = everything();
    for agent in NATIVE {
        let registered: BTreeSet<&str> = cairn_integrate::adapter_for(agent)
            .registered_events()
            .iter()
            .copied()
            .collect();
        for f in &all {
            if registered.contains(f.event.as_str()) {
                continue;
            }
            let produced = cairn_integrate::normalize(
                agent,
                &f.event,
                &RawPayload::new(f.payload.clone(), "/home/dev/app"),
            );
            assert!(
                produced.is_none(),
                "{} produced {:?} from `{}`, an event it does not register",
                agent.as_str(),
                produced.map(|e| e.event.as_str()),
                f.event
            );
        }
    }
}

#[test]
fn an_event_that_cannot_name_its_session_is_declined() {
    // Recorded per adapter, because routing by the vendor's own identifier is
    // what keeps concurrent agents apart (FR-118, US10 #6).
    for agent in NATIVE {
        let fixtures = load(agent);
        let f = fixtures
            .iter()
            .find(|f| f.file == "declined_no_session_identity.json")
            .unwrap_or_else(|| panic!("{} records no identity-absent payload", agent.as_str()));
        assert!(
            cairn_integrate::adapter_for(agent)
                .registered_events()
                .contains(&f.event.as_str()),
            "the identity-absent fixture must use an event the adapter does register, \
             or it proves nothing"
        );
        assert!(
            normalize(f).is_none(),
            "{} routed an event carrying no session identity",
            agent.as_str()
        );
    }
}

// ------------------------------------------------------------- lifecycle ---

mod lifecycle {
    use super::*;

    /// T074: no adapter maps an idle, quiet, or inactive signal to
    /// `session_closed` — for every adapter, not only OpenCode.
    ///
    /// This is the single most consequential mapping in the feature. Treating
    /// "the agent went quiet" as "the session ended" would produce a handoff
    /// declaring work complete in the middle of it, and it would do so on
    /// every long-running turn (FR-116, FR-230, SC-111).
    #[test]
    fn idle_never_closes() {
        // Every event name any vendor uses for quiet, plus the ones a future
        // vendor plausibly would.
        let quiet = [
            "session.idle",
            "session.status",
            "Stop",
            "StopFailure",
            "SubagentStop",
            "idle",
            "inactive",
            "session.inactive",
            "agent.idle",
            "quiet",
            "session.quiet",
            "timeout",
            "session.timeout",
        ];
        let payloads: Vec<Value> = everything().into_iter().map(|f| f.payload).collect();

        for agent in NATIVE {
            for name in quiet {
                for payload in &payloads {
                    let produced = cairn_integrate::normalize(
                        agent,
                        name,
                        &RawPayload::new(payload.clone(), "/home/dev/app"),
                    );
                    let Some(event) = produced else { continue };
                    assert_ne!(
                        event.event,
                        CanonicalEvent::SessionClosed,
                        "{} closed a session from the quiet signal `{name}`",
                        agent.as_str()
                    );
                }
            }
        }
    }

    /// T074: quiescence following an error is exactly one checkpoint and no
    /// invented outcome.
    ///
    /// The tempting bug is to read the preceding error and synthesize a
    /// failure observation from it, or to read the quiet signal as the turn
    /// having succeeded. Both invent a fact the vendor never reported
    /// (FR-117, FR-231, SC-134).
    #[test]
    fn quiesce_after_error() {
        struct Case {
            agent: AgentId,
            error: (&'static str, Value),
            quiesce: (&'static str, Value),
        }
        let cases = [
            Case {
                agent: AgentId::Opencode,
                error: (
                    "session.error",
                    serde_json::json!({
                        "sessionID": "ses_1",
                        "error": { "name": "ProviderAuthError", "data": { "message": "401" } }
                    }),
                ),
                quiesce: ("session.idle", serde_json::json!({ "sessionID": "ses_1" })),
            },
            Case {
                agent: AgentId::ClaudeCode,
                error: (
                    "PostToolUseFailure",
                    serde_json::json!({
                        "session_id": "s-1",
                        "tool_name": "Bash",
                        "tool_input": { "command": "cargo test" },
                        "error": { "message": "3 tests failed" }
                    }),
                ),
                quiesce: (
                    "Stop",
                    serde_json::json!({ "session_id": "s-1", "stop_hook_active": false }),
                ),
            },
            Case {
                agent: AgentId::Codex,
                error: (
                    "PostToolUse",
                    serde_json::json!({
                        "session_id": "s-1",
                        "tool_name": "shell",
                        "tool_input": { "command": ["cargo", "test"] },
                        "tool_response": { "exit_code": 1 }
                    }),
                ),
                quiesce: ("Stop", serde_json::json!({ "session_id": "s-1" })),
            },
        ];

        for case in cases {
            let agent = case.agent.as_str();
            // The error itself is whatever the vendor established — that half
            // is tested elsewhere. What matters here is what follows it.
            let _ = cairn_integrate::normalize(
                case.agent,
                case.error.0,
                &RawPayload::new(case.error.1.clone(), "/home/dev/app"),
            );

            let quiesced = cairn_integrate::normalize(
                case.agent,
                case.quiesce.0,
                &RawPayload::new(case.quiesce.1, "/home/dev/app"),
            )
            .unwrap_or_else(|| panic!("{agent} produced no checkpoint from `{}`", case.quiesce.0));

            assert_eq!(
                quiesced.event,
                CanonicalEvent::AgentQuiesced,
                "{agent} read a quiet signal as something stronger"
            );
            assert!(
                quiesced.observation.is_none(),
                "{agent} synthesized an outcome from a quiet signal: {:?}",
                quiesced.observation
            );
            assert!(
                !quiesced.event.produces_durable_handoff(),
                "{agent} treated quiescence as a boundary"
            );
        }
    }

    /// T074: the conditional failure capability, both ways.
    ///
    /// One payload that establishes failure produces `tool_failed`. One that
    /// is merely suggestive — output that contains the word "error", no exit
    /// code, no error field — produces no failure at all. Absent evidence is
    /// absent, not a guess in either direction.
    #[test]
    fn conditional_failure_is_established_or_not_claimed() {
        let profile = CapabilityProfile::base(AgentId::Opencode);
        assert_eq!(
            profile.get(Capability::LifecycleToolFailure).availability,
            Availability::Conditional,
            "OpenCode's tool failure capability is conditional (D32)"
        );

        let by_file = |name: &str| -> Option<cairn_core::lifecycle::CanonicalLifecycleEvent> {
            let f = load(AgentId::Opencode)
                .into_iter()
                .find(|f| f.file == name)
                .unwrap_or_else(|| panic!("missing fixture {name}"));
            normalize(&f)
        };

        let established = by_file("tool_execute_after_failure.json").expect("an event");
        assert_eq!(established.event, CanonicalEvent::ToolFailed);
        let observation = established.observation.expect("a tool event carries one");
        assert_eq!(
            observation.kind,
            cairn_core::domain::ObservationType::Error,
            "an established failure was not recorded as one"
        );
        assert_eq!(observation.outcome.as_deref(), Some("error"));
        assert_eq!(observation.exit_code, Some(101));

        let ambiguous = by_file("tool_execute_after_ambiguous.json").expect("an event");
        assert_eq!(
            ambiguous.event,
            CanonicalEvent::ToolSucceeded,
            "an ambiguous payload was read as a failure"
        );
        let observation = ambiguous.observation.expect("a tool event carries one");
        assert_ne!(
            observation.kind,
            cairn_core::domain::ObservationType::Error,
            "a failure was asserted from output that only mentions the word"
        );
        // And no outcome is claimed in either direction: this run's result is
        // genuinely unknown, and "passed" would be a fabrication.
        assert_eq!(
            observation.outcome.as_deref(),
            Some("unknown"),
            "an unreported test result was recorded as a result"
        );
    }

    /// The conditional capability names what it depends on, in the developer's
    /// terms, rather than reporting a bare `conditional` (FR-111).
    #[test]
    fn a_conditional_capability_names_its_condition() {
        let profile = CapabilityProfile::base(AgentId::Opencode);
        let state = profile.get(Capability::LifecycleToolFailure);
        let condition = state.depends_on.as_deref().unwrap_or_default();
        assert!(
            condition.contains("failure"),
            "the condition does not say what it depends on: {condition:?}"
        );
        assert!(profile
            .conditional_behaviors()
            .iter()
            .any(|b| b.contains(condition)));
    }
}
