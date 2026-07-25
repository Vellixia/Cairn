use std::io::Read;
use std::str::FromStr;

use cairn_protocol::{
    methods, AmbiguousEntity, CandidateIds, ErrorBody, ErrorCode, ErrorData, GoalContractV1,
    GoalContractViolations, IdempotencyKey, ProjectId, TaskCreateParams, TaskGetParams, TaskId,
    TaskListParams, TaskListResult, TaskReviseParams, TaskRevisionId,
};

use crate::{ipc, output, TaskCommand};

pub async fn run(json: bool, command: TaskCommand) -> i32 {
    match command {
        TaskCommand::Create {
            project_id,
            title,
            goal_contract,
            idempotency_key,
        } => {
            let params = match build_create(project_id, title, goal_contract, idempotency_key) {
                Ok(params) => params,
                Err(error) => return output::emit("task.create", json, Err(error)),
            };
            output::emit(
                "task.create",
                json,
                ipc::call(methods::TASK_CREATE, &params).await,
            )
        }
        TaskCommand::Revise {
            task_id,
            task,
            project_id,
            parent_revision_id,
            goal_contract,
            idempotency_key,
        } => {
            let task_id = match resolve_task_id(json, task_id, task, project_id).await {
                Ok(id) => id,
                Err(error) => return output::emit("task.revise", json, Err(error)),
            };
            let goal_contract = match read_contract(&goal_contract) {
                Ok(contract) => contract,
                Err(error) => return output::emit("task.revise", json, Err(error)),
            };
            let idempotency_key = match parse_or_generate_key(idempotency_key) {
                Ok(key) => key,
                Err(error) => return output::emit("task.revise", json, Err(error)),
            };
            let parent_revision_id = match parent_revision_id
                .as_deref()
                .map(parse_revision_id)
                .transpose()
            {
                Ok(value) => value,
                Err(error) => return output::emit("task.revise", json, Err(error)),
            };
            let params = TaskReviseParams {
                idempotency_key,
                task_id,
                parent_revision_id,
                goal_contract,
            };
            output::emit(
                "task.revise",
                json,
                ipc::call(methods::TASK_REVISE, &params).await,
            )
        }
        TaskCommand::List {
            project_id,
            after_task_id,
            limit,
        } => {
            let project_id = match parse_project_id(&project_id) {
                Ok(id) => id,
                Err(error) => return output::emit("task.list", json, Err(error)),
            };
            let after_task_id = match after_task_id.as_deref().map(parse_task_id).transpose() {
                Ok(id) => id,
                Err(error) => return output::emit("task.list", json, Err(error)),
            };
            output::emit(
                "task.list",
                json,
                ipc::call(
                    methods::TASK_LIST,
                    &TaskListParams {
                        project_id,
                        after_task_id,
                        limit,
                    },
                )
                .await,
            )
        }
        TaskCommand::Show {
            task_id,
            task,
            project_id,
            revision_id,
        } => {
            let task_id = match resolve_task_id(json, task_id, task, project_id).await {
                Ok(id) => id,
                Err(error) => return output::emit("task.show", json, Err(error)),
            };
            let revision_id = match revision_id.as_deref().map(parse_revision_id).transpose() {
                Ok(id) => id,
                Err(error) => return output::emit("task.show", json, Err(error)),
            };
            output::emit(
                "task.show",
                json,
                ipc::call(
                    methods::TASK_GET,
                    &TaskGetParams {
                        task_id,
                        revision_id,
                    },
                )
                .await,
            )
        }
    }
}

fn build_create(
    project_id: String,
    title: String,
    goal_contract: String,
    idempotency_key: Option<String>,
) -> Result<TaskCreateParams, ErrorBody> {
    Ok(TaskCreateParams {
        idempotency_key: parse_or_generate_key(idempotency_key)?,
        project_id: parse_project_id(&project_id)?,
        title,
        goal_contract: read_contract(&goal_contract)?,
    })
}

async fn resolve_task_id(
    json: bool,
    task_id: Option<String>,
    task_title: Option<String>,
    project_id: Option<String>,
) -> Result<TaskId, ErrorBody> {
    if json && task_title.is_some() {
        return Err(usage(
            "JSON mode requires --task-id and does not accept --task",
        ));
    }
    if let Some(task_id) = task_id {
        return parse_task_id(&task_id);
    }
    let task_title = task_title.ok_or_else(|| usage("--task-id is required"))?;
    let project_id = project_id
        .as_deref()
        .ok_or_else(|| usage("--project-id is required with --task"))
        .and_then(parse_project_id)?;
    let normalized = normalize_display(&task_title);
    let mut matches = Vec::new();
    let mut after_task_id = None;
    loop {
        let value = ipc::call(
            methods::TASK_LIST,
            &TaskListParams {
                project_id,
                after_task_id,
                limit: Some(100),
            },
        )
        .await?;
        let page: TaskListResult = serde_json::from_value(value)
            .map_err(|_| internal("daemon returned an invalid task list"))?;
        matches.extend(
            page.tasks
                .into_iter()
                .filter(|task| task.title == normalized)
                .map(|task| task.task_id),
        );
        after_task_id = page.next_after_task_id;
        if after_task_id.is_none() {
            break;
        }
    }
    matches.sort_unstable();
    matches.dedup();
    match matches.len() {
        0 => Err(ErrorBody::new(
            ErrorCode::TaskNotFound,
            "task title did not match a task in the selected project",
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
                "task title is ambiguous",
                ErrorData::AmbiguousName {
                    entity: AmbiguousEntity::Task,
                    candidate_ids: CandidateIds::new(candidates)
                        .expect("candidate list is bounded"),
                    truncated,
                },
            ))
        }
    }
}

fn read_contract(source: &str) -> Result<GoalContractV1, ErrorBody> {
    let bytes = if source == "-" {
        let mut bytes = Vec::new();
        std::io::stdin()
            .read_to_end(&mut bytes)
            .map_err(|_| usage("could not read goal contract from stdin"))?;
        bytes
    } else {
        std::fs::read(source).map_err(|_| usage("could not read goal contract file"))?
    };
    GoalContractV1::from_json_slice(&bytes).map_err(|error| {
        let violations = GoalContractViolations::new(error.violations().to_vec())
            .expect("domain violation bounds match protocol");
        ErrorBody::with_data(
            ErrorCode::InvalidGoalContract,
            "invalid goal contract",
            ErrorData::InvalidGoalContract { violations },
        )
    })
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

fn parse_task_id(value: &str) -> Result<TaskId, ErrorBody> {
    TaskId::from_str(value).map_err(|_| usage("invalid task id"))
}

fn parse_revision_id(value: &str) -> Result<TaskRevisionId, ErrorBody> {
    TaskRevisionId::from_str(value).map_err(|_| usage("invalid task revision id"))
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
