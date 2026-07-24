use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AggregateType {
    Repository,
    Worktree,
    Session,
    Project,
    Task,
}

impl AggregateType {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Repository => "repository",
            Self::Worktree => "worktree",
            Self::Session => "session",
            Self::Project => "project",
            Self::Task => "task",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "repository" => Self::Repository,
            "worktree" => Self::Worktree,
            "session" => Self::Session,
            "project" => Self::Project,
            "task" => Self::Task,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub struct AggregateIdentity {
    pub aggregate_type: AggregateType,
    pub aggregate_id: String,
}

impl AggregateIdentity {
    pub fn new(
        aggregate_type: AggregateType,
        aggregate_id: impl Into<String>,
    ) -> Result<Self, AggregateError> {
        let aggregate_id = aggregate_id.into();
        if aggregate_id.trim().is_empty() {
            return Err(AggregateError::EmptyId);
        }
        Ok(Self {
            aggregate_type,
            aggregate_id,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AggregateEnvelope {
    pub schema_version: u16,
    pub aggregate_type: AggregateType,
    pub aggregate_id: String,
    pub aggregate_seq: u64,
}

impl AggregateEnvelope {
    pub const SCHEMA_VERSION: u16 = 1;

    pub fn new(identity: AggregateIdentity, aggregate_seq: u64) -> Result<Self, AggregateError> {
        if aggregate_seq == 0 {
            return Err(AggregateError::NonPositiveSequence);
        }
        Ok(Self {
            schema_version: Self::SCHEMA_VERSION,
            aggregate_type: identity.aggregate_type,
            aggregate_id: identity.aggregate_id,
            aggregate_seq,
        })
    }

    pub fn validate(&self) -> Result<(), AggregateError> {
        if self.schema_version != Self::SCHEMA_VERSION {
            return Err(AggregateError::UnsupportedPayloadVersion(
                self.schema_version,
            ));
        }
        if self.aggregate_id.trim().is_empty() {
            return Err(AggregateError::EmptyId);
        }
        if self.aggregate_seq == 0 {
            return Err(AggregateError::NonPositiveSequence);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub enum EventOperationMethod {
    #[serde(rename = "project.create")]
    ProjectCreate,
    #[serde(rename = "project.update")]
    ProjectUpdate,
    #[serde(rename = "project.repository_associate")]
    ProjectRepositoryAssociate,
    #[serde(rename = "task.create")]
    TaskCreate,
    #[serde(rename = "task.revise")]
    TaskRevise,
    #[serde(rename = "session.bind")]
    SessionBind,
    #[serde(rename = "session.start")]
    SessionStart,
}

impl EventOperationMethod {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ProjectCreate => "project.create",
            Self::ProjectUpdate => "project.update",
            Self::ProjectRepositoryAssociate => "project.repository_associate",
            Self::TaskCreate => "task.create",
            Self::TaskRevise => "task.revise",
            Self::SessionBind => "session.bind",
            Self::SessionStart => "session.start",
        }
    }
}

/// Inputs for the globally unique per-event key. `operation_identity` is the
/// caller's raw UUID for keyed Feature 002 mutations or the existing stable
/// session identity for Feature 001-compatible bound start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DerivedEventKeyInput<'a> {
    pub operation_identity: &'a str,
    pub method: EventOperationMethod,
    pub event_position: u16,
    pub event_type: &'a str,
}

pub fn derive_event_key(input: &DerivedEventKeyInput<'_>) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"cairn.event-idempotency.v1\0");
    update_part(&mut hasher, input.operation_identity.as_bytes());
    update_part(&mut hasher, input.method.as_str().as_bytes());
    update_part(&mut hasher, &input.event_position.to_be_bytes());
    update_part(&mut hasher, input.event_type.as_bytes());
    hasher.finalize().to_hex().to_string()
}

fn update_part(hasher: &mut blake3::Hasher, value: &[u8]) {
    hasher.update(&(value.len() as u64).to_be_bytes());
    hasher.update(value);
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum AggregateError {
    #[error("aggregate identity is empty")]
    EmptyId,
    #[error("aggregate sequence must be positive")]
    NonPositiveSequence,
    #[error("unsupported payload version {0}")]
    UnsupportedPayloadVersion(u16),
}
