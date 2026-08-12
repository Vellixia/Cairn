//! Daemon state and the resolution steps every request shares.

use cairn_core::wire::{codes, WireError};
use cairn_core::CairnConfig;
use cairn_git::RepoInstance;
use cairn_store::outbox::SyncPolicy;
use cairn_store::{repo, Store};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

/// Everything a request handler needs.
///
/// Cheap to clone: every field is either `Copy` or already behind an `Arc`.
/// The sealed close spawns a synthesis task that outlives its request, and it
/// needs its own handle (D22).
#[derive(Clone)]
pub struct Daemon {
    pub store: Store,
    /// Identifies this daemon run. Sessions from a previous run are the ones
    /// reconciled at startup (FR-009, D16).
    pub run_id: Uuid,
    pub config: Arc<RwLock<CairnConfig>>,
    pub user_id: Uuid,
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub server: Arc<RwLock<ServerCredentials>>,
    /// Resolved repository instances, keyed by working directory.
    ///
    /// Discovery costs two `git` subprocesses, which is far too much to pay on
    /// every captured tool call (SC-007). A checkout's identity does not move
    /// under a running daemon, and `cairn init` clears the entry.
    pub repos: Arc<RwLock<HashMap<String, RepoInstance>>>,
    /// Epoch milliseconds of the last handled request, for the idle timeout.
    pub last_activity: Arc<AtomicI64>,
    /// Capture requests accepted but not yet written.
    ///
    /// Capture is fire-and-forget (H3), so a handoff asked for immediately
    /// after the last tool call could otherwise be synthesized before that
    /// call's observation lands — and report the session incompletely.
    pub in_flight_captures: Arc<AtomicUsize>,
    /// Serializes outbox drains inside this process.
    ///
    /// Claiming rows in the store is what makes concurrent drains *safe*; this
    /// is what makes them *orderly*. Without it, `cairn sync now` returns as
    /// soon as it has delivered its own claim while the background worker still
    /// holds the rest of the queue, and then reports a depth that is accurate
    /// and useless. One in-process mutex — no lease, no lock service (FR-059).
    pub sync_drain: Arc<tokio::sync::Mutex<()>>,
}

/// Increments the in-flight capture count and decrements it on drop, whatever
/// happens in between.
pub struct CaptureGuard(Arc<AtomicUsize>);

impl CaptureGuard {
    pub fn new(counter: &Arc<AtomicUsize>) -> Self {
        counter.fetch_add(1, Ordering::SeqCst);
        Self(Arc::clone(counter))
    }
}

impl Drop for CaptureGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Daemon {
    /// Wait, briefly and boundedly, for accepted captures to be written.
    ///
    /// Bounded because a handoff must be produced either way: waiting forever
    /// would trade one defect for a worse one.
    pub async fn quiesce_captures(&self) {
        const LIMIT: std::time::Duration = std::time::Duration::from_millis(500);
        let deadline = std::time::Instant::now() + LIMIT;
        while self.in_flight_captures.load(Ordering::SeqCst) > 0 {
            if std::time::Instant::now() >= deadline {
                tracing::debug!("handoff proceeding with captures still in flight");
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    }

    pub fn touch(&self) {
        self.last_activity
            .store(chrono::Utc::now().timestamp_millis(), Ordering::Relaxed);
    }

    pub fn idle_for(&self) -> std::time::Duration {
        let last = self.last_activity.load(Ordering::Relaxed);
        let millis = (chrono::Utc::now().timestamp_millis() - last).max(0) as u64;
        std::time::Duration::from_millis(millis)
    }

    /// True when no session anywhere is still `active`.
    pub async fn no_active_sessions(&self) -> bool {
        match repo::list_projects(&self.store).await {
            Ok(projects) => {
                for p in projects {
                    match repo::list_sessions(&self.store, p.id).await {
                        Ok(sessions) if sessions.iter().any(|s| s.is_active()) => return false,
                        Ok(_) => {}
                        Err(_) => return false,
                    }
                }
                true
            }
            Err(_) => false,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ServerCredentials {
    pub url: Option<String>,
    pub token: Option<String>,
}

impl ServerCredentials {
    /// Load the API token from disk, if the user has set one (D10).
    pub fn load(config: &CairnConfig) -> Self {
        let token = std::fs::read_to_string(cairn_core::paths::token_path())
            .ok()
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty());
        Self {
            url: config.server_url.clone(),
            token,
        }
    }
}

/// A resolved repository, its project, and whether that project may sync.
pub struct Resolved {
    pub repo: RepoInstance,
    pub project: cairn_core::domain::Project,
    pub policy: SyncPolicy,
}

impl Resolved {
    pub fn worktree(&self) -> String {
        self.repo.worktree_path.display().to_string()
    }
}

impl Daemon {
    /// Resolve `cwd` to a local repository instance and its project.
    ///
    /// Reports a clear error and creates nothing when the directory is not a
    /// repository or Git is unavailable (FR-005).
    pub async fn resolve(&self, cwd: &str) -> Result<Resolved, WireError> {
        let repo_instance = self.repo_instance(cwd).await?;
        let common = repo_instance.git_common_dir.display().to_string();
        let name = repo_instance.name.clone();
        let remote = repo_instance.remote.clone();

        let project = repo::ensure_project(&self.store, &common, &name, remote.as_deref())
            .await
            .map_err(storage_err)?;
        let policy = SyncPolicy::from_project(&project);
        Ok(Resolved {
            repo: repo_instance,
            project,
            policy,
        })
    }

    /// The repository instance for `cwd`, discovered once and remembered.
    pub async fn repo_instance(&self, cwd: &str) -> Result<RepoInstance, WireError> {
        if let Some(cached) = self.repos.read().await.get(cwd) {
            return Ok(cached.clone());
        }
        let discovered = discover(cwd).await?;
        self.repos
            .write()
            .await
            .insert(cwd.to_string(), discovered.clone());
        Ok(discovered)
    }

    /// Forget a cached instance, so a re-registered checkout is re-discovered.
    pub async fn forget_repo(&self, cwd: &str) {
        self.repos.write().await.remove(cwd);
    }
}

/// Git is blocking; keep it off the async executor.
pub async fn discover(cwd: &str) -> Result<RepoInstance, WireError> {
    let path = PathBuf::from(cwd);
    tokio::task::spawn_blocking(move || cairn_git::discover(&path))
        .await
        .map_err(|e| WireError::new(codes::STORAGE_UNAVAILABLE, e.to_string()))?
        .map_err(git_err)
}

pub async fn git_branches(worktree: PathBuf) -> Result<Vec<String>, WireError> {
    tokio::task::spawn_blocking(move || cairn_git::local_branches(&worktree))
        .await
        .map_err(|e| WireError::new(codes::STORAGE_UNAVAILABLE, e.to_string()))?
        .map_err(git_err)
}

pub async fn git_status(worktree: PathBuf) -> Result<cairn_git::GitStatus, WireError> {
    tokio::task::spawn_blocking(move || cairn_git::status(&worktree))
        .await
        .map_err(|e| WireError::new(codes::STORAGE_UNAVAILABLE, e.to_string()))?
        .map_err(git_err)
}

pub fn git_err(e: cairn_git::GitError) -> WireError {
    match e {
        cairn_git::GitError::NotARepository(p) => WireError::new(
            codes::NOT_A_REPOSITORY,
            format!("{p} is not inside a Git repository; run Cairn from a repository"),
        ),
        cairn_git::GitError::GitMissing(m) => WireError::new(
            codes::NOT_A_REPOSITORY,
            format!("git is not available on PATH: {m}"),
        ),
        other => WireError::new(codes::STORAGE_UNAVAILABLE, other.to_string()),
    }
}

pub fn storage_err(e: cairn_store::StoreError) -> WireError {
    match e {
        cairn_store::StoreError::NotFound(what) => WireError::not_found(what),
        other => WireError::new(codes::STORAGE_UNAVAILABLE, other.to_string()),
    }
}

/// Convert a `RepositoryState` from Git's view.
pub fn repo_state(st: &cairn_git::GitStatus) -> cairn_core::domain::RepositoryState {
    cairn_core::domain::RepositoryState {
        branch: st.branch.clone(),
        commit_sha: st.commit_sha.clone(),
        staged: st.staged,
        unstaged: st.unstaged,
        untracked: st.untracked,
    }
}
