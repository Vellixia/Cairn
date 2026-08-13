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
