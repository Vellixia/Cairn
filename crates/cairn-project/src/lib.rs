//! Focused project/task mutation policy for Feature 002.
//!
//! Transport stays in the daemon/CLI, persistence stays in
//! `cairn-storage-local`, and session lifecycle policy stays in
//! `cairn-session`.

pub mod error;
pub mod goal_contract;
pub mod project_service;
pub mod task_service;

pub use error::{IdempotencyConflictKind, ProjectTaskError};
pub use goal_contract::parse_goal_contract_json;
pub use project_service::*;
pub use task_service::*;
