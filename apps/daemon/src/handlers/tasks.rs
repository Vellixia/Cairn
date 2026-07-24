use cairn_domain::GoalContractV1;
use cairn_project::{CreateTask, ReviseTask};
use cairn_protocol::{
    ErrorCode, ErrorData, GoalContractViolations, TaskCreateParams, TaskCreateResult, TaskDto,
    TaskGetParams, TaskGetResult, TaskListParams, TaskListResult, TaskReviseParams,
    TaskReviseResult, TaskRevisionDto, TaskSummaryDto,
};
use serde::Deserialize;

use super::{HandlerError, HandlerResult};
use crate::state::AppState;

#[derive(Deserialize)]
struct CreateHead {
    idempotency_key: cairn_domain::IdempotencyKey,
    project_id: cairn_domain::ProjectId,
    title: String,
}

#[derive(Deserialize)]
struct ReviseHead {
    idempotency_key: cairn_domain::IdempotencyKey,
    task_id: cairn_domain::TaskId,
    parent_revision_id: Option<cairn_domain::TaskRevisionId>,
}

pub fn parse_create(value: &serde_json::Value) -> HandlerResult<TaskCreateParams> {
    let head: CreateHead = serde_json::from_value(value.clone()).map_err(HandlerError::from)?;
    let goal_contract = parse_contract(value)?;
    Ok(TaskCreateParams {
        idempotency_key: head.idempotency_key,
        project_id: head.project_id,
        title: head.title,
        goal_contract,
    })
}

pub fn parse_revise(value: &serde_json::Value) -> HandlerResult<TaskReviseParams> {
    let head: ReviseHead = serde_json::from_value(value.clone()).map_err(HandlerError::from)?;
    let goal_contract = parse_contract(value)?;
    Ok(TaskReviseParams {
        idempotency_key: head.idempotency_key,
        task_id: head.task_id,
        parent_revision_id: head.parent_revision_id,
        goal_contract,
    })
}

fn parse_contract(value: &serde_json::Value) -> HandlerResult<GoalContractV1> {
    let contract = value
        .as_object()
        .and_then(|object| object.get("goal_contract"))
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    GoalContractV1::from_value(contract).map_err(|error| {
        let violations = GoalContractViolations::new(error.violations().to_vec())
            .expect("domain violation bounds match protocol");
        HandlerError::new(ErrorCode::InvalidGoalContract, "invalid goal contract")
            .with_data(ErrorData::InvalidGoalContract { violations })
    })
}

pub async fn create(state: &AppState, params: TaskCreateParams) -> HandlerResult<TaskCreateResult> {
    let result = state
        .inner
        .tasks
        .create(CreateTask {
            idempotency_key: params.idempotency_key,
            project_id: params.project_id,
            title: params.title,
            goal_contract: params.goal_contract,
        })
        .await
        .map_err(HandlerError::from)?;
    tracing::info!(
        task_id = %result.task.id,
        revision_id = %result.revision.id,
        revision_number = result.revision.revision_number,
        goal_contract_version = result.revision.goal_contract.schema_version(),
        goal_contract_fingerprint = %result.revision.goal_contract_fingerprint,
        created = result.created,
        "task create"
    );
    Ok(TaskCreateResult {
        task: result.task.into(),
        revision: result.revision.into(),
        created: result.created,
    })
}

pub async fn revise(state: &AppState, params: TaskReviseParams) -> HandlerResult<TaskReviseResult> {
    let result = state
        .inner
        .tasks
        .revise(ReviseTask {
            idempotency_key: params.idempotency_key,
            task_id: params.task_id,
            parent_revision_id: params.parent_revision_id,
            goal_contract: params.goal_contract,
        })
        .await
        .map_err(HandlerError::from)?;
    tracing::info!(
        task_id = %result.task.id,
        revision_id = %result.revision.id,
        revision_number = result.revision.revision_number,
        goal_contract_version = result.revision.goal_contract.schema_version(),
        goal_contract_fingerprint = %result.revision.goal_contract_fingerprint,
        created = result.created,
        "task revise"
    );
    Ok(TaskReviseResult {
        task: result.task.into(),
        revision: result.revision.into(),
        created: result.created,
    })
}

pub async fn list(state: &AppState, params: TaskListParams) -> HandlerResult<TaskListResult> {
    let limit = params.limit.unwrap_or(50);
    if !(1..=100).contains(&limit) {
        return Err(HandlerError::new(
            ErrorCode::Usage,
            "task list limit must be between 1 and 100",
        ));
    }
    let page = state
        .inner
        .tasks
        .list(params.project_id, params.after_task_id, limit)
        .await
        .map_err(HandlerError::from)?;
    Ok(TaskListResult {
        tasks: page
            .tasks
            .into_iter()
            .map(|entry| TaskSummaryDto {
                task_id: entry.task.id,
                project_id: entry.task.project_id,
                title: entry.task.title,
                latest_revision_number: entry.task.latest_revision_number,
                latest_revision_fingerprint: entry.latest_revision_fingerprint,
                created_at: entry.task.created_at,
                updated_at: entry.task.updated_at,
            })
            .collect(),
        next_after_task_id: page.next_after_task_id,
    })
}

pub async fn get(state: &AppState, params: TaskGetParams) -> HandlerResult<TaskGetResult> {
    let detail = state
        .inner
        .tasks
        .get(params.task_id, params.revision_id)
        .await
        .map_err(HandlerError::from)?;
    Ok(TaskGetResult {
        task: TaskDto::from(detail.task),
        revision: TaskRevisionDto::from(detail.revision),
    })
}
