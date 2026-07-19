//! T027/T028: repository registration (identity read-or-create/restore,
//! copy detection, bare rejection) and fresh-reconciliation inspection.

use std::path::Path;

use cairn_domain::{RepoUuid, Timestamp, WorktreeUuid};
use cairn_events::EventBuilder;
use cairn_git::{discover, identity, ignored};
use cairn_protocol::*;
use cairn_storage_local::{events as ev, repos, worktrees, RepositoryRow, WorktreeRow};
use uuid::Uuid;

use super::{convert, HandlerError, HandlerResult};
use crate::state::AppState;

fn norm_path(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

/// Resolve a repository row + worktree row for `path`, requiring existing
/// registration (used by inspect/snapshot/session paths).
pub async fn resolve_registered(
    state: &AppState,
    path: Option<&str>,
    repository_id: Option<&str>,
) -> HandlerResult<(RepositoryRow, WorktreeRow, discover::RepoLayout)> {
    let root_hint = match (path, repository_id) {
        (Some(p), _) => p.to_string(),
        (None, Some(id)) => {
            let row = repos::get_by_id(state.pool(), id).await?.ok_or_else(|| {
                HandlerError::new(ErrorCode::NotRegistered, "unknown repository id")
            })?;
            row.canonical_path
        }
        (None, None) => {
            return Err(HandlerError::new(
                ErrorCode::Usage,
                "path or repository_id required",
            ))
        }
    };

    let layout = discover::discover(Path::new(&root_hint)).await?;
    let markers = identity::read_markers(&layout)?;
    let repo_uuid = markers.repo_uuid.ok_or_else(|| {
        HandlerError::new(
            ErrorCode::NotRegistered,
            "repository not initialized with cairn (run `cairn init`)",
        )
    })?;
    let repo = repos::get_by_uuid(state.pool(), &repo_uuid.to_string())
        .await?
        .ok_or_else(|| {
            HandlerError::new(
                ErrorCode::NotRegistered,
                "repository not registered (run `cairn init`)",
            )
        })?;
    let wt_uuid = markers.worktree_uuid.ok_or_else(|| {
        HandlerError::new(
            ErrorCode::NotRegistered,
            "worktree not initialized with cairn (run `cairn init`)",
        )
    })?;
    let worktree = worktrees::get_by_uuid(state.pool(), &wt_uuid.to_string())
        .await?
        .ok_or_else(|| HandlerError::new(ErrorCode::NotRegistered, "worktree not registered"))?;

    // Moves/renames: keep canonical path metadata current (FR-002).
    let current = norm_path(&layout.worktree_root);
    if repo.canonical_path != current && layout.is_main {
        repos::update_canonical_path(state.pool(), &repo.id, &current).await?;
    }
    if worktree.path != current {
        worktrees::update_path(state.pool(), &worktree.id, &current).await?;
    }

    Ok((repo, worktree, layout))
}

pub async fn register(state: &AppState, params: RegisterParams) -> HandlerResult<RegisterResult> {
    let layout = discover::discover(Path::new(&params.path)).await?;
    let root = norm_path(&layout.worktree_root);
    let markers = identity::read_markers(&layout)?;
    let runner = cairn_git::GitRunner::new(&layout.worktree_root);
    let remote = discover::default_remote(&runner).await?;
    let now = Timestamp::now();

    let mut identity_outcome = IdentityOutcome::Existing;
    let mut created_repo = false;
    let mut restored = false;
    let mut copied_from: Option<String> = None;

    // ---- resolve repository identity ----
    let repo_row = match markers.repo_uuid {
        Some(uuid) => {
            match repos::get_by_uuid(state.pool(), &uuid.to_string()).await? {
                Some(existing) => {
                    // Copy detection (Q1/R4): same uuid live at two paths.
                    let stored_path = Path::new(&existing.canonical_path);
                    let is_copy = existing.canonical_path != root
                        && stored_path.exists()
                        && marker_matches(stored_path, uuid).await;
                    if is_copy {
                        let new_uuid = Uuid::now_v7();
                        identity::write_repo_marker(&layout, new_uuid)?;
                        identity_outcome = IdentityOutcome::NewAfterCopy;
                        created_repo = true;
                        copied_from = Some(existing.id.clone());
                        new_repo_row(new_uuid, &root, &remote, copied_from.clone(), &now)
                    } else {
                        if existing.canonical_path != root {
                            repos::update_canonical_path(state.pool(), &existing.id, &root).await?;
                        }
                        existing
                    }
                }
                None => {
                    // Marker present, DB has no row (fresh DB): adopt marker.
                    created_repo = true;
                    identity_outcome = IdentityOutcome::Created;
                    new_repo_row(uuid, &root, &remote, None, &now)
                }
            }
        }
        None => {
            // Marker loss / first init (analysis U1).
            let candidates = repos::get_by_canonical_path(state.pool(), &root).await?;
            match candidates.len() {
                1 => {
                    let existing = candidates.into_iter().next().expect("len checked");
                    let uuid = Uuid::parse_str(&existing.repo_uuid)
                        .map_err(|e| HandlerError::new(ErrorCode::Internal, e.to_string()))?;
                    identity::write_repo_marker(&layout, uuid)?;
                    identity_outcome = IdentityOutcome::Restored;
                    restored = true;
                    existing
                }
                0 => {
                    let uuid = Uuid::now_v7();
                    identity::write_repo_marker(&layout, uuid)?;
                    created_repo = true;
                    identity_outcome = IdentityOutcome::Created;
                    new_repo_row(uuid, &root, &remote, None, &now)
                }
                _ => {
                    // Ambiguous history: never silently attach (U1).
                    let uuid = Uuid::now_v7();
                    identity::write_repo_marker(&layout, uuid)?;
                    created_repo = true;
                    identity_outcome = IdentityOutcome::NewAfterMarkerLoss;
                    new_repo_row(uuid, &root, &remote, None, &now)
                }
            }
        }
    };

    // ---- resolve worktree identity ----
    let mut created_worktree = false;
    let wt_row = match markers.worktree_uuid {
        Some(uuid) if !created_repo => {
            match worktrees::get_by_uuid(state.pool(), &uuid.to_string()).await? {
                Some(existing) => {
                    if existing.path != root {
                        worktrees::update_path(state.pool(), &existing.id, &root).await?;
                    }
                    existing
                }
                None => {
                    created_worktree = true;
                    new_worktree_row(uuid, &repo_row.id, &root, layout.is_main, &now)
                }
            }
        }
        _ => {
            // New repo identity (or missing marker) => fresh worktree identity.
            let uuid = Uuid::now_v7();
            identity::write_worktree_marker(&layout, uuid)?;
            created_worktree = true;
            new_worktree_row(uuid, &repo_row.id, &root, layout.is_main, &now)
        }
    };

    // ---- persist rows + events in one serialized transaction ----
    if created_repo || created_worktree || restored {
        let repo_c = repo_row.clone();
        let wt_c = wt_row.clone();
        let remote_c = remote.clone();
        let root_c = root.clone();
        ev::serialized_txn(
            state.pool(),
            &state.inner.writers,
            &wt_row.id.clone(),
            Box::new(move |conn| {
                Box::pin(async move {
                    if created_repo {
                        repos::insert(&mut *conn, &repo_c).await?;
                        let event = EventBuilder::repository_registered(
                            &repo_c.id,
                            &repo_c.repo_uuid,
                            &root_c,
                            remote_c.as_ref().map(|(n, u)| (n.as_str(), u.as_str())),
                        );
                        ev::append_event(&mut *conn, &event).await?;
                    }
                    if created_worktree {
                        worktrees::insert(&mut *conn, &wt_c).await?;
                        let event = EventBuilder::worktree_registered(
                            &repo_c.id,
                            &wt_c.id,
                            &wt_c.worktree_uuid,
                            &wt_c.path,
                            wt_c.is_main != 0,
                        );
                        ev::append_event(&mut *conn, &event).await?;
                    }
                    if restored {
                        let event = EventBuilder::identity_marker_restored(
                            &repo_c.id,
                            Some(&wt_c.id),
                            &repo_c.canonical_path,
                        );
                        ev::append_event(&mut *conn, &event).await?;
                    }
                    Ok(())
                })
            }),
        )
        .await?;
    }

    let final_repo = repos::get_by_id(state.pool(), &repo_row.id)
        .await?
        .ok_or_else(|| HandlerError::new(ErrorCode::Internal, "repository row vanished"))?;
    let final_wt = worktrees::get_by_id(state.pool(), &wt_row.id)
        .await?
        .ok_or_else(|| HandlerError::new(ErrorCode::Internal, "worktree row vanished"))?;

    tracing::info!(
        repository_id = %final_repo.id,
        outcome = ?identity_outcome,
        created = created_repo,
        "repository register"
    );
    let _ = copied_from; // linked via repo row

    Ok(RegisterResult {
        repository: convert::repository_dto(&final_repo)?,
        worktree: convert::worktree_dto(&final_wt)?,
        created: created_repo,
        identity_outcome,
    })
}

async fn marker_matches(root: &Path, uuid: Uuid) -> bool {
    match discover::discover(root).await {
        Ok(layout) => matches!(
            identity::read_markers(&layout),
            Ok(m) if m.repo_uuid == Some(uuid)
        ),
        Err(_) => false,
    }
}

fn new_repo_row(
    uuid: Uuid,
    root: &str,
    remote: &Option<(String, String)>,
    copied_from: Option<String>,
    now: &Timestamp,
) -> RepositoryRow {
    RepositoryRow {
        id: RepoUuid::new_v7().to_string(),
        repo_uuid: uuid.to_string(),
        canonical_path: root.to_string(),
        default_remote_name: remote.as_ref().map(|(n, _)| n.clone()),
        default_remote_url: remote.as_ref().map(|(_, u)| u.clone()),
        copied_from_repository_id: copied_from,
        registered_at: now.to_rfc3339(),
    }
}

fn new_worktree_row(
    uuid: Uuid,
    repository_id: &str,
    path: &str,
    is_main: bool,
    now: &Timestamp,
) -> WorktreeRow {
    WorktreeRow {
        id: WorktreeUuid::new_v7().to_string(),
        repository_id: repository_id.to_string(),
        worktree_uuid: uuid.to_string(),
        path: path.to_string(),
        is_main: i64::from(is_main),
        registered_at: now.to_rfc3339(),
    }
}

pub async fn inspect(state: &AppState, params: InspectParams) -> HandlerResult<InspectionDto> {
    let (repo, worktree, layout) = resolve_registered(
        state,
        params.path.as_deref(),
        params.repository_id.as_deref(),
    )
    .await?;
    let runner = cairn_git::GitRunner::new(&layout.worktree_root);
    let report = cairn_git::status::status(&runner).await?;
    let summary = ignored::ignored_summary(&layout.worktree_root)?;
    let remote = discover::default_remote(&runner).await?;
    let _ = &repo;

    Ok(InspectionDto {
        root: norm_path(&layout.worktree_root),
        branch: report.branch.clone(),
        detached: report.branch.is_none(),
        head_commit: report.head_oid.clone(),
        default_remote: remote.map(|(name, url)| RemoteDto { name, url }),
        staged: report
            .staged
            .iter()
            .map(|c| FileChangeDto {
                path: c.path.clone(),
                status: c.status.as_str().to_string(),
                orig_path: c.orig_path.clone(),
            })
            .collect(),
        unstaged: report
            .unstaged
            .iter()
            .map(|c| FileChangeDto {
                path: c.path.clone(),
                status: c.status.as_str().to_string(),
                orig_path: c.orig_path.clone(),
            })
            .collect(),
        untracked: report.untracked.clone(),
        ignored_summary: IgnoredSummaryDto {
            total_count: summary.total_count,
            by_source: IgnoredBySourceDto {
                gitignore: summary.gitignore_count,
                cairnignore: summary.cairnignore_count,
            },
            collapsed_roots: summary
                .collapsed_roots
                .into_iter()
                .map(|(path, count)| IgnoredRootDto { path, count })
                .collect(),
            samples: summary.samples,
            truncated: summary.truncated,
        },
        worktree: WorktreeInfoDto {
            worktree_id: Some(worktree.id.clone()),
            path: worktree.path.clone(),
            is_main: worktree.is_main != 0,
        },
        in_progress: discover::in_progress_operation(&layout).map(str::to_string),
    })
}

pub async fn ignored_files(
    state: &AppState,
    params: IgnoredFilesParams,
) -> HandlerResult<IgnoredFilesResult> {
    let repo = repos::get_by_id(state.pool(), &params.repository_id)
        .await?
        .ok_or_else(|| HandlerError::new(ErrorCode::NotRegistered, "unknown repository id"))?;
    let limit = params.limit.unwrap_or(200).min(1000) as usize;
    let (paths, next_cursor) = ignored::ignored_page(
        Path::new(&repo.canonical_path),
        params.cursor.as_deref(),
        limit,
        params.glob.as_deref(),
    )?;
    Ok(IgnoredFilesResult { paths, next_cursor })
}
