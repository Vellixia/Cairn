use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{ProjectId, ProjectRepositoryAssociationId, Timestamp};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProjectStatus {
    Active,
    Archived,
}

impl ProjectStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Archived => "archived",
        }
    }

    pub const fn accepts_mutations(self) -> bool {
        matches!(self, Self::Active)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ProjectValidationError {
    #[error("project name is empty")]
    EmptyName,
    #[error("repository identity is empty")]
    EmptyRepositoryId,
    #[error("association event sequence must be positive")]
    InvalidEventSequence,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Project {
    pub id: ProjectId,
    pub name: String,
    pub description: Option<String>,
    pub status: ProjectStatus,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl Project {
    pub fn new(
        id: ProjectId,
        name: impl Into<String>,
        description: Option<String>,
        now: Timestamp,
    ) -> Result<Self, ProjectValidationError> {
        let name = normalize_display(name.into());
        if name.is_empty() {
            return Err(ProjectValidationError::EmptyName);
        }
        let description = description.map(normalize_display);
        Ok(Self {
            id,
            name,
            description,
            status: ProjectStatus::Active,
            created_at: now,
            updated_at: now,
        })
    }

    pub fn set_status(&mut self, status: ProjectStatus, now: Timestamp) {
        self.status = status;
        self.updated_at = now;
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ProjectRepositoryAssociation {
    pub id: ProjectRepositoryAssociationId,
    pub project_id: ProjectId,
    pub repository_id: String,
    pub associated_at: Timestamp,
    pub event_seq: i64,
}

impl ProjectRepositoryAssociation {
    pub fn new(
        id: ProjectRepositoryAssociationId,
        project_id: ProjectId,
        repository_id: impl Into<String>,
        associated_at: Timestamp,
        event_seq: i64,
    ) -> Result<Self, ProjectValidationError> {
        let repository_id = repository_id.into();
        if repository_id.trim().is_empty() {
            return Err(ProjectValidationError::EmptyRepositoryId);
        }
        if event_seq <= 0 {
            return Err(ProjectValidationError::InvalidEventSequence);
        }
        Ok(Self {
            id,
            project_id,
            repository_id,
            associated_at,
            event_seq,
        })
    }
}

fn normalize_display(value: String) -> String {
    value
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .trim()
        .to_string()
}
