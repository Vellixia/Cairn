//! Fixtures for the daemon's component tests (tier 2, `docs/testing.md`).
//!
//! A `Daemon` over an in-memory store, built in microseconds. Everything the
//! daemon does to *storage* — reconciliation, idle reaping, link state, handoff
//! synthesis — is plain logic over that store, and needs neither a socket nor a
//! spawned process to exercise. Before this existed the only way to reach any of
//! it was to spawn `cairn` and `cairnd` and drive them over the wire, which is
//! why `cairnd` had nine unit tests for two and a half thousand lines.
//!
//! What this deliberately does *not* fake: the store is real SQLite with the
//! real schema, and the repository paths are real strings pointing at nothing.
//! A missing worktree is a state the daemon must already handle (FR-009), so
//! pointing at nothing is a fixture, not a mock.
//!
//! Paths sit under [`NOWHERE`] rather than `/tmp` so they cannot accidentally
//! resolve. A fixture under `/tmp` would read Git state successfully on any
//! host where `/tmp` happens to sit inside a checkout, and the tests that
//! assert the *missing-worktree* fallback would then pass without exercising
//! it.

use crate::state::{Daemon, Resolved, ServerCredentials};
use cairn_core::domain::{ObservationType, Project, Session};
use cairn_core::CairnConfig;
use cairn_git::RepoInstance;
use cairn_store::outbox::SyncPolicy;
use cairn_store::repo::NewObservation;
use cairn_store::{repo, Store};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicI64, AtomicUsize};
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

/// A daemon with an empty in-memory store and no server configured.
pub async fn daemon() -> Daemon {
    daemon_with(CairnConfig::default(), ServerCredentials::default()).await
}

/// A daemon with a specific config and server credentials.
pub async fn daemon_with(config: CairnConfig, server: ServerCredentials) -> Daemon {
    let store = Store::open_memory().await.expect("in-memory store");
    let user_id = repo::ensure_local_user(&store).await.expect("local user");
    Daemon {
        store,
        lifecycle_kinds: Arc::new(RwLock::new(HashMap::new())),
        run_id: Uuid::now_v7(),
        config: Arc::new(RwLock::new(config)),
        traits_refreshed: Arc::new(RwLock::new(std::collections::HashMap::new())),
        user_id,
        started_at: chrono::Utc::now(),
        server: Arc::new(RwLock::new(server)),
        repos: Arc::new(RwLock::new(HashMap::new())),
        last_activity: Arc::new(AtomicI64::new(chrono::Utc::now().timestamp_millis())),
        in_flight_captures: Arc::new(AtomicUsize::new(0)),
        sync_drain: Arc::new(tokio::sync::Mutex::new(())),
        outage_cache: Arc::new(tokio::sync::Mutex::new(
            crate::deliver::OutageCache::default(),
        )),
    }
}

/// Root for fixture worktrees: absolute, and guaranteed not to exist.
pub const NOWHERE: &str = "/cairn-fixture-no-such-worktree";

fn worktree(name: &str) -> String {
    format!("{NOWHERE}/{name}")
}

/// A project, named after its directory as real discovery would name it.
pub async fn project(d: &Daemon, name: &str, remote: Option<&str>) -> Project {
    repo::ensure_project(&d.store, &format!("{}/.git", worktree(name)), name, remote)
        .await
        .expect("project")
}

/// A session belonging to `run`, so a test can decide whether it looks like it
/// came from *this* daemon run or a previous one.
///
/// The sync policy is taken from the project rather than forced local: starting
/// a session on a linked project is what queues its provenance (FR-055), and a
/// test about the outbox needs that to actually happen.
pub async fn session_in_run(d: &Daemon, p: &Project, key: &str, run: Uuid) -> Session {
    repo::start_session(
        &d.store,
        repo::StartSession {
            project_id: p.id,
            user_id: d.user_id,
            agent: "claude-code",
            agent_session_key: key,
            branch: "main",
            commit_sha: Some("abc1234"),
            worktree_path: &worktree(&p.name),
            task_id: None,
            daemon_run_id: run,
            policy: SyncPolicy::from_project(p),
        },
    )
    .await
    .expect("session")
}

/// A session belonging to this daemon's current run.
pub async fn session(d: &Daemon, p: &Project, key: &str) -> Session {
    session_in_run(d, p, key, d.run_id).await
}

/// A session on a named branch, for the branch-scoped lookups.
pub async fn session_on_branch(d: &Daemon, p: &Project, key: &str, branch: &str) -> Session {
    repo::start_session(
        &d.store,
        repo::StartSession {
            project_id: p.id,
            user_id: d.user_id,
            agent: "claude-code",
            agent_session_key: key,
            branch,
            commit_sha: Some("abc1234"),
            worktree_path: &worktree(&p.name),
            task_id: None,
            daemon_run_id: d.run_id,
            policy: SyncPolicy::from_project(p),
        },
    )
    .await
    .expect("session")
}

/// End `s`, so it stops being active.
pub async fn end(d: &Daemon, p: &Project, s: &Session) {
    repo::end_session(
        &d.store,
        s.id,
        cairn_core::domain::SessionStatus::Completed,
        Some("done"),
        SyncPolicy::from_project(p),
    )
    .await
    .expect("end session");
}

/// Record one file edit against `session`.
pub async fn observe_edit(d: &Daemon, s: &Session, path: &str) {
    repo::insert_observation(
        &d.store,
        NewObservation {
            session_id: s.id,
            kind: ObservationType::FileChanged,
            branch: "main",
            commit_sha: Some("abc1234"),
            path: Some(path),
            command: None,
            exit_code: None,
            outcome: None,
            summary: &format!("Edited {path}"),
            details: None,
            payload_bytes: 0,
            truncated: false,
        },
    )
    .await
    .expect("observation");
}

/// A `Resolved` for `p`, as a request handler would receive it.
///
/// The worktree path points at a directory that does not exist. That is the
/// interesting case rather than a shortcut: the daemon has to keep working when
/// a checkout has been moved or deleted under it (FR-009).
pub fn resolved(p: &Project) -> Resolved {
    Resolved {
        repo: RepoInstance {
            git_common_dir: PathBuf::from(&p.git_common_dir),
            worktree_path: PathBuf::from(worktree(&p.name)),
            name: p.name.clone(),
            remote: p.repository_remote.clone(),
        },
        policy: SyncPolicy::from_project(p),
        project: p.clone(),
    }
}

/// Re-read a project, so a test asserts on stored state rather than on the
/// value it happened to be handed earlier.
pub async fn reload(d: &Daemon, id: Uuid) -> Project {
    repo::project(&d.store, id).await.expect("project reload")
}

/// A daemon plus a real Git repository, for the handlers that need one.
///
/// Most request handlers record the branch and commit they ran against, because
/// Cairn's data is branch-scoped and repository state is *derived* from Git
/// rather than guessed (Principle VI). So they cannot be exercised over a
/// fabricated path — they need a checkout that answers `git status`.
///
/// A real checkout is still tier 2: `git init` costs a few milliseconds and no
/// Cairn binary is spawned. The instance is seeded straight into the daemon's
/// repository cache, so `Daemon::resolve` finds it without a discovery
/// subprocess.
pub struct Repo {
    pub daemon: Daemon,
    pub dir: tempfile::TempDir,
    pub cwd: String,
}

/// Run `git` in `dir`, with identity and signing supplied by the environment.
///
/// Passing these as variables rather than three `git config` calls per fixture
/// matters: this gate is meant to be fast enough to run on every save, and each
/// avoided subprocess is real time across the whole tier.
fn git(dir: &std::path::Path, args: &[&str]) {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.com")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .output()
        .expect("git runs");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

impl Repo {
    /// A daemon over a fresh repository with one commit on `main`.
    pub async fn new() -> Self {
        Self::with(CairnConfig::default()).await
    }

    pub async fn with(config: CairnConfig) -> Self {
        let daemon = daemon_with(config, ServerCredentials::default()).await;
        let dir = tempfile::TempDir::new().expect("temp repo");
        let path = dir.path();
        git(path, &["init", "--initial-branch=main"]);
        std::fs::write(path.join("README.md"), "# fixture\n").expect("write");
        git(path, &["add", "."]);
        git(path, &["commit", "--no-gpg-sign", "-m", "init"]);

        // Canonicalized, because macOS hands out `/var/…` temp paths that Git
        // reports back as `/private/var/…`. A mismatch here would make the
        // seeded cache entry unreachable and send every call to discovery.
        let cwd = std::fs::canonicalize(path)
            .expect("canonical repo path")
            .display()
            .to_string();
        let instance = cairn_git::discover(std::path::Path::new(&cwd)).expect("discover");
        daemon
            .repos
            .write()
            .await
            .insert(cwd.clone(), instance.clone());

        Self { daemon, dir, cwd }
    }

    /// Check out a new branch, so branch-scoped behaviour can be driven.
    pub fn checkout(&self, branch: &str) {
        git(self.dir.path(), &["checkout", "-q", "-b", branch]);
    }

    /// Write a file into the working tree.
    pub fn write(&self, rel: &str, contents: &str) {
        let path = self.dir.path().join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("mkdir");
        }
        std::fs::write(path, contents).expect("write");
    }
}
