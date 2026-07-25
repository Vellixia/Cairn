mod support;

use std::io::Write;
use std::sync::{Arc, Mutex};

use cairn_domain::{AgentInstanceId, GoalContractV1, IdempotencyKey};
use cairn_protocol::*;
use fixtures_repositories::FixtureRepo;
use support::TestDaemon;

#[derive(Clone, Default)]
struct LogBuffer(Arc<Mutex<Vec<u8>>>);

struct LogWriter(Arc<Mutex<Vec<u8>>>);

impl Write for LogWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for LogBuffer {
    type Writer = LogWriter;

    fn make_writer(&'a self) -> Self::Writer {
        LogWriter(self.0.clone())
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn diagnostics_errors_logs_and_storage_obey_feature002_privacy_boundaries() {
    const GOAL: &str = "PRIVATE_GOAL_002_7f2c";
    const SCOPE: &str = "PRIVATE_SCOPE_002_d10a";
    const ACCEPTANCE: &str = "PRIVATE_ACCEPTANCE_002_19bc";
    const CONSTRAINT: &str = "PRIVATE_CONSTRAINT_002_a2e4";
    const IGNORED: &str = "PRIVATE_IGNORED_002_434d";
    const ENVIRONMENT: &str = "PRIVATE_ENVIRONMENT_002_81aa";
    const DATABASE_PATH: &str = "/private/cairn/database-path-sentinel-002";
    const HOME_PATH: &str = "/private/cairn/home-path-sentinel-002";
    const SQL_TEXT: &str = "SELECT private_sql_sentinel_002 FROM secrets";
    const MIGRATION_CAUSE: &str = "private-migration-cause-002";
    const INTERNAL_DETAIL: &str = "private-rust-error-chain-002";

    let logs = LogBuffer::default();
    let _ = tracing_subscriber::fmt()
        .with_ansi(false)
        .without_time()
        .with_writer(logs.clone())
        .try_init();
    unsafe {
        std::env::set_var("CAIRN_PRIVACY_SENTINEL_002", ENVIRONMENT);
    }

    let daemon = TestDaemon::start().await;
    let repository = FixtureRepo::new().unwrap();
    std::fs::write(
        repository.root().join(".gitignore"),
        "private-ignored.txt\n",
    )
    .unwrap();
    std::fs::write(repository.root().join("private-ignored.txt"), IGNORED).unwrap();
    let registered: RegisterResult = serde_json::from_value(
        daemon
            .call(
                methods::REPOSITORY_REGISTER,
                &RegisterParams {
                    path: repository.root().to_string_lossy().to_string(),
                },
            )
            .await
            .unwrap(),
    )
    .unwrap();
    let started: SessionStartResult = serde_json::from_value(
        daemon
            .call(
                methods::SESSION_START,
                &SessionStartParams {
                    path: Some(repository.root().to_string_lossy().to_string()),
                    repository_id: None,
                    agent_type: "privacy".into(),
                    agent_instance_id: AgentInstanceId(uuid::Uuid::now_v7()),
                    agent_pid: None,
                    scope: Some(SessionScopeDto::LocalUnbound),
                },
            )
            .await
            .unwrap(),
    )
    .unwrap();
    let raw_token = started.resume_token.clone().unwrap();
    let project: ProjectCreateResult = serde_json::from_value(
        daemon
            .call(
                methods::PROJECT_CREATE,
                &ProjectCreateParams {
                    idempotency_key: IdempotencyKey::new_v7(),
                    name: "Privacy project".into(),
                    description: None,
                },
            )
            .await
            .unwrap(),
    )
    .unwrap();
    daemon
        .call(
            methods::PROJECT_REPOSITORY_ASSOCIATE,
            &ProjectRepositoryAssociateParams {
                idempotency_key: IdempotencyKey::new_v7(),
                project_id: project.project.project_id,
                repository_id: registered.repository.repository_id,
            },
        )
        .await
        .unwrap();
    let goal_contract = GoalContractV1::new(
        GOAL.into(),
        vec![SCOPE.into()],
        vec![],
        vec![ACCEPTANCE.into()],
        vec![CONSTRAINT.into()],
    )
    .unwrap();
    let success = daemon
        .call(
            methods::TASK_CREATE,
            &TaskCreateParams {
                idempotency_key: IdempotencyKey::new_v7(),
                project_id: project.project.project_id,
                title: "Privacy task".into(),
                goal_contract,
            },
        )
        .await
        .unwrap();
    let success_text = serde_json::to_string(&success).unwrap();
    assert!(
        success_text.contains(GOAL),
        "explicit task result may contain authored content"
    );

    let invalid = daemon
        .call(
            methods::TASK_CREATE,
            &serde_json::json!({
                "idempotency_key": IdempotencyKey::new_v7(),
                "project_id": project.project.project_id,
                "title": "Invalid privacy task",
                "goal_contract": {
                    "schema_version": 1,
                    "goal": "",
                    "included_scope": [SCOPE],
                    "excluded_scope": [],
                    "acceptance_criteria": [ACCEPTANCE],
                    "constraints": [CONSTRAINT]
                },
                "private_internal_detail": INTERNAL_DETAIL
            }),
        )
        .await
        .unwrap_err();
    let migration_error = ErrorBody {
        code: ErrorCode::MigrationFailed,
        message: "local storage migration failed".into(),
        data: Some(ErrorData::MigrationFailure { target_version: 2 }),
    };
    let replay_error = cairn_events::replay::MixedReplayError::InvalidReference;
    let error_surfaces = format!(
        "{}\n{}\n{}",
        serde_json::to_string(&invalid).unwrap(),
        serde_json::to_string(&migration_error).unwrap(),
        replay_error
    );
    for forbidden in [
        GOAL,
        SCOPE,
        ACCEPTANCE,
        CONSTRAINT,
        IGNORED,
        ENVIRONMENT,
        raw_token.as_str(),
        DATABASE_PATH,
        HOME_PATH,
        SQL_TEXT,
        MIGRATION_CAUSE,
        INTERNAL_DETAIL,
    ] {
        assert!(
            !error_surfaces.contains(forbidden),
            "typed errors leaked a privacy sentinel"
        );
    }

    let pool = cairn_storage_local::open_pool_at(&daemon.db_path())
        .await
        .unwrap();
    let stored_goal = sqlx::query_scalar::<_, String>(
        "SELECT goal_contract_json FROM task_revisions ORDER BY revision_number LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    for approved in [GOAL, SCOPE, ACCEPTANCE, CONSTRAINT] {
        assert!(
            stored_goal.contains(approved),
            "approved local task content missing"
        );
    }
    let mut database_bytes = std::fs::read(daemon.db_path()).unwrap();
    for suffix in ["-wal", "-shm"] {
        let path = std::path::PathBuf::from(format!("{}{}", daemon.db_path().display(), suffix));
        if let Ok(bytes) = std::fs::read(path) {
            database_bytes.extend(bytes);
        }
    }
    let database_text = String::from_utf8_lossy(&database_bytes);
    for forbidden in [
        IGNORED,
        ENVIRONMENT,
        raw_token.as_str(),
        DATABASE_PATH,
        HOME_PATH,
        SQL_TEXT,
        MIGRATION_CAUSE,
        INTERNAL_DETAIL,
    ] {
        assert!(
            !database_text.contains(forbidden),
            "database leaked prohibited sentinel"
        );
    }

    let log_text = String::from_utf8(logs.0.lock().unwrap().clone()).unwrap();
    for forbidden in [
        GOAL,
        SCOPE,
        ACCEPTANCE,
        CONSTRAINT,
        IGNORED,
        ENVIRONMENT,
        raw_token.as_str(),
        DATABASE_PATH,
        HOME_PATH,
        SQL_TEXT,
        MIGRATION_CAUSE,
        INTERNAL_DETAIL,
    ] {
        assert!(
            !log_text.contains(forbidden),
            "diagnostic log leaked a privacy sentinel"
        );
    }
    pool.close().await;
    unsafe {
        std::env::remove_var("CAIRN_PRIVACY_SENTINEL_002");
    }
    drop(repository);
    daemon.stop().await;
}
