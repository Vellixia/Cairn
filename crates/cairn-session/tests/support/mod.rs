#![allow(dead_code)]

use std::path::PathBuf;
use std::sync::Arc;

use cairn_domain::{
    EventId, GoalContractV1, IdempotencyKey, SessionId, SessionState, SnapshotId, Timestamp,
};
use cairn_events::{EventBuilder, SessionStartedPayload};
use cairn_project::{AssociateRepository, CreateProject, CreateTask, ProjectService, TaskService};
use cairn_session::{SessionConfig, SessionService};
use cairn_storage_local::records::{RepositoryRow, SessionRow, SnapshotRow, WorktreeRow};
use cairn_storage_local::{
    events, open_pool_at, repos, sessions, snapshots, worktrees, WorktreeWriters,
};
use sqlx::SqlitePool;
use tempfile::TempDir;

pub struct Harness {
    _temp: TempDir,
    pub path: PathBuf,
    pub pool: SqlitePool,
}

#[derive(Clone)]
pub struct Context {
    pub repository_id: String,
    pub worktree_id: String,
    pub session: SessionRow,
    pub project_id: cairn_domain::ProjectId,
    pub task_id: cairn_domain::TaskId,
    pub revision_id: cairn_domain::TaskRevisionId,
}

#[derive(Clone)]
pub struct StartContext {
    pub repository_id: String,
    pub worktree_id: String,
    pub snapshot: SnapshotRow,
}

impl Harness {
    pub async fn new() -> Self {
        let temp = tempfile::tempdir().expect("temporary test directory");
        let path = temp.path().join("cairn.db");
        let pool = open_pool_at(&path).await.expect("open migrated database");
        Self {
            _temp: temp,
            path,
            pool,
        }
    }

    pub async fn independent_pool(&self) -> SqlitePool {
        open_pool_at(&self.path).await.expect("independent pool")
    }

    pub fn sessions(&self) -> SessionService {
        SessionService::new(
            self.pool.clone(),
            Arc::new(WorktreeWriters::new()),
            SessionConfig::from_env(),
        )
    }

    pub async fn start_context(&self) -> StartContext {
        let repository_id = EventId::new_v7().to_string();
        let worktree_id = EventId::new_v7().to_string();
        let snapshot_id = SnapshotId::new_v7().to_string();
        let now = Timestamp::now().to_rfc3339();
        repos::insert(
            &self.pool,
            &RepositoryRow {
                id: repository_id.clone(),
                repo_uuid: EventId::new_v7().to_string(),
                canonical_path: format!("/repo/{repository_id}"),
                default_remote_name: None,
                default_remote_url: None,
                copied_from_repository_id: None,
                registered_at: now.clone(),
            },
        )
        .await
        .unwrap();
        worktrees::insert(
            &self.pool,
            &WorktreeRow {
                id: worktree_id.clone(),
                repository_id: repository_id.clone(),
                worktree_uuid: EventId::new_v7().to_string(),
                path: format!("/repo/{repository_id}/worktree"),
                is_main: 1,
                registered_at: now.clone(),
            },
        )
        .await
        .unwrap();
        let snapshot = SnapshotRow {
            id: snapshot_id,
            worktree_id: worktree_id.clone(),
            branch: Some("main".into()),
            head_commit: "0".repeat(40),
            staged_fp: "1".repeat(64),
            unstaged_fp: "2".repeat(64),
            untracked_fp: "3".repeat(64),
            snapshot_fp: "4".repeat(64),
            fp_schema_version: 1,
            created_at: now,
        };
        snapshots::insert(&self.pool, &snapshot).await.unwrap();
        StartContext {
            repository_id,
            worktree_id,
            snapshot,
        }
    }

    pub async fn add_project_scope(
        &self,
        context: &StartContext,
    ) -> (
        cairn_domain::ProjectId,
        cairn_domain::TaskId,
        cairn_domain::TaskRevisionId,
    ) {
        let project_service = ProjectService::new(self.pool.clone());
        let project = project_service
            .create(CreateProject {
                idempotency_key: IdempotencyKey::new_v7(),
                name: "Bound start project".into(),
                description: None,
            })
            .await
            .unwrap()
            .project;
        project_service
            .associate_repository(AssociateRepository {
                idempotency_key: IdempotencyKey::new_v7(),
                project_id: project.id,
                repository_id: context.repository_id.clone(),
            })
            .await
            .unwrap();
        let task = TaskService::new(self.pool.clone())
            .create(CreateTask {
                idempotency_key: IdempotencyKey::new_v7(),
                project_id: project.id,
                title: "Bound start task".into(),
                goal_contract: contract("bound start revision one"),
            })
            .await
            .unwrap();
        (project.id, task.task.id, task.revision.id)
    }

    pub async fn context(&self, state: SessionState) -> Context {
        let repository_id = EventId::new_v7().to_string();
        let worktree_id = EventId::new_v7().to_string();
        let snapshot_id = SnapshotId::new_v7().to_string();
        let session_id = SessionId::new_v7().to_string();
        let now = Timestamp::now();
        let now_s = now.to_rfc3339();
        repos::insert(
            &self.pool,
            &RepositoryRow {
                id: repository_id.clone(),
                repo_uuid: EventId::new_v7().to_string(),
                canonical_path: format!("/repo/{repository_id}"),
                default_remote_name: None,
                default_remote_url: None,
                copied_from_repository_id: None,
                registered_at: now_s.clone(),
            },
        )
        .await
        .unwrap();
        worktrees::insert(
            &self.pool,
            &WorktreeRow {
                id: worktree_id.clone(),
                repository_id: repository_id.clone(),
                worktree_uuid: EventId::new_v7().to_string(),
                path: format!("/repo/{repository_id}/worktree"),
                is_main: 1,
                registered_at: now_s.clone(),
            },
        )
        .await
        .unwrap();
        let snapshot = SnapshotRow {
            id: snapshot_id.clone(),
            worktree_id: worktree_id.clone(),
            branch: Some("main".into()),
            head_commit: "0".repeat(40),
            staged_fp: "1".repeat(64),
            unstaged_fp: "2".repeat(64),
            untracked_fp: "3".repeat(64),
            snapshot_fp: "4".repeat(64),
            fp_schema_version: 1,
            created_at: now_s.clone(),
        };
        snapshots::insert(&self.pool, &snapshot).await.unwrap();
        let terminal = matches!(state, SessionState::Stopped | SessionState::Interrupted);
        let session = SessionRow {
            id: session_id.clone(),
            repository_id: repository_id.clone(),
            worktree_id: worktree_id.clone(),
            local_user: "tester".into(),
            agent_type: "test-agent".into(),
            agent_instance_id: EventId::new_v7().to_string(),
            agent_pid: Some(4242),
            resume_token_hash: "5".repeat(64),
            lease_expires_at: now.plus_seconds(900).to_rfc3339(),
            state: state.as_str().into(),
            start_snapshot_id: snapshot_id.clone(),
            current_snapshot_id: snapshot_id.clone(),
            started_at: now_s.clone(),
            ended_at: terminal.then(|| now_s.clone()),
            last_heartbeat_at: now_s.clone(),
            recovering_since: (state == SessionState::Recovering).then(|| now_s.clone()),
            binding_mode: "local_unbound".into(),
        };
        sessions::insert(&self.pool, &session).await.unwrap();
        let started = EventBuilder::session_started(
            &repository_id,
            &worktree_id,
            &session_id,
            &SessionStartedPayload {
                agent_type: session.agent_type.clone(),
                agent_instance_id: session.agent_instance_id.clone(),
                start_snapshot_id: snapshot_id,
                local_user: session.local_user.clone(),
            },
        );
        let writers = WorktreeWriters::new();
        events::serialized_txn(
            &self.pool,
            &writers,
            &worktree_id,
            Box::new(move |conn| {
                Box::pin(async move {
                    events::append_event(conn, &started).await?;
                    Ok(())
                })
            }),
        )
        .await
        .unwrap();

        let project_service = ProjectService::new(self.pool.clone());
        let project = project_service
            .create(CreateProject {
                idempotency_key: IdempotencyKey::new_v7(),
                name: "Binding project".into(),
                description: None,
            })
            .await
            .unwrap()
            .project;
        project_service
            .associate_repository(AssociateRepository {
                idempotency_key: IdempotencyKey::new_v7(),
                project_id: project.id,
                repository_id: repository_id.clone(),
            })
            .await
            .unwrap();
        let created = TaskService::new(self.pool.clone())
            .create(CreateTask {
                idempotency_key: IdempotencyKey::new_v7(),
                project_id: project.id,
                title: "Binding task".into(),
                goal_contract: contract("revision one"),
            })
            .await
            .unwrap();
        Context {
            repository_id,
            worktree_id,
            session,
            project_id: project.id,
            task_id: created.task.id,
            revision_id: created.revision.id,
        }
    }
}

pub fn contract(goal: &str) -> GoalContractV1 {
    GoalContractV1::new(
        goal.into(),
        vec!["included".into()],
        vec!["excluded".into()],
        vec!["accepted".into()],
        vec!["constraint".into()],
    )
    .unwrap()
}

pub async fn stable_session_events(pool: &SqlitePool, session_id: &str) -> Vec<String> {
    events::list_events(pool, None, None, Some(session_id), None, 100)
        .await
        .unwrap()
        .into_iter()
        .map(|event| {
            format!(
                "{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}",
                event.seq,
                event.id,
                event.idempotency_key,
                event.event_type,
                event.repository_id.as_deref().unwrap_or(""),
                event.worktree_id.as_deref().unwrap_or(""),
                event.session_id.as_deref().unwrap_or(""),
                event.snapshot_id.as_deref().unwrap_or(""),
                event.payload,
                event.recorded_at,
                event.aggregate_id.as_deref().unwrap_or(""),
                event.aggregate_seq.unwrap_or_default(),
            )
        })
        .collect()
}
