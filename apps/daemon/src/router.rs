//! Method router: parse params, dispatch, serialize. When local state is
//! corrupted every method (except daemon.status) answers STATE_CORRUPTED —
//! never fabricated state (FR-033).

use cairn_protocol::{methods, ErrorCode, Request, Response};
use serde::de::DeserializeOwned;

use crate::handlers::{self, HandlerError};
use crate::state::AppState;

fn parse<T: DeserializeOwned>(params: &serde_json::Value) -> Result<T, HandlerError> {
    serde_json::from_value(params.clone()).map_err(HandlerError::from)
}

pub async fn dispatch(state: &AppState, req: Request) -> Response {
    if let Some(reason) = &state.inner.corrupted {
        if req.method != methods::DAEMON_STATUS {
            return Response::err(&req.id, ErrorCode::StateCorrupted, reason.clone());
        }
    }

    let result: Result<serde_json::Value, HandlerError> = match req.method.as_str() {
        methods::DAEMON_STATUS => handlers::daemon::status(state)
            .await
            .map(|r| serde_json::to_value(r).expect("serializable")),
        methods::REPOSITORY_REGISTER => match parse(&req.params) {
            Ok(p) => handlers::repository::register(state, p)
                .await
                .map(|r| serde_json::to_value(r).expect("serializable")),
            Err(e) => Err(e),
        },
        methods::REPOSITORY_INSPECT => match parse(&req.params) {
            Ok(p) => handlers::repository::inspect(state, p)
                .await
                .map(|r| serde_json::to_value(r).expect("serializable")),
            Err(e) => Err(e),
        },
        methods::REPOSITORY_IGNORED_FILES => match parse(&req.params) {
            Ok(p) => handlers::repository::ignored_files(state, p)
                .await
                .map(|r| serde_json::to_value(r).expect("serializable")),
            Err(e) => Err(e),
        },
        methods::SNAPSHOT_CREATE => match parse(&req.params) {
            Ok(p) => handlers::snapshot::create(state, p)
                .await
                .map(|r| serde_json::to_value(r).expect("serializable")),
            Err(e) => Err(e),
        },
        methods::SESSION_START => match parse(&req.params) {
            Ok(p) => handlers::session::start(state, p)
                .await
                .map(|r| serde_json::to_value(r).expect("serializable")),
            Err(e) => Err(e),
        },
        methods::SESSION_GET => match parse(&req.params) {
            Ok(p) => handlers::session::get(state, p)
                .await
                .map(|r| serde_json::to_value(r).expect("serializable")),
            Err(e) => Err(e),
        },
        methods::SESSION_LIST => match parse(&req.params) {
            Ok(p) => handlers::session::list(state, p)
                .await
                .map(|r| serde_json::to_value(r).expect("serializable")),
            Err(e) => Err(e),
        },
        methods::SESSION_HEARTBEAT => match parse(&req.params) {
            Ok(p) => handlers::session::heartbeat(state, p)
                .await
                .map(|r| serde_json::to_value(r).expect("serializable")),
            Err(e) => Err(e),
        },
        methods::SESSION_REATTACH => match parse(&req.params) {
            Ok(p) => handlers::session::reattach(state, p)
                .await
                .map(|r| serde_json::to_value(r).expect("serializable")),
            Err(e) => Err(e),
        },
        methods::SESSION_STOP => match parse(&req.params) {
            Ok(p) => handlers::session::stop(state, p)
                .await
                .map(|r| serde_json::to_value(r).expect("serializable")),
            Err(e) => Err(e),
        },
        methods::EVENTS_LIST => match parse(&req.params) {
            Ok(p) => handlers::events::list(state, p)
                .await
                .map(|r| serde_json::to_value(r).expect("serializable")),
            Err(e) => Err(e),
        },
        other => Err(HandlerError::new(
            ErrorCode::Usage,
            format!("unknown method: {other}"),
        )),
    };

    match result {
        Ok(value) => Response {
            id: req.id,
            result: Some(value),
            error: None,
        },
        Err(e) => Response {
            id: req.id,
            result: None,
            error: Some(cairn_protocol::ErrorBody {
                code: e.code,
                message: e.message,
                data: e.data,
            }),
        },
    }
}
