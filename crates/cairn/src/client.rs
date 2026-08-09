//! Talking to `cairnd` over the local socket, with automatic daemon start
//! (FR-046).

use cairn_core::wire::{codes, Envelope, Request, WireError};
use std::path::PathBuf;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

/// How long to wait for a freshly spawned daemon to bind its socket.
const DAEMON_START_TIMEOUT: Duration = Duration::from_millis(3000);
const DAEMON_POLL: Duration = Duration::from_millis(25);

pub fn socket_path() -> PathBuf {
    cairn_core::paths::socket_path()
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
            start_daemon()?;
            wait_for_daemon()
                .await
                .ok_or_else(|| WireError::new(codes::DAEMON_UNAVAILABLE, "cairnd did not start"))?
        }
    };
    let (_read_half, mut write_half) = stream.into_split();
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
            start_daemon()?;
            wait_for_daemon()
                .await
                .ok_or_else(|| WireError::new(codes::DAEMON_UNAVAILABLE, "cairnd did not start"))?
        }
    };
    converse(stream, request).await
}

async fn converse(stream: UnixStream, request: &Request) -> Result<serde_json::Value, WireError> {
    let (read_half, mut write_half) = stream.into_split();
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

async fn connect() -> Option<UnixStream> {
    UnixStream::connect(socket_path()).await.ok()
}

/// Spawn `cairnd` detached, so the agent session is never waiting on it.
pub fn start_daemon() -> Result<(), WireError> {
    let exe = daemon_binary();
    std::process::Command::new(&exe)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
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

/// Find `cairnd`: next to this binary first, then `PATH`.
fn daemon_binary() -> PathBuf {
    if let Ok(explicit) = std::env::var("CAIRND_BIN") {
        if !explicit.is_empty() {
            return PathBuf::from(explicit);
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let sibling = dir.join("cairnd");
            if sibling.exists() {
                return sibling;
            }
        }
    }
    PathBuf::from("cairnd")
}

async fn wait_for_daemon() -> Option<UnixStream> {
    let deadline = std::time::Instant::now() + DAEMON_START_TIMEOUT;
    while std::time::Instant::now() < deadline {
        if let Some(stream) = connect().await {
            return Some(stream);
        }
        tokio::time::sleep(DAEMON_POLL).await;
    }
    None
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
pub fn send_oneway_blocking(request: &Request, deadline: Duration) -> Result<(), WireError> {
    use std::io::Write as _;
    use std::os::unix::net::UnixStream as StdUnixStream;

    let started = std::time::Instant::now();
    let path = socket_path();

    let mut stream = match StdUnixStream::connect(&path) {
        Ok(s) => s,
        Err(_) => {
            // Cold daemon: start it, then wait — bounded by the deadline.
            start_daemon()?;
            loop {
                if started.elapsed() >= deadline {
                    return Err(WireError::new(
                        codes::DAEMON_UNAVAILABLE,
                        "cairnd did not start within the capture deadline",
                    ));
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
