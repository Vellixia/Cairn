//! One error shape for the whole API (contracts/server-api.md).

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub code: &'static str,
    pub message: String,
}

impl ApiError {
    pub fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
        }
    }
    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "unauthorized", message)
    }
    /// A non-member gets a refusal, never an empty result (FR-057).
    pub fn forbidden(message: impl Into<String>) -> Self {
        Self::new(StatusCode::FORBIDDEN, "forbidden", message)
    }
    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, "not_found", message)
    }
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "invalid_request", message)
    }
    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", message)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = json!({ "error": { "code": self.code, "message": self.message } });
        (self.status, Json(body)).into_response()
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(e: sqlx::Error) -> Self {
        match e {
            sqlx::Error::RowNotFound => ApiError::not_found("no such record"),
            other => ApiError::internal(other.to_string()),
        }
    }
}

pub type ApiResult<T> = Result<T, ApiError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_constructor_maps_to_its_status_and_code() {
        assert_eq!(ApiError::unauthorized("x").status, StatusCode::UNAUTHORIZED);
        assert_eq!(ApiError::unauthorized("x").code, "unauthorized");
        assert_eq!(ApiError::forbidden("x").status, StatusCode::FORBIDDEN);
        assert_eq!(ApiError::not_found("x").status, StatusCode::NOT_FOUND);
        assert_eq!(ApiError::invalid("x").code, "invalid_request");
        assert_eq!(ApiError::internal("x").code, "internal");
    }

    #[test]
    fn row_not_found_becomes_not_found() {
        let e = sqlx::Error::RowNotFound;
        let api: ApiError = e.into();
        assert_eq!(api.status, StatusCode::NOT_FOUND);
        assert_eq!(api.code, "not_found");
    }

    #[test]
    fn other_sqlx_error_becomes_internal() {
        let e: sqlx::Error = sqlx::Error::Configuration(anyhow::anyhow!("bad cfg").into());
        let api: ApiError = e.into();
        assert_eq!(api.status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(api.code, "internal");
    }
}
