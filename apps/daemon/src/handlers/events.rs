//! T024: `v1.events.list` — stable seq ordering, bounded cursor pagination,
//! composable repository/worktree/session filters (analysis G1).

use cairn_domain::Timestamp;
use cairn_protocol::{ErrorCode, EventDto, EventsListParams, EventsListResult};
use cairn_storage_local::events as ev;

use super::{HandlerError, HandlerResult};
use crate::state::AppState;

const DEFAULT_LIMIT: u32 = 100;
const MAX_LIMIT: u32 = 1000;

pub async fn list(state: &AppState, params: EventsListParams) -> HandlerResult<EventsListResult> {
    if params.aggregate_type.is_some() != params.aggregate_id.is_some() {
        return Err(HandlerError::new(
            ErrorCode::Usage,
            "aggregate_type and aggregate_id must be supplied together",
        ));
    }
    let limit = params.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let rows = ev::list_events_filtered(
        state.pool(),
        params.repository_id.as_deref(),
        params.worktree_id.as_deref(),
        params.session_id.as_deref(),
        params.aggregate_type.map(|value| value.as_str()),
        params.aggregate_id.as_deref(),
        params.after_seq,
        limit,
    )
    .await?;

    let full_page = rows.len() as u32 == limit;
    let next_after_seq = if full_page {
        rows.last().map(|r| r.seq)
    } else {
        None
    };

    let events = rows
        .into_iter()
        .map(|r| {
            Ok(EventDto {
                seq: r.seq,
                id: r.id,
                event_type: r.event_type,
                repository_id: r.repository_id,
                worktree_id: r.worktree_id,
                session_id: r.session_id,
                snapshot_id: r.snapshot_id,
                aggregate_type: r
                    .aggregate_type
                    .as_deref()
                    .map(|value| cairn_protocol::EventAggregateType::parse(value).ok_or(()))
                    .transpose()
                    .map_err(|()| {
                        HandlerError::new(ErrorCode::StateCorrupted, "invalid event aggregate type")
                    })?,
                aggregate_id: r.aggregate_id,
                aggregate_seq: r.aggregate_seq,
                payload: serde_json::from_str(&r.payload)
                    .map_err(|e| HandlerError::new(ErrorCode::Internal, e.to_string()))?,
                recorded_at: Timestamp::parse(&r.recorded_at)
                    .map_err(|e| HandlerError::new(ErrorCode::Internal, e.to_string()))?,
            })
        })
        .collect::<HandlerResult<Vec<_>>>()?;

    Ok(EventsListResult {
        events,
        next_after_seq,
    })
}
