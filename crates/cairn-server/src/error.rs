//! One error shape for the whole API (contracts/server-api.md).

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};

#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub code: &'static str,
    pub message: String,
    /// Fields the caller must be able to *act* on, merged into the error object
    /// beside `code` and `message`.
    ///
    /// A refusal whose remedy depends on a specific value — which state a team
    /// entry is actually in, for instance — puts that value here rather than
    /// only in the sentence. A client that has to regex a message to decide what
    /// to do next is a client that breaks when the sentence is reworded, and the
    /// sentence exists for a person to read.
    ///
    /// Never overrides `code` or `message`: those two are the shape every
    /// existing consumer already depends on.
    pub detail: Option<Value>,
}

impl ApiError {
    pub fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
            detail: None,
        }
    }
    /// Attach the fields above. An object; anything else is ignored, because
    /// there is no key to merge a bare value under.
    pub fn with_detail(mut self, detail: Value) -> Self {
        self.detail = Some(detail);
        self
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
        (
            self.status,
            Json(error_body(self.code, &self.message, self.detail)),
        )
            .into_response()
    }
}

/// The one error body shape, built where a test can reach it without a running
/// server.
fn error_body(code: &'static str, message: &str, detail: Option<Value>) -> Value {
    let mut error = serde_json::Map::new();
    error.insert("code".to_string(), json!(code));
    error.insert("message".to_string(), json!(message));
    if let Some(Value::Object(detail)) = detail {
        for (key, value) in detail {
            // `code` and `message` are what every existing consumer matches on.
            // A detail field is additional information, never a way to restate
            // the two facts the shape guarantees.
            if !error.contains_key(&key) {
                error.insert(key, value);
            }
        }
    }
    json!({ "error": error })
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

    /// A detail field rides beside `code` and `message`, and neither of those
    /// can be displaced by one — a refusal that could rename its own `code`
    /// would be a refusal no client could match on.
    #[test]
    fn a_detail_field_joins_the_error_object_without_displacing_it() {
        let body = error_body(
            "state_conflict",
            "team knowledge is at state retired, not the state this request required",
            Some(json!({ "state": "retired", "code": "hijacked" })),
        );
        assert_eq!(body["error"]["code"], "state_conflict");
        assert_eq!(body["error"]["state"], "retired");
        assert!(body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("at state retired"));
    }

    /// An error with nothing attached keeps the two-field shape Feature 001's
    /// clients were written against.
    #[test]
    fn an_error_without_detail_carries_exactly_code_and_message() {
        let body = error_body("not_found", "no such record", None);
        assert_eq!(
            body,
            json!({ "error": { "code": "not_found", "message": "no such record" } })
        );
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
