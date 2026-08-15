//! Test harness for the end-to-end suite.
//!
//! Every test runs against a **real** temporary Git repository, a real SQLite
//! file and a real `cairnd` process. The interesting failure modes all live at
//! the boundary between Git, storage and the agent, where a mock proves
//! nothing (D13).

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::TempDir;

/// An isolated Cairn installation: its own state directory, socket, daemon and
/// Git repository.
pub struct Sandbox {
    pub home: TempDir,
    pub repo: TempDir,
    pub socket: PathBuf,
}

impl Sandbox {
    /// Create a sandbox with an initialized repository holding one commit.
    pub fn new() -> Self {
        let home = TempDir::new().expect("home");
        let repo = TempDir::new().expect("repo");
        let socket = sandbox_socket();

        let home_path = home.path().to_path_buf();
        let s = Self { home, repo, socket };
        s.git(&["init", "--initial-branch=main"]);
        s.git(&["config", "user.email", "test@example.com"]);
        s.git(&["config", "user.name", "Cairn Test"]);
        s.git(&["config", "commit.gpgsign", "false"]);
        s.write_file("README.md", "# fixture\n");
        s.git(&["add", "."]);
        s.git(&["commit", "-m", "init", "--no-gpg-sign"]);

        // Give hooks generous deadlines *in tests only*.
        //
        // The suite runs many sandboxes at once, each with its own daemon and a
        // process per hook, which saturates a laptop and makes hooks miss the
        // 250 ms production deadline. Dropping work then is correct fail-soft
        // behaviour, but it turns semantic tests into load tests. The real
        // deadlines are asserted where they belong: the fail-soft test drives an
        // unreachable daemon, and the reduced-context test sets a 1 ms deadline
        // deliberately.
        std::fs::create_dir_all(&home_path).expect("home");
        std::fs::write(
            home_path.join("config.json"),
            serde_json::json!({
                "capture_deadline_ms": 5000,
                "context_deadline_ms": 15000,
            })
            .to_string(),
        )
        .expect("write config");

        // Warm the daemon before any hook fires, as `SessionStart` does in
        // practice.
        let started = s.cairn(&["init"]);
        assert!(started.ok(), "sandbox init failed: {}", started.stderr);
        s
    }

    pub fn repo_path(&self) -> &Path {
        self.repo.path()
    }

    pub fn db_path(&self) -> PathBuf {
        self.home.path().join("cairn.sqlite3")
    }

    /// A file SQLite keeps beside the database: `-wal`, `-shm`.
    ///
    /// Composed rather than formatted through `Path::display`, which is lossy for
    /// any path the platform does not hand back as UTF-8 — and would then name a
    /// file that is not the one we meant.
    pub fn sidecar(&self, suffix: &str) -> PathBuf {
        let mut path = self.db_path().into_os_string();
        path.push(suffix);
        PathBuf::from(path)
    }

    pub fn write_file(&self, name: &str, contents: &str) {
        let path = self.repo.path().join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("mkdir");
        }
        std::fs::write(path, contents).expect("write");
    }

    pub fn git(&self, args: &[&str]) -> Output {
        let out = Command::new("git")
            .arg("-C")
            .arg(self.repo.path())
            .args(args)
            .output()
            .expect("git runs");
        assert!(
            out.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        out
    }

    /// Run `cairn` inside the sandbox.
    pub fn cairn(&self, args: &[&str]) -> CliResult {
        self.cairn_with_env(args, &[])
    }

    /// `cairn`, with extra environment for a test that needs to change how the
    /// CLI or the daemon it starts is configured.
    pub fn cairn_with_env(&self, args: &[&str], env: &[(&str, &str)]) -> CliResult {
        let mut command = Command::new(binary("cairn"));
        command
            .args(args)
            .current_dir(self.repo.path())
            .env("CAIRN_HOME", self.home.path())
            .env("CAIRN_SOCKET", &self.socket)
            .env("CAIRND_BIN", binary("cairnd"))
            // Feature 002 writes per-user agent configuration. The sandbox
            // gives it a home of its own so a test can never reach the
            // developer's real `~/.claude` or `~/.codex`.
            .env("HOME", self.fake_home())
            .env("XDG_CONFIG_HOME", self.fake_home().join(".config"));
        for (key, value) in env {
            command.env(key, value);
        }
        let out = command.output().expect("cairn runs");
        CliResult {
            code: out.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&out.stdout).to_string(),
            stderr: String::from_utf8_lossy(&out.stderr).to_string(),
        }
    }

    /// `CAIRN_HOME` — everything Cairn itself writes on this machine.
    pub fn cairn_home(&self) -> std::path::PathBuf {
        self.home.path().to_path_buf()
    }

    /// The per-user home the sandbox's agents live in.
    pub fn fake_home(&self) -> std::path::PathBuf {
        let p = self.home.path().join("fake-home");
        let _ = std::fs::create_dir_all(&p);
        p
    }

    /// The repository this sandbox works in.
    pub fn repo_dir(&self) -> std::path::PathBuf {
        self.repo.path().to_path_buf()
    }

    /// Add a linked worktree of the sandbox repository, on a branch of the same
    /// name, and return its path.
    ///
    /// Kept inside the sandbox's own home so parallel tests never collide on a
    /// shared path — and so it is removed with everything else.
    pub fn add_worktree(&self, name: &str) -> std::path::PathBuf {
        let path = self.home.path().join("worktrees").join(name);
        std::fs::create_dir_all(path.parent().expect("worktrees parent")).expect("worktrees dir");
        self.git(&["worktree", "add", "-b", name, &path.display().to_string()]);
        path
    }

    /// Pretend an agent is installed, by creating the directory its detection
    /// looks for. No vendor binary is involved, which is what keeps these
    /// tests hermetic (FR-204, SC-124).
    pub fn install_agent(&self, agent: &str) {
        let home = self.fake_home();
        let dir = match agent {
            "claude-code" => home.join(".claude"),
            "codex" => home.join(".codex"),
            "opencode" => home.join(".config").join("opencode"),
            "cc-switch" => home.join(".cc-switch"),
            other => panic!("unknown agent {other}"),
        };
        std::fs::create_dir_all(&dir).expect("agent home");
    }

    /// A content hash of every file under the repository and the fake home.
    ///
    /// Used to prove an operation wrote nothing: comparing the listing as well
    /// as the contents catches a temporary file that was created and removed.
    pub fn checksum_tree(&self) -> std::collections::BTreeMap<String, String> {
        let mut out = std::collections::BTreeMap::new();
        for root in [self.repo.path().to_path_buf(), self.fake_home()] {
            collect_tree(&root, &root, &mut out);
        }
        out
    }

    /// Run `cairn`, requiring it to succeed.
    ///
    /// Fixture setup that fails must fail *here*, naming the command and its
    /// output. An ignored setup failure does not stay quiet: it comes back much
    /// later as an assertion about state that was never created — "the memory
    /// did not reach the server" when the memory was never written at all.
    pub fn must(&self, args: &[&str]) -> CliResult {
        let result = self.cairn(args);
        assert!(
            result.ok(),
            "`cairn {}` failed with exit {}\nstdout: {}\nstderr: {}",
            args.join(" "),
            result.code,
            result.stdout.trim(),
            result.stderr.trim()
        );
        result
    }

    /// Run `cairn --json` and parse the envelope's `data`.
    pub fn json(&self, args: &[&str]) -> serde_json::Value {
        let mut full = vec!["--json"];
        full.extend_from_slice(args);
        let result = self.cairn(&full);
        let envelope: serde_json::Value =
            serde_json::from_str(&result.stdout).unwrap_or_else(|e| {
                panic!("unparsable envelope from {args:?}: {e}\n{}", result.stdout)
            });
        assert_eq!(
            envelope["ok"],
            true,
            "{args:?} failed: {}",
            serde_json::to_string(&envelope["error"]).unwrap_or_default()
        );
        envelope["data"].clone()
    }

    /// Read a handoff, waiting for one a sealed boundary still owes.
    ///
    /// A hook-driven `SessionEnd` is acknowledged after termination is
    /// durably recorded, and its handoff is produced immediately afterwards
    /// rather than inside the request (FR-240, D22) — that is what makes a
    /// vendor's one-second handler budget survivable without giving up the
    /// completion guarantee. The handoff's *substance* is unchanged; only the
    /// moment it becomes readable is, and the documented bound is five
    /// seconds on a running daemon.
    ///
    /// `args` are the arguments after `handoff show`, e.g. `["--session", id]`.
    pub fn handoff_after_close(&self, args: &[&str]) -> serde_json::Value {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let mut full = vec!["--json", "handoff", "show"];
        full.extend_from_slice(args);
        loop {
            let result = self.cairn(&full);
            if let Ok(envelope) = serde_json::from_str::<serde_json::Value>(&result.stdout) {
                if envelope["ok"] == true {
                    let handoff = envelope["data"]["handoff"].clone();
                    if !handoff.is_null() {
                        return handoff;
                    }
                }
            }
            assert!(
                std::time::Instant::now() < deadline,
                "no handoff within the documented bound after a sealed close: `cairn {}`",
                full.join(" ")
            );
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
    }

    /// Run `cairn --json`, expecting failure, and return the error object.
    pub fn json_err(&self, args: &[&str]) -> serde_json::Value {
        let mut full = vec!["--json"];
        full.extend_from_slice(args);
        let result = self.cairn(&full);
        let envelope: serde_json::Value = serde_json::from_str(&result.stdout).expect("envelope");
        assert_eq!(envelope["ok"], false, "expected failure from {args:?}");
        envelope["error"].clone()
    }

    /// Deliver a Claude Code hook event with the given payload.
    pub fn hook(&self, event: &str, payload: serde_json::Value) -> CliResult {
        self.hook_as("claude-code", event, payload)
    }

    /// Drive a hook for a named adapter.
    ///
    /// Claude Code's entry is `cairn hook <Event>`, unchanged from Feature
    /// 001; the others name themselves, because the same event word means a
    /// different payload shape to a different vendor.
    pub fn hook_as(&self, agent: &str, event: &str, payload: serde_json::Value) -> CliResult {
        self.hook_in(&self.repo_dir(), agent, event, payload)
    }

    /// Drive a hook from a specific directory.
    ///
    /// A second worktree of the same repository is a different directory, and
    /// the directory is the only thing that tells the two apart — an agent
    /// working there reports it as its `cwd` exactly like this (US10 #5).
    pub fn hook_in(
        &self,
        dir: &std::path::Path,
        agent: &str,
        event: &str,
        payload: serde_json::Value,
    ) -> CliResult {
        use std::io::Write;
        use std::process::Stdio;

        let mut args: Vec<String> = vec!["hook".into(), event.into()];
        if agent != "claude-code" {
            args.push("--agent".into());
            args.push(agent.into());
        }
        let mut child = Command::new(binary("cairn"))
            .args(&args)
            .current_dir(dir)
            .env("CAIRN_HOME", self.home.path())
            .env("CAIRN_SOCKET", &self.socket)
            .env("CAIRND_BIN", binary("cairnd"))
            .env("HOME", self.fake_home())
            .env("XDG_CONFIG_HOME", self.fake_home().join(".config"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("hook runs");

        let mut body = payload;
        body["cwd"] = serde_json::json!(dir.display().to_string());
        child
            .stdin
            .as_mut()
            .expect("stdin")
            .write_all(body.to_string().as_bytes())
            .expect("write payload");

        let out = child.wait_with_output().expect("hook completes");
        CliResult {
            code: out.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&out.stdout).to_string(),
            stderr: String::from_utf8_lossy(&out.stderr).to_string(),
        }
    }

    /// Wait until `predicate` holds, or fail.
    ///
    /// Capture is fire-and-forget (H3), so a hook returns before the daemon has
    /// written the observation. Polling is how a caller observes that write —
    /// and a timeout here is a real failure, not a smoothed-over one.
    pub fn settle<F>(&self, what: &str, predicate: F)
    where
        F: Fn(&Self) -> bool,
    {
        self.settle_within(what, std::time::Duration::from_secs(5), predicate);
    }

    /// `settle` with an explicit deadline.
    ///
    /// The sync worker backs off after a transient failure, so a test that
    /// deliberately takes the server away and gives it back can be waiting on a
    /// worker that is mid-backoff rather than on a defect.
    pub fn settle_within<F>(&self, what: &str, timeout: std::time::Duration, predicate: F)
    where
        F: Fn(&Self) -> bool,
    {
        let deadline = std::time::Instant::now() + timeout;
        while std::time::Instant::now() < deadline {
            if predicate(self) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        panic!("timed out waiting for: {what}");
    }

    /// Wait until the project has recorded `n` observations.
    pub fn settle_observations(&self, n: i64) {
        self.settle(&format!("{n} observations"), |s| {
            s.json(&["status"])["observation_count"].as_i64() == Some(n)
        });
    }

    /// Wait until the project has at least `n` sessions.
    pub fn settle_session_count(&self, n: usize) {
        self.settle(&format!("{n} session(s)"), |s| {
            s.json(&["session", "list"])["sessions"]
                .as_array()
                .map(|a| a.len() >= n)
                .unwrap_or(false)
        });
    }

    /// Wait until the newest session has recorded a turn checkpoint.
    ///
    /// `Stop` is capture class and fire-and-forget by contract (FR-015,
    /// FR-193): the hook returns before the write lands, so a test that reads
    /// immediately is racing Cairn's own deadline rather than testing it.
    pub fn settle_turn_checkpoint(&self) {
        self.settle("a turn checkpoint", |s| {
            s.json(&["session", "list"])["sessions"][0]["last_turn_ended_at"].is_string()
        });
    }

    /// Wait until a handoff with `trigger` is readable.
    ///
    /// `PreCompact` is requested fire-and-forget, so the write lands shortly
    /// after the hook returns.
    pub fn settle_handoff(&self, trigger: &str) {
        self.settle(&format!("a {trigger} handoff"), |s| {
            let result = s.cairn(&["--json", "handoff", "show"]);
            let envelope: serde_json::Value =
                serde_json::from_str(&result.stdout).unwrap_or(serde_json::Value::Null);
            envelope["data"]["handoff"]["trigger"].as_str() == Some(trigger)
        });
    }

    /// Wait until the newest session reaches `status`.
    pub fn settle_session_status(&self, status: &str) {
        self.settle(&format!("session status {status}"), |s| {
            s.json(&["session", "list"])["sessions"][0]["status"].as_str() == Some(status)
        });
    }

    /// Rewrite the hook deadlines this sandbox runs under.
    ///
    /// A deadline of one millisecond is how an unresponsive handler is induced
    /// without making the test depend on machine load.
    pub fn set_deadlines(&self, capture_ms: u64, context_ms: u64) {
        std::fs::write(
            self.home.path().join("config.json"),
            serde_json::json!({
                "capture_deadline_ms": capture_ms,
                "context_deadline_ms": context_ms,
            })
            .to_string(),
        )
        .expect("write config");
    }

    /// Stop the daemon and wait until it has actually let go of the socket.
    ///
    /// `daemon stop` returns as soon as the shutdown is requested, so a caller
    /// that goes straight on to touching the store would be racing a process
    /// that still has it open. The budget is the same 5 seconds `settle` allows,
    /// which is generous enough that a loaded machine cannot make this flake and
    /// short enough that a daemon which really is stuck is reported as such.
    pub fn stop_daemon(&self) {
        self.cairn(&["daemon", "stop"]);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            if !daemon_listening(&self.socket) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        panic!("the daemon is still listening on {}", self.socket.display());
    }

    /// Stop and restart the daemon — the deterministic session boundary (D16).
    pub fn restart_daemon(&self) {
        self.stop_daemon();
        // The next command starts it again automatically (FR-046).
        self.cairn(&["daemon", "start"]);
    }

    /// Observation identifiers, oldest first, read from the local database.
    ///
    /// Observations are deliberately local and have no listing command, so a
    /// test that needs one reads the real store rather than inventing an id.
    pub fn observation_ids(&self) -> Vec<String> {
        self.query_column(
            "SELECT id FROM observations WHERE deleted_at IS NULL ORDER BY occurred_at, id",
        )
    }

    /// Run a single-column query against the local store.
    pub fn query_column(&self, sql: &str) -> Vec<String> {
        let path = self.db_path();
        let sql = sql.to_string();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        runtime.block_on(async move {
            let url = format!("sqlite://{}?mode=ro", path.display());
            let pool = sqlx::SqlitePool::connect(&url).await.expect("open store");
            let rows: Vec<String> = sqlx::query_scalar(&sql)
                .fetch_all(&pool)
                .await
                .expect("query");
            pool.close().await;
            rows
        })
    }

    /// Execute a statement against the local store, for fixtures that must
    /// corrupt state deliberately.
    pub fn execute_sql(&self, sql: &str) {
        let path = self.db_path();
        let sql = sql.to_string();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        runtime.block_on(async move {
            let url = format!("sqlite://{}", path.display());
            let pool = sqlx::SqlitePool::connect(&url).await.expect("open store");
            sqlx::query(&sql).execute(&pool).await.expect("execute");
            pool.close().await;
        });
    }

    /// `PRAGMA integrity_check` against the real database file.
    pub fn integrity_check(&self) -> String {
        self.query_column("PRAGMA integrity_check")
            .first()
            .cloned()
            .unwrap_or_else(|| "missing".to_string())
    }

    /// Raw bytes of the database file, for asserting what is *not* in it.
    pub fn db_bytes(&self) -> Vec<u8> {
        // Checkpoint the WAL so recent writes are in the main file.
        self.cairn(&["status"]);
        let mut bytes = std::fs::read(self.db_path()).unwrap_or_default();
        for suffix in ["-wal", "-shm"] {
            if let Ok(mut more) = std::fs::read(self.sidecar(suffix)) {
                bytes.append(&mut more);
            }
        }
        bytes
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        // Never panic while unwinding. A panic in a destructor is not
        // recoverable: it aborts the whole test binary, replacing whichever
        // assertion actually failed with a SIGABRT and a core dump. Shutting
        // the daemon down is best effort, so it is guarded rather than
        // asserted.
        if let Some(exe) = try_binary("cairn") {
            let _ = Command::new(exe)
                .args(["daemon", "stop"])
                .current_dir(self.repo.path())
                .env("CAIRN_HOME", self.home.path())
                .env("CAIRN_SOCKET", &self.socket)
                .output();
        }
        // Named pipes have no filesystem entry to clean up; only Unix leaves
        // a stale socket file behind when the daemon above did not remove it.
        #[cfg(unix)]
        {
            let _ = std::fs::remove_file(&self.socket);
        }
    }
}

impl Default for Sandbox {
    fn default() -> Self {
        Self::new()
    }
}

pub struct CliResult {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl CliResult {
    pub fn ok(&self) -> bool {
        self.code == 0
    }
}

/// Locate a workspace binary next to the test executable, or `None`.
///
/// For callers that must not panic — notably `Drop`.
pub fn try_binary(name: &str) -> Option<PathBuf> {
    let mut dir = std::env::current_exe().ok()?;
    dir.pop(); // deps/
    if dir.ends_with("deps") {
        dir.pop();
    }
    let candidate = dir.join(binary_file_name(name));
    candidate.exists().then_some(candidate)
}

/// Locate a workspace binary next to the test executable.
pub fn binary(name: &str) -> PathBuf {
    let mut dir = std::env::current_exe().expect("test exe");
    dir.pop(); // deps/
    if dir.ends_with("deps") {
        dir.pop();
    }
    let candidate = dir.join(binary_file_name(name));
    assert!(
        candidate.exists(),
        "{} not built; run `cargo build --workspace` first",
        candidate.display()
    );
    candidate
}

fn binary_file_name(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

/// A fresh socket path (Unix) or pipe name (Windows). Usable both for a
/// `Sandbox` and for a test that drives `cairnd` directly.
#[cfg(unix)]
pub fn sandbox_socket() -> PathBuf {
    // Unix socket paths are length-limited; keep this one short.
    std::env::temp_dir().join(format!("cairn-t-{}-{}.sock", std::process::id(), unique()))
}

/// Stops the daemon serving a hand-rolled socket, and removes the socket file.
///
/// [`Sandbox`] does this in its own `Drop`, so anything built on `Sandbox` is
/// already covered. A test that takes a bare [`sandbox_socket`] instead — to
/// drive `cairn` against a deliberately broken environment, say — has no such
/// owner, and every one of those was leaking: `cairn` spawns `cairnd` *before*
/// the request fails, so a daemon bound the socket and then nothing ever stopped
/// it. The socket lives in the system temp directory rather than inside the
/// test's own `TempDir`, so it outlived the test too.
///
/// Left running they accumulate across runs — 18 of them on the machine where
/// this was found — competing for CPU with the suite that spawned them and
/// making timing-sensitive tests fail for a reason that has nothing to do with
/// the code under test.
pub struct DaemonSocket {
    pub path: PathBuf,
}

impl DaemonSocket {
    pub fn new() -> Self {
        Self {
            path: sandbox_socket(),
        }
    }
}

// Stands in for the `PathBuf` these tests used to hold, so adopting the guard
// is a one-line change at each call site rather than a rewrite.
impl std::ops::Deref for DaemonSocket {
    type Target = Path;
    fn deref(&self) -> &Path {
        &self.path
    }
}

impl AsRef<Path> for DaemonSocket {
    fn as_ref(&self) -> &Path {
        &self.path
    }
}

impl AsRef<std::ffi::OsStr> for DaemonSocket {
    fn as_ref(&self) -> &std::ffi::OsStr {
        self.path.as_os_str()
    }
}

impl Default for DaemonSocket {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for DaemonSocket {
    fn drop(&mut self) {
        // Best effort, and never a panic: this runs during unwinding when a
        // test has already failed, and a panic here would replace that failure
        // with a SIGABRT.
        if let Some(exe) = try_binary("cairn") {
            let _ = Command::new(exe)
                .args(["daemon", "stop"])
                .env("CAIRN_SOCKET", &self.path)
                .output();
        }
        #[cfg(unix)]
        {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

#[cfg(windows)]
pub fn sandbox_socket() -> PathBuf {
    PathBuf::from(format!(
        r"\\.\pipe\cairn-t-{}-{}",
        std::process::id(),
        unique()
    ))
}

/// Whether something is currently listening on `socket` — used to poll for a
/// daemon having actually bound (or stopped) rather than assuming a fixed
/// delay was enough.
#[cfg(unix)]
pub fn daemon_listening(socket: &Path) -> bool {
    std::os::unix::net::UnixStream::connect(socket).is_ok()
}

#[cfg(windows)]
pub fn daemon_listening(socket: &Path) -> bool {
    match std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(socket)
    {
        Ok(_) => true,
        // ERROR_PIPE_BUSY: every instance is taken, which still means alive.
        Err(e) => e.raw_os_error() == Some(231),
    }
}

/// A value no two sandboxes can share.
///
/// A timestamp alone is not enough: `SystemTime` is coarser than a nanosecond
/// on macOS, so two sandboxes built in the same instant took the *same socket
/// path* — and then one test's commands reached the other test's daemon and
/// store. Every cross-test oddity in this suite traced back to that.
fn unique() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    nanos.wrapping_mul(1000) ^ COUNTER.fetch_add(1, Ordering::SeqCst)
}

// ---------------------------------------------------------------------------
// MCP driver
// ---------------------------------------------------------------------------

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::process::Stdio;

/// A live `cairn mcp` server speaking JSON-RPC 2.0 over stdio.
pub struct Mcp {
    child: std::process::Child,
    reader: BufReader<std::process::ChildStdout>,
    next_id: i64,
}

impl Mcp {
    pub fn start(s: &Sandbox) -> Self {
        let mut child = Command::new(binary("cairn"))
            .arg("mcp")
            .current_dir(s.repo_path())
            .env("CAIRN_HOME", s.home.path())
            .env("CAIRN_SOCKET", &s.socket)
            .env("CAIRND_BIN", binary("cairnd"))
            // Same fake home as every other entry point: the MCP server is a
            // way into the same daemon, and inheriting the developer's real
            // home would make one process in the sandbox able to escape it.
            .env("HOME", s.fake_home())
            .env("XDG_CONFIG_HOME", s.fake_home().join(".config"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("cairn mcp runs");
        let reader = BufReader::new(child.stdout.take().expect("stdout"));
        Self {
            child,
            reader,
            next_id: 1,
        }
    }

    pub fn call(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        let request = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        let stdin = self.child.stdin.as_mut().expect("stdin");
        writeln!(stdin, "{request}").expect("write");
        stdin.flush().expect("flush");

        let mut line = String::new();
        self.reader.read_line(&mut line).expect("read");
        let response: Value = serde_json::from_str(&line).expect("json-rpc response");
        assert_eq!(response["id"], id);
        response["result"].clone()
    }

    /// Call a tool and return its text content.
    pub fn tool(&mut self, name: &str, mut args: Value, cwd: &str) -> String {
        args["cwd"] = json!(cwd);
        let result = self.call("tools/call", json!({ "name": name, "arguments": args }));
        assert_eq!(result["isError"], false, "{name} failed: {result}");
        result["content"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .to_string()
    }

    /// Call a tool and return the whole `tools/call` result, with the tool's
    /// own JSON body parsed in place of the text block.
    ///
    /// `tool` is the ergonomic form for a test asserting on one string. This is
    /// what a *comparison* needs: `isError`, the content envelope and every
    /// field of the body, so that a dropped or renamed field is visible rather
    /// than swallowed by a `contains` (SC-323).
    pub fn tool_result(&mut self, name: &str, mut args: Value, cwd: &str) -> Value {
        args["cwd"] = json!(cwd);
        let mut result = self.call("tools/call", json!({ "name": name, "arguments": args }));
        if let Some(text) = result["content"][0]["text"].as_str() {
            if let Ok(body) = serde_json::from_str::<Value>(text) {
                result["content"][0]["text"] = body;
            }
        }
        result
    }
}

impl Drop for Mcp {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

// ---------------------------------------------------------------------------
// Server fixture (US6, US7)
// ---------------------------------------------------------------------------

/// A running `cairn-server` against a real PostgreSQL.
///
/// Requires `CAIRN_TEST_DATABASE_URL`. Tests that need it report a clear skip
/// rather than passing vacuously.
pub struct Server {
    pub base: String,
    child: std::process::Child,
}

impl Server {
    /// `None` when no test database is configured.
    // The child is owned by `Server` and reaped in `Drop`; it must outlive
    // `start`, which is exactly what clippy's lint cannot see.
    #[allow(clippy::zombie_processes)]
    pub fn start() -> Option<Self> {
        let url = std::env::var("CAIRN_TEST_DATABASE_URL")
            .ok()
            .filter(|u| !u.is_empty())?;

        // A free port found by probing is only free until someone else takes
        // it. These tests run in parallel and each wants its own server, so two
        // sandboxes can be handed the same port between closing the probe and
        // the server binding it — and the loser exits instead of serving.
        // Retrying with a fresh port is the whole remedy.
        let mut last = String::new();
        for _ in 0..4 {
            match Self::try_start(&url) {
                Ok(server) => return Some(server),
                Err(e) => last = e,
            }
        }
        panic!("cairn-server would not start: {last}");
    }

    fn try_start(url: &str) -> Result<Self, String> {
        let port = {
            let probe = std::net::TcpListener::bind("127.0.0.1:0").expect("free port");
            let port = probe.local_addr().expect("addr").port();
            drop(probe);
            port
        };
        let addr = format!("127.0.0.1:{port}");

        // A small pool per server. Every test in this suite starts its own
        // server against one PostgreSQL, and the production default of ten
        // connections each exhausts it once enough of them run in parallel —
        // the later servers then never come up, failing tests that have nothing
        // wrong with them.
        let mut child = Command::new(binary("cairn-server"))
            .args([
                "--addr",
                &addr,
                "--database-url",
                url,
                "--max-connections",
                "4",
            ])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("cairn-server runs");

        let base = format!("http://{addr}");
        for _ in 0..250 {
            // A server that could not bind is gone, and no amount of polling
            // will bring it back. Notice, and let the caller try another port.
            if let Ok(Some(status)) = child.try_wait() {
                return Err(format!("cairn-server at {base} exited early ({status})"));
            }
            if ureq_get(&format!("{base}/api/health")).is_some() {
                return Ok(Self { base, child });
            }
            std::thread::sleep(std::time::Duration::from_millis(40));
        }
        let _ = child.kill();
        let _ = child.wait();
        Err(format!("cairn-server did not become healthy at {base}"))
    }

    /// Every text value in every table of the shared database, concatenated.
    ///
    /// The endpoints are one view of what reached the server; this is the
    /// other. A privacy assertion made only against the API would pass on a
    /// server that stored something and merely declined to serve it back
    /// (SC-119, SC-133).
    pub fn dump(&self) -> String {
        let url = std::env::var("CAIRN_TEST_DATABASE_URL").expect("a test database");
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        runtime.block_on(async move {
            let pool = sqlx::PgPool::connect(&url).await.expect("open server db");
            let tables: Vec<String> = sqlx::query_scalar(
                "SELECT tablename FROM pg_tables WHERE schemaname = 'public' ORDER BY tablename",
            )
            .fetch_all(&pool)
            .await
            .expect("list tables");

            let mut out = String::new();
            for table in tables {
                // Render every row as JSON so no column type is skipped, and
                // so a value nested inside a JSON column is still visible.
                let sql = format!("SELECT to_jsonb(t)::text FROM \"{table}\" t");
                let rows: Vec<String> = sqlx::query_scalar(&sql)
                    .fetch_all(&pool)
                    .await
                    .unwrap_or_default();
                for row in rows {
                    out.push_str(&table);
                    out.push(' ');
                    out.push_str(&row);
                    out.push('\n');
                }
            }
            pool.close().await;
            out
        })
    }

    /// Register a user and return a fresh personal API token.
    pub fn new_user_token(&self, label: &str) -> String {
        let email = format!("{label}-{}@example.test", unique());
        let body = serde_json::json!({
            "email": email, "display_name": label, "password": "hunter2hunter2"
        });
        self.post_json("/api/auth/register", &body, None);

        let login = self.post_json_raw("/api/auth/login", &body, None);
        let cookie = login
            .1
            .into_iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("set-cookie"))
            .map(|(_, v)| v.split(';').next().unwrap_or_default().to_string())
            .expect("session cookie");

        let created = self.post_json(
            "/api/tokens",
            &serde_json::json!({ "name": label }),
            Some(&cookie),
        );
        created["token"].as_str().expect("token").to_string()
    }

    pub fn post_json(
        &self,
        path: &str,
        body: &serde_json::Value,
        cookie: Option<&str>,
    ) -> serde_json::Value {
        self.post_json_raw(path, body, cookie).0
    }

    fn post_json_raw(
        &self,
        path: &str,
        body: &serde_json::Value,
        cookie: Option<&str>,
    ) -> (serde_json::Value, Vec<(String, String)>) {
        let mut args = vec![
            "-s".to_string(),
            "-D".to_string(),
            "-".to_string(),
            "-X".to_string(),
            "POST".to_string(),
            "-H".to_string(),
            "content-type: application/json".to_string(),
            "-d".to_string(),
            body.to_string(),
        ];
        if let Some(c) = cookie {
            args.push("-H".into());
            args.push(format!("cookie: {c}"));
        }
        args.push(format!("{}{path}", self.base));
        let out = Command::new("curl")
            .args(&args)
            .output()
            .expect("curl runs");
        split_response(&String::from_utf8_lossy(&out.stdout))
    }

    /// GET with a bearer token, returning the parsed body.
    pub fn get_json(&self, path: &str, token: &str) -> serde_json::Value {
        let out = Command::new("curl")
            .args([
                "-s",
                "-H",
                &format!("authorization: Bearer {token}"),
                &format!("{}{path}", self.base),
            ])
            .output()
            .expect("curl runs");
        serde_json::from_slice(&out.stdout).unwrap_or(serde_json::Value::Null)
    }

    /// GET returning the HTTP status code only.
    pub fn get_status(&self, path: &str, token: &str) -> u16 {
        let out = Command::new("curl")
            .args([
                "-s",
                "-o",
                "/dev/null",
                "-w",
                "%{http_code}",
                "-H",
                &format!("authorization: Bearer {token}"),
                &format!("{}{path}", self.base),
            ])
            .output()
            .expect("curl runs");
        String::from_utf8_lossy(&out.stdout)
            .trim()
            .parse()
            .unwrap_or(0)
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn split_response(raw: &str) -> (serde_json::Value, Vec<(String, String)>) {
    let mut headers = Vec::new();
    let mut body_start = 0;
    for (i, line) in raw.split_inclusive('\n').enumerate() {
        body_start += line.len();
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            break;
        }
        if let Some((k, v)) = trimmed.split_once(':') {
            headers.push((k.trim().to_string(), v.trim().to_string()));
        }
        let _ = i;
    }
    let body = &raw[body_start.min(raw.len())..];
    (
        serde_json::from_str(body).unwrap_or(serde_json::Value::Null),
        headers,
    )
}

fn ureq_get(url: &str) -> Option<String> {
    let out = Command::new("curl")
        .args(["-s", "--max-time", "2", url])
        .output()
        .ok()?;
    if out.status.success() && !out.stdout.is_empty() {
        Some(String::from_utf8_lossy(&out.stdout).to_string())
    } else {
        None
    }
}

/// POST a JSON body with a bearer token, returning the parsed response.
///
/// A free function rather than a `Server` method so several threads can drive
/// the same endpoint at once, which is the only way to test a race for real.
pub fn post_json_bearer(
    base: &str,
    path: &str,
    body: &serde_json::Value,
    token: &str,
) -> serde_json::Value {
    let out = Command::new("curl")
        .args([
            "-s",
            "-X",
            "POST",
            "-H",
            "content-type: application/json",
            "-H",
            &format!("authorization: Bearer {token}"),
            "-d",
            &body.to_string(),
            &format!("{base}{path}"),
        ])
        .output()
        .expect("curl runs");
    serde_json::from_slice(&out.stdout).unwrap_or(serde_json::Value::Null)
}

/// Connect a sandbox to a running server with the given token.
pub fn attach_server(s: &Sandbox, server: &Server, token: &str) {
    let result = s.cairn(&["auth", "token", "set", token, "--server", &server.base]);
    assert!(result.ok(), "auth token set failed: {}", result.stderr);
}

/// Every file under `dir`, keyed by its path relative to `root`.
fn collect_tree(
    root: &std::path::Path,
    dir: &std::path::Path,
    out: &mut std::collections::BTreeMap<String, String>,
) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.is_dir() {
            collect_tree(root, &path, out);
        } else {
            let key = format!(
                "{}:{}",
                root.display(),
                path.strip_prefix(root).unwrap_or(&path).display()
            );
            let body = std::fs::read(&path).unwrap_or_default();
            out.insert(key, format!("{:x}", md5_like(&body)));
        }
    }
}

/// A cheap content digest. Not cryptographic — this only has to notice that a
/// file changed.
fn md5_like(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// A store at the **real** v0.1.0-alpha.4 schema, populated with the state a
/// store in use actually carries.
///
/// Feature 003's migration has to be proved against the thing users have, not
/// against a clean database and not against a hand-written approximation of the
/// historical DDL. So this builds the fixture by running migrations 1–4 through
/// `cairn_store::migrate` itself and stops there
/// (migration.md §What an existing store actually contains).
///
/// Everything it writes is fixed — identifiers, timestamps, content — so a test
/// can compare a table byte for byte before and after migrating and attribute
/// any difference to the migration rather than to the fixture.
pub mod alpha4 {
    use std::path::{Path, PathBuf};
    use tempfile::TempDir;

    /// The schema version this fixture stands the store up at.
    pub const SCHEMA: i64 = 4;

    /// Identifiers the fixture uses, so assertions can name a row rather than
    /// rediscover it.
    pub mod ids {
        pub const USER: &str = "00000000-0000-7000-8000-0000000000a0";
        pub const PROJECT_LINKED: &str = "10000000-0000-7000-8000-000000000001";
        pub const PROJECT_UNLINKED: &str = "10000000-0000-7000-8000-000000000002";
        pub const SERVER_PROJECT: &str = "1f000000-0000-7000-8000-0000000000ff";

        pub const TASK_EMPTY_CRITERIA: &str = "20000000-0000-7000-8000-000000000001";
        pub const TASK_DUPLICATE_CRITERIA: &str = "20000000-0000-7000-8000-000000000002";
        pub const TASK_NORMAL: &str = "20000000-0000-7000-8000-000000000003";
        /// Tombstoned, and carrying criteria that must produce **no** rows.
        pub const TASK_DELETED: &str = "20000000-0000-7000-8000-000000000004";

        pub const SESSION_ACTIVE: &str = "30000000-0000-7000-8000-000000000001";
        /// `handoff_pending = 1`: terminal but owed a handoff (D22).
        pub const SESSION_OWED_HANDOFF: &str = "30000000-0000-7000-8000-000000000002";
        pub const SESSION_INTERRUPTED: &str = "30000000-0000-7000-8000-000000000003";
        pub const SESSION_DELETED: &str = "30000000-0000-7000-8000-000000000004";

        pub const OBS_LIVE: &str = "40000000-0000-7000-8000-000000000001";
        /// Tombstoned: a `memory_evidence` row points at it and must keep
        /// resolving to "evidence deleted" (FR-052, FR-505).
        pub const OBS_DELETED: &str = "40000000-0000-7000-8000-000000000002";

        pub const MEM_ACTIVE_A: &str = "50000000-0000-7000-8000-00000000000a";
        pub const MEM_ACTIVE_B: &str = "50000000-0000-7000-8000-00000000000b";
        pub const MEM_STALE: &str = "50000000-0000-7000-8000-00000000000c";
        pub const MEM_LOCAL_ONLY: &str = "50000000-0000-7000-8000-00000000000d";
        /// A supersession chain three links deep: C1 → C2 → C3 → C4 (current).
        pub const MEM_CHAIN_1: &str = "50000000-0000-7000-8000-000000000011";
        pub const MEM_CHAIN_2: &str = "50000000-0000-7000-8000-000000000012";
        pub const MEM_CHAIN_3: &str = "50000000-0000-7000-8000-000000000013";
        pub const MEM_CHAIN_4: &str = "50000000-0000-7000-8000-000000000014";

        pub const HANDOFF_PRE_COMPACT: &str = "60000000-0000-7000-8000-000000000001";
        pub const HANDOFF_SESSION_END: &str = "60000000-0000-7000-8000-000000000002";
        pub const HANDOFF_RECOVERED: &str = "60000000-0000-7000-8000-000000000003";

        pub const OUTBOX_PENDING: &str = "70000000-0000-7000-8000-000000000001";
        pub const OUTBOX_IN_FLIGHT: &str = "70000000-0000-7000-8000-000000000002";
        pub const OUTBOX_DELIVERED: &str = "70000000-0000-7000-8000-000000000003";
        pub const OUTBOX_FAILED: &str = "70000000-0000-7000-8000-000000000004";
    }

    /// Every table an alpha.4 store carries, for row-count and snapshot
    /// assertions. Ordered so a diff reads top-down.
    pub const PRE_EXISTING_TABLES: &[&str] = &[
        "users",
        "projects",
        "tasks",
        "sessions",
        "observations",
        "memories",
        "memory_evidence",
        "handoffs",
        "outbox",
        "sync_meta",
        "agent_integrations",
        "manager_integrations",
        "installed_resources",
        "resource_bindings",
        "capability_evidence",
        "migration_states",
        "recovery_artifacts",
    ];

    /// A populated store standing at schema 4.
    pub struct Alpha4Store {
        dir: TempDir,
    }

    impl Alpha4Store {
        /// Build the fixture: migrations 1–4 through the real path, then the
        /// state of a store in use.
        pub fn build() -> Self {
            let dir = TempDir::new().expect("alpha4 dir");
            let store = Self { dir };
            store.block_on(async {
                let pool = store.open().await;
                cairn_store::migrate::run_to(&pool, SCHEMA)
                    .await
                    .expect("migrations 1-4 apply");
                populate(&pool).await;
                pool.close().await;
            });
            store
        }

        pub fn db_path(&self) -> PathBuf {
            self.dir.path().join("cairn.sqlite3")
        }

        pub fn dir(&self) -> &Path {
            self.dir.path()
        }

        /// Apply every remaining migration — the operation under test.
        pub fn migrate_to_latest(&self) -> i64 {
            self.block_on(async {
                let pool = self.open().await;
                let v = cairn_store::migrate::run(&pool).await.expect("migrate");
                pool.close().await;
                v
            })
        }

        /// Apply migrations up to `target`, refusing a store already past it.
        pub fn migrate_to(&self, target: i64) -> Result<i64, String> {
            self.block_on(async {
                let pool = self.open().await;
                let out = cairn_store::migrate::run_to(&pool, target)
                    .await
                    .map_err(|e| e.to_string());
                pool.close().await;
                out
            })
        }

        pub fn schema_version(&self) -> i64 {
            // Cast in SQL: `query_scalar::<String>` will not decode an INTEGER
            // column, and every reader here wants one string type.
            self.scalar("SELECT CAST(COALESCE(MAX(version), 0) AS TEXT) FROM schema_migrations")
                .parse()
                .expect("version is an integer")
        }

        pub fn row_count(&self, table: &str) -> i64 {
            self.scalar(&format!("SELECT CAST(COUNT(*) AS TEXT) FROM {table}"))
                .parse()
                .expect("count is an integer")
        }

        /// Row counts for every pre-existing table, for the "zero rows lost"
        /// assertion (SC-322).
        pub fn row_counts(&self) -> std::collections::BTreeMap<String, i64> {
            PRE_EXISTING_TABLES
                .iter()
                .map(|t| ((*t).to_string(), self.row_count(t)))
                .collect()
        }

        /// Every value of the named columns, ordered by `id`, as one string per
        /// row — the byte-identity comparison migration.md §Proof asserts.
        pub fn snapshot(&self, table: &str, columns: &[&str]) -> Vec<String> {
            let list = columns
                .iter()
                .map(|c| format!("COALESCE(CAST({c} AS TEXT), '<null>')"))
                .collect::<Vec<_>>()
                .join(" || '\u{1f}' || ");
            self.query_column(&format!("SELECT {list} FROM {table} ORDER BY rowid"))
        }

        pub fn query_column(&self, sql: &str) -> Vec<String> {
            let sql = sql.to_string();
            self.block_on(async {
                let pool = self.open().await;
                let rows: Vec<String> = sqlx::query_scalar(&sql)
                    .fetch_all(&pool)
                    .await
                    .unwrap_or_else(|e| panic!("query {sql:?} failed: {e}"));
                pool.close().await;
                rows
            })
        }

        pub fn scalar(&self, sql: &str) -> String {
            self.query_column(sql)
                .into_iter()
                .next()
                .unwrap_or_default()
        }

        pub fn execute(&self, sql: &str) {
            self.try_execute(sql)
                .unwrap_or_else(|e| panic!("execute {sql:?} failed: {e}"));
        }

        /// Execute, returning the database's error instead of panicking.
        ///
        /// A constraint that is never exercised is a constraint that may be
        /// inert — a `CHECK` calling a JSON1 function, for instance, is
        /// accepted at `CREATE TABLE` whether or not it does anything at
        /// insert. Asserting the refusal is the only way to know.
        pub fn try_execute(&self, sql: &str) -> Result<(), String> {
            let sql = sql.to_string();
            self.block_on(async {
                let pool = self.open().await;
                let out = sqlx::query(&sql)
                    .execute(&pool)
                    .await
                    .map(|_| ())
                    .map_err(|e| e.to_string());
                pool.close().await;
                out
            })
        }

        async fn open(&self) -> sqlx::SqlitePool {
            use sqlx::sqlite::SqliteConnectOptions;
            let options = SqliteConnectOptions::new()
                .filename(self.db_path())
                .create_if_missing(true)
                .foreign_keys(true);
            sqlx::SqlitePool::connect_with(options)
                .await
                .expect("open alpha4 store")
        }

        fn block_on<F: std::future::Future>(&self, f: F) -> F::Output {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("runtime")
                .block_on(f)
        }
    }

    const T0: &str = "2026-01-02T03:04:05Z";
    const T1: &str = "2026-01-03T03:04:05Z";
    const T2: &str = "2026-01-04T03:04:05Z";
    const T3: &str = "2026-01-05T03:04:05Z";

    async fn exec(pool: &sqlx::SqlitePool, sql: &str) {
        sqlx::query(sql)
            .execute(pool)
            .await
            .unwrap_or_else(|e| panic!("fixture statement failed: {e}\n{sql}"));
    }

    /// The state migration.md §What an existing store actually contains names.
    async fn populate(pool: &sqlx::SqlitePool) {
        use ids::*;

        exec(
            pool,
            &format!(
                "INSERT INTO users (id, email, display_name, created_at)
                 VALUES ('{USER}', 'dev@example.com', 'Dev', '{T0}')"
            ),
        )
        .await;

        // One linked project and one that never was — `server_project_id` is
        // set on exactly one of them.
        exec(pool, &format!(
            "INSERT INTO projects (id, name, git_common_dir, repository_remote, linked, server_project_id, created_at, updated_at)
             VALUES ('{PROJECT_LINKED}', 'linked', '/fixture/linked/.git', 'github.com/acme/linked', 1, '{SERVER_PROJECT}', '{T0}', '{T1}'),
                    ('{PROJECT_UNLINKED}', 'unlinked', '/fixture/unlinked/.git', NULL, 0, NULL, '{T0}', '{T0}')"
        )).await;

        // Criteria arrays as they really occur: empty, duplicated, ordinary,
        // and one on a tombstoned task that must produce no criterion rows.
        exec(pool, &format!(
            "INSERT INTO tasks (id, project_id, title, goal, acceptance_criteria, status, created_at, updated_at, deleted_at)
             VALUES ('{TASK_EMPTY_CRITERIA}', '{PROJECT_LINKED}', 'No criteria', 'Ship it', '[]', 'todo', '{T0}', '{T0}', NULL),
                    ('{TASK_DUPLICATE_CRITERIA}', '{PROJECT_LINKED}', 'Repeated criteria', 'Ship it twice',
                     '[\"Do the thing\",\"Do the thing\",\"Then stop\"]', 'in_progress', '{T0}', '{T1}', NULL),
                    ('{TASK_NORMAL}', '{PROJECT_LINKED}', 'Ordinary', 'Ship it once',
                     '[\"Parse the input\",\"Emit the output\"]', 'todo', '{T1}', '{T1}', NULL),
                    ('{TASK_DELETED}', '{PROJECT_LINKED}', '', '', '[\"Gone\",\"Also gone\"]', 'done', '{T0}', '{T2}', '{T2}')"
        )).await;

        exec(pool, &format!(
            "INSERT INTO sessions (id, project_id, task_id, user_id, agent, branch, commit_sha, worktree_path,
                                   agent_session_key, previous_session_id, status, started_at, ended_at, last_event_at,
                                   last_turn_ended_at, daemon_run_id, end_reason, handoff_pending, handoff_attempts,
                                   handoff_error, deleted_at)
             VALUES ('{SESSION_ACTIVE}', '{PROJECT_LINKED}', '{TASK_NORMAL}', '{USER}', 'claude-code', 'main', 'aaaa111',
                     '/fixture/linked', 'agent-key-active', NULL, 'active', '{T1}', NULL, '{T2}', '{T2}',
                     '{SESSION_ACTIVE}', NULL, 0, 0, NULL, NULL),
                    ('{SESSION_OWED_HANDOFF}', '{PROJECT_LINKED}', '{TASK_DUPLICATE_CRITERIA}', '{USER}', 'codex', 'main', 'aaaa111',
                     '/fixture/linked', 'agent-key-owed', NULL, 'completed', '{T0}', '{T1}', '{T1}', NULL,
                     '{SESSION_OWED_HANDOFF}', 'session_end', 1, 2, 'synthesis deferred', NULL),
                    ('{SESSION_INTERRUPTED}', '{PROJECT_LINKED}', NULL, '{USER}', 'opencode', 'feature/x', 'bbbb222',
                     '/fixture/linked', 'agent-key-interrupted', '{SESSION_OWED_HANDOFF}', 'interrupted', '{T0}', '{T1}', '{T1}', NULL,
                     '{SESSION_INTERRUPTED}', 'idle', 0, 0, NULL, NULL),
                    ('{SESSION_DELETED}', '{PROJECT_UNLINKED}', NULL, '{USER}', 'claude-code', 'main', NULL,
                     '/fixture/unlinked', 'agent-key-deleted', NULL, 'completed', '{T0}', '{T0}', '{T0}', NULL,
                     '{SESSION_DELETED}', NULL, 0, 0, NULL, '{T2}')"
        )).await;

        exec(pool, &format!(
            "INSERT INTO observations (id, session_id, type, occurred_at, branch, commit_sha, path, command, exit_code,
                                       outcome, summary, details, payload_bytes, truncated, vendor_tool, deleted_at)
             VALUES ('{OBS_LIVE}', '{SESSION_ACTIVE}', 'test_run', '{T1}', 'main', 'aaaa111', NULL,
                     'cargo test', 0, 'passed', 'suite green', NULL, 32, 0, 'Bash', NULL),
                    ('{OBS_DELETED}', '{SESSION_OWED_HANDOFF}', 'file_changed', '{T0}', 'main', 'aaaa111', 'src/lib.rs',
                     NULL, NULL, NULL, '', NULL, 0, 0, NULL, '{T2}')"
        )).await;

        // The supersession chain is inserted newest-first so each
        // `superseded_by_id` points at a row that already exists.
        exec(pool, &format!(
            "INSERT INTO memories (id, project_id, type, scope, scope_key, content, state, superseded_by_id,
                                   origin_session_id, local_only, created_at, updated_at, deleted_at)
             VALUES ('{MEM_CHAIN_4}', '{PROJECT_LINKED}', 'decision', 'project', '{PROJECT_LINKED}',
                     'The production database is CockroachDB.', 'active', NULL, '{SESSION_ACTIVE}', 0, '{T3}', '{T3}', NULL)"
        )).await;
        exec(pool, &format!(
            "INSERT INTO memories (id, project_id, type, scope, scope_key, content, state, superseded_by_id,
                                   origin_session_id, local_only, created_at, updated_at, deleted_at)
             VALUES ('{MEM_CHAIN_3}', '{PROJECT_LINKED}', 'decision', 'project', '{PROJECT_LINKED}',
                     'The production database is MySQL.', 'superseded', '{MEM_CHAIN_4}', '{SESSION_OWED_HANDOFF}', 0, '{T2}', '{T3}', NULL)"
        )).await;
        exec(pool, &format!(
            "INSERT INTO memories (id, project_id, type, scope, scope_key, content, state, superseded_by_id,
                                   origin_session_id, local_only, created_at, updated_at, deleted_at)
             VALUES ('{MEM_CHAIN_2}', '{PROJECT_LINKED}', 'decision', 'project', '{PROJECT_LINKED}',
                     'The production database is SQLite.', 'superseded', '{MEM_CHAIN_3}', '{SESSION_OWED_HANDOFF}', 0, '{T1}', '{T2}', NULL)"
        )).await;
        exec(pool, &format!(
            "INSERT INTO memories (id, project_id, type, scope, scope_key, content, state, superseded_by_id,
                                   origin_session_id, local_only, created_at, updated_at, deleted_at)
             VALUES ('{MEM_CHAIN_1}', '{PROJECT_LINKED}', 'decision', 'project', '{PROJECT_LINKED}',
                     'The production database is PostgreSQL.', 'superseded', '{MEM_CHAIN_2}', '{SESSION_OWED_HANDOFF}', 0, '{T0}', '{T1}', NULL)"
        )).await;
        exec(pool, &format!(
            "INSERT INTO memories (id, project_id, type, scope, scope_key, content, state, superseded_by_id,
                                   origin_session_id, local_only, created_at, updated_at, deleted_at)
             VALUES ('{MEM_ACTIVE_A}', '{PROJECT_LINKED}', 'convention', 'project', '{PROJECT_LINKED}',
                     'Errors are returned, never logged and swallowed.', 'active', NULL, '{SESSION_ACTIVE}', 0, '{T0}', '{T0}', NULL),
                    ('{MEM_ACTIVE_B}', '{PROJECT_LINKED}', 'fact', 'branch', 'feature/x',
                     'The fixture branch pins the API to port 8080.', 'active', NULL, '{SESSION_INTERRUPTED}', 0, '{T1}', '{T1}', NULL),
                    ('{MEM_STALE}', '{PROJECT_LINKED}', 'fact', 'branch', 'branch/gone',
                     'A branch that no longer resolves recorded this.', 'stale', NULL, '{SESSION_INTERRUPTED}', 0, '{T0}', '{T2}', NULL),
                    ('{MEM_LOCAL_ONLY}', '{PROJECT_LINKED}', 'failure', 'project', '{PROJECT_LINKED}',
                     'A local-only note that must never reach the server.', 'active', NULL, '{SESSION_ACTIVE}', 1, '{T1}', '{T1}', NULL)"
        )).await;

        // One reference to a live observation and one to a tombstoned one.
        exec(pool, &format!(
            "INSERT INTO memory_evidence (memory_id, observation_id, content_digest)
             VALUES ('{MEM_ACTIVE_A}', '{OBS_LIVE}', 'digest-live'),
                    ('{MEM_CHAIN_1}', '{OBS_DELETED}', 'digest-deleted')"
        )).await;

        exec(pool, &format!(
            "INSERT INTO handoffs (id, session_id, trigger, goal, progress, completed_work, remaining_work, changed_files,
                                   decisions, failures, tests_executed, repository_state, next_step, agent_note, evidence,
                                   created_at, deleted_at)
             VALUES ('{HANDOFF_PRE_COMPACT}', '{SESSION_ACTIVE}', 'pre_compact', 'Ship it once', 'parsing done',
                     '[\"parser\"]', '[\"emitter\"]', '[\"src/lib.rs\"]', '[]', '[]', '[]', '{{}}', 'write the emitter', NULL, '[\"{OBS_LIVE}\"]', '{T1}', NULL),
                    ('{HANDOFF_SESSION_END}', '{SESSION_OWED_HANDOFF}', 'session_end', 'Ship it twice', 'stopped',
                     '[]', '[]', '[]', '[]', '[]', '[]', '{{}}', 'resume', 'an agent note', '[]', '{T1}', NULL),
                    ('{HANDOFF_RECOVERED}', '{SESSION_INTERRUPTED}', 'recovered', '', 'interrupted',
                     '[]', '[]', '[]', '[]', '[]', '[]', '{{}}', 'unknown', NULL, '[]', '{T1}', NULL)"
        )).await;

        // All four outbox states, including a claimed `in_flight` row.
        exec(pool, &format!(
            "INSERT INTO outbox (id, project_id, server_project_id, entity_type, entity_id, operation,
                                 idempotency_key, payload, state, attempts, last_error, created_at, delivered_at, claimed_at)
             VALUES ('{OUTBOX_PENDING}', '{PROJECT_LINKED}', '{SERVER_PROJECT}', 'memory', '{MEM_ACTIVE_A}', 'upsert',
                     'idem-pending', '{{\"id\":\"{MEM_ACTIVE_A}\"}}', 'pending', 0, NULL, '{T1}', NULL, NULL),
                    ('{OUTBOX_IN_FLIGHT}', '{PROJECT_LINKED}', '{SERVER_PROJECT}', 'memory', '{MEM_ACTIVE_B}', 'upsert',
                     'idem-in-flight', '{{\"id\":\"{MEM_ACTIVE_B}\"}}', 'in_flight', 1, NULL, '{T1}', NULL, '{T2}'),
                    ('{OUTBOX_DELIVERED}', '{PROJECT_LINKED}', '{SERVER_PROJECT}', 'task', '{TASK_NORMAL}', 'upsert',
                     'idem-delivered', '{{\"id\":\"{TASK_NORMAL}\"}}', 'delivered', 1, NULL, '{T0}', '{T1}', NULL),
                    ('{OUTBOX_FAILED}', '{PROJECT_LINKED}', '{SERVER_PROJECT}', 'handoff', '{HANDOFF_SESSION_END}', 'upsert',
                     'idem-failed', '{{\"id\":\"{HANDOFF_SESSION_END}\"}}', 'failed', 5, 'server refused the content', '{T0}', NULL, NULL)"
        )).await;

        // A cursor part-way through a pull.
        exec(
            pool,
            &format!(
                "INSERT INTO sync_meta (project_id, last_success_at, pull_cursor)
                 VALUES ('{PROJECT_LINKED}', '{T1}', '{T1}')"
            ),
        )
        .await;
    }
}

/// The pre-feature baseline (`tests/knowledge/baseline/`).
///
/// Two later suites need to know what Cairn produced **before** Feature 003
/// existed: `us10_min_safe_context::no_regression` (metric 13, SC-308) and
/// `mcp_backward_compatibility` (metric 36, SC-323). Both are no-regression
/// claims, and a no-regression claim measured against output recaptured after
/// the change proves nothing — so the baseline is captured once, committed, and
/// never regenerated against a Feature 003 build.
///
/// # Why normalized rather than raw
///
/// A response carries identifiers, timestamps and sandbox paths that differ
/// every run. Comparing those would fail for reasons that have nothing to do
/// with regression. [`normalize`] replaces exactly those, by shape, and leaves
/// every other value — including every field *name* — untouched. What survives
/// the normalization is what the comparison is actually about.
pub mod baseline {
    use serde_json::Value;
    use std::path::PathBuf;

    pub fn dir() -> PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("knowledge")
            .join("baseline")
    }

    pub fn path(name: &str) -> PathBuf {
        dir().join(name)
    }

    /// Read a recorded baseline. Absent means the capture step never ran, which
    /// is a broken checkout rather than a passing test.
    pub fn load(name: &str) -> Value {
        let p = path(name);
        let text = std::fs::read_to_string(&p).unwrap_or_else(|e| {
            panic!(
                "baseline {} is missing ({e}); it is committed, not generated at test time",
                p.display()
            )
        });
        serde_json::from_str(&text).expect("baseline is valid JSON")
    }

    /// Write a baseline. Only the ignored capture test calls this.
    pub fn record(name: &str, value: &Value) {
        std::fs::create_dir_all(dir()).expect("baseline dir");
        let text = serde_json::to_string_pretty(value).expect("serializes");
        std::fs::write(path(name), format!("{text}\n")).expect("write baseline");
    }

    /// Replace values that legitimately differ between two runs.
    ///
    /// Field *names* are never touched: a renamed or dropped field is exactly
    /// the regression these baselines exist to catch.
    pub fn normalize(value: &Value) -> Value {
        match value {
            Value::Object(map) => Value::Object(
                map.iter()
                    .map(|(k, v)| (k.clone(), normalize(v)))
                    .collect(),
            ),
            Value::Array(items) => Value::Array(items.iter().map(normalize).collect()),
            Value::String(s) => Value::String(normalize_string(s)),
            other => other.clone(),
        }
    }

    fn normalize_string(s: &str) -> String {
        if is_uuid(s) {
            return "<uuid>".into();
        }
        if is_rfc3339(s) {
            return "<timestamp>".into();
        }
        if is_absolute_path(s) {
            return "<path>".into();
        }
        if is_commit_sha(s) {
            return "<commit>".into();
        }
        if is_sandbox_name(s) {
            return "<sandbox>".into();
        }
        // A string that *contains* one of the above: a rendered briefing, or a
        // response whose text block embeds a whole JSON document. Replace each
        // volatile token in place, preserving every delimiter, so the
        // surrounding prose and every field name still take part in the
        // comparison.
        //
        // Token characters are the ones a volatile value is built from — a
        // UUID's hyphens, a timestamp's colons, a path's separators — so a
        // quoted or bracketed value is still seen whole.
        const IN_TOKEN: fn(char) -> bool =
            |c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':' | '+' | '/' | '\\');

        let mut out = String::with_capacity(s.len());
        let mut token = String::new();
        for ch in s.chars() {
            if IN_TOKEN(ch) {
                token.push(ch);
                continue;
            }
            push_token(&mut out, &token);
            token.clear();
            out.push(ch);
        }
        push_token(&mut out, &token);
        out
    }

    fn push_token(out: &mut String, token: &str) {
        if token.is_empty() {
            return;
        }
        if is_uuid(token) {
            out.push_str("<uuid>");
        } else if is_rfc3339(token) {
            out.push_str("<timestamp>");
        } else if is_absolute_path(token) {
            out.push_str("<path>");
        } else if is_commit_sha(token) {
            out.push_str("<commit>");
        } else if is_sandbox_name(token) {
            out.push_str("<sandbox>");
        } else {
            out.push_str(token);
        }
    }

    fn is_uuid(s: &str) -> bool {
        s.len() == 36
            && s.as_bytes()
                .iter()
                .enumerate()
                .all(|(i, b)| match i {
                    8 | 13 | 18 | 23 => *b == b'-',
                    _ => b.is_ascii_hexdigit(),
                })
    }

    fn is_rfc3339(s: &str) -> bool {
        // `2026-01-02T03:04:05Z` and its offset and fractional forms.
        s.len() >= 20
            && s.as_bytes().get(4) == Some(&b'-')
            && s.as_bytes().get(7) == Some(&b'-')
            && s.as_bytes().get(10).is_some_and(|b| *b == b'T')
            && s.chars().take(4).all(|c| c.is_ascii_digit())
    }

    fn is_absolute_path(s: &str) -> bool {
        s.starts_with('/')
            || s.starts_with("\\\\")
            || (s.len() > 2 && s.as_bytes()[1] == b':' && s.as_bytes()[0].is_ascii_alphabetic())
    }

    /// A full Git object name. The sandbox commits its own fixture, so this
    /// differs every run and says nothing about regression.
    fn is_commit_sha(s: &str) -> bool {
        s.len() == 40 && s.bytes().all(|b| b.is_ascii_hexdigit())
    }

    /// The project name a sandbox derives from its temporary directory —
    /// `.tmpAb3xY9`. Its *length* is fixed, so the token estimate it
    /// contributes to stays comparable while the name itself does not.
    fn is_sandbox_name(s: &str) -> bool {
        s.len() == 10
            && s.starts_with(".tmp")
            && s[4..].bytes().all(|b| b.is_ascii_alphanumeric())
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use serde_json::json;

        #[test]
        fn field_names_survive_normalization() {
            let v = json!({"id": "018f1a2b-3c4d-7e5f-8a9b-0c1d2e3f4a5b", "count": 3});
            let n = normalize(&v);
            assert_eq!(n["id"], "<uuid>");
            assert_eq!(n["count"], 3);
            assert!(n.as_object().unwrap().contains_key("id"));
        }

        #[test]
        fn timestamps_and_paths_are_replaced() {
            let v = json!(["2026-01-02T03:04:05Z", "/tmp/x/y", "src/lib.rs"]);
            let n = normalize(&v);
            assert_eq!(n[0], "<timestamp>");
            assert_eq!(n[1], "<path>");
            assert_eq!(n[2], "src/lib.rs", "a relative path is content, not noise");
        }

        #[test]
        fn a_volatile_token_inside_a_sentence_is_replaced_in_place() {
            let v = json!("session 018f1a2b-3c4d-7e5f-8a9b-0c1d2e3f4a5b ended");
            assert_eq!(normalize(&v), json!("session <uuid> ended"));
        }
    }
}

/// A real local store with a project, for the knowledge suites.
///
/// Feature 003's canonical-knowledge tests need a store and the repository API,
/// not a daemon or a repository on disk: what they exercise is what the *store*
/// decides when a proposal arrives. Driving `cairn` over a socket for that would
/// test the socket.
///
/// The suites that need the whole path — `cairn memory add` through the daemon —
/// use [`Sandbox`] instead, and T042 is where that lands.
pub mod store_fixture {
    use cairn_core::domain::{MemoryScope, MemoryType};
    use cairn_store::knowledge::SubjectRead;
    use cairn_store::outbox::SyncPolicy;
    use cairn_store::repo::{self, CreateOutcome, NewMemory};
    use cairn_store::Store;
    use tempfile::TempDir;
    use uuid::Uuid;

    pub struct Fixture {
        _dir: TempDir,
        pub store: Store,
        pub project: Uuid,
        pub scope_key: String,
    }

    impl Fixture {
        pub async fn new() -> Self {
            let dir = TempDir::new().expect("dir");
            let store = Store::open(&dir.path().join("cairn.sqlite3"))
                .await
                .expect("open");
            let project = Uuid::now_v7();
            let now = "2026-01-02T03:04:05Z".to_string();
            sqlx::query(
                "INSERT INTO projects (id, name, git_common_dir, repository_remote, linked,
                                       server_project_id, created_at, updated_at, deleted_at)
                 VALUES (?1, 'knowledge-fixture', ?2, NULL, 0, NULL, ?3, ?3, NULL)",
            )
            .bind(project.to_string())
            .bind(format!("/fixture/{project}/.git"))
            .bind(&now)
            .execute(store.pool())
            .await
            .expect("project");

            Self {
                _dir: dir,
                store,
                project,
                scope_key: project.to_string(),
            }
        }

        pub fn blocking() -> (tokio::runtime::Runtime, Self) {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("runtime");
            let f = rt.block_on(Self::new());
            (rt, f)
        }

        /// Record a proposal exactly as the store would on a real write.
        pub async fn propose(
            &self,
            session: Uuid,
            topic: Option<&str>,
            value: Option<&str>,
            content: &str,
        ) -> CreateOutcome {
            self.propose_scoped(session, MemoryScope::Project, None, topic, value, content)
                .await
        }

        pub async fn propose_scoped(
            &self,
            session: Uuid,
            scope: MemoryScope,
            scope_key: Option<&str>,
            topic: Option<&str>,
            value: Option<&str>,
            content: &str,
        ) -> CreateOutcome {
            let key = scope_key.unwrap_or(&self.scope_key).to_string();
            repo::create_memory_reconciled(
                &self.store,
                NewMemory {
                    project_id: self.project,
                    kind: MemoryType::Fact,
                    scope,
                    scope_key: &key,
                    content,
                    origin_session_id: session,
                    local_only: false,
                    evidence: &[],
                    topic_key: topic,
                    value_key: value,
                    importance: cairn_core::Importance::Normal,
                },
                SyncPolicy {
                    linked: false,
                    server_project_id: None,
                },
                cairn_store::repo::DEFAULT_RECONCILE_MEMBERS_MAX,
            )
            .await
            .expect("propose")
        }

        pub async fn subject(&self, topic: &str) -> SubjectRead {
            cairn_store::knowledge::subject(
                &self.store,
                self.project,
                MemoryScope::Project,
                &self.scope_key,
                topic,
                cairn_store::repo::DEFAULT_RECONCILE_MEMBERS_MAX,
            )
            .await
            .expect("subject")
        }

        pub async fn count(&self, sql: &str) -> i64 {
            sqlx::query_scalar::<_, i64>(sql)
                .fetch_one(self.store.pool())
                .await
                .unwrap_or_else(|e| panic!("{sql}: {e}"))
        }

        /// Rewrite a memory's clock columns.
        ///
        /// Used only to prove they cannot matter: nothing in the derivation
        /// reads them, and the way to demonstrate that is to move them and show
        /// the answer does not.
        pub async fn set_clock(&self, memory: Uuid, created_at: &str, updated_at: &str) {
            sqlx::query("UPDATE memories SET created_at = ?2, updated_at = ?3 WHERE id = ?1")
                .bind(memory.to_string())
                .bind(created_at)
                .bind(updated_at)
                .execute(self.store.pool())
                .await
                .expect("set clock");
        }
    }
}
