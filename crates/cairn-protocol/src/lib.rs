//! Wire contracts shared by daemon, CLI, and future adapters.
//!
//! Single source of truth for IPC method names, DTOs, error codes, and the
//! CLI JSON envelope. Every type derives `JsonSchema`; contract tests golden-
//! diff the exported schemas as a breaking-change tripwire (plan T010).

pub mod dto;
pub mod errors;
pub mod methods;

pub use dto::*;
pub use errors::*;
pub use methods::*;

// Re-export the identity types used in DTOs so thin clients (CLI, adapters)
// need not depend on cairn-domain directly.
pub use cairn_domain::{AgentInstanceId, SessionId, SessionState, Timestamp, WatcherStartStage};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// IPC protocol version namespace.
pub const PROTOCOL_VERSION: &str = "v1";
/// CLI machine-readable envelope schema marker (additive-only within v1).
pub const CLI_SCHEMA: &str = "cairn.cli.v1";

/// One request line on the IPC channel (JSON-lines framing).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Request {
    pub id: String,
    pub method: String,
    #[serde(default)]
    pub params: serde_json::Value,
}

/// One response line on the IPC channel. Exactly one of result/error is set.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Response {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorBody>,
}

impl Response {
    pub fn ok<T: Serialize>(id: &str, result: &T) -> Self {
        Self {
            id: id.to_string(),
            result: Some(serde_json::to_value(result).expect("serializable result")),
            error: None,
        }
    }

    pub fn err(id: &str, code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            id: id.to_string(),
            result: None,
            error: Some(ErrorBody::new(code, message)),
        }
    }
}

/// Typed data carried by stable protocol errors. Watcher readiness failures
/// deliberately expose only their discriminant and stable stage (FR-038).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[serde(deny_unknown_fields)]
pub enum ErrorData {
    WatcherStartFailure { stage: WatcherStartStage },
}

impl ErrorData {
    pub const fn watcher_start_failure(stage: WatcherStartStage) -> Self {
        Self::WatcherStartFailure { stage }
    }

    pub const fn watcher_stage(self) -> WatcherStartStage {
        match self {
            Self::WatcherStartFailure { stage } => stage,
        }
    }
}

/// Structured error payload. Its custom schema requires typed watcher data
/// whenever `code` is `WATCHER_START_FAILED`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ErrorBody {
    pub code: ErrorCode,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<ErrorData>,
}

impl ErrorBody {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }

    pub fn watcher_start_failed(stage: WatcherStartStage) -> Self {
        Self {
            code: ErrorCode::WatcherStartFailed,
            message: "session watcher readiness failed".into(),
            data: Some(ErrorData::watcher_start_failure(stage)),
        }
    }
}

#[derive(JsonSchema)]
#[allow(dead_code)]
struct ErrorBodySchema {
    code: ErrorCode,
    message: String,
    data: Option<ErrorData>,
}

impl JsonSchema for ErrorBody {
    fn schema_name() -> String {
        "ErrorBody".into()
    }

    fn json_schema(generator: &mut schemars::gen::SchemaGenerator) -> schemars::schema::Schema {
        use std::collections::BTreeSet;

        use schemars::schema::{
            InstanceType, ObjectValidation, Schema, SchemaObject, SubschemaValidation,
        };

        let mut schema = ErrorBodySchema::json_schema(generator).into_object();
        schema.metadata().title = Some(Self::schema_name());

        let code_match = SchemaObject {
            enum_values: Some(vec![serde_json::json!("WATCHER_START_FAILED")]),
            ..Default::default()
        };
        let mut if_object = ObjectValidation {
            required: BTreeSet::from(["code".to_string()]),
            ..Default::default()
        };
        if_object
            .properties
            .insert("code".into(), Schema::Object(code_match));
        let if_schema = Schema::Object(SchemaObject {
            instance_type: Some(InstanceType::Object.into()),
            object: Some(Box::new(if_object)),
            ..Default::default()
        });

        let mut then_object = ObjectValidation {
            required: BTreeSet::from(["data".to_string()]),
            ..Default::default()
        };
        then_object
            .properties
            .insert("data".into(), generator.subschema_for::<ErrorData>());
        let then_schema = Schema::Object(SchemaObject {
            instance_type: Some(InstanceType::Object.into()),
            object: Some(Box::new(then_object)),
            ..Default::default()
        });

        schema.subschemas = Some(Box::new(SubschemaValidation {
            if_schema: Some(Box::new(if_schema)),
            then_schema: Some(Box::new(then_schema)),
            ..Default::default()
        }));
        Schema::Object(schema)
    }
}

/// CLI machine-readable envelope (contracts/cli-json-contract.md).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CliEnvelope {
    pub schema: String,
    pub ok: bool,
    pub command: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorBody>,
}

impl CliEnvelope {
    pub fn ok<T: Serialize>(command: &str, data: &T) -> Self {
        Self {
            schema: CLI_SCHEMA.into(),
            ok: true,
            command: command.into(),
            data: Some(serde_json::to_value(data).expect("serializable data")),
            error: None,
        }
    }

    pub fn err(command: &str, error: ErrorBody) -> Self {
        Self {
            schema: CLI_SCHEMA.into(),
            ok: false,
            command: command.into(),
            data: None,
            error: Some(error),
        }
    }
}
