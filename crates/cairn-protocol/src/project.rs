use cairn_domain::{
    Project, ProjectId, ProjectRepositoryAssociation, ProjectRepositoryAssociationId,
    ProjectStatus, Timestamp,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProjectCreateParams {
    pub idempotency_key: cairn_domain::IdempotencyKey,
    pub name: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProjectCreateResult {
    pub project: ProjectDto,
    pub created: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProjectListParams {
    pub status: Option<ProjectStatus>,
    pub after_project_id: Option<ProjectId>,
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProjectListResult {
    pub projects: Vec<ProjectDto>,
    pub next_after_project_id: Option<ProjectId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProjectGetParams {
    pub project_id: ProjectId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProjectGetResult {
    pub project: ProjectDto,
    pub repository_associations: Vec<ProjectRepositoryAssociationDto>,
    pub task_count: u64,
    pub bound_session_count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProjectUpdateParams {
    pub idempotency_key: cairn_domain::IdempotencyKey,
    pub project_id: ProjectId,
    pub name: Option<String>,
    pub description: Option<String>,
    #[serde(default)]
    pub clear_description: bool,
    pub status: Option<ProjectStatus>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProjectUpdateResult {
    pub project: ProjectDto,
    pub updated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProjectRepositoryAssociateParams {
    pub idempotency_key: cairn_domain::IdempotencyKey,
    pub project_id: ProjectId,
    pub repository_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProjectRepositoryAssociateResult {
    pub association: ProjectRepositoryAssociationDto,
    pub created: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ProjectDto {
    pub project_id: ProjectId,
    pub name: String,
    pub description: Option<String>,
    pub status: ProjectStatus,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl From<Project> for ProjectDto {
    fn from(project: Project) -> Self {
        Self {
            project_id: project.id,
            name: project.name,
            description: project.description,
            status: project.status,
            created_at: project.created_at,
            updated_at: project.updated_at,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ProjectRepositoryAssociationDto {
    pub association_id: ProjectRepositoryAssociationId,
    pub project_id: ProjectId,
    pub repository_id: String,
    pub associated_at: Timestamp,
}

impl From<ProjectRepositoryAssociation> for ProjectRepositoryAssociationDto {
    fn from(association: ProjectRepositoryAssociation) -> Self {
        Self {
            association_id: association.id,
            project_id: association.project_id,
            repository_id: association.repository_id,
            associated_at: association.associated_at,
        }
    }
}
