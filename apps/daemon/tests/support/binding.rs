use cairn_protocol::*;
use fixtures_repositories::FixtureRepo;

use super::TestDaemon;

pub struct BindingFixture {
    pub repository: FixtureRepo,
    pub repository_id: String,
    pub session_id: SessionId,
    pub project_id: ProjectId,
    pub task_id: TaskId,
    pub revision_id: TaskRevisionId,
    pub agent_instance_id: AgentInstanceId,
    pub resume_token: String,
}

pub struct BoundStartFixture {
    pub repository: FixtureRepo,
    pub repository_id: String,
    pub project_id: ProjectId,
    pub task_id: TaskId,
    pub revision_id: TaskRevisionId,
}

impl BoundStartFixture {
    pub async fn create(daemon: &TestDaemon) -> Self {
        let repository = FixtureRepo::new().unwrap();
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
        let repository_id = registered.repository.repository_id;
        let project: ProjectCreateResult = serde_json::from_value(
            daemon
                .call(
                    methods::PROJECT_CREATE,
                    &ProjectCreateParams {
                        idempotency_key: IdempotencyKey::new_v7(),
                        name: "Bound start acceptance".into(),
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
                    repository_id: repository_id.clone(),
                },
            )
            .await
            .unwrap();
        let task: TaskCreateResult = serde_json::from_value(
            daemon
                .call(
                    methods::TASK_CREATE,
                    &TaskCreateParams {
                        idempotency_key: IdempotencyKey::new_v7(),
                        project_id: project.project.project_id,
                        title: "Bound start session".into(),
                        goal_contract: contract("bound start revision one"),
                    },
                )
                .await
                .unwrap(),
        )
        .unwrap();
        Self {
            repository,
            repository_id,
            project_id: project.project.project_id,
            task_id: task.task.task_id,
            revision_id: task.revision.revision_id,
        }
    }

    pub fn start_params(&self, agent_instance_id: AgentInstanceId) -> SessionStartParams {
        SessionStartParams {
            path: Some(self.repository.root().to_string_lossy().to_string()),
            repository_id: None,
            agent_type: "bound-start-acceptance".into(),
            agent_instance_id,
            agent_pid: None,
            scope: Some(SessionScopeDto::ProjectBound {
                project_id: self.project_id,
                task_revision_id: self.revision_id,
            }),
        }
    }
}

impl BindingFixture {
    pub async fn create(daemon: &TestDaemon) -> Self {
        let repository = FixtureRepo::new().unwrap();
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
        let repository_id = registered.repository.repository_id;
        let agent_instance_id = AgentInstanceId(uuid::Uuid::now_v7());
        let started: SessionStartResult = serde_json::from_value(
            daemon
                .call(
                    methods::SESSION_START,
                    &SessionStartParams {
                        path: Some(repository.root().to_string_lossy().to_string()),
                        repository_id: None,
                        agent_type: "binding-acceptance".into(),
                        agent_instance_id,
                        agent_pid: None,
                        scope: None,
                    },
                )
                .await
                .unwrap(),
        )
        .unwrap();
        let project: ProjectCreateResult = serde_json::from_value(
            daemon
                .call(
                    methods::PROJECT_CREATE,
                    &ProjectCreateParams {
                        idempotency_key: IdempotencyKey::new_v7(),
                        name: "Binding acceptance".into(),
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
                    repository_id: repository_id.clone(),
                },
            )
            .await
            .unwrap();
        let task: TaskCreateResult = serde_json::from_value(
            daemon
                .call(
                    methods::TASK_CREATE,
                    &TaskCreateParams {
                        idempotency_key: IdempotencyKey::new_v7(),
                        project_id: project.project.project_id,
                        title: "Bind session".into(),
                        goal_contract: contract("revision one"),
                    },
                )
                .await
                .unwrap(),
        )
        .unwrap();
        Self {
            repository,
            repository_id,
            session_id: started.session.session_id,
            project_id: project.project.project_id,
            task_id: task.task.task_id,
            revision_id: task.revision.revision_id,
            agent_instance_id,
            resume_token: started.resume_token.expect("new session resume token"),
        }
    }

    pub fn bind_params(&self, idempotency_key: IdempotencyKey) -> SessionBindParams {
        SessionBindParams {
            idempotency_key,
            session_id: self.session_id,
            project_id: self.project_id,
            task_revision_id: self.revision_id,
        }
    }
}

pub fn contract(goal: &str) -> GoalContractV1 {
    GoalContractV1::new(
        goal.into(),
        vec!["binding".into()],
        vec![],
        vec!["persists".into()],
        vec!["append-only".into()],
    )
    .unwrap()
}

pub fn retry_iterations() -> usize {
    std::env::var("CAIRN_BINDING_RETRY_ITERS")
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|count| *count > 0)
        .unwrap_or(8)
}
