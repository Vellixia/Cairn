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

use crate::state::{Daemon, ServerCredentials};
use cairn_core::domain::{Project, Session};
use cairn_core::CairnConfig;
use cairn_store::{repo, Store};
use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, AtomicUsize};
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

/// Point `CAIRN_HOME` at a throwaway directory, once per test process.
///
/// # Why this exists
///
/// `cairn_core::paths::home()` falls back to the platform data directory when
/// `CAIRN_HOME` is unset, and `Daemon::mutate_credentials` writes `config.json`
/// and the token file through those paths. The store in these fixtures is
/// in-memory, so it looked as though nothing here touched the disk — but the
/// credential half does, and none of the twenty call sites in `sync.rs` set the
/// variable.
///
/// So `cargo test -p cairnd` wrote its fixtures into the developer's **real**
/// Cairn home. Observed on a developer machine: `config.json` carrying
/// `server_url: https://one.example` and a token file containing the literal
/// `token-a`, both straight out of the tests below, which unlinked that machine
/// from its server and left its outbox queued for a host that does not exist.
///
/// **Set once for the whole process, not per test.** `set_var` mutates state
/// every thread shares, and `cargo test` runs tests on many threads; setting it
/// repeatedly would race with the reads it is meant to fix. `Once` makes it a
/// single write, and every fixture funnels through `daemon_with`, so it happens
/// before anything can read a path.
///
/// **The directory is kept, and nothing deletes it.** It has to outlive every
/// test in the process, and a `TempDir` dropped at the end of `call_once` would
/// take it away while the suite was still running. `TempDir::keep` is the API
/// that says so; `mem::forget` would do the same thing while reading like an
/// oversight. Process exit does not remove it either, so each test run leaves
/// one directory of a few kilobytes under the system temp directory — which is
/// the price of not writing into the developer's real home, and is stated here
/// rather than described as cleanup that does not happen.
fn isolate_home() -> &'static std::path::Path {
    static HOME: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
    HOME.get_or_init(|| {
        let dir = tempfile::TempDir::new()
            .expect("a temporary CAIRN_HOME")
            .keep();
        std::env::set_var("CAIRN_HOME", &dir);
        dir
    })
}

/// A daemon with an empty in-memory store and no server configured.
pub async fn daemon() -> Daemon {
    daemon_with(CairnConfig::default(), ServerCredentials::default()).await
}

/// A daemon with a specific config and server credentials.
pub async fn daemon_with(config: CairnConfig, server: ServerCredentials) -> Daemon {
    isolate_home();
    let store = Store::open_memory().await.expect("in-memory store");
    let user_id = repo::ensure_local_user(&store).await.expect("local user");
    Daemon {
        store,
        lifecycle_kinds: Arc::new(RwLock::new(HashMap::new())),
        run_id: Uuid::now_v7(),
        config: Arc::new(RwLock::new(config)),
        user_id,
        started_at: chrono::Utc::now(),
        server: Arc::new(RwLock::new(server)),
        repos: Arc::new(RwLock::new(HashMap::new())),
        last_activity: Arc::new(AtomicI64::new(chrono::Utc::now().timestamp_millis())),
        in_flight_captures: Arc::new(AtomicUsize::new(0)),
        sync_drain: Arc::new(tokio::sync::Mutex::new(())),
        last_observed_instance: Arc::new(RwLock::new(None)),
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
            daemon_run_id: run,
        },
    )
    .await
    .expect("session")
}

/// A session belonging to this daemon's current run.
pub async fn session(d: &Daemon, p: &Project, key: &str) -> Session {
    session_in_run(d, p, key, d.run_id).await
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
    _dir: tempfile::TempDir,
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

        Self {
            daemon,
            _dir: dir,
            cwd,
        }
    }
}

#[cfg(test)]
mod isolation {
    use super::*;

    /// **A fixture must never be able to write into a developer's real Cairn
    /// home.**
    ///
    /// The store these fixtures build is in-memory, which is what made this look
    /// safe for so long. `mutate_credentials` is the half that is not: it writes
    /// `config.json` and the token file through `cairn_core::paths`, and those
    /// fall back to the platform data directory when `CAIRN_HOME` is unset.
    /// Twenty call sites in `sync.rs` do exactly that, and on a real machine they
    /// left `server_url: https://one.example` and a token file reading `token-a`
    /// in the developer's own Cairn home — unlinking it from its server.
    ///
    /// Asserted on the variable rather than by comparing against the real data
    /// directory, because the failure is precisely that the variable is absent:
    /// with it unset every path in this process resolves somewhere real.
    ///
    /// The redirect has to survive a `CAIRN_HOME` the platform allows but
    /// `std::env::var` cannot return.
    ///
    /// A temporary directory inherits `TMPDIR`, which on Unix is free to hold
    /// bytes that are not UTF-8. Resolved through `var` that reads as unset and
    /// every path falls back to the real platform directory — the isolation
    /// would report success while writing to exactly the place it exists to
    /// protect. `cairn_core::paths::home` reads `var_os` for that reason; this
    /// pins the property from the side that depends on it.
    #[test]
    fn the_isolated_home_is_the_one_that_resolves() {
        let isolated = isolate_home();
        assert_eq!(
            cairn_core::paths::home(),
            isolated,
            "CAIRN_HOME names {} but paths resolve elsewhere, so the fixtures \
             are writing somewhere real",
            isolated.display()
        );
    }
}
