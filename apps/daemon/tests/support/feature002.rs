#![allow(dead_code)]

use std::path::{Path, PathBuf};

use cairn_domain::AgentInstanceId;
use fixtures_repositories::FixtureRepo;
use uuid::Uuid;

pub fn repository() -> FixtureRepo {
    FixtureRepo::new().expect("Feature 002 repository fixture")
}

pub fn deterministic_agent_instance(index: u128) -> AgentInstanceId {
    AgentInstanceId(Uuid::from_u128(index))
}

pub fn feature001_database_fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/databases/feature-001-v1.sqlite3")
}
