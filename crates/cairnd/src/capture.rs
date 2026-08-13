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
use cairn_core::redact;
use cairn_core::wire::ObservationInput;
use cairn_core::CairnConfig;
use cairn_store::repo::{self, NewObservation};
use cairn_store::Store;
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
