//! `cairnd` — the local daemon.
//!
//! Owns the SQLite store, serves the CLI, the hooks and the MCP server over a
//! local socket, and never touches the network unless a project was explicitly
//! linked.

mod briefing;
mod capture;
mod handlers;
mod handoffs;
mod integrations;
mod recover;
mod state;
mod sync;

use cairn_core::domain::new_id;
use cairn_core::wire::{Envelope, Request, WireError};
use cairn_core::CairnConfig;
use cairn_store::{repo, Store};
use clap::Parser;
use state::{Daemon, ServerCredentials};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::RwLock;

#[derive(Parser, Debug)]
#[command(name = "cairnd", about = "Cairn local daemon", version)]
struct Args {
    /// Socket to listen on. Defaults to `CAIRN_HOME/cairnd.sock`.
    #[arg(long)]
    socket: Option<PathBuf>,
    /// Exit after this many seconds with no request and no active session.
    /// 0 disables the idle timeout; socket-ownership checking is always on.
    #[arg(long, env = "CAIRN_IDLE_TIMEOUT", default_value_t = 0)]
    idle_timeout: u64,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    init_tracing();

    let socket_path = args.socket.unwrap_or_else(cairn_core::paths::socket_path);
    cairn_core::paths::ensure_home()?;

    // Single instance: a live socket means another daemon owns it.
    if socket_path.exists() {
        if UnixStream::connect(&socket_path).await.is_ok() {
            tracing::info!(socket = %socket_path.display(), "another cairnd is already running");
            return Ok(());
        }
        std::fs::remove_file(&socket_path)?;
    }

    let store = Store::open(&cairn_core::paths::db_path()).await?;
    let user_id = repo::ensure_local_user(&store).await?;
    let config = CairnConfig::load();
    let server = ServerCredentials::load(&config);

    let daemon = Arc::new(Daemon {
        store,
        lifecycle_kinds: Arc::new(RwLock::new(Default::default())),
        run_id: new_id(),
        config: Arc::new(RwLock::new(config)),
        user_id,
        started_at: chrono::Utc::now(),
        server: Arc::new(RwLock::new(server)),
        repos: Arc::new(RwLock::new(std::collections::HashMap::new())),
        last_activity: Arc::new(std::sync::atomic::AtomicI64::new(
            chrono::Utc::now().timestamp_millis(),
        )),
        in_flight_captures: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        sync_drain: Arc::new(tokio::sync::Mutex::new(())),
    });

    // Daemon start is the deterministic boundary that reconciles sessions left
    // active by a previous run (FR-009, D16).
    let reconciled = recover::reconcile_previous_runs(&daemon).await;
    if reconciled > 0 {
        tracing::info!(reconciled, "reconciled sessions from a previous run");
    }
    let stale = recover::reconcile_stale_memory(&daemon).await;
    if stale > 0 {
        tracing::info!(stale, "memory marked stale");
    }
    // Queued work a previous run claimed but never delivered is ours again.
    // The backstop for the process dying between the seal and the synthesis —
    // not the only retry path, which is the point of D22 (FR-240).
    let owed = recover::sweep_pending_handoffs(&daemon, std::time::Duration::ZERO).await;
    if owed > 0 {
        tracing::info!(owed, "produced handoffs owed by a previous run");
    }

    let released = recover::release_abandoned_claims(&daemon).await;
    if released > 0 {
        tracing::info!(released, "released outbox claims from a previous run");
    }

    // Bind privately, then rename into place. Removing the path and binding it
    // as two steps lets a second daemon starting at the same moment unlink a
    // socket that is already serving; `rename` publishes atomically instead.
    let staging = socket_path.with_extension(format!("tmp{}", std::process::id()));
    let _ = std::fs::remove_file(&staging);
    // Automatic delivery. Queued work reaches the server without anyone typing
    // `cairn sync now` (FR-056, C1).
    tokio::spawn(sync::run_worker(Arc::clone(&daemon)));

    // Sessions nobody is driving any more. Start-time reconciliation only sees
    // previous runs, so a long-lived daemon needs this to notice one that went
    // quiet under its own run.
    {
        let daemon = Arc::clone(&daemon);
        tokio::spawn(async move {
            let mut ticks = tokio::time::interval(std::time::Duration::from_secs(15 * 60));
            // The first tick fires immediately; that is wanted, since a daemon
            // may be starting after a long absence.
            loop {
                ticks.tick().await;
                let reaped =
                    recover::reap_idle_sessions(&daemon, recover::IDLE_SESSION_TIMEOUT).await;
                // The same tick sweeps any boundary still owing a handoff, so
                // progress does not depend on a restart (FR-240, D22).
                let swept =
                    recover::sweep_pending_handoffs(&daemon, recover::HANDOFF_SWEEP_AFTER).await;
                if swept > 0 {
                    tracing::info!(swept, "produced handoffs owed by sealed boundaries");
                }
                if reaped > 0 {
                    tracing::info!(reaped, "closed idle sessions");
                }
            }
        });
    }

    let listener = UnixListener::bind(&staging)?;

    // Do not clobber a daemon that is already serving.
    //
    // `rename` publishes atomically, but atomically *replacing* a healthy
    // incumbent is the race: its clients keep a live connection to a daemon
    // whose supervisor will notice the theft up to a tick later and exit
    // underneath them, which the client sees as `cairnd closed the connection`.
    // When several daemons start at once — a cold socket and a burst of hooks —
    // the herd steals the socket from each other in turn. Standing down when
    // someone is already answering collapses that to one publisher.
    if tokio::net::UnixStream::connect(&socket_path).await.is_ok() {
        tracing::info!(
            socket = %socket_path.display(),
            "a daemon is already serving this socket; standing down"
        );
        let _ = std::fs::remove_file(&staging);
        return Ok(());
    }
    std::fs::rename(&staging, &socket_path)?;
    let owned = socket_identity(&socket_path);
    tracing::info!(socket = %socket_path.display(), run_id = %daemon.run_id, "cairnd listening");

    let (shutdown_tx, mut shutdown_rx) = tokio::sync::mpsc::channel::<()>(1);

    // A daemon that has lost its socket serves nobody. Without this it lingers
    // forever, which is how orphaned daemons accumulated (H2).
    tokio::spawn(supervise(
        Arc::clone(&daemon),
        socket_path.clone(),
        owned,
        std::time::Duration::from_secs(args.idle_timeout),
        shutdown_tx.clone(),
    ));

    loop {
        tokio::select! {
            accepted = listener.accept() => {
                match accepted {
                    Ok((stream, _)) => {
                        let daemon = Arc::clone(&daemon);
                        let shutdown = shutdown_tx.clone();
                        tokio::spawn(async move {
                            if let Err(e) = serve(daemon, stream, shutdown).await {
                                tracing::debug!(error = %e, "connection ended");
                            }
                        });
                    }
                    Err(e) => tracing::warn!(error = %e, "accept failed"),
                }
            }
            _ = shutdown_rx.recv() => {
                tracing::info!("shutdown requested");
                break;
            }
            _ = tokio::signal::ctrl_c() => {
                tracing::info!("interrupted");
                break;
            }
        }
    }

    let _ = std::fs::remove_file(&socket_path);
    // Bounded: the background sync worker keeps acquiring pool connections, so
    // an unbounded `close()` waits for a loop that never ends and the process
    // never exits (H2).
    let _ = tokio::time::timeout(std::time::Duration::from_secs(2), daemon.store.close()).await;
    Ok(())
}

/// Device and inode of the socket this daemon published, or `None`.
fn socket_identity(path: &std::path::Path) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(path).ok().map(|m| (m.dev(), m.ino()))
}

/// Exit when this daemon stops being the one clients reach, or when it has been
/// idle for `idle_timeout` with no active session.
///
/// `idle_timeout` of zero disables the idle half; ownership is always checked.
async fn supervise(
    daemon: Arc<Daemon>,
    socket_path: std::path::PathBuf,
    owned: Option<(u64, u64)>,
    idle_timeout: std::time::Duration,
    shutdown: tokio::sync::mpsc::Sender<()>,
) {
    const TICK: std::time::Duration = std::time::Duration::from_secs(2);
    loop {
        tokio::time::sleep(TICK).await;

        if owned.is_some() && socket_identity(&socket_path) != owned {
            tracing::info!(
                socket = %socket_path.display(),
                "another daemon owns this socket; exiting"
            );
            let _ = shutdown.send(()).await;
            return;
        }

        if !idle_timeout.is_zero()
            && daemon.idle_for() > idle_timeout
            && daemon.no_active_sessions().await
        {
            tracing::info!(?idle_timeout, "idle with no active session; exiting");
            let _ = shutdown.send(()).await;
            return;
        }
    }
}

/// One connection: newline-delimited JSON requests, one envelope per reply.
async fn serve(
    daemon: Arc<Daemon>,
    stream: UnixStream,
    shutdown: tokio::sync::mpsc::Sender<()>,
) -> anyhow::Result<()> {
    let (read_half, mut write_half) = stream.into_split();
    let mut lines = BufReader::new(read_half).lines();

    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        let envelope = match serde_json::from_str::<Request>(&line) {
            Ok(request) => {
                daemon.touch();
                // Held until this request is answered, so a handoff can wait
                // for captures that have already arrived (H3).
                //
                // A capture now usually arrives as a canonical event rather
                // than a bare `Observe`, and one that is not counted is one a
                // boundary will not wait for (D22 phase two).
                let is_capture = match &request {
                    Request::Observe { .. } => true,
                    Request::CanonicalEvent { event, .. } => !event.event.is_boundary_class(),
                    _ => false,
                };
                let _capture =
                    is_capture.then(|| state::CaptureGuard::new(&daemon.in_flight_captures));
                let stop = matches!(request, Request::DaemonShutdown);
                let reply = handlers::dispatch(&daemon, request).await;
                if stop {
                    let _ = shutdown.send(()).await;
                }
                reply
            }
            Err(e) => Envelope::err(WireError::invalid(format!("unparsable request: {e}"))),
        };
        let mut body = serde_json::to_string(&envelope)?;
        body.push('\n');
        write_half.write_all(body.as_bytes()).await?;
        write_half.flush().await?;
    }
    Ok(())
}

/// Send the daemon's log somewhere a person can read it.
///
/// `cairn` starts the daemon detached with stderr on /dev/null, so for as long
/// as tracing only went to stderr the daemon effectively had no log at all —
/// diagnosing it meant running `cairnd` by hand. It writes to a file now, and
/// still to stderr when someone runs it in the foreground.
fn init_tracing() {
    use tracing_subscriber::{fmt, EnvFilter};
    let filter = EnvFilter::try_from_env("CAIRN_LOG").unwrap_or_else(|_| EnvFilter::new("info"));

    let _ = cairn_core::paths::ensure_home();
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(cairn_core::paths::daemon_log_path());

    match file {
        Ok(file) => {
            let _ = fmt()
                .with_env_filter(filter)
                // A log read with `cat` should not be full of escape codes.
                .with_ansi(false)
                .with_writer(move || file.try_clone().expect("log handle"))
                .try_init();
        }
        // A daemon that cannot open its log still has work to do.
        Err(_) => {
            let _ = fmt()
                .with_env_filter(filter)
                .with_writer(std::io::stderr)
                .try_init();
        }
    }
}
