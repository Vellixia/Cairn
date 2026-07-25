#![allow(dead_code)]

use std::path::PathBuf;

use cairn_domain::{EventId, Timestamp};
use cairn_storage_local::records::{RepositoryRow, WorktreeRow};
use cairn_storage_local::{open_pool_at, repos, worktrees};
use sqlx::SqlitePool;
use tempfile::TempDir;

pub struct Harness {
    _temp: TempDir,
    pub path: PathBuf,
    pub pool: SqlitePool,
}

impl Harness {
    pub async fn new() -> Self {
        let temp = tempfile::tempdir().expect("temporary test directory");
        let path = temp.path().join("cairn.db");
        let pool = open_pool_at(&path).await.expect("open migrated database");
        Self {
            _temp: temp,
            path,
            pool,
        }
    }

    pub async fn independent_pool(&self) -> SqlitePool {
        open_pool_at(&self.path)
            .await
            .expect("open independent pool")
    }

    pub async fn repository_with_worktrees(&self, count: usize) -> (String, Vec<String>) {
        let repository_id = EventId::new_v7().to_string();
        let now = Timestamp::now().to_rfc3339();
        repos::insert(
            &self.pool,
            &RepositoryRow {
                id: repository_id.clone(),
                repo_uuid: EventId::new_v7().to_string(),
                canonical_path: format!("/initial/{repository_id}"),
                default_remote_name: Some("origin".into()),
                default_remote_url: Some("https://example.invalid/repo.git".into()),
                copied_from_repository_id: None,
                registered_at: now.clone(),
            },
        )
        .await
        .expect("insert repository");

        let mut worktree_ids = Vec::with_capacity(count);
        for index in 0..count {
            let worktree_id = EventId::new_v7().to_string();
            worktrees::insert(
                &self.pool,
                &WorktreeRow {
                    id: worktree_id.clone(),
                    repository_id: repository_id.clone(),
                    worktree_uuid: EventId::new_v7().to_string(),
                    path: format!("/initial/{repository_id}/worktree-{index}"),
                    is_main: i64::from(index == 0),
                    registered_at: now.clone(),
                },
            )
            .await
            .expect("insert worktree");
            worktree_ids.push(worktree_id);
        }
        (repository_id, worktree_ids)
    }
}
