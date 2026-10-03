//! Alpha.7 legacy edge migration through the installed setup path.
//!
//! The alpha.7 tag ends at local schema 12. Its migrations 1–12 are retained
//! unchanged, so this builds that released schema, leaves its writes in WAL,
//! then lets the current `cairn setup` start the daemon and perform the real
//! bootstrap. The source database is deliberately kept open: closing the last
//! writer can checkpoint away the pending-operation fixture this test exists to
//! protect.

use cairn_core::event::{EventAgent, EventKind, SafeCanonicalEvent, CONTRACT_VERSION};
use cairn_core::eventid;
use cairn_e2e::feature005::{Pg, LOCAL_SCHEMA_V12};
use cairn_e2e::Sandbox;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use uuid::Uuid;

fn sidecar(path: &std::path::Path, suffix: &str) -> std::path::PathBuf {
    let mut sidecar = path.as_os_str().to_owned();
    sidecar.push(suffix);
    sidecar.into()
}

#[test]
fn setup_preserves_an_alpha7_wal_and_moves_only_safe_pending_operations() {
    let Some(pg) = Pg::start() else {
        eprintln!("skipped: CAIRN_TEST_DATABASE_URL is not set");
        return;
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    runtime.block_on(async {
    let sandbox = Sandbox::new();
    sandbox.git(&[
        "remote",
        "set-url",
        "origin",
        "git@example.test:feature005.git",
    ]);
    let legacy = sandbox.cairn_home().join("cairn.sqlite3");
    let options = SqliteConnectOptions::new()
        .filename(&legacy)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .expect("open alpha.7 fixture in WAL mode");
    cairn_store::migrate::run_to(&pool, LOCAL_SCHEMA_V12)
        .await
        .expect("build the alpha.7 schema");

    let user = Uuid::now_v7();
    let project = Uuid::now_v7();
    let task = Uuid::now_v7();
    let criterion = Uuid::now_v7();
    let session = Uuid::now_v7();
    let account = Uuid::now_v7();
    sqlx::query("INSERT INTO users VALUES (?, NULL, 'alpha7 user', 'now')")
        .bind(user.to_string())
        .execute(&pool)
        .await
        .expect("seed user");
    sqlx::query("INSERT INTO projects (id,name,git_common_dir,linked,created_at,updated_at) VALUES (?,'alpha7 project','/alpha7',1,'now','now')")
        .bind(project.to_string())
        .execute(&pool)
        .await
        .expect("seed project");
    sqlx::query("INSERT INTO tasks (id,project_id,title,goal,status,created_at,updated_at) VALUES (?,?,'offline task','must remain offline','todo','now','now')")
        .bind(task.to_string())
        .bind(project.to_string())
        .execute(&pool)
        .await
        .expect("seed task");
    sqlx::query("INSERT INTO task_criteria (id,task_id,ordinal,label,text,state,verification,revision,created_at,updated_at) VALUES (?,?,1,'AC-1','offline criterion','pending','unverified',1,'now','now')")
        .bind(criterion.to_string())
        .bind(task.to_string())
        .execute(&pool)
        .await
        .expect("seed task criterion");
    sqlx::query("INSERT INTO sessions (id,project_id,task_id,user_id,agent,branch,worktree_path,agent_session_key,status,started_at,last_event_at,daemon_run_id) VALUES (?,?,?,?, 'codex','main','/alpha7','alpha7-session','active','now','now','run')")
        .bind(session.to_string())
        .bind(project.to_string())
        .bind(task.to_string())
        .bind(user.to_string())
        .execute(&pool)
        .await
        .expect("seed task-bound session");

    let event = SafeCanonicalEvent {
        event_id: eventid::event_id(session, 1),
        contract_version: CONTRACT_VERSION,
        kind: EventKind::AgentQuiesced,
        agent: EventAgent::Codex,
        vendor_event: None,
        session_id: session,
        session_seq: 1,
        occurred_at: chrono::Utc::now(),
        content: None,
    };
    let event_payload = serde_json::to_string(&event).expect("serialize event");
    sqlx::query("INSERT INTO event_spool (event_id,session_id,project_id,account_id,session_seq,kind,payload,payload_bytes,boundary_class,state,created_at) VALUES (?,?,?,?,?,?,?,?,?,'pending','now')")
        .bind(event.event_id.to_string())
        .bind(session.to_string())
        .bind(project.to_string())
        .bind(account.to_string())
        .bind(1_i64)
        .bind(event.kind.as_str())
        .bind(&event_payload)
        .bind(event_payload.len() as i64)
        .bind(event.boundary_class())
        .execute(&pool)
        .await
        .expect("seed pending event");
    let command = eventid::command_id("session", &session.to_string(), 1);
    sqlx::query("INSERT INTO command_spool (command_id,scope_kind,scope_key,session_id,project_id,account_id,command_seq,kind,payload,state,created_at) VALUES (?,'session',?,?,?,?,1,'remember','{}','in_flight','now')")
        .bind(command.to_string())
        .bind(session.to_string())
        .bind(session.to_string())
        .bind(project.to_string())
        .bind(account.to_string())
        .execute(&pool)
        .await
        .expect("seed in-flight command");

    let wal = sidecar(&legacy, "-wal");
    let source_db = std::fs::read(&legacy).expect("read source database");
    let source_wal = std::fs::read(&wal).expect("fixture writes remain in WAL");

    let account = pg.owner.id.to_string();
    let result = sandbox.cairn_with_env(
        &["--json", "setup"],
        &[
            ("CAIRN_SERVER_URL", &pg.server.base),
            ("CAIRN_SERVER_TOKEN", &pg.owner.token),
            ("CAIRN_ACCOUNT_ID", &account),
        ],
    );
    assert!(
        result.ok(),
        "setup failed with {}\nstdout: {}\nstderr: {}",
        result.code,
        result.stdout,
        result.stderr
    );
    let setup: serde_json::Value = serde_json::from_str(&result.stdout).expect("setup JSON");
    let report = &setup["data"]["legacy_migration"];
    assert_eq!(report["status"], "migrated", "{report}");
    assert_eq!(report["pending_operations"], 2, "{report}");
    assert_eq!(report["retained_operations"], 0, "{report}");

    assert_eq!(
        std::fs::read(&legacy).expect("read source after setup"),
        source_db
    );
    assert_eq!(
        std::fs::read(&wal).expect("read source WAL after setup"),
        source_wal
    );

    let edge =
        sqlx::SqlitePool::connect(&format!("sqlite://{}?mode=ro", sandbox.db_path().display()))
            .await
            .expect("open migrated edge");
    let events: Vec<String> = sqlx::query_scalar("SELECT event_id FROM event_spool")
        .fetch_all(&edge)
        .await
        .expect("read migrated event");
    let commands: Vec<String> = sqlx::query_scalar("SELECT command_id FROM command_spool")
        .fetch_all(&edge)
        .await
        .expect("read migrated command");
    assert_eq!(events, vec![event.event_id.to_string()]);
    assert_eq!(commands, vec![command.to_string()]);
    for table in ["tasks", "task_criteria"] {
        let exists: i64 = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?)",
        )
        .bind(table)
        .fetch_one(&edge)
        .await
        .expect("check thin edge schema");
        assert_eq!(exists, 0, "{table} must remain offline");
    }
    edge.close().await;

    let bundle: serde_json::Value = serde_json::from_slice(
        &std::fs::read(
            sandbox
                .cairn_home()
                .join("removed_feature/tasks-v1/removed_feature.json"),
        )
        .expect("read removed-feature bundle"),
    )
    .expect("parse removed-feature bundle");
    let records = bundle["records"].as_array().expect("bundle records");
    assert!(records
        .iter()
        .any(|row| row["source_table"] == "tasks" && row["source_id"] == task.to_string()));
    assert!(records
        .iter()
        .any(|row| row["source_table"] == "task_criteria"
            && row["source_id"] == criterion.to_string()));
    assert!(records
        .iter()
        .all(|row| row["disposition"] == "retained"));
    });
}
