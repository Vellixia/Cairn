//! T034: snapshot service — insert-or-get by (worktree_id, snapshot_fp),
//! `snapshot.created` event only on create.

use std::path::Path;

use cairn_domain::{SnapshotId, Timestamp, FP_SCHEMA_VERSION};
use cairn_events::EventBuilder;
use cairn_git::fingerprint::fingerprint_state;
use cairn_protocol::{SnapshotCreateParams, SnapshotCreateResult};
use cairn_storage_local::{events as ev, snapshots, SnapshotRow};

use super::{convert, HandlerResult};
use crate::state::AppState;

/// Compute the authoritative snapshot for a worktree and persist it
/// (deduplicated). Returns (row, created).
pub async fn ensure_snapshot(
    state: &AppState,
    repository_id: &str,
    worktree_id: &str,
    root: &Path,
) -> HandlerResult<(SnapshotRow, bool)> {
    let fp = fingerprint_state(root).await?;
    let components = fp.components;
    let snapshot_fp = components.final_fingerprint();
    let now = Timestamp::now();

    let row = SnapshotRow {
        id: SnapshotId::new_v7().to_string(),
        worktree_id: worktree_id.to_string(),
        branch: components.branch.clone(),
        head_commit: components.head_commit.clone(),
        staged_fp: components.staged_fp.clone(),
        unstaged_fp: components.unstaged_fp.clone(),
        untracked_fp: components.untracked_fp.clone(),
        snapshot_fp: snapshot_fp.clone(),
        fp_schema_version: i64::from(FP_SCHEMA_VERSION),
        created_at: now.to_rfc3339(),
    };

    let repo_id = repository_id.to_string();
    let wt_id = worktree_id.to_string();
    let out = ev::serialized_txn(
        state.pool(),
        &state.inner.writers,
        worktree_id,
        Box::new(move |conn| {
            Box::pin(async move {
                if let Some(existing) =
                    snapshots::get_by_fingerprint(&mut *conn, &wt_id, &row.snapshot_fp).await?
                {
                    return Ok((existing, false));
                }
                snapshots::insert(&mut *conn, &row).await?;
                let event = EventBuilder::snapshot_created(
                    &repo_id,
                    &wt_id,
                    &row.id,
                    &row.snapshot_fp,
                    row.branch.as_deref(),
                    &row.head_commit,
                );
                ev::append_event(&mut *conn, &event).await?;
                Ok((row, true))
            })
        }),
    )
    .await?;
    Ok(out)
}

pub async fn create(
    state: &AppState,
    params: SnapshotCreateParams,
) -> HandlerResult<SnapshotCreateResult> {
    let (repo, worktree, layout) = super::repository::resolve_registered(
        state,
        params.path.as_deref(),
        params.repository_id.as_deref(),
    )
    .await?;
    let (row, created) =
        ensure_snapshot(state, &repo.id, &worktree.id, &layout.worktree_root).await?;
    Ok(SnapshotCreateResult {
        snapshot: convert::snapshot_dto(&row)?,
        created,
    })
}
