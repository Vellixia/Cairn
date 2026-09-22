//! Daemon state and the resolution steps every request shares.

use cairn_core::wire::{codes, WireError};
use cairn_core::CairnConfig;
use cairn_git::RepoInstance;
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
    /// Which canonical event kinds have been seen for one vendor-supplied
    /// session key, this daemon run.
    ///
    /// `stable_session_identifier` is established only when two or more
    /// canonical events, of at least two different kinds, carried a
    /// vendor-supplied identifier and routed to the same Cairn session (D19a).
    /// A single event carrying a non-empty string is not sufficient, and
    /// Feature 001's synthesized fallback never reaches here because an
    /// adapter declines an event it cannot route.
    pub lifecycle_kinds: Arc<RwLock<std::collections::HashMap<String, Vec<&'static str>>>>,
    /// Identifies this daemon run. Sessions from a previous run are the ones
    /// reconciled at startup (FR-009, D16).
    pub run_id: Uuid,
    pub config: Arc<RwLock<CairnConfig>>,
    /// This machine's own local identity, minted once by
    /// `repo::ensure_local_user`. Owns everything project-scoped.
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
    /// The bounded, account-bound outage cache for server-side retrieval
    /// (T072, `contracts/retrieval-delivery.md` §12.3). A cache, not durable
    /// state: in-memory, lost on restart, rebuilt by the next successful
    /// retrieval. Invalidated from [`Daemon::mutate_credentials`], the single
    /// door for sign-out, credential change and account change alike
    /// (FR-790a).
    pub outage_cache: Arc<tokio::sync::Mutex<crate::deliver::OutageCache>>,
    /// The instance the endpoint last reported, if this process has asked.
    ///
    /// **Observation, never authority.** The store's binding is the singular
    /// `team:*` lane (`cursor::bound_server_instance`, D438/FR-495/FR-496) and
    /// a mismatching server never becomes it — FR-791 refuses that server, and
    /// nothing here changes what a queued row is bound to.
    ///
    /// It exists because FR-792 asks a question the binding cannot answer.
    /// "Which rows belong to a different deployment than the one answering?"
    /// is measured against the server *answering*, and comparing rows to the
    /// store's own binding always says none — the backlog then reads as a
    /// queue that mysteriously stopped, which is the outcome FR-792 exists to
    /// prevent. Before this, the spool report was handed whichever instance
    /// sorted first among every lane, so it happened to be right about as
    /// often as the ids fell the right way.
    ///
    /// **Telemetry, and nothing that decides a report** (FR-792c). It is in
    /// memory and per process, which is exactly why it may not decide one: a
    /// daemon exits within one supervision tick of another owning its socket,
    /// so the process that observed a replacement server is routinely gone by
    /// the time an operator asks what happened, and a reason held only here is
    /// lost precisely when it is wanted. Status takes its own bounded sample
    /// instead (`sync::probe_peer_instance`, FR-792a); this stays because it is
    /// genuinely useful in a log when reconstructing what a daemon saw.
    pub last_observed_instance: Arc<RwLock<Option<Uuid>>>,
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
    /// The account id this token belongs to on `url`, once learned.
    ///
    /// See [`cairn_core::config::CairnConfig::server_account_id`] for why this
    /// is not the local user id.
    pub account_id: Option<uuid::Uuid>,
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
            account_id: config.server_account_id,
        }
    }
}

impl Daemon {
    /// The account this machine is authenticated as, or `None`.
    ///
    /// No fallback, by design: a caller that needs an account and has none must
    /// refuse, not substitute. See [`owner_identity`](Self::owner_identity).
    pub async fn account_identity(&self) -> Option<Uuid> {
        self.server.read().await.account_id
    }
}

/// A resolved repository and its local correlation project.
pub struct Resolved {
    pub repo: RepoInstance,
    pub project: cairn_core::domain::Project,
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
        Ok(Resolved {
            repo: repo_instance,
            project,
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
    ///
    /// Also drops every project's trait-refresh stamp. `forget_repo` is the
    /// daemon's one "this checkout is not what I thought it was" signal — `cairn
    /// init` is its only caller — and a working tree that changed identity is
    /// exactly the case where derived traits must not be trusted for the rest of
    /// the refresh interval. Clearing the whole map rather than one entry costs
    /// one re-derivation per project and avoids having to know which project the
    /// stale `cwd` belonged to, which is the question `forget_repo` is being
    /// told it cannot answer.
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
        // A refusal already carries the contract's stable code. Passing it
        // through is what keeps `revision_conflict` distinguishable from
        // `storage_unavailable` at the agent surface.
        cairn_store::StoreError::Refused { code, message } => WireError::new(code, message),
        other => WireError::new(codes::STORAGE_UNAVAILABLE, other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testsupport as fx;

    #[test]
    fn git_errors_map_to_the_code_that_tells_the_user_what_to_do() {
        let e = git_err(cairn_git::GitError::NotARepository("/tmp/x".into()));
        assert_eq!(e.code, codes::NOT_A_REPOSITORY);
        assert!(e.message.contains("run Cairn from a repository"));

        // Missing Git is reported as "not a repository" too: from the user's
        // side the remedy is the same shape, and it must never read as storage
        // trouble.
        let e = git_err(cairn_git::GitError::GitMissing("no git".into()));
        assert_eq!(e.code, codes::NOT_A_REPOSITORY);
        assert!(e.message.contains("PATH"));
    }

    #[test]
    fn a_missing_row_is_not_found_rather_than_storage_trouble() {
        let e = storage_err(cairn_store::StoreError::NotFound("task".into()));
        assert_eq!(e.code, codes::NOT_FOUND);
    }

    /// Discovery is cached, and the cache is actually consulted.
    ///
    /// Two `git` subprocesses per captured tool call would not fit the capture
    /// budget (SC-007), so the instance is discovered once and remembered. The
    /// test has teeth because the seeded path does not exist: if `repo_instance`
    /// fell through to discovery it would fail rather than return.
    #[tokio::test]
    async fn a_cached_repository_instance_is_returned_without_discovery() {
        let d = fx::daemon().await;
        let cwd = format!("{}/cached", fx::NOWHERE);
        let seeded = RepoInstance {
            git_common_dir: PathBuf::from(format!("{cwd}/.git")),
            worktree_path: PathBuf::from(&cwd),
            name: "cached".into(),
            remote: Some("github.com/example/cached".into()),
        };
        d.repos.write().await.insert(cwd.clone(), seeded.clone());

        let got = d
            .repo_instance(&cwd)
            .await
            .expect("the cache answers for a path discovery could not");
        assert_eq!(got.name, seeded.name);
        assert_eq!(got.remote, seeded.remote);
    }

    /// `cairn init` must be able to invalidate the cache, so a re-registered
    /// checkout is discovered again rather than served from memory.
    #[tokio::test]
    async fn forgetting_a_repository_drops_it_from_the_cache() {
        let d = fx::daemon().await;
        let cwd = format!("{}/forgotten", fx::NOWHERE);
        d.repos.write().await.insert(
            cwd.clone(),
            RepoInstance {
                git_common_dir: PathBuf::from(format!("{cwd}/.git")),
                worktree_path: PathBuf::from(&cwd),
                name: "forgotten".into(),
                remote: None,
            },
        );

        d.forget_repo(&cwd).await;

        assert!(d.repos.read().await.get(&cwd).is_none());
        // And the next call now has to discover, which for this path fails —
        // proving the entry is genuinely gone rather than merely stale.
        assert!(
            d.repo_instance(&cwd).await.is_err(),
            "a forgotten path must be rediscovered, not served from the cache"
        );
    }

    /// The idle-exit check must not fire while a session is still open.
    #[tokio::test]
    async fn a_daemon_with_an_active_session_is_not_idle() {
        let d = fx::daemon().await;
        assert!(
            d.no_active_sessions().await,
            "an empty store has nothing active"
        );

        let p = fx::project(&d, "busy", None).await;
        let s = fx::session(&d, &p, "open").await;
        assert!(!d.no_active_sessions().await);

        repo::end_session(
            &d.store,
            s.id,
            cairn_core::domain::SessionStatus::Completed,
            Some("done"),
        )
        .await
        .expect("end");
        assert!(d.no_active_sessions().await);
    }
}
