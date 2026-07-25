use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{GoalContractV1, ProjectId, TaskId, TaskRevisionId, Timestamp};

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TaskValidationError {
    #[error("task title is empty")]
    EmptyTitle,
    #[error("revision number must be positive")]
    InvalidRevisionNumber,
    #[error("revision one cannot have a parent")]
    RevisionOneHasParent,
    #[error("parent revision must belong to the same task and be earlier")]
    InvalidParent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Task {
    pub id: TaskId,
    pub project_id: ProjectId,
    pub title: String,
    pub latest_revision_number: u64,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl Task {
    pub fn new(
        id: TaskId,
        project_id: ProjectId,
        title: impl Into<String>,
        now: Timestamp,
    ) -> Result<Self, TaskValidationError> {
        let title = title
            .into()
            .replace("\r\n", "\n")
            .replace('\r', "\n")
            .trim()
            .to_string();
        if title.is_empty() {
            return Err(TaskValidationError::EmptyTitle);
        }
        Ok(Self {
            id,
            project_id,
            title,
            latest_revision_number: 1,
            created_at: now,
            updated_at: now,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TaskRevision {
    pub id: TaskRevisionId,
    pub task_id: TaskId,
    pub revision_number: u64,
    pub parent_revision_id: Option<TaskRevisionId>,
    pub goal_contract: GoalContractV1,
    pub goal_contract_fingerprint: String,
    pub created_at: Timestamp,
}

impl TaskRevision {
    pub fn new(
        id: TaskRevisionId,
        task_id: TaskId,
        revision_number: u64,
        parent_revision_id: Option<TaskRevisionId>,
        goal_contract: GoalContractV1,
        created_at: Timestamp,
    ) -> Result<Self, TaskValidationError> {
        if revision_number == 0 {
            return Err(TaskValidationError::InvalidRevisionNumber);
        }
        if revision_number == 1 && parent_revision_id.is_some() {
            return Err(TaskValidationError::RevisionOneHasParent);
        }
        let goal_contract_fingerprint = goal_contract.fingerprint();
        Ok(Self {
            id,
            task_id,
            revision_number,
            parent_revision_id,
            goal_contract,
            goal_contract_fingerprint,
            created_at,
        })
    }

    pub fn validate_parent(&self, parent: Option<&Self>) -> Result<(), TaskValidationError> {
        match (self.parent_revision_id, parent) {
            (None, None) if self.revision_number == 1 => Ok(()),
            (None, None) => Ok(()),
            (Some(expected), Some(parent))
                if expected == parent.id
                    && parent.task_id == self.task_id
                    && parent.revision_number < self.revision_number =>
            {
                Ok(())
            }
            _ => Err(TaskValidationError::InvalidParent),
        }
    }
}
