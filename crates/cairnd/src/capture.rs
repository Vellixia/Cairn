//! The capture pipeline (FR-011 – FR-015, FR-049, FR-050, contracts).
//!
//! Order matters and is fixed: exclusion, then redaction, then structured
//! extraction, then bounding, then the write. Steps one and two run here in the
//! daemon *before* anything is persisted, so nothing sensitive is stored even
//! briefly.
//!
//! Nothing in this file can produce an outbox row. Raw observations are local
//! (FR-055).

use cairn_core::bound::{bound_json, bound_text, payload_bytes};
use cairn_core::domain::Observation;
use cairn_core::event::{
    CaptureOutput, Disposition, EventAgent, SafeCanonicalEvent, SafeEventDraft,
};
use cairn_core::redact;
use cairn_core::vocabulary::SessionVocabulary;
use cairn_core::wire::ObservationInput;
use cairn_core::CairnConfig;
use cairn_store::repo::{self, NewObservation};
use cairn_store::spool::{self, SpoolCapacity};
use cairn_store::Store;
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

pub struct CaptureContext<'a> {
    pub session_id: Uuid,
    pub branch: &'a str,
    pub commit_sha: Option<&'a str>,
}

/// Filter, redact, bound and store one observation.
///
/// `Ok(None)` means the observation was deliberately dropped, which is not an
/// error: an exclusion doing its job looks exactly like this.
pub async fn capture(
    store: &Store,
    config: &CairnConfig,
    ctx: CaptureContext<'_>,
    input: ObservationInput,
) -> Result<Option<Observation>, cairn_store::StoreError> {
    // 1. Exclusions — drop entirely, before anything is written.
    if let Some(path) = &input.path {
        if config.is_path_excluded(path) {
            return Ok(None);
        }
    }
    if let Some(command) = &input.command {
        if config.is_command_excluded(command) {
            return Ok(None);
        }
    }

    // 2. Redaction.
    let summary = redact::redact(&input.summary);
    let path = input.path.as_deref().map(redact::redact);
    let command = input.command.as_deref().map(redact::redact);
    let details = input.details.as_ref().map(redact::redact_json);

    // 3 and 4. Structured fields, bounded.
    let cap = config.payload_cap_bytes;
    let bounded_summary = bound_text(&summary, cap.min(1024));
    let (bounded_details, details_truncated) = match &details {
        Some(d) => {
            let (v, t) = bound_json(d, cap.saturating_sub(bounded_summary.text.len()).max(256));
            (Some(v), t)
        }
        None => (None, false),
    };
    let truncated = bounded_summary.truncated || details_truncated;

    let bytes = payload_bytes(
        &bounded_summary.text,
        path.as_deref(),
        command.as_deref(),
        bounded_details.as_ref(),
    );
    debug_assert!(bytes <= cap, "payload bound must hold: {bytes} > {cap}");

    // 5. Write, with the repository state at the moment of capture (FR-014).
    let observation = repo::insert_observation(
        store,
        NewObservation {
            session_id: ctx.session_id,
            kind: input.kind,
            branch: ctx.branch,
            commit_sha: ctx.commit_sha,
            path: path.as_deref(),
            command: command.as_deref(),
            exit_code: input.exit_code,
            outcome: input.outcome.as_deref(),
            summary: &bounded_summary.text,
            details: bounded_details.as_ref(),
            payload_bytes: bytes as i64,
            truncated,
        },
    )
    .await?;
    Ok(Some(observation))
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_core::domain::{new_id, ObservationType};
    use cairn_store::outbox::SyncPolicy;
    use cairn_store::repo::StartSession;

    const LOCAL: SyncPolicy = SyncPolicy {
        linked: false,
        server_project_id: None,
    };

    async fn fixture() -> (Store, Uuid) {
        let store = Store::open_memory().await.unwrap();
        let user = repo::ensure_local_user(&store).await.unwrap();
        let p = repo::ensure_project(&store, "/tmp/c/.git", "c", None)
            .await
            .unwrap();
        let s = repo::start_session(
            &store,
            StartSession {
                project_id: p.id,
                user_id: user,
                agent: "claude-code",
                agent_session_key: "k",
                branch: "main",
                commit_sha: None,
                worktree_path: "/tmp/c",
                task_id: None,
                daemon_run_id: new_id(),
                policy: LOCAL,
            },
        )
        .await
        .unwrap();
        (store, s.id)
    }

    fn input(kind: ObservationType, summary: &str) -> ObservationInput {
        ObservationInput {
            kind,
            path: None,
            command: None,
            exit_code: None,
            outcome: None,
            summary: summary.into(),
            details: None,
            vendor_tool: None,
        }
    }

    #[tokio::test]
    async fn excluded_path_produces_no_observation() {
        let (store, session_id) = fixture().await;
        let config = CairnConfig {
            excluded_paths: vec!["secrets/**".into()],
            ..Default::default()
        };
        let mut i = input(ObservationType::FileRead, "read secrets/prod.env");
        i.path = Some("secrets/prod.env".into());

        let out = capture(
            &store,
            &config,
            CaptureContext {
                session_id,
                branch: "main",
                commit_sha: None,
            },
            i,
        )
        .await
        .unwrap();
        assert!(out.is_none());
        assert!(repo::observations_for_session(&store, session_id)
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn secrets_are_redacted_before_storage() {
        let (store, session_id) = fixture().await;
        let config = CairnConfig::default();
        let mut i = input(
            ObservationType::CommandRun,
            "export API_KEY=sk-abcdefghijklmnopqrstuvwx",
        );
        i.command = Some("export API_KEY=sk-abcdefghijklmnopqrstuvwx".into());

        let o = capture(
            &store,
            &config,
            CaptureContext {
                session_id,
                branch: "main",
                commit_sha: None,
            },
            i,
        )
        .await
        .unwrap()
        .unwrap();

        assert!(!o.summary.contains("sk-abcdefghijklmnop"), "{}", o.summary);
        assert!(!o.command.unwrap().contains("sk-abcdefghijklmnop"));
    }

    #[tokio::test]
    async fn oversized_payloads_are_bounded_and_flagged() {
        let (store, session_id) = fixture().await;
        let config = CairnConfig::default();
        let mut i = input(ObservationType::CommandRun, &"x".repeat(50_000));
        i.details = Some(serde_json::json!({ "output": "y".repeat(50_000) }));

        let o = capture(
            &store,
            &config,
            CaptureContext {
                session_id,
                branch: "main",
                commit_sha: None,
            },
            i,
        )
        .await
        .unwrap()
        .unwrap();

        assert!(o.truncated);
        assert!(
            o.payload_bytes <= config.payload_cap_bytes as i64,
            "payload {} exceeded cap {}",
            o.payload_bytes,
            config.payload_cap_bytes
        );
    }

    #[tokio::test]
    async fn repository_state_is_recorded_at_capture() {
        let (store, session_id) = fixture().await;
        let o = capture(
            &store,
            &CairnConfig::default(),
            CaptureContext {
                session_id,
                branch: "feature/x",
                commit_sha: Some("deadbeef"),
            },
            input(ObservationType::Discovery, "found the thing"),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(o.branch, "feature/x");
        assert_eq!(o.commit_sha.as_deref(), Some("deadbeef"));
    }

    #[tokio::test]
    async fn excluded_command_produces_no_observation() {
        let (store, session_id) = fixture().await;
        let config = CairnConfig {
            excluded_commands: vec!["aws sts*".into()],
            ..Default::default()
        };
        let mut i = input(ObservationType::CommandRun, "aws sts get-caller-identity");
        i.command = Some("aws sts get-caller-identity".into());

        let out = capture(
            &store,
            &config,
            CaptureContext {
                session_id,
                branch: "main",
                commit_sha: None,
            },
            i,
        )
        .await
        .unwrap();
        assert!(
            out.is_none(),
            "excluded command must produce no observation"
        );
        assert!(repo::observations_for_session(&store, session_id)
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn a_non_excluded_command_is_captured() {
        let (store, session_id) = fixture().await;
        let config = CairnConfig {
            excluded_commands: vec!["aws sts*".into()],
            ..Default::default()
        };
        let mut i = input(ObservationType::CommandRun, "cargo test");
        i.command = Some("cargo test".into());

        let out = capture(
            &store,
            &config,
            CaptureContext {
                session_id,
                branch: "main",
                commit_sha: None,
            },
            i,
        )
        .await
        .unwrap();
        assert!(out.is_some(), "a non-excluded command must be captured");
    }

    #[tokio::test]
    async fn excluded_then_redacted_ordering_excluded_wins() {
        // When a path is both excluded and contains a secret, exclusion
        // wins: no observation at all, so redaction is moot.
        let (store, session_id) = fixture().await;
        let config = CairnConfig {
            excluded_paths: vec!["secrets/**".into()],
            ..Default::default()
        };
        let mut i = input(
            ObservationType::FileRead,
            "read secrets/prod.env with sk-key",
        );
        i.path = Some("secrets/prod.env".into());

        let out = capture(
            &store,
            &config,
            CaptureContext {
                session_id,
                branch: "main",
                commit_sha: None,
            },
            i,
        )
        .await
        .unwrap();
        assert!(out.is_none(), "exclusion must win over redaction");
    }

    #[tokio::test]
    async fn concurrent_captures_to_one_session_all_land() {
        let (store, session_id) = fixture().await;
        let config = CairnConfig::default();

        let mut handles = Vec::new();
        for i in 0..20 {
            let store = store.clone();
            let config = config.clone();
            handles.push(tokio::spawn(async move {
                let kind = if i % 2 == 0 {
                    ObservationType::FileRead
                } else {
                    ObservationType::CommandRun
                };
                let mut inp = input(kind, &format!("concurrent event {i}"));
                inp.command = Some(format!("cmd {i}"));
                capture(
                    &store,
                    &config,
                    CaptureContext {
                        session_id,
                        branch: "main",
                        commit_sha: None,
                    },
                    inp,
                )
                .await
                .unwrap()
            }));
        }
        for h in handles {
            let _ = h.await.unwrap();
        }
        let observations = repo::observations_for_session(&store, session_id)
            .await
            .unwrap();
        assert_eq!(observations.len(), 20, "all concurrent captures must land");
    }

    #[tokio::test]
    /// Observations are stored as structured fields — command, exit code,
    /// outcome — rather than as captured prose (FR-012).
    async fn a_failure_observation_is_captured_correctly() {
        // PostToolUseFailure produces an `error` observation. The capture
        // pipeline must handle it without inferring failure from a success.
        let (store, session_id) = fixture().await;
        let mut i = input(
            ObservationType::Error,
            "command failed: cargo test -- no such file",
        );
        i.command = Some("cargo test".into());
        i.exit_code = Some(101);

        let o = capture(
            &store,
            &CairnConfig::default(),
            CaptureContext {
                session_id,
                branch: "main",
                commit_sha: None,
            },
            i,
        )
        .await
        .unwrap()
        .unwrap();

        assert_eq!(o.kind, ObservationType::Error);
        assert_eq!(o.exit_code, Some(101));
        assert!(o.summary.contains("cargo test"));
    }
}

// ---------------------------------------------------------------------------
// Feature 005 safe-event capture (T050, T051)
// ---------------------------------------------------------------------------

/// What one vendor event's capture did.
///
/// Counted rather than described. A summary that carried what it spooled would
/// be a second copy of the payload in a place nothing needs one.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SpoolSummary {
    pub spooled: u64,
    /// Capture-class rows the overflow policy shed to make room.
    pub overflow_dropped: u64,
    /// Events refused because the spool is saturated and nothing shedable
    /// remains.
    pub saturated: u64,
    /// Captures the local pipeline declined, each already counted under its own
    /// disposition.
    pub declined: u64,
}

/// The vocabulary and established keys a hook needs to build a semantic signal.
///
/// Derived here rather than in the hook because the hook is a short-lived
/// process with no store, and derived from *this session's* events plus *this
/// project's* established keys because those are the two sources
/// `contracts/extraction.md` §13.3 names. Both are required: a client that
/// justified tokens only from session events would refuse tokens it could
/// legitimately have justified from an established key, and that refusal is
/// permanent — the decision is destroyed rather than deferred.
///
/// Sending this to the hook rather than sending the hook's text here is the
/// whole point. A prompt fragment must not cross the capture-process boundary
/// (FR-730); a set of tokens already visible to anyone who can read the
/// repository may.
pub async fn session_vocabulary(
    store: &Store,
    project_id: Uuid,
    session_id: Uuid,
) -> Result<(SessionVocabulary, BTreeMap<String, String>), cairn_store::StoreError> {
    let keys: Vec<(Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT topic_key, value_key FROM memories
          WHERE project_id = ?1 AND topic_key IS NOT NULL
            AND state = 'active' AND deleted_at IS NULL",
    )
    .bind(project_id.to_string())
    .fetch_all(store.pool())
    .await?;

    let topics: Vec<String> = keys.iter().filter_map(|(t, _)| t.clone()).collect();
    let values: Vec<String> = keys.iter().filter_map(|(_, v)| v.clone()).collect();

    // The subject's established value, for the one step that may supply an
    // object the text did not name (`contracts/extraction.md` §13.5). A subject
    // with two established values is left out: naming one of them would be a
    // choice the evidence does not make.
    let mut established: BTreeMap<String, String> = BTreeMap::new();
    let mut ambiguous: BTreeSet<String> = BTreeSet::new();
    for (topic, value) in &keys {
        let (Some(topic), Some(value)) = (topic, value) else {
            continue;
        };
        let (Some(topic), Some(value)) = (
            cairn_core::knowledge::normalize_topic_key(topic),
            cairn_core::knowledge::normalize_value_key(value),
        ) else {
            continue;
        };
        match established.get(&topic) {
            Some(held) if held != &value => {
                ambiguous.insert(topic);
            }
            _ => {
                established.insert(topic, value);
            }
        }
    }
    for topic in ambiguous {
        established.remove(&topic);
    }

    let mut vocabulary = SessionVocabulary::new()
        .with_established_keys(topics.iter().map(String::as_str))
        .with_established_value_keys(values.iter().map(String::as_str));
    for event in cairn_store::spool::session_events(store, session_id).await? {
        vocabulary.observe_at(Some(event.session_seq), event.kind, event.content.as_ref());
    }
    Ok((vocabulary, established))
}

/// Spool one vendor event's approved canonical events, in order.
///
/// Identity is assigned by the store, inside the transaction that inserts each
/// row — never here and never by the hook. A hook is a separate short-lived
/// process and cannot hold a counter, and two concurrent invocations choosing
/// their own ordinals would derive colliding `event_id`s, which the server
/// answers `duplicate`, silently discarding a real event (`data-model.md` §1.4).
///
/// Order matters and is the order the drafts arrive in: a semantic signal may
/// only cite a token an earlier ordinal established, so spooling the signal
/// before the `file_changed` that justified it would make the server refuse a
/// claim the client legitimately built.
///
/// Every decline becomes both a counted disposition and a `capture_declined`
/// event. The counter makes the rate visible locally; the event makes it
/// visible centrally. Neither carries any part of what was declined (FR-741,
/// FR-749d).
pub async fn spool_safe_events(
    store: &Store,
    project_id: Uuid,
    account_id: Uuid,
    session_id: Uuid,
    // The agent comes from the caller, which knows which adapter ran, and never
    // from the first event in the output. A capture that produced only declines
    // has no first event, and inferring one would file a Codex decline under
    // Claude Code — a health report that named the wrong agent would send
    // somebody to look at an adapter that is working.
    agent: EventAgent,
    output: &CaptureOutput,
) -> Result<SpoolSummary, cairn_store::StoreError> {
    let capacity = SpoolCapacity::default();
    let mut summary = SpoolSummary::default();

    // **Which server this work is queued for** (FR-791). Read here, once, and
    // stamped on every row this call spools: a row's binding is decided when it
    // is written and never re-decided, so a deployment replaced later cannot
    // acquire work that was queued for its predecessor.
    //
    // `None` when this store has never established a lane — the honest answer,
    // and the one the spool's first-binding rule is written for. Guessing an
    // instance here would be the endpoint-as-identity mistake in a new place.
    let server_instance_id = cairn_store::cursor::bound_server_instance(store).await?;

    let declined_events: Vec<SafeEventDraft> = output
        .declines
        .iter()
        .map(|decline| decline.as_event(agent, None))
        .collect();

    for decline in &output.declines {
        spool::record_disposition(
            store,
            project_id,
            agent.as_str(),
            decline.kind.as_str(),
            decline.disposition(),
        )
        .await?;
        summary.declined += 1;
    }

    for draft in output.events.iter().chain(declined_events.iter()) {
        let event = SafeCanonicalEvent {
            // Overwritten by the store, and passed as nil rather than as a
            // guess so nothing here can be mistaken for an identity.
            event_id: Uuid::nil(),
            contract_version: cairn_core::event::CONTRACT_VERSION,
            kind: draft.kind,
            agent: draft.agent,
            vendor_event: draft.vendor_event.clone(),
            session_id,
            session_seq: 0,
            occurred_at: chrono::Utc::now(),
            content: draft.content.clone(),
        };

        // The client's own validation, before the row exists. It is a courtesy
        // and not the mechanism — the server checks independently — but an
        // event that cannot pass it would occupy a spool row for the length of
        // its attempt budget only to be refused.
        if event.validate().is_err() {
            spool::record_disposition(
                store,
                project_id,
                draft.agent.as_str(),
                draft.kind.as_str(),
                Disposition::PrivacyRefused,
            )
            .await?;
            summary.declined += 1;
            continue;
        }

        match spool::spool_event(
            store,
            capacity,
            spool::NewEvent {
                project_id,
                account_id,
                event,
                server_instance_id,
            },
        )
        .await?
        {
            spool::EventAdmission::Spooled {
                overflow_dropped, ..
            } => {
                summary.spooled += 1;
                summary.overflow_dropped += overflow_dropped;
            }
            // Refused visibly. The count is the only trace a saturated store
            // leaves of the event it could not take, and losing it silently is
            // the one thing FR-785 does not allow.
            spool::EventAdmission::Saturated { .. } => summary.saturated += 1,
        }
    }
    Ok(summary)
}

// ---------------------------------------------------------------------------
// Capture drops the hook could not report (FR-749c)
// ---------------------------------------------------------------------------

/// Count every capture drop the hook journalled, then clear the journal.
///
/// **Where `capture_deadline_exceeded` comes from.** The disposition is named
/// by FR-749c, is in `data-model.md` §4's closed vocabulary, is admitted by both
/// schemas' CHECK constraints and has a column of its own in the health funnel —
/// and until this function existed nothing in the product ever produced one. A
/// capture-class event that missed its deadline was dropped (which FR-749b
/// permits), the hook exited zero (which FR-749b requires), and Cairn's own
/// account of itself recorded nothing at all (which FR-749c forbids). The agent
/// saw success; so did capture health.
///
/// The hook cannot count it — it is a short-lived process with no store — and
/// the daemon cannot count what never reached it, so the two are joined by a
/// journal file the hook appends to and this drains.
///
/// **Read-then-truncate, and the window is stated rather than hidden.** A line
/// appended between the read and the truncate is lost, and losing a *count* of
/// drops is a smaller harm than double-counting one or than holding a lock on
/// the capture fast path: the hook's append must never block on this, because
/// the hook runs when something is already unreachable. The file is removed
/// rather than emptied so an ordinary session leaves nothing behind.
///
/// Every failure here is swallowed. A daemon that cannot collect drop records
/// must not stop draining the spools beside it.
pub async fn collect_capture_drops(d: &crate::state::Daemon) -> usize {
    let path = cairn_core::paths::capture_drop_journal_path();
    let Ok(text) = std::fs::read_to_string(&path) else {
        return 0;
    };
    if std::fs::remove_file(&path).is_err() {
        // Could not clear it: stop rather than count the same drops again on
        // every tick forever.
        return 0;
    }

    let mut counted = 0usize;
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let Ok(entry) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let cwd = entry
            .get("cwd")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let Some(project_id) = d.resolve(cwd).await.ok().map(|r| r.project.id) else {
            // No project for that directory any more. The drop is real and
            // there is nowhere honest to count it, so it is reported here and
            // not filed under some other project.
            tracing::info!(
                "a journalled capture drop names a directory this store no longer \
                 resolves to a project; it is not counted"
            );
            continue;
        };
        // **The funnel's own spelling, not the CLI's.** `AgentId::as_str` is
        // `claude-code` and `EventAgent::as_str` is `claude_code`, and every
        // other row in this table carries the second. Writing the first here
        // would file a drop under an agent the rest of capture health does not
        // recognize — a loss recorded in a place nobody looks is the silence
        // FR-749c is about, arrived at one step later. An agent outside the
        // capture population is reported rather than invented.
        let Some(agent) = entry
            .get("agent")
            .and_then(|v| v.as_str())
            .and_then(crate::handlers::event_agent)
        else {
            tracing::info!("a journalled capture drop names an agent this build does not capture from; it is not counted");
            continue;
        };
        let agent = agent.as_str();
        // The canonical kind when the adapter had already determined one, and
        // the vendor event name when it had not. Never invented: a capture-only
        // drop genuinely has no canonical kind, and filing it under one would
        // put a loss in a row that describes something else.
        let kind = entry
            .get("kind")
            .and_then(|v| v.as_str())
            .or_else(|| entry.get("vendor_event").and_then(|v| v.as_str()))
            .unwrap_or("unknown");
        if spool::record_disposition(
            &d.store,
            project_id,
            agent,
            kind,
            Disposition::CaptureDeadlineExceeded,
        )
        .await
        .is_ok()
        {
            counted += 1;
        }
    }
    if counted > 0 {
        tracing::info!(
            drops = counted,
            "counted capture-class events this machine dropped before they reached \
             the daemon (FR-749c)"
        );
    }
    counted
}
