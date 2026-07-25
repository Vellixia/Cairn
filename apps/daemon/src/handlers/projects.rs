use cairn_project::{AssociateRepository, CreateProject, UpdateProject};
use cairn_protocol::{
    ProjectCreateParams, ProjectCreateResult, ProjectDto, ProjectGetParams, ProjectGetResult,
    ProjectListParams, ProjectListResult, ProjectRepositoryAssociateParams,
    ProjectRepositoryAssociateResult, ProjectRepositoryAssociationDto, ProjectUpdateParams,
    ProjectUpdateResult,
};

use super::{HandlerError, HandlerResult};
use crate::state::AppState;

pub async fn create(
    state: &AppState,
    params: ProjectCreateParams,
) -> HandlerResult<ProjectCreateResult> {
    let result = state
        .inner
        .projects
        .create(CreateProject {
            idempotency_key: params.idempotency_key,
            name: params.name,
            description: params.description,
        })
        .await
        .map_err(HandlerError::from)?;
    tracing::info!(project_id = %result.project.id, created = result.created, "project create");
    Ok(ProjectCreateResult {
        project: result.project.into(),
        created: result.created,
    })
}

pub async fn list(state: &AppState, params: ProjectListParams) -> HandlerResult<ProjectListResult> {
    let limit = params.limit.unwrap_or(50);
    if !(1..=100).contains(&limit) {
        return Err(HandlerError::new(
            cairn_protocol::ErrorCode::Usage,
            "project list limit must be between 1 and 100",
        ));
    }
    let page = state
        .inner
        .projects
        .list(params.status, params.after_project_id, limit)
        .await
        .map_err(HandlerError::from)?;
    Ok(ProjectListResult {
        projects: page.projects.into_iter().map(ProjectDto::from).collect(),
        next_after_project_id: page.next_after_project_id,
    })
}

pub async fn get(state: &AppState, params: ProjectGetParams) -> HandlerResult<ProjectGetResult> {
    let detail = state
        .inner
        .projects
        .get(params.project_id)
        .await
        .map_err(HandlerError::from)?;
    Ok(ProjectGetResult {
        project: detail.project.into(),
        repository_associations: detail
            .associations
            .into_iter()
            .map(ProjectRepositoryAssociationDto::from)
            .collect(),
        task_count: detail.task_count,
        bound_session_count: detail.bound_session_count,
    })
}

pub async fn update(
    state: &AppState,
    params: ProjectUpdateParams,
) -> HandlerResult<ProjectUpdateResult> {
    let result = state
        .inner
        .projects
        .update(UpdateProject {
            idempotency_key: params.idempotency_key,
            project_id: params.project_id,
            name: params.name,
            description: params.description,
            clear_description: params.clear_description,
            status: params.status,
        })
        .await
        .map_err(HandlerError::from)?;
    tracing::info!(project_id = %result.project.id, updated = result.updated, "project update");
    Ok(ProjectUpdateResult {
        project: result.project.into(),
        updated: result.updated,
    })
}

pub async fn associate_repository(
    state: &AppState,
    params: ProjectRepositoryAssociateParams,
) -> HandlerResult<ProjectRepositoryAssociateResult> {
    let result = state
        .inner
        .projects
        .associate_repository(AssociateRepository {
            idempotency_key: params.idempotency_key,
            project_id: params.project_id,
            repository_id: params.repository_id,
        })
        .await
        .map_err(HandlerError::from)?;
    tracing::info!(
        project_id = %result.association.project_id,
        repository_id = %result.association.repository_id,
        created = result.created,
        "project repository associate"
    );
    Ok(ProjectRepositoryAssociateResult {
        association: result.association.into(),
        created: result.created,
    })
}
