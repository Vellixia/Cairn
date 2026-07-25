//! T044: reconciliation loop. Hints trigger it; Git reconciliation produces
//! the authoritative snapshot. Unchanged fingerprint → no-op (touch case,
//! FR-022). Changed → snapshot + state-change events + live-session current
//! snapshot updates, all through the per-worktree serialized writer.

use std::path::PathBuf;

use cairn_domain::SessionState;
use cairn_events::{BranchChangedPayload, EventBuilder, StateChangedPayload};
use cairn_storage_local::{events as ev, sessions as sdao, snapshots, SnapshotRow};

use crate::handlers::snapshot::ensure_snapshot;
use crate::handlers::HandlerResult;
use crate::state::AppState;

pub struct Reconciler {
    state: AppState,
    repository_id: String,
    worktree_id: String,
    root: PathBuf,
    /// Last authoritative snapshot observed for this worktree.
    prev: Option<SnapshotRow>,
}

impl Reconciler {
    pub async fn new(
        state: AppState,
        repository_id: String,
        worktree_id: String,
        root: PathBuf,
    ) -> Self {
        // Seed from a live session's current snapshot when available.
        let prev = seed_prev(&state, &worktree_id).await;
        Self {
            state,
            repository_id,
            worktree_id,
            root,
            prev,
        }
    }

    pub async fn reconcile(&mut self) -> HandlerResult<bool> {
        let (snapshot, _created) = ensure_snapshot(
            &self.state,
            &self.repository_id,
            &self.worktree_id,
            &self.root,
        )
        .await?;

        let prev_fp = self.prev.as_ref().map(|p| p.snapshot_fp.clone());
        if prev_fp.as_deref() == Some(snapshot.snapshot_fp.as_str()) {
            return Ok(false); // advisory hint did not correspond to a real change
        }

        let branch_changed = match &self.prev {
            Some(prev) => {
                prev.branch != snapshot.branch || prev.head_commit != snapshot.head_commit
            }
            None => false,
        };

        let state_payload = StateChangedPayload {
            worktree_id: self.worktree_id.clone(),
            from_snapshot_id: self.prev.as_ref().map(|p| p.id.clone()),
            to_snapshot_id: snapshot.id.clone(),
        };
        let branch_payload = self.prev.as_ref().map(|prev| BranchChangedPayload {
            from_branch: prev.branch.clone(),
            to_branch: snapshot.branch.clone(),
            from_head: Some(prev.head_commit.clone()),
            to_head: snapshot.head_commit.clone(),
        });

        let repo_id = self.repository_id.clone();
        let wt_id = self.worktree_id.clone();
        let snap_id = snapshot.id.clone();
        ev::serialized_txn(
            self.state.pool(),
            &self.state.inner.writers,
            &self.worktree_id,
            Box::new(move |conn| {
                Box::pin(async move {
                    let event =
                        EventBuilder::repository_state_changed(&repo_id, &wt_id, &state_payload);
                    ev::append_event(&mut *conn, &event).await?;
                    if branch_changed {
                        if let Some(bp) = &branch_payload {
                            let event = EventBuilder::branch_changed(&repo_id, &wt_id, bp);
                            ev::append_event(&mut *conn, &event).await?;
                        }
                    }
                    // Projection: every live session in this worktree tracks
                    // the new current snapshot (same transaction).
                    let live: Vec<(String,)> = sqlx::query_as(
                        "SELECT id FROM sessions WHERE worktree_id = ? \
                         AND state IN ('active','recovering')",
                    )
                    .bind(&wt_id)
                    .fetch_all(&mut *conn)
                    .await
                    .map_err(cairn_storage_local::StorageError::from)?;
                    for (sid,) in live {
                        sdao::update_current_snapshot(&mut *conn, &sid, &snap_id).await?;
                    }
                    Ok(())
                })
            }),
        )
        .await?;

        tracing::debug!(worktree = %self.worktree_id, fp = %snapshot.snapshot_fp, "state change recorded");
        self.prev = Some(snapshot);
        Ok(true)
    }
}

async fn seed_prev(state: &AppState, worktree_id: &str) -> Option<SnapshotRow> {
    let live = [SessionState::Active, SessionState::Recovering];
    let rows = sdao::list(state.pool(), None, Some(&live)).await.ok()?;
    let session = rows.into_iter().find(|r| r.worktree_id == worktree_id)?;
    snapshots::get_by_id(state.pool(), &session.current_snapshot_id)
        .await
        .ok()?
}
