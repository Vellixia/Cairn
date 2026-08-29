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
    /// When each project's traits were last derived from its working tree.
    ///
    /// Derivation is cheap — eleven `Path::exists` calls and one directory
    /// listing — but it is still a filesystem read plus a write transaction, and
    /// `resolve` runs on every request. This bounds it to once per project per
    /// [`TRAIT_REFRESH_INTERVAL`] instead.
    pub traits_refreshed: Arc<RwLock<std::collections::HashMap<Uuid, std::time::Instant>>>,
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

/// How long a project's derived traits are trusted before being re-derived.
///
/// A manifest appearing mid-session — `cargo init` in a fresh repository, a
/// `Dockerfile` added — becomes visible to applicability matching within this
/// window rather than at the next daemon restart.
pub const TRAIT_REFRESH_INTERVAL: std::time::Duration = std::time::Duration::from_secs(60);

impl Daemon {
    /// This project's traits, derived from its working tree if they are stale.
    ///
    /// **The one production entry point for project traits** (FR-437, FR-439).
    /// Every applicability-sensitive read goes through here, so there is one
    /// place where "the traits are current" becomes true and no reader can
    /// accidentally consult an empty set.
    ///
    /// It is an accessor rather than a step inside `resolve` because `resolve`
    /// runs on every request and most requests do not care about traits;
    /// deriving there would pay for a filesystem scan and a write transaction on
    /// every session event. Refreshing here, bounded by
    /// [`TRAIT_REFRESH_INTERVAL`], pays for it only when something is about to
    /// read the answer.
    ///
    /// A derivation failure returns whatever is already stored rather than an
    /// error. Traits narrow what recall admits; failing a briefing because a
    /// directory could not be listed would trade a smaller answer for no answer.
    pub async fn project_traits(&self, r: &Resolved) -> Vec<cairn_core::domain::ProjectTrait> {
        let due = {
            let seen = self.traits_refreshed.read().await;
            match seen.get(&r.project.id) {
                Some(at) => at.elapsed() >= TRAIT_REFRESH_INTERVAL,
                None => true,
            }
        };

        if due {
            let worktree = r.repo.worktree_path.clone();
            match cairn_store::traits::refresh_traits(&self.store, r.project.id, &worktree).await {
                Ok(derived) => {
                    self.traits_refreshed
                        .write()
                        .await
                        .insert(r.project.id, std::time::Instant::now());
                    return derived;
                }
                Err(e) => {
                    tracing::debug!(project = %r.project.id, error = %e, "traits not refreshed");
                }
            }
        }

        cairn_store::traits::traits_for_project(&self.store, r.project.id)
            .await
            .unwrap_or_default()
    }

    /// The identity that owns this machine's global knowledge — personal
    /// records, and the proposer and actor recorded on team ones.
    ///
    /// The linked server's account id when there is one, and the local user id
    /// otherwise. Those are different identities on purpose (FR-567, FR-568): a
    /// user account is per-server, so the same human on two servers is two
    /// accounts, and their personal knowledge must sit in two disjoint sets of
    /// rows rather than one pool. Keying on the local id would merge them the
    /// moment a store was relinked, and there would be no way to unmerge them
    /// afterwards.
    ///
    /// Team knowledge uses the same identity for the same reason: the server
    /// records `proposed_by_user_id` and `ratified_by_user_id` as *its* account
    /// ids, so a locally recorded proposal keyed on the local identity would
    /// stop being the caller's own proposal the moment the row came back from a
    /// pull — and the role-filtered listing that shows a member their own
    /// pending proposals would stop showing it.
    ///
    /// Notes written before any link are owned by
    /// [`UNATTRIBUTED_OWNER`](cairn_core::domain::UNATTRIBUTED_OWNER), and stay
    /// owned by it. Linking later does not reassign them to whichever account the
    /// machine now authenticates as: that would attribute work to an identity
    /// that did not do it, and would push it to a server the user had not chosen
    /// to send it to when they wrote it.
    ///
    /// **The fallback is not this machine's id** (FR-603). It was, and that made
    /// a local machine identity look like an account: it is identity-shaped, it
    /// is a component of a `personal:*` lane key, and every routing decision that
    /// asked "whose knowledge is this" got a confident answer naming something
    /// the server has never heard of. The sentinel cannot be mistaken for an
    /// account by any comparison, and no lane can be keyed by it.
    ///
    /// **Only local reads and local personal writes may call this.** Anything
    /// that routes, enqueues, pushes or pulls must call
    /// [`account_identity`](Self::account_identity) and fail closed when it
    /// returns `None`.
    pub async fn owner_identity(&self) -> Uuid {
        self.account_identity()
            .await
            .unwrap_or(cairn_core::domain::UNATTRIBUTED_OWNER)
    }

    /// The account this machine is authenticated as, or `None`.
    ///
    /// No fallback, by design: a caller that needs an account and has none must
    /// refuse, not substitute. See [`owner_identity`](Self::owner_identity).
    pub async fn account_identity(&self) -> Option<Uuid> {
        self.server.read().await.account_id
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
        self.traits_refreshed.write().await.clear();
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

/// The sync policy for a project, for the background paths that hold an id
/// rather than a resolved worktree.
///
/// A project that cannot be read is treated as unlinked: a maintenance pass must
/// not fail because it could not decide whether to enqueue.
pub async fn sync_policy_for_project(d: &Daemon, project_id: Uuid) -> SyncPolicy {
    match repo::project(&d.store, project_id).await {
        Ok(p) => SyncPolicy::from_project(&p),
        Err(_) => SyncPolicy {
            linked: false,
            server_project_id: None,
        },
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

    #[test]
    fn repo_state_carries_git_counts_across_unchanged() {
        let st = cairn_git::GitStatus {
            branch: "feature".into(),
            commit_sha: Some("deadbee".into()),
            staged: 1,
            unstaged: 2,
            untracked: 3,
            changed_files: vec!["a".into()],
        };
        let converted = repo_state(&st);
        assert_eq!(converted.branch, "feature");
        assert_eq!(converted.commit_sha.as_deref(), Some("deadbee"));
        assert_eq!(
            (converted.staged, converted.unstaged, converted.untracked),
            (1, 2, 3)
        );
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

    /// Capture is fire-and-forget (H3), so a handoff asked for immediately after
    /// the last tool call must wait for the write to land — but boundedly, since
    /// a handoff has to be produced either way.
    #[tokio::test]
    async fn quiesce_returns_once_the_last_capture_guard_is_dropped() {
        let d = fx::daemon().await;
        let guard = CaptureGuard::new(&d.in_flight_captures);
        assert_eq!(d.in_flight_captures.load(Ordering::SeqCst), 1);

        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            drop(guard);
        });

        let started = std::time::Instant::now();
        d.quiesce_captures().await;

        assert_eq!(
            d.in_flight_captures.load(Ordering::SeqCst),
            0,
            "quiesce should not return while a capture is still in flight"
        );
        assert!(
            started.elapsed() >= std::time::Duration::from_millis(15),
            "returning instantly would mean it never waited: {:?}",
            started.elapsed()
        );
    }

    /// And it gives up rather than waiting forever.
    #[tokio::test]
    async fn quiesce_is_bounded_when_a_capture_never_lands() {
        let d = fx::daemon().await;
        // Leaked on purpose: a capture that never completes is exactly the case
        // the bound exists for.
        std::mem::forget(CaptureGuard::new(&d.in_flight_captures));

        let started = std::time::Instant::now();
        d.quiesce_captures().await;
        let waited = started.elapsed();

        assert!(
            waited >= std::time::Duration::from_millis(400),
            "it should actually wait for the in-flight capture, waited {waited:?}"
        );
        assert!(
            waited < std::time::Duration::from_millis(2000),
            "but it must give up: a handoff is produced either way, waited {waited:?}"
        );
        assert_eq!(d.in_flight_captures.load(Ordering::SeqCst), 1);
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
            SyncPolicy::from_project(&p),
        )
        .await
        .expect("end");
        assert!(d.no_active_sessions().await);
    }
}
