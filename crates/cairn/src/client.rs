//! Talking to `cairnd` over the local socket, with automatic daemon start
//! (FR-046).

use cairn_core::wire::{codes, Envelope, Request, WireError};
use std::path::PathBuf;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};

/// How long to wait for a freshly spawned daemon to bind its socket.
const DAEMON_START_TIMEOUT: Duration = Duration::from_millis(3000);
const DAEMON_POLL: Duration = Duration::from_millis(25);

/// The stream type of a connection to `cairnd`: a Unix domain socket on
/// Unix, a named pipe on Windows. Everything below this line only needs
/// `AsyncRead + AsyncWrite`, so the two platforms share one implementation.
#[cfg(unix)]
type IpcStream = tokio::net::UnixStream;
#[cfg(windows)]
type IpcStream = tokio::net::windows::named_pipe::NamedPipeClient;

pub fn socket_path() -> PathBuf {
    cairn_core::paths::socket_path()
}

/// How much of the daemon's log to read back. The interesting part is a line or
/// two; the cap is only so a runaway log cannot be pulled into memory whole.
const LOG_TAIL_LIMIT: u64 = 64 * 1024;

/// Where `cairnd.log` ended before this process spawned a daemon.
///
/// Everything appended after the mark was written by the daemon we started, so
/// that is the only part worth reading when the socket never appears. Lines from
/// an earlier run are somebody else's failure and must not be blamed for this
/// one.
struct DaemonLogMark(u64);

impl DaemonLogMark {
    /// Take the mark. Do this *before* spawning, or the reason we are looking
    /// for may already be behind it.
    fn take() -> Self {
        Self(
            std::fs::metadata(cairn_core::paths::daemon_log_path())
                .map(|m| m.len())
                .unwrap_or(0),
        )
    }

    /// Why the daemon never bound its socket, falling back to `unexplained` when
    /// it left nothing behind to say.
    ///
    /// A store the daemon could not open is the case worth distinguishing: it is
    /// not a missing daemon, and starting one is not the remedy.
    fn diagnose(&self, unexplained: &str) -> WireError {
        match self.store_failure() {
            Some(reason) => {
                let marker = cairn_core::startup::STORE_OPEN_FAILED;
                WireError::new(
                    codes::STORAGE_UNAVAILABLE,
                    if reason.is_empty() {
                        marker.to_string()
                    } else {
                        format!("{marker}: {reason}")
                    },
                )
            }
            None => WireError::new(codes::DAEMON_UNAVAILABLE, unexplained),
        }
    }

    /// The store-open failure the daemon logged, if it logged one.
    fn store_failure(&self) -> Option<String> {
        let appended = self.appended()?;
        // Newest first: a burst of clients can each have started a daemon, and
        // the most recent line is the one that describes the store as it is now.
        appended.lines().rev().find_map(|line| {
            let (_, reason) = line.split_once(cairn_core::startup::STORE_OPEN_FAILED)?;
            Some(reason.trim_start_matches([':', ' ']).trim().to_string())
        })
    }

    /// What the daemon appended to its log since the mark was taken.
    fn appended(&self) -> Option<String> {
        use std::io::{Read, Seek, SeekFrom};

        let mut file = std::fs::File::open(cairn_core::paths::daemon_log_path()).ok()?;
        let len = file.metadata().ok()?.len();
        // A log shorter than the mark was truncated or replaced under us, so the
        // mark means nothing any more; read the whole (bounded) tail instead.
        let from = if len < self.0 { 0 } else { self.0 };
        file.seek(SeekFrom::Start(
            from.max(len.saturating_sub(LOG_TAIL_LIMIT)),
        ))
        .ok()?;
        // Bytes, then a lossy decode. `read_to_string` refuses the whole tail
        // over a single byte that is not UTF-8, and the offset above can land
        // mid-codepoint, so that byte is easy to come by — losing the diagnosis
        // to the encoding of a log we only want to find one line in.
        let mut tail = Vec::new();
        file.take(LOG_TAIL_LIMIT).read_to_end(&mut tail).ok()?;
        Some(String::from_utf8_lossy(&tail).into_owned())
    }
}

/// Send one request, starting the daemon first if it is not running.
pub async fn send(request: &Request) -> Result<serde_json::Value, WireError> {
    send_with_deadline(request, Duration::from_secs(30)).await
}

/// Send one request without waiting for the answer (H3).
///
/// The capture class needs no reply, so it must not pay for one: the hook
/// writes the request to the daemon socket and returns. A Unix domain socket
/// hands the bytes to the receiving buffer on `write`, so the daemon still
/// processes the request after this process exits.
///
/// Delivery stays bounded: connecting and writing are both under `deadline`,
/// and anything slower is dropped rather than waited on.
pub async fn send_oneway(request: &Request, deadline: Duration) -> Result<(), WireError> {
    match tokio::time::timeout(deadline, write_only(request)).await {
        Ok(result) => result,
        Err(_) => Err(WireError::new(
            codes::DAEMON_UNAVAILABLE,
            format!(
                "cairnd did not accept the write within {}ms",
                deadline.as_millis()
            ),
        )),
    }
}

async fn write_only(request: &Request) -> Result<(), WireError> {
    let stream = match connect().await {
        Some(s) => s,
        None => {
            let mark = DaemonLogMark::take();
            start_daemon()?;
            wait_for_daemon(&mark).await?
        }
    };
    let (_read_half, mut write_half) = tokio::io::split(stream);
    let mut line = serde_json::to_string(request)
        .map_err(|e| WireError::invalid(format!("unencodable request: {e}")))?;
    line.push('\n');
    write_half
        .write_all(line.as_bytes())
        .await
        .map_err(|e| WireError::new(codes::DAEMON_UNAVAILABLE, e.to_string()))?;
    write_half
        .flush()
        .await
        .map_err(|e| WireError::new(codes::DAEMON_UNAVAILABLE, e.to_string()))?;
    Ok(())
}

/// Send one request under a hard deadline.
///
/// The hook paths use this: capture gets 250 ms and drops its work when the
/// deadline passes; context gets longer because it must actually answer (D15).
pub async fn send_with_deadline(
    request: &Request,
    deadline: Duration,
) -> Result<serde_json::Value, WireError> {
    match tokio::time::timeout(deadline, exchange(request)).await {
        Ok(result) => result,
        Err(_) => Err(WireError::new(
            codes::DAEMON_UNAVAILABLE,
            format!("cairnd did not answer within {}ms", deadline.as_millis()),
        )),
    }
}

/// Backoff between attempts when the daemon is mid-handover.
///
/// One retry was not enough: a losing daemon serves until its supervisor
/// notices, up to a tick later, so a single 120 ms wait could land on another
/// daemon about to exit. Bounded, and the caller's deadline still caps the
/// whole exchange.
const RETRY_BACKOFF_MS: [u64; 3] = [120, 350, 800];

async fn exchange(request: &Request) -> Result<serde_json::Value, WireError> {
    let mut last = attempt(request).await;
    for wait in RETRY_BACKOFF_MS {
        match last {
            // The daemon may have been restarting underneath us (FR-046).
            Err(ref e) if e.code == codes::DAEMON_UNAVAILABLE => {
                tokio::time::sleep(Duration::from_millis(wait)).await;
                last = attempt(request).await;
            }
            other => return other,
        }
    }
    last
}

async fn attempt(request: &Request) -> Result<serde_json::Value, WireError> {
    let stream = match connect().await {
        Some(s) => s,
        None => {
            let mark = DaemonLogMark::take();
            start_daemon()?;
            wait_for_daemon(&mark).await?
        }
    };
    converse(stream, request).await
}

async fn converse<S>(stream: S, request: &Request) -> Result<serde_json::Value, WireError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let (read_half, mut write_half) = tokio::io::split(stream);
    let mut line = serde_json::to_string(request)
        .map_err(|e| WireError::invalid(format!("unencodable request: {e}")))?;
    line.push('\n');
    write_half
        .write_all(line.as_bytes())
        .await
        .map_err(|e| WireError::new(codes::DAEMON_UNAVAILABLE, e.to_string()))?;
    write_half
        .flush()
        .await
        .map_err(|e| WireError::new(codes::DAEMON_UNAVAILABLE, e.to_string()))?;

    let mut reader = BufReader::new(read_half);
    let mut response = String::new();
    reader
        .read_line(&mut response)
        .await
        .map_err(|e| WireError::new(codes::DAEMON_UNAVAILABLE, e.to_string()))?;
    if response.trim().is_empty() {
        return Err(WireError::new(
            codes::DAEMON_UNAVAILABLE,
            "cairnd closed the connection",
        ));
    }
    let envelope: Envelope = serde_json::from_str(&response)
        .map_err(|e| WireError::new(codes::DAEMON_UNAVAILABLE, format!("bad reply: {e}")))?;
    envelope.into_result()
}

#[cfg(unix)]
async fn connect() -> Option<IpcStream> {
    tokio::net::UnixStream::connect(socket_path()).await.ok()
}

/// `ERROR_PIPE_BUSY` — every instance the daemon has standing is currently
/// serving someone else.
#[cfg(windows)]
const ERROR_PIPE_BUSY: i32 = 231;

/// How long a busy pipe is still considered a live daemon worth waiting for.
///
/// The daemon keeps one instance pending and opens the next as soon as that
/// one is taken, so a burst of clients — twelve hooks at once, say — queues
/// against a single instance and each waits its turn. Every exchange is one
/// line in and one line out, so the queue drains fast; this only has to be
/// longer than the burst, not generous. The caller's own deadline still caps
/// the whole attempt, so a capture hook never spends this budget.
#[cfg(windows)]
const PIPE_BUSY_BUDGET: Duration = Duration::from_secs(3);

/// Open the client end of the named pipe, waiting out a busy one.
///
/// A busy pipe is a healthy daemon with its instances taken, so it is worth
/// retrying. A cold or absent daemon fails with a different error and is
/// reported as "not running" at once, so the caller can start one.
#[cfg(windows)]
async fn connect() -> Option<IpcStream> {
    use tokio::net::windows::named_pipe::ClientOptions;
    let name = socket_path().to_string_lossy().into_owned();
    let deadline = std::time::Instant::now() + PIPE_BUSY_BUDGET;
    loop {
        match ClientOptions::new().open(&name) {
            Ok(client) => return Some(client),
            Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY) => {
                if std::time::Instant::now() >= deadline {
                    return None;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            Err(_) => return None,
        }
    }
}

/// Keep this process's standard handles out of the daemon we are about to
/// spawn.
///
/// Windows `CreateProcess` hands the child *every* handle marked inheritable,
/// not just the three named in `STARTUPINFO`, and Rust's `Command` always
/// asks for inheritance. Redirecting the daemon's own stdio to NUL therefore
/// does not do what it looks like it does: the daemon still receives a
/// duplicate of whatever pipe this process was given for stdout and stderr,
/// and — being a daemon — holds it open long after we exit. Whoever is
/// reading the other end waits for an EOF that never arrives. That is a
/// deadlock for any caller that captures our output: a shell pipeline, the
/// end-to-end suite, or the agent running a capture hook.
///
/// Clearing the inherit flag across the spawn is what makes `Stdio::null()`
/// mean what it says. The previous flags go back on when the guard drops,
/// which is immediately after the spawn.
#[cfg(windows)]
struct StdHandlesNotInherited(Vec<(windows_sys::Win32::Foundation::HANDLE, u32)>);

#[cfg(windows)]
impl StdHandlesNotInherited {
    fn apply() -> Self {
        use windows_sys::Win32::Foundation::{
            GetHandleInformation, SetHandleInformation, HANDLE_FLAG_INHERIT, INVALID_HANDLE_VALUE,
        };
        use windows_sys::Win32::System::Console::{
            GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
        };

        let mut saved = Vec::new();
        for id in [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
            // SAFETY: `GetStdHandle` returns a handle this process already
            // owns (or a sentinel, which is filtered out below); querying and
            // setting its flags borrows it without taking ownership, so
            // nothing here can close a handle still in use.
            unsafe {
                let handle = GetStdHandle(id);
                if handle.is_null() || handle == INVALID_HANDLE_VALUE {
                    continue;
                }
                let mut flags = 0u32;
                if GetHandleInformation(handle, &mut flags) == 0 {
                    continue;
                }
                if flags & HANDLE_FLAG_INHERIT == 0 {
                    continue;
                }
                if SetHandleInformation(handle, HANDLE_FLAG_INHERIT, 0) != 0 {
                    saved.push((handle, flags));
                }
            }
        }
        Self(saved)
    }
}

#[cfg(windows)]
impl Drop for StdHandlesNotInherited {
    fn drop(&mut self) {
        use windows_sys::Win32::Foundation::{SetHandleInformation, HANDLE_FLAG_INHERIT};
        for (handle, flags) in self.0.drain(..) {
            // SAFETY: as in `apply` — the handle is this process's own and is
            // only having a flag restored to what it was.
            unsafe {
                SetHandleInformation(handle, HANDLE_FLAG_INHERIT, flags & HANDLE_FLAG_INHERIT);
            }
        }
    }
}

/// Spawn `cairnd` detached, so the agent session is never waiting on it.
pub fn start_daemon() -> Result<(), WireError> {
    let exe = daemon_binary();
    #[allow(unused_mut)]
    let mut command = std::process::Command::new(&exe);
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    // No console window for a process that has nothing to print to one.
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    // Held across the spawn, and only across the spawn.
    #[cfg(windows)]
    let _handles = StdHandlesNotInherited::apply();
    command
        .spawn()
        .map(|mut child| {
            // The daemon outlives this process. Reap it if it exits during
            // startup so no zombie is left behind.
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        })
        .map_err(|e| {
            WireError::new(
                codes::DAEMON_UNAVAILABLE,
                format!("could not start {}: {e}", exe.display()),
            )
        })
}

#[cfg(unix)]
const DAEMON_NAME: &str = "cairnd";
#[cfg(windows)]
const DAEMON_NAME: &str = "cairnd.exe";

/// Find `cairnd`: next to this binary first, then `PATH`.
fn daemon_binary() -> PathBuf {
    if let Ok(explicit) = std::env::var("CAIRND_BIN") {
        if !explicit.is_empty() {
            return PathBuf::from(explicit);
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let sibling = dir.join(DAEMON_NAME);
            if sibling.exists() {
                return sibling;
            }
        }
    }
    PathBuf::from(DAEMON_NAME)
}

/// Wait for the daemon we just spawned to start answering, and explain it if it
/// never does.
///
/// The log is consulted only once the wait is over, never to cut it short. A
/// burst of clients can start several daemons at once; the ones that lose the
/// socket can log store contention on their way out while the winner is still
/// migrating a fresh database and has not bound yet. Reading the log early would
/// take a loser's transient complaint for the fatal answer and report it instead
/// of waiting the winner out. Waiting first costs a corrupt store the full
/// timeout, once, which is still faster than the four attempts it used to take
/// to arrive at a worse message.
async fn wait_for_daemon(mark: &DaemonLogMark) -> Result<IpcStream, WireError> {
    let deadline = std::time::Instant::now() + DAEMON_START_TIMEOUT;
    while std::time::Instant::now() < deadline {
        if let Some(stream) = connect().await {
            return Ok(stream);
        }
        tokio::time::sleep(DAEMON_POLL).await;
    }
    Err(mark.diagnose("cairnd did not start"))
}

/// True when a daemon is currently listening.
pub async fn daemon_running() -> bool {
    connect().await.is_some()
}

// ---------------------------------------------------------------------------
// Blocking fast path (SC-007)
// ---------------------------------------------------------------------------

/// Write one request with blocking I/O and no async runtime.
///
/// A capture hook runs once per tool call, so the cost of building a Tokio
/// runtime — threads, reactor, timers — is paid 200 times a session for a
/// single `connect` and `write`. This does the same work with `std`.
///
/// Delivery stays bounded: the connect and write both carry `deadline`, and
/// anything slower is dropped rather than waited on.
#[cfg(unix)]
pub fn send_oneway_blocking(request: &Request, deadline: Duration) -> Result<(), WireError> {
    use std::io::Write as _;
    use std::os::unix::net::UnixStream as StdUnixStream;

    let started = std::time::Instant::now();
    let path = socket_path();

    let mut stream = match StdUnixStream::connect(&path) {
        Ok(s) => s,
        Err(_) => {
            // Cold daemon: start it, then wait — bounded by the deadline.
            let mark = DaemonLogMark::take();
            start_daemon()?;
            loop {
                if started.elapsed() >= deadline {
                    // As in `wait_for_daemon`: the log is read only now that
                    // waiting is over, so a daemon that lost the socket cannot
                    // have its complaint mistaken for the answer.
                    return Err(mark.diagnose("cairnd did not start within the capture deadline"));
                }
                match StdUnixStream::connect(&path) {
                    Ok(s) => break s,
                    Err(_) => std::thread::sleep(Duration::from_millis(10)),
                }
            }
        }
    };

    let remaining = deadline.saturating_sub(started.elapsed());
    if remaining.is_zero() {
        return Err(WireError::new(
            codes::DAEMON_UNAVAILABLE,
            "capture deadline exceeded",
        ));
    }
    stream
        .set_write_timeout(Some(remaining))
        .map_err(|e| WireError::new(codes::DAEMON_UNAVAILABLE, e.to_string()))?;

    let mut line = serde_json::to_string(request)
        .map_err(|e| WireError::invalid(format!("unencodable request: {e}")))?;
    line.push('\n');
    stream
        .write_all(line.as_bytes())
        .map_err(|e| WireError::new(codes::DAEMON_UNAVAILABLE, e.to_string()))?;
    stream
        .flush()
        .map_err(|e| WireError::new(codes::DAEMON_UNAVAILABLE, e.to_string()))?;
    Ok(())
}

/// Write one request, bounded by `deadline` (SC-007).
///
/// `std` has no named pipe support, so there is no equivalent of the Unix
/// blocking fast path here: this builds a single-threaded Tokio runtime —
/// far cheaper than the multi-threaded one `#[tokio::main]` builds elsewhere
/// in this binary — and runs the same bounded `send_oneway` on it.
#[cfg(windows)]
pub fn send_oneway_blocking(request: &Request, deadline: Duration) -> Result<(), WireError> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| {
            WireError::new(
                codes::DAEMON_UNAVAILABLE,
                format!("could not start a runtime: {e}"),
            )
        })?;
    runtime.block_on(send_oneway(request, deadline))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Serializes the tests below.
    ///
    /// The environment is process-global while `cargo test` runs a binary's
    /// tests on parallel threads, so two tests each setting and restoring
    /// `CAIRND_BIN` can interleave: one reads the other's temporary value, or
    /// restores a value the other was still relying on. Neither test is wrong
    /// on its own, which is what makes the resulting failure look random.
    static ENV: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Take the env lock, surviving a poisoned mutex from an earlier panic —
    /// a failed test elsewhere should not cascade into these.
    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        ENV.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[test]
    fn cairnd_bin_env_var_wins_over_everything() {
        let _guard = env_lock();
        let prev = std::env::var("CAIRND_BIN").ok();
        std::env::set_var("CAIRND_BIN", "/explicit/cairnd");
        assert_eq!(daemon_binary(), PathBuf::from("/explicit/cairnd"));
        if let Some(v) = prev {
            std::env::set_var("CAIRND_BIN", v);
        } else {
            std::env::remove_var("CAIRND_BIN");
        }
    }

    #[test]
    fn empty_cairnd_bin_is_ignored() {
        let _guard = env_lock();
        let prev = std::env::var("CAIRND_BIN").ok();
        std::env::set_var("CAIRND_BIN", "");
        let got = daemon_binary();
        assert_ne!(
            got,
            PathBuf::from(""),
            "empty env must not yield empty path"
        );
        std::env::remove_var("CAIRND_BIN");
        if let Some(v) = prev {
            std::env::set_var("CAIRND_BIN", v);
        }
    }

    #[test]
    fn socket_path_honours_cairn_socket() {
        let _guard = env_lock();
        let prev = std::env::var("CAIRN_SOCKET").ok();
        std::env::set_var("CAIRN_SOCKET", "/tmp/cairn-test-sock");
        assert_eq!(socket_path(), PathBuf::from("/tmp/cairn-test-sock"));
        match prev {
            Some(v) => std::env::set_var("CAIRN_SOCKET", v),
            None => std::env::remove_var("CAIRN_SOCKET"),
        }
    }
}
