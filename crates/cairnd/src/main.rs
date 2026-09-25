//! `cairnd` — the local daemon.
//!
//! Owns the SQLite store, serves the CLI, the hooks and the MCP server over a
//! local socket, and never touches the network unless a project was explicitly
//! linked.

mod arrival;
mod capture;
mod deliver;
mod handlers;
mod integrations;
mod state;
mod sync;
#[cfg(test)]
mod testsupport;

use cairn_core::domain::new_id;
use cairn_core::wire::{Envelope, Request, WireError};
use cairn_core::CairnConfig;
use cairn_store::{repo, Store};
use clap::Parser;
use state::{Daemon, ServerCredentials};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
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

    let result = run(
        socket_path,
        std::time::Duration::from_secs(args.idle_timeout),
    )
    .await;

    // The log is the only place this can be said. `cairn` spawns us detached
    // with stderr on the null device, so whatever `main` returns is printed to
    // nobody.
    if let Err(ref e) = result {
        tracing::error!(
            error = %one_line(e),
            "{}",
            cairn_core::startup::STARTUP_FAILED
        );
    }
    result
}

/// An error as a single log line.
///
/// The CLI reads these lines back one at a time, so a message that wrapped
/// would arrive truncated at the first newline.
fn one_line(e: &anyhow::Error) -> String {
    e.to_string().replace(['\n', '\r'], " ")
}

/// Open the store, build the daemon state, and run the start-of-day recovery
/// that reconciles whatever a previous run left behind (FR-009, D16).
///
/// Shared by both transports, but each gives it a different guarantee. On
/// Windows the exclusive pipe create *is* the ownership check, so by the time
/// `run` calls this, ownership is real. On Unix `run` only calls it after a
/// cheap pre-check — not proof, since two daemons starting at once can both
/// pass it — and the actual race is settled later, at the rename. A losing
/// Unix daemon can therefore still run this once before standing down; that
/// is wasted work, not a correctness problem, since the loser's process (and
/// everything this spawned) exits immediately after.
async fn setup() -> anyhow::Result<Arc<Daemon>> {
    let (store, user_id, legacy_migration) = open_store().await?;
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
        outage_cache: Arc::new(tokio::sync::Mutex::new(deliver::OutageCache::default())),
        legacy_migration,
    });

    // Automatic delivery. Queued work reaches the server without anyone typing
    // an agent process remaining alive (FR-056, C1).
    tokio::spawn(sync::run_worker(Arc::clone(&daemon)));

    Ok(daemon)
}

/// Open the local store, naming the reason in the log if it cannot be opened.
///
/// A store that will not open is fatal and the daemon exits, so this is the
/// last chance to say why. The line carries the marker the CLI looks for, which
/// is what turns `cairnd did not start` into `storage_unavailable` with the
/// real cause attached (see `cairn_core::startup`).
async fn open_store() -> anyhow::Result<(Store, uuid::Uuid, serde_json::Value)> {
    let opened = async {
        let artifacts = cairn_core::paths::home()
            .join("removed_feature")
            .join("tasks-v1");
        let (store, report) = cairn_store::transfer::bootstrap_legacy_edge(
            &cairn_core::paths::legacy_db_path(),
            &cairn_core::paths::db_path(),
            &artifacts,
        )
        .await?;
        // Part of opening the store as far as a user is concerned: it is the
        // first read and the first write, so a database that is present but
        // unusable fails here rather than at `open`.
        let user_id = repo::ensure_local_user(&store).await?;
        Ok::<_, anyhow::Error>((store, user_id, serde_json::to_value(report)?))
    }
    .await;

    opened.inspect_err(|e| record_store_failure(&one_line(e)))
}

/// Write the store-open failure where the CLI will find it.
///
/// Deliberately not a `tracing` event. Tracing is filtered by `CAIRN_LOG`, and
/// `CAIRN_LOG=off` — or any directive that silences this target — would drop the
/// one line the CLI depends on, putting a damaged store back to being reported
/// as a daemon that never started. This line is a contract (see
/// `cairn_core::startup`), not diagnostics, so no filter gets a say in it. It is
/// shaped like the lines around it so the log still reads as one thing.
///
/// Best effort: a log that cannot be written leaves the CLI with its older,
/// vaguer message, which is what it had before this line existed.
fn record_store_failure(reason: &str) {
    use std::io::Write;

    let line = format!(
        "{} ERROR cairnd: {}: {reason}\n",
        chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.6fZ"),
        cairn_core::startup::STORE_OPEN_FAILED
    );
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(cairn_core::paths::daemon_log_path())
    {
        let _ = file.write_all(line.as_bytes());
    }
}

async fn shutdown_store(daemon: &Daemon) {
    // Bounded: the background sync worker keeps acquiring pool connections, so
    // an unbounded `close()` waits for a loop that never ends and the process
    // never exits (H2).
    let _ = tokio::time::timeout(std::time::Duration::from_secs(2), daemon.store.close()).await;
}

/// Whether the daemon should exit for lack of anything to do.
async fn idle_expired(daemon: &Daemon, idle_timeout: std::time::Duration) -> bool {
    !idle_timeout.is_zero() && daemon.idle_for() > idle_timeout && daemon.no_active_sessions().await
}

// ---------------------------------------------------------------------------
// Unix: a Unix domain socket under CAIRN_HOME.
// ---------------------------------------------------------------------------

#[cfg(unix)]
async fn run(socket_path: PathBuf, idle_timeout: std::time::Duration) -> anyhow::Result<()> {
    use tokio::net::{UnixListener, UnixStream};

    // Single instance: a live socket means another daemon owns it.
    if socket_path.exists() {
        if UnixStream::connect(&socket_path).await.is_ok() {
            tracing::info!(socket = %socket_path.display(), "another cairnd is already running");
            return Ok(());
        }
        std::fs::remove_file(&socket_path)?;
    }

    let daemon = setup().await?;

    // Bind privately, then rename into place. Removing the path and binding it
    // as two steps lets a second daemon starting at the same moment unlink a
    // socket that is already serving; `rename` publishes atomically instead.
    let staging = socket_path.with_extension(format!("tmp{}", std::process::id()));
    let _ = std::fs::remove_file(&staging);
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
    if UnixStream::connect(&socket_path).await.is_ok() {
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
    let arrivals = arrival::Arrivals::new();

    let (shutdown_tx, mut shutdown_rx) = tokio::sync::mpsc::channel::<()>(1);

    // A daemon that has lost its socket serves nobody. Without this it lingers
    // forever, which is how orphaned daemons accumulated (H2).
    tokio::spawn(supervise(
        Arc::clone(&daemon),
        socket_path.clone(),
        owned,
        idle_timeout,
        shutdown_tx.clone(),
    ));

    loop {
        tokio::select! {
            accepted = listener.accept() => {
                match accepted {
                    Ok((stream, _)) => {
                        let daemon = Arc::clone(&daemon);
                        let shutdown = shutdown_tx.clone();
                        // Taken here, where the order is still the order the
                        // hooks ran in: `accept` returns connections in the
                        // order they were made, and everything after this point
                        // is a task that races the others (`arrival`).
                        let ticket = arrivals.take();
                        tokio::spawn(async move {
                            if let Err(e) = serve(daemon, stream, shutdown, ticket).await {
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
    shutdown_store(&daemon).await;
    Ok(())
}

/// Device and inode of the socket this daemon published, or `None`.
#[cfg(unix)]
fn socket_identity(path: &std::path::Path) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(path).ok().map(|m| (m.dev(), m.ino()))
}

/// Exit when this daemon stops being the one clients reach, or when it has been
/// idle for `idle_timeout` with no active session.
///
/// `idle_timeout` of zero disables the idle half; ownership is always checked.
#[cfg(unix)]
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

        if idle_expired(&daemon, idle_timeout).await {
            tracing::info!(?idle_timeout, "idle with no active session; exiting");
            let _ = shutdown.send(()).await;
            return;
        }
    }
}

// ---------------------------------------------------------------------------
// Windows: a named pipe. There is no filesystem entry to stage-and-rename, so
// ownership works differently: `first_pipe_instance` makes the *first* create
// call the exclusive check-and-claim, atomically, with no race to lose.
// ---------------------------------------------------------------------------

#[cfg(windows)]
async fn run(pipe_name: PathBuf, idle_timeout: std::time::Duration) -> anyhow::Result<()> {
    use tokio::net::windows::named_pipe::{NamedPipeServer, PipeMode, ServerOptions};

    let name = pipe_name.to_string_lossy().into_owned();

    let mut server: NamedPipeServer = match ServerOptions::new()
        .first_pipe_instance(true)
        .pipe_mode(PipeMode::Byte)
        .create(name.as_str())
    {
        Ok(server) => server,
        // ERROR_ACCESS_DENIED: another cairnd already holds this pipe name.
        Err(e) if e.raw_os_error() == Some(5) => {
            tracing::info!(pipe = %name, "another cairnd is already running");
            return Ok(());
        }
        Err(e) => return Err(e.into()),
    };

    // The pipe instance exists the moment `create` returns, so a client can
    // open a handle to it immediately — before anyone has called `connect`.
    // `setup` (opening the store, running recovery) can take real time, and
    // leaving that first connection unserviced for all of it is exactly the
    // kind of gap a client should never have to wait out. Run both
    // concurrently instead of one after the other, so the accept is already
    // in flight for as much of `setup` as possible.
    let (daemon, first_connect) = tokio::join!(setup(), server.connect());
    let daemon = daemon?;
    first_connect?;
    tracing::info!(pipe = %name, run_id = %daemon.run_id, "cairnd listening");
    let arrivals = arrival::Arrivals::new();

    let (shutdown_tx, mut shutdown_rx) = tokio::sync::mpsc::channel::<()>(1);
    tokio::spawn(supervise(
        Arc::clone(&daemon),
        idle_timeout,
        shutdown_tx.clone(),
    ));

    // The join above already delivered this first connection; hand it off
    // and open the next instance before entering the steady-state loop.
    {
        let handled = server;
        server = ServerOptions::new()
            .pipe_mode(PipeMode::Byte)
            .create(name.as_str())?;
        let daemon = Arc::clone(&daemon);
        let shutdown = shutdown_tx.clone();
        // The first connection is still a connection, and it is the *earliest*
        // one: skipping it here would leave the hook that opened this daemon
        // racing every hook behind it (`arrival`).
        let ticket = arrivals.take();
        tokio::spawn(async move {
            if let Err(e) = serve(daemon, handled, shutdown, ticket).await {
                tracing::debug!(error = %e, "connection ended");
            }
        });
    }

    loop {
        tokio::select! {
            connected = server.connect() => {
                let handled = server;
                // Stand up the next instance before handling (or discarding)
                // this one, so a client arriving meanwhile is never refused.
                server = match ServerOptions::new().pipe_mode(PipeMode::Byte).create(name.as_str())
                {
                    Ok(next) => next,
                    Err(e) => {
                        tracing::error!(error = %e, "could not open the next pipe instance");
                        break;
                    }
                };
                match connected {
                    Ok(()) => {
                        let daemon = Arc::clone(&daemon);
                        let shutdown = shutdown_tx.clone();
                        // As on Unix, and for the same reason: a named pipe
                        // hands waiting clients over in the order they
                        // connected, and that order is not recoverable once
                        // each is a task of its own (`arrival`).
                        let ticket = arrivals.take();
                        tokio::spawn(async move {
                            if let Err(e) = serve(daemon, handled, shutdown, ticket).await {
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

    shutdown_store(&daemon).await;
    Ok(())
}

/// Exit when this daemon has been idle for `idle_timeout` with no active
/// session.
///
/// There is no Unix-style theft check: a named pipe cannot be silently
/// replaced out from under the process holding it the way a socket *file*
/// can, so the exclusive create in `run` is the only ownership check needed.
#[cfg(windows)]
async fn supervise(
    daemon: Arc<Daemon>,
    idle_timeout: std::time::Duration,
    shutdown: tokio::sync::mpsc::Sender<()>,
) {
    const TICK: std::time::Duration = std::time::Duration::from_secs(2);
    loop {
        tokio::time::sleep(TICK).await;
        if idle_expired(&daemon, idle_timeout).await {
            tracing::info!(?idle_timeout, "idle with no active session; exiting");
            let _ = shutdown.send(()).await;
            return;
        }
    }
}

// ---------------------------------------------------------------------------
// Shared connection handling.
// ---------------------------------------------------------------------------

/// One connection: newline-delimited JSON requests, one envelope per reply.
/// Whether this request's events must take their ordinals in accept order.
///
/// Everything that carries capture output, except a boundary: a boundary is
/// gated by nothing because it can be *waited on* by the captures around it
/// (H3), and the two rules together are what keep the gate acyclic. A boundary
/// is also not evidence any rule reads a sequence of — `session_opened` and
/// `session_closed` bracket the stream rather than sitting inside the patterns
/// R1–R8 match.
fn orders_by_arrival(request: &Request) -> bool {
    match request {
        Request::CaptureEvents { .. } => true,
        Request::CanonicalEvent { event, capture, .. } => {
            capture.is_some() && !event.event.is_boundary_class()
        }
        _ => false,
    }
}

async fn serve<S>(
    daemon: Arc<Daemon>,
    stream: S,
    shutdown: tokio::sync::mpsc::Sender<()>,
    mut ticket: arrival::Ticket,
) -> anyhow::Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let (read_half, mut write_half) = tokio::io::split(stream);
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
                    Request::CanonicalEvent { event, .. } => !event.event.is_boundary_class(),
                    _ => false,
                };
                // **Its ordinal is its place in the session, so it takes
                // the place it arrived in** (`arrival`). Waited out before the
                // capture is counted as in flight, never after: a boundary can
                // wait for the captures already in flight (H3), and a capture
                // counted while still at the gate would be one a boundary ahead
                // of it waits for and it waits behind — a stall of the whole
                // bound, in the one place a session is being closed.
                if orders_by_arrival(&request) {
                    let limit = std::time::Duration::from_millis(
                        daemon.config.read().await.capture_deadline_ms,
                    );
                    ticket.wait_turn(limit).await;
                } else {
                    ticket.retire();
                }
                let _capture =
                    is_capture.then(|| state::CaptureGuard::new(&daemon.in_flight_captures));
                let stop = matches!(request, Request::DaemonShutdown);
                let reply = handlers::dispatch(&daemon, request).await;
                // The ordinals this request was going to consume are consumed,
                // so the next connection may have its turn. A no-op for
                // anything retired above.
                ticket.retire();
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

#[cfg(test)]
mod serve_tests {
    use super::*;
    use cairn_core::event::CaptureOutput;
    use std::time::Duration;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    /// Serve one request on a connection of its own, under `ticket`.
    ///
    /// The request is written in full before the handle is returned, so a test
    /// that finds no reply is looking at something holding the *reply* and not
    /// at a request it forgot to send.
    fn connection(
        daemon: Arc<Daemon>,
        ticket: arrival::Ticket,
        request: &Request,
    ) -> tokio::task::JoinHandle<Option<String>> {
        let (theirs, ours) = tokio::io::duplex(64 * 1024);
        let mut line = serde_json::to_string(request).expect("encodable");
        line.push('\n');
        let (shutdown, keep) = tokio::sync::mpsc::channel::<()>(1);
        tokio::spawn(async move {
            // The receiver is held for as long as the connection is, so a
            // `DaemonShutdown` on it could not fail for want of one.
            let _keep = keep;
            tokio::spawn(async move {
                let _ = serve(daemon, theirs, shutdown, ticket).await;
            });
            let (read, mut write) = tokio::io::split(ours);
            write.write_all(line.as_bytes()).await.expect("write");
            write.flush().await.expect("flush");
            BufReader::new(read)
                .lines()
                .next_line()
                .await
                .expect("read")
        })
    }

    /// A daemon whose gate will not give up during a test.
    ///
    /// The wait is bounded by the capture deadline so that a connection which
    /// never retires cannot stall capture indefinitely, and the default bound
    /// is 250 ms — short enough that "waited" and "waited then gave up" are the
    /// same observation inside a test's own timeout. Raising it makes the two
    /// distinguishable, which is what lets the control below fail when it
    /// should.
    async fn repo() -> testsupport::Repo {
        testsupport::Repo::with(CairnConfig {
            capture_deadline_ms: 30_000,
            ..CairnConfig::default()
        })
        .await
    }

    fn capture(cwd: &str) -> Request {
        Request::CaptureEvents {
            cwd: cwd.to_string(),
            agent: "opencode".to_string(),
            agent_session_key: "arrival".to_string(),
            output: CaptureOutput::default(),
        }
    }

    /// **A capture takes the ordinal belonging to the hook that produced it.**
    ///
    /// The ordinal is allocated inside `dispatch`, so "waits for its turn" and
    /// "takes its ordinal in accept order" are the same statement. Here the
    /// connection accepted *second* is the only one served: its request is
    /// complete and its connection is live, so the sole thing that can be
    /// holding its reply is the gate. Retiring the first ticket releases it.
    ///
    /// **Falsified by** deleting the `orders_by_arrival` branch in `serve`, or
    /// by taking the ticket inside the spawned task rather than in the accept
    /// loop: both restore the race that cost SC-701 a trial in one run of ten.
    #[tokio::test]
    async fn a_capture_waits_for_the_captures_accepted_before_it() {
        let r = repo().await;
        let cwd = r.cwd.clone();
        let daemon = Arc::new(r.daemon);
        let arrivals = arrival::Arrivals::new();
        let mut first = arrivals.take();
        let second = arrivals.take();

        let mut behind = connection(Arc::clone(&daemon), second, &capture(&cwd));
        assert!(
            tokio::time::timeout(Duration::from_millis(250), &mut behind)
                .await
                .is_err(),
            "a capture accepted second was served ahead of the capture accepted first"
        );

        first.retire();
        let reply = tokio::time::timeout(Duration::from_secs(5), behind)
            .await
            .expect("the gate did not open when the earlier capture retired")
            .expect("join");
        assert!(reply.is_some(), "the connection closed without a reply");
    }

    /// The gate is for capture and for nothing else.
    ///
    /// A boundary can wait for the captures already in flight (H3), so gating
    /// one would put a stall exactly where a session is being closed; and a
    /// read has no ordinal to take. Without this, every health read during a
    /// session would queue behind that session's own tool calls.
    #[tokio::test]
    async fn a_request_that_carries_no_capture_is_not_gated() {
        let r = repo().await;
        let _cwd = r.cwd.clone();
        let daemon = Arc::new(r.daemon);
        let arrivals = arrival::Arrivals::new();
        // Never retired, standing in for a capture still being written.
        let _ahead = arrivals.take();
        let behind = arrivals.take();

        let answered = connection(daemon, behind, &Request::DaemonStatus);
        // Well inside the gate's bound, so "was not gated" and "was gated and
        // gave up" cannot both pass this.
        let reply = tokio::time::timeout(Duration::from_secs(2), answered)
            .await
            .expect("a read accepted behind an unfinished capture was made to wait for it")
            .expect("join");
        assert!(reply.is_some(), "the connection closed without a reply");
    }
}
