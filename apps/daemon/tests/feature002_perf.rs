use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use cairn_domain::{AgentInstanceId, GoalContractV1, IdempotencyKey, SessionId, Timestamp};
use cairn_project::{
    AssociateRepository, CreateProject, CreateTask, ProjectService, ReviseTask, TaskService,
    UpdateProject,
};
use cairn_session::{BindSession, SessionConfig, SessionService};
use cairn_storage_local::records::{RepositoryRow, SessionRow, SnapshotRow, WorktreeRow};
use cairn_storage_local::writer::WorktreeWriters;

const PROJECT_COUNT: usize = 100;
const TASK_COUNT: usize = 1_000;
const REVISIONS_PER_TASK: usize = 5;
const THRESHOLD: Duration = Duration::from_secs(2);
/// Projects that also receive a repository association and a bound session.
const ASSOCIATED_PROJECTS: usize = 20;
/// Task creations discarded as cold-cache warm-up before sampling begins.
const TASK_WARMUP: usize = 10;
/// Samples collected per warmed read/update operation.
const READ_SAMPLES: usize = 100;

fn contract(task: usize, revision: usize) -> GoalContractV1 {
    GoalContractV1::new(
        format!("performance task {task} revision {revision}"),
        vec!["deterministic fixture".into()],
        vec![],
        vec!["under two seconds".into()],
        vec!["offline".into()],
    )
    .unwrap()
}

fn p95(samples: &[Duration]) -> Duration {
    assert!(
        !samples.is_empty(),
        "performance measurement set must not be empty"
    );
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    let rank = (sorted.len() * 95).div_ceil(100).saturating_sub(1);
    sorted[rank]
}

fn assert_p95(name: &str, samples: &[Duration]) {
    let value = p95(samples);
    println!(
        "feature002_perf operation={name} samples={} p95_ms={:.3} threshold_ms=2000",
        samples.len(),
        value.as_secs_f64() * 1_000.0
    );
    assert!(
        value <= THRESHOLD,
        "{name} p95 exceeded two seconds: {value:?}"
    );
}

fn tool_version(command: &str) -> String {
    Command::new(command)
        .arg("--version")
        .output()
        .ok()
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .unwrap_or_else(|| "unavailable".into())
        .trim()
        .to_string()
}

async fn seed_repository_and_session(
    pool: &sqlx::SqlitePool,
    index: usize,
) -> (String, String, SessionId) {
    let repository_id = uuid::Uuid::now_v7().to_string();
    let worktree_id = uuid::Uuid::now_v7().to_string();
    let snapshot_id = uuid::Uuid::now_v7().to_string();
    let session_id = SessionId::new_v7();
    let now = Timestamp::now().to_rfc3339();
    cairn_storage_local::repos::insert(
        pool,
        &RepositoryRow {
            id: repository_id.clone(),
            repo_uuid: uuid::Uuid::new_v4().to_string(),
            canonical_path: format!("/performance/repository-{index}"),
            default_remote_name: None,
            default_remote_url: None,
            copied_from_repository_id: None,
            registered_at: now.clone(),
        },
    )
    .await
    .unwrap();
    cairn_storage_local::worktrees::insert(
        pool,
        &WorktreeRow {
            id: worktree_id.clone(),
            repository_id: repository_id.clone(),
            worktree_uuid: uuid::Uuid::new_v4().to_string(),
            path: format!("/performance/repository-{index}"),
            is_main: 1,
            registered_at: now.clone(),
        },
    )
    .await
    .unwrap();
    cairn_storage_local::snapshots::insert(
        pool,
        &SnapshotRow {
            id: snapshot_id.clone(),
            worktree_id: worktree_id.clone(),
            branch: Some("main".into()),
            head_commit: "performance-head".into(),
            staged_fp: "performance-staged".into(),
            unstaged_fp: "performance-unstaged".into(),
            untracked_fp: "performance-untracked".into(),
            snapshot_fp: uuid::Uuid::new_v4().to_string(),
            fp_schema_version: 1,
            created_at: now.clone(),
        },
    )
    .await
    .unwrap();
    cairn_storage_local::sessions::insert(
        pool,
        &SessionRow {
            id: session_id.to_string(),
            repository_id: repository_id.clone(),
            worktree_id: worktree_id.clone(),
            local_user: "performance".into(),
            agent_type: "performance".into(),
            agent_instance_id: AgentInstanceId(uuid::Uuid::new_v4()).to_string(),
            agent_pid: None,
            resume_token_hash: "performance-hash".into(),
            lease_expires_at: now.clone(),
            state: "active".into(),
            start_snapshot_id: snapshot_id.clone(),
            current_snapshot_id: snapshot_id,
            started_at: now.clone(),
            ended_at: None,
            last_heartbeat_at: now,
            recovering_since: None,
            binding_mode: "local_unbound".into(),
        },
    )
    .await
    .unwrap();
    (repository_id, worktree_id, session_id)
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "SC-010 acceptance; execute explicitly in release profile"]
async fn feature002_operations_meet_two_second_p95_at_authoritative_fixture_size() {
    assert!(
        !cfg!(debug_assertions),
        "SC-010 must execute with --release"
    );
    let dir = tempfile::tempdir().unwrap();
    let pool = cairn_storage_local::open_pool_at(&dir.path().join("performance.sqlite3"))
        .await
        .unwrap();
    let projects = ProjectService::new(pool.clone());
    let tasks = TaskService::new(pool.clone());
    let sessions = SessionService::new(
        pool.clone(),
        Arc::new(WorktreeWriters::new()),
        SessionConfig::from_env(),
    );

    let mut project_create = Vec::new();
    let mut project_rows = Vec::with_capacity(PROJECT_COUNT);
    for index in 0..PROJECT_COUNT {
        let started = Instant::now();
        let project = projects
            .create(CreateProject {
                idempotency_key: IdempotencyKey::new_v7(),
                name: format!("Performance project {index:03}"),
                description: None,
            })
            .await
            .unwrap()
            .project;
        if index > 0 {
            project_create.push(started.elapsed());
        }
        project_rows.push(project);
    }

    let mut association_samples = Vec::new();
    let mut session_rows = Vec::new();
    for (index, project) in project_rows.iter().enumerate().take(ASSOCIATED_PROJECTS) {
        let (repository_id, _, session_id) = seed_repository_and_session(&pool, index).await;
        let started = Instant::now();
        projects
            .associate_repository(AssociateRepository {
                idempotency_key: IdempotencyKey::new_v7(),
                project_id: project.id,
                repository_id,
            })
            .await
            .unwrap();
        if index > 0 {
            association_samples.push(started.elapsed());
        }
        session_rows.push((session_id, project.id));
    }

    let mut task_create = Vec::new();
    let mut task_rows = Vec::with_capacity(TASK_COUNT);
    let mut binding_revisions = vec![None; ASSOCIATED_PROJECTS];
    for index in 0..TASK_COUNT {
        let project_index = index % PROJECT_COUNT;
        let started = Instant::now();
        let task = tasks
            .create(CreateTask {
                idempotency_key: IdempotencyKey::new_v7(),
                project_id: project_rows[project_index].id,
                title: format!("Performance task {index:04}"),
                goal_contract: contract(index, 1),
            })
            .await
            .unwrap();
        if index >= TASK_WARMUP {
            task_create.push(started.elapsed());
        }
        if project_index < ASSOCIATED_PROJECTS && binding_revisions[project_index].is_none() {
            binding_revisions[project_index] = Some(task.revision.id);
        }
        task_rows.push(task);
    }

    let mut task_revise = Vec::new();
    for (index, task) in task_rows.iter().enumerate() {
        for revision in 2..=REVISIONS_PER_TASK {
            let started = Instant::now();
            tasks
                .revise(ReviseTask {
                    idempotency_key: IdempotencyKey::new_v7(),
                    task_id: task.task.id,
                    parent_revision_id: None,
                    goal_contract: contract(index, revision),
                })
                .await
                .unwrap();
            task_revise.push(started.elapsed());
        }
    }

    let mut binding_samples = Vec::new();
    for (index, (session_id, project_id)) in session_rows.iter().enumerate() {
        let started = Instant::now();
        sessions
            .bind(BindSession {
                idempotency_key: IdempotencyKey::new_v7(),
                session_id: *session_id,
                project_id: *project_id,
                task_revision_id: binding_revisions[index].unwrap(),
            })
            .await
            .unwrap();
        if index > 0 {
            binding_samples.push(started.elapsed());
        }
    }

    // Warm up read paths before collecting stable samples.
    projects.list(None, None, 100).await.unwrap();
    projects.get(project_rows[0].id).await.unwrap();
    tasks.list(project_rows[0].id, None, 100).await.unwrap();
    tasks.get(task_rows[0].task.id, None).await.unwrap();

    let mut project_list = Vec::new();
    let mut project_show = Vec::new();
    let mut task_list = Vec::new();
    let mut task_show = Vec::new();
    let mut project_update = Vec::new();
    for index in 0..READ_SAMPLES {
        let started = Instant::now();
        projects.list(None, None, 100).await.unwrap();
        project_list.push(started.elapsed());
        let started = Instant::now();
        projects.get(project_rows[index].id).await.unwrap();
        project_show.push(started.elapsed());
        let started = Instant::now();
        tasks.list(project_rows[index].id, None, 100).await.unwrap();
        task_list.push(started.elapsed());
        let started = Instant::now();
        tasks.get(task_rows[index].task.id, None).await.unwrap();
        task_show.push(started.elapsed());
        let started = Instant::now();
        projects
            .update(UpdateProject {
                idempotency_key: IdempotencyKey::new_v7(),
                project_id: project_rows[index].id,
                name: None,
                description: Some(format!("updated {index}")),
                clear_description: false,
                status: None,
            })
            .await
            .unwrap();
        project_update.push(started.elapsed());
    }

    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM projects")
            .fetch_one(&pool)
            .await
            .unwrap(),
        PROJECT_COUNT as i64
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM tasks")
            .fetch_one(&pool)
            .await
            .unwrap(),
        TASK_COUNT as i64
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM task_revisions")
            .fetch_one(&pool)
            .await
            .unwrap(),
        (TASK_COUNT * REVISIONS_PER_TASK) as i64
    );

    let sqlite_version = sqlx::query_scalar::<_, String>("SELECT sqlite_version()")
        .fetch_one(&pool)
        .await
        .unwrap();
    println!(
        "feature002_perf_environment os={} arch={} rustc={} cargo={} sqlite={} projects={} tasks={} revisions_per_task={} profile=release warmup=read-path-once",
        std::env::consts::OS,
        std::env::consts::ARCH,
        tool_version("rustc"),
        tool_version("cargo"),
        sqlite_version,
        PROJECT_COUNT,
        TASK_COUNT,
        REVISIONS_PER_TASK,
    );
    for (name, samples, expected) in [
        ("project_create", &project_create, PROJECT_COUNT - 1),
        ("project_list", &project_list, READ_SAMPLES),
        ("project_show", &project_show, READ_SAMPLES),
        ("project_update", &project_update, READ_SAMPLES),
        (
            "repository_associate",
            &association_samples,
            ASSOCIATED_PROJECTS - 1,
        ),
        ("task_create", &task_create, TASK_COUNT - TASK_WARMUP),
        ("task_list", &task_list, READ_SAMPLES),
        ("task_show", &task_show, READ_SAMPLES),
        (
            "task_revise",
            &task_revise,
            TASK_COUNT * (REVISIONS_PER_TASK - 1),
        ),
        ("session_bind", &binding_samples, ASSOCIATED_PROJECTS - 1),
    ] {
        assert_eq!(
            samples.len(),
            expected,
            "{name} measurement set is incomplete"
        );
        assert_p95(name, samples);
    }
}
