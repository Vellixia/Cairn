use std::str::FromStr;

use cairn_protocol::{
    methods, AmbiguousEntity, CandidateIds, ErrorBody, ErrorCode, ErrorData, IdempotencyKey,
    ProjectCreateParams, ProjectGetParams, ProjectId, ProjectListParams, ProjectListResult,
    ProjectRepositoryAssociateParams, ProjectStatus, ProjectUpdateParams,
};

use crate::{ipc, output, ProjectCommand, ProjectRepositoryCommand};

pub async fn run(json: bool, command: ProjectCommand) -> i32 {
    match command {
        ProjectCommand::Create {
            name,
            description,
            idempotency_key,
        } => {
            let idempotency_key = match parse_or_generate_key(idempotency_key) {
                Ok(key) => key,
                Err(error) => return output::emit("project.create", json, Err(error)),
            };
            let params = ProjectCreateParams {
                idempotency_key,
                name,
                description,
            };
            output::emit(
                "project.create",
                json,
                ipc::call(methods::PROJECT_CREATE, &params).await,
            )
        }
        ProjectCommand::List {
            status,
            after_project_id,
            limit,
        } => {
            let status = status.as_deref().map(parse_status).transpose();
            let after_project_id = after_project_id
                .as_deref()
                .map(parse_project_id)
                .transpose();
            let params = match status.and_then(|status| {
                after_project_id.map(|after_project_id| ProjectListParams {
                    status,
                    after_project_id,
                    limit,
                })
            }) {
                Ok(params) => params,
                Err(error) => return output::emit("project.list", json, Err(error)),
            };
            output::emit(
                "project.list",
                json,
                ipc::call(methods::PROJECT_LIST, &params).await,
            )
        }
        ProjectCommand::Show {
            project_id,
            project,
        } => {
            let project_id = match resolve_project_id(json, project_id, project).await {
                Ok(id) => id,
                Err(error) => return output::emit("project.show", json, Err(error)),
            };
            output::emit(
                "project.show",
                json,
                ipc::call(methods::PROJECT_GET, &ProjectGetParams { project_id }).await,
            )
        }
        ProjectCommand::Update {
            project_id,
            project,
            name,
            description,
            clear_description,
            status,
            idempotency_key,
        } => {
            let project_id = match resolve_project_id(json, project_id, project).await {
                Ok(id) => id,
                Err(error) => return output::emit("project.update", json, Err(error)),
            };
            let idempotency_key = match parse_or_generate_key(idempotency_key) {
                Ok(key) => key,
                Err(error) => return output::emit("project.update", json, Err(error)),
            };
            let status = match status.as_deref().map(parse_status).transpose() {
                Ok(status) => status,
                Err(error) => return output::emit("project.update", json, Err(error)),
            };
            let params = ProjectUpdateParams {
                idempotency_key,
                project_id,
                name,
                description,
                clear_description,
                status,
            };
            output::emit(
                "project.update",
                json,
                ipc::call(methods::PROJECT_UPDATE, &params).await,
            )
        }
        ProjectCommand::Repository(ProjectRepositoryCommand::Add {
            project_id,
            project,
            repository_id,
            idempotency_key,
        }) => {
            let project_id = match resolve_project_id(json, project_id, project).await {
                Ok(id) => id,
                Err(error) => {
                    return output::emit("project.repository.add", json, Err(error));
                }
            };
            let idempotency_key = match parse_or_generate_key(idempotency_key) {
                Ok(key) => key,
                Err(error) => {
                    return output::emit("project.repository.add", json, Err(error));
                }
            };
            let params = ProjectRepositoryAssociateParams {
                idempotency_key,
                project_id,
                repository_id,
            };
            output::emit(
                "project.repository.add",
                json,
                ipc::call(methods::PROJECT_REPOSITORY_ASSOCIATE, &params).await,
            )
        }
    }
}

async fn resolve_project_id(
    json: bool,
    project_id: Option<String>,
    project_name: Option<String>,
) -> Result<ProjectId, ErrorBody> {
    if json && project_name.is_some() {
        return Err(usage(
            "JSON mode requires --project-id and does not accept --project",
        ));
    }
    if let Some(project_id) = project_id {
        return parse_project_id(&project_id);
    }
    let project_name = project_name.ok_or_else(|| usage("--project-id is required"))?;
    let normalized = normalize_display(&project_name);
    let mut matches = Vec::new();
    let mut after_project_id = None;
    loop {
        let params = ProjectListParams {
            status: None,
            after_project_id,
            limit: Some(100),
        };
        let value = ipc::call(methods::PROJECT_LIST, &params).await?;
        let page: ProjectListResult = serde_json::from_value(value)
            .map_err(|_| internal("daemon returned an invalid project list"))?;
        matches.extend(
            page.projects
                .into_iter()
                .filter(|project| project.name == normalized)
                .map(|project| project.project_id),
        );
        after_project_id = page.next_after_project_id;
        if after_project_id.is_none() {
            break;
        }
    }
    matches.sort_unstable();
    matches.dedup();
    match matches.len() {
        0 => Err(ErrorBody::new(
            ErrorCode::ProjectNotFound,
            "project name did not match a local project",
        )),
        1 => Ok(matches[0]),
        _ => {
            let truncated = matches.len() > CandidateIds::MAX;
            let candidates = matches
                .into_iter()
                .take(CandidateIds::MAX)
                .map(|id| id.to_string())
                .collect();
            Err(ErrorBody::with_data(
                ErrorCode::AmbiguousName,
                "project name is ambiguous",
                ErrorData::AmbiguousName {
                    entity: AmbiguousEntity::Project,
                    candidate_ids: CandidateIds::new(candidates)
                        .expect("candidate list is bounded"),
                    truncated,
                },
            ))
        }
    }
}

fn parse_or_generate_key(value: Option<String>) -> Result<IdempotencyKey, ErrorBody> {
    value.map_or_else(
        || Ok(IdempotencyKey::new_v7()),
        |value| IdempotencyKey::from_str(&value).map_err(|_| usage("invalid idempotency key")),
    )
}

fn parse_project_id(value: &str) -> Result<ProjectId, ErrorBody> {
    ProjectId::from_str(value).map_err(|_| usage("invalid project id"))
}

fn parse_status(value: &str) -> Result<ProjectStatus, ErrorBody> {
    match value {
        "active" => Ok(ProjectStatus::Active),
        "archived" => Ok(ProjectStatus::Archived),
        _ => Err(usage("project status must be active or archived")),
    }
}

fn normalize_display(value: &str) -> String {
    value
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .trim()
        .to_string()
}

fn usage(message: impl Into<String>) -> ErrorBody {
    ErrorBody::new(ErrorCode::Usage, message)
}

fn internal(message: impl Into<String>) -> ErrorBody {
    ErrorBody::new(ErrorCode::Internal, message)
}
