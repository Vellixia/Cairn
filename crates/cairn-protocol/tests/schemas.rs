//! T010: JSON Schema export + golden diff (breaking-change tripwire) and
//! sample-payload validation for every v1 method.

use std::path::PathBuf;

use cairn_protocol::*;
use schemars::{schema_for, JsonSchema};

fn schemas_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("schemas")
}

fn golden(name: &str) -> serde_json::Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("goldens")
        .join("ipc")
        .join(name);
    serde_json::from_str(&std::fs::read_to_string(path).expect("read IPC golden"))
        .expect("valid IPC golden JSON")
}

fn check_schema<T: JsonSchema>(name: &str) -> Result<(), String> {
    let schema = schema_for!(T);
    let rendered = serde_json::to_string_pretty(&schema).expect("serializable schema");
    let path = schemas_dir().join(format!("{name}.json"));
    if !path.exists() {
        // Bootstrap: the first generation IS the golden. Committed to git;
        // any later drift fails below.
        std::fs::create_dir_all(schemas_dir()).expect("create schemas dir");
        std::fs::write(&path, &rendered).expect("write golden schema");
        return Ok(());
    }
    let golden = std::fs::read_to_string(&path).expect("read golden schema");
    if golden.replace("\r\n", "\n") != rendered.replace("\r\n", "\n") {
        if std::env::var("UPDATE_SCHEMAS").is_ok() {
            std::fs::write(&path, &rendered).expect("update golden schema");
            return Ok(());
        }
        return Err(format!(
            "schema drift in {name}: breaking-change tripwire. \
             If intentional, rerun with UPDATE_SCHEMAS=1 and review the diff."
        ));
    }
    Ok(())
}

#[test]
fn golden_schemas_are_stable() {
    let mut failures = Vec::new();
    macro_rules! check {
        ($t:ty, $name:literal) => {
            if let Err(e) = check_schema::<$t>($name) {
                failures.push(e);
            }
        };
    }
    check!(Request, "request");
    check!(Response, "response");
    check!(CliEnvelope, "cli-envelope");
    check!(ErrorBody, "error-body");
    check!(DaemonStatusResult, "daemon-status-result");
    check!(RegisterParams, "register-params");
    check!(RegisterResult, "register-result");
    check!(InspectParams, "inspect-params");
    check!(InspectionDto, "inspection");
    check!(IgnoredFilesParams, "ignored-files-params");
    check!(IgnoredFilesResult, "ignored-files-result");
    check!(SnapshotCreateParams, "snapshot-create-params");
    check!(SnapshotCreateResult, "snapshot-create-result");
    check!(SessionStartParams, "session-start-params");
    check!(SessionStartResult, "session-start-result");
    check!(SessionGetParams, "session-get-params");
    check!(SessionGetResult, "session-get-result");
    check!(SessionListParams, "session-list-params");
    check!(SessionListResult, "session-list-result");
    check!(SessionHeartbeatParams, "session-heartbeat-params");
    check!(SessionHeartbeatResult, "session-heartbeat-result");
    check!(SessionReattachParams, "session-reattach-params");
    check!(SessionReattachResult, "session-reattach-result");
    check!(SessionStopParams, "session-stop-params");
    check!(SessionStopResult, "session-stop-result");
    check!(EventsListParams, "events-list-params");
    check!(EventsListResult, "events-list-result");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn sample_payloads_round_trip() {
    // Error codes serialize SCREAMING_SNAKE (stable wire values).
    assert_eq!(
        serde_json::to_value(ErrorCode::NotAWorktree).unwrap(),
        serde_json::json!("NOT_A_WORKTREE")
    );
    assert_eq!(
        serde_json::to_value(ErrorCode::LeaseMismatch).unwrap(),
        serde_json::json!("LEASE_MISMATCH")
    );

    // Request/response envelope shapes.
    let req: Request =
        serde_json::from_str(r#"{"id":"1","method":"v1.daemon.status","params":{}}"#).unwrap();
    assert_eq!(req.method, methods::DAEMON_STATUS);

    let start: SessionStartParams = serde_json::from_str(
        r#"{"path":"/repo","agent_type":"claude-code",
            "agent_instance_id":"018f4e6e-5f2b-7c3e-9a4d-2f6e8b1c9d0a","agent_pid":42}"#,
    )
    .unwrap();
    assert_eq!(start.agent_type, "claude-code");
    assert_eq!(start.agent_pid, Some(42));

    // Unknown fields ignored on input (additive-only v1 evolution).
    let _: InspectParams = serde_json::from_str(r#"{"path":"/x","future_field":true}"#).unwrap();

    // Exit-code mapping is part of the CLI contract.
    assert_eq!(ErrorCode::NotARepository.exit_code(), 3);
    assert_eq!(ErrorCode::NotAWorktree.exit_code(), 3);
    assert_eq!(ErrorCode::SessionAmbiguous.exit_code(), 4);
    assert_eq!(ErrorCode::DaemonUnavailable.exit_code(), 5);
    assert_eq!(ErrorCode::StateCorrupted.exit_code(), 6);
    assert_eq!(ErrorCode::Usage.exit_code(), 2);
    assert_eq!(ErrorCode::LeaseMismatch.exit_code(), 1);
    assert_eq!(
        serde_json::to_value(ErrorCode::WatcherStartFailed).unwrap(),
        serde_json::json!("WATCHER_START_FAILED")
    );
    assert_eq!(ErrorCode::WatcherStartFailed.exit_code(), 1);
}

#[test]
fn all_methods_have_unique_names() {
    let mut names: Vec<&str> = methods::ALL_METHODS.to_vec();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), methods::ALL_METHODS.len());
    assert!(methods::ALL_METHODS.iter().all(|m| m.starts_with("v1.")));
}

#[test]
fn watcher_failure_ipc_goldens_are_typed_and_stable() {
    let request_value = golden("session-start-request.json");
    let request: Request = serde_json::from_value(request_value.clone()).unwrap();
    assert_eq!(request.method, methods::SESSION_START);
    assert_eq!(serde_json::to_value(request).unwrap(), request_value);

    for (stage, file) in [
        (
            WatcherStartStage::Install,
            "watcher-start-failed-install.json",
        ),
        (
            WatcherStartStage::Reconcile,
            "watcher-start-failed-reconcile.json",
        ),
    ] {
        let expected = golden(file);
        let decoded: Response = serde_json::from_value(expected.clone()).unwrap();
        let error = decoded.error.as_ref().expect("error response");
        assert_eq!(error.code, ErrorCode::WatcherStartFailed);
        assert_eq!(error.code.exit_code(), 1);
        assert_eq!(error.data.map(ErrorData::watcher_stage), Some(stage));
        assert_eq!(serde_json::to_value(&decoded).unwrap(), expected);

        let constructed = Response {
            id: "watcher-contract-1".into(),
            result: None,
            error: Some(ErrorBody::watcher_start_failed(stage)),
        };
        assert_eq!(serde_json::to_value(constructed).unwrap(), expected);
    }
}

#[test]
fn watcher_failure_payload_is_closed_and_stage_compatible() {
    for stage in ["install", "reconcile"] {
        let data: ErrorData = serde_json::from_value(serde_json::json!({
            "kind": "watcher_start_failure",
            "stage": stage,
        }))
        .unwrap();
        assert_eq!(data.watcher_stage().as_str(), stage);
    }

    for invalid in [
        serde_json::json!({"kind":"watcher_start_failure","stage":"startup"}),
        serde_json::json!({"kind":"watcher_failure","stage":"install"}),
        serde_json::json!({"kind":"watcher_start_failure"}),
        serde_json::json!({
            "kind":"watcher_start_failure",
            "stage":"install",
            "internal_path":"/private/secret/repository",
        }),
    ] {
        assert!(
            serde_json::from_value::<ErrorData>(invalid).is_err(),
            "closed watcher payload accepted an incompatible shape"
        );
    }
}

#[test]
fn watcher_failure_schema_has_discriminator_and_breaking_change_tripwire() {
    let schema = serde_json::to_value(schemars::schema_for!(ErrorBody)).unwrap();
    assert_eq!(
        schema["if"]["properties"]["code"]["enum"],
        serde_json::json!(["WATCHER_START_FAILED"]),
        "WATCHER_START_FAILED must remain the conditional schema discriminator"
    );
    assert!(
        schema["then"]["required"]
            .as_array()
            .is_some_and(|required| required.iter().any(|field| field == "data")),
        "watcher failures must require typed data"
    );
    assert_eq!(
        schema["then"]["properties"]["data"]["$ref"],
        serde_json::json!("#/definitions/ErrorData"),
        "watcher failures must reject null or unconstrained data"
    );
    let rendered = serde_json::to_string(&schema).unwrap();
    for stable_token in [
        "watcher_start_failure",
        "install",
        "reconcile",
        "additionalProperties",
    ] {
        assert!(
            rendered.contains(stable_token),
            "schema compatibility tripwire lost {stable_token}"
        );
    }
}
