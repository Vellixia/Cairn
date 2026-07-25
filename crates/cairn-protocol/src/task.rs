use cairn_domain::{
    GoalContractV1, ProjectId, Task, TaskId, TaskRevision, TaskRevisionId, Timestamp,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TaskCreateParams {
    pub idempotency_key: cairn_domain::IdempotencyKey,
    pub project_id: ProjectId,
    pub title: String,
    pub goal_contract: GoalContractV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TaskCreateResult {
    pub task: TaskDto,
    pub revision: TaskRevisionDto,
    pub created: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TaskReviseParams {
    pub idempotency_key: cairn_domain::IdempotencyKey,
    pub task_id: TaskId,
    pub parent_revision_id: Option<TaskRevisionId>,
    pub goal_contract: GoalContractV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TaskReviseResult {
    pub task: TaskDto,
    pub revision: TaskRevisionDto,
    pub created: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TaskListParams {
    pub project_id: ProjectId,
    pub after_task_id: Option<TaskId>,
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TaskSummaryDto {
    pub task_id: TaskId,
    pub project_id: ProjectId,
    pub title: String,
    pub latest_revision_number: u64,
    pub latest_revision_fingerprint: String,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TaskListResult {
    pub tasks: Vec<TaskSummaryDto>,
    pub next_after_task_id: Option<TaskId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TaskGetParams {
    pub task_id: TaskId,
    pub revision_id: Option<TaskRevisionId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TaskGetResult {
    pub task: TaskDto,
    pub revision: TaskRevisionDto,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TaskDto {
    pub task_id: TaskId,
    pub project_id: ProjectId,
    pub title: String,
    pub latest_revision_number: u64,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl From<Task> for TaskDto {
    fn from(task: Task) -> Self {
        Self {
            task_id: task.id,
            project_id: task.project_id,
            title: task.title,
            latest_revision_number: task.latest_revision_number,
            created_at: task.created_at,
            updated_at: task.updated_at,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TaskRevisionDto {
    pub revision_id: TaskRevisionId,
    pub task_id: TaskId,
    pub revision_number: u64,
    pub parent_revision_id: Option<TaskRevisionId>,
    pub goal_contract: GoalContractV1,
    pub goal_contract_fingerprint: String,
    pub created_at: Timestamp,
}

impl From<TaskRevision> for TaskRevisionDto {
    fn from(revision: TaskRevision) -> Self {
        Self {
            revision_id: revision.id,
            task_id: revision.task_id,
            revision_number: revision.revision_number,
            parent_revision_id: revision.parent_revision_id,
            goal_contract: revision.goal_contract,
            goal_contract_fingerprint: revision.goal_contract_fingerprint,
            created_at: revision.created_at,
        }
    }
}
