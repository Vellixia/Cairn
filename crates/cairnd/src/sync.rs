//! Opt-in synchronization with the Cairn server (FR-053 – FR-058, D9, D14).
//!
//! Local → server for what this machine produced, plus read access to shared
//! records produced by others. Delivery is idempotent, offline is normal, and
//! an unlinked project never produces a request.

use crate::state::{Daemon, Resolved, storage_err};
use cairn_core::domain::*;
use cairn_core::wire::*;
use cairn_store::{cursor, outbox, repo};
use serde_json::json;
use std::time::Duration;
use uuid::Uuid;

const BATCH: i64 = 100;

/// How often the typed-spool worker wakes after a successful pass.
const WORKER_TICK: Duration = Duration::from_millis(500);
/// Bounded retry delay after ordinary delivery failure.
const BACKOFF_MIN: Duration = Duration::from_millis(500);
const BACKOFF_MAX: Duration = Duration::from_secs(30);

type Reply = Result<serde_json::Value, WireError>;

/// Delivery retry state. Typed records own their individual retry schedules;
/// this only prevents an unreachable server from being probed continuously.
struct WorkerBackoff {
    backoff: Duration,
}

impl WorkerBackoff {
    fn new() -> Self {
        Self {
            backoff: BACKOFF_MIN,
        }
    }
    fn success(&mut self) {
        self.backoff = BACKOFF_MIN;
    }
    fn failure(&mut self) -> Duration {
        let delay = self.backoff;
        self.backoff = (self.backoff * 2).min(BACKOFF_MAX);
        delay
    }
}

/// Drain the outbox automatically, forever (FR-056, D9).
///
/// `cairn sync now` stays available as an explicit trigger, but it is not the
/// only one: work queued while the server was unreachable is delivered when it
/// comes back, with no manual step. Transient failures back off **per
/// namespace** (D427); permanent rejections are already recorded as `failed`
/// by `drain`/`drain_global` and are not retried, and never count as transient
/// (§4a) — an ingest content refusal must never throttle the namespace it
/// arrived in.
/// How many spooled rows one drain pass claims.
///
/// Below the ingest batch bound of 256 rather than equal to it, so a full pass
/// is comfortably inside the request body limit even with the largest events
/// the model allows. A pass that had to be refused for size would release every
/// row it claimed and try the identical batch again next tick, forever.
const SPOOL_DRAIN_BATCH: i64 = 128;

pub async fn run_worker(daemon: std::sync::Arc<Daemon>) {
    // **Claims a previous process took to the grave, released before anything
    // else runs** (T096).
    //
    // A row is claimed by setting `state = 'in_flight'` and stamping
    // `claimed_at`, and a drainer that dies between the claim and the settle
    // leaves it there. `claim_events` does reclaim an expired lease, so nothing
    // is lost — but the lease is `CLAIM_LEASE_SECONDS`, and until it expires the
    // row counts as in flight, which reads as "delivery is progressing" when no
    // process is delivering anything. A daemon that has just started knows
    // better than any lease can: it holds no claims, so any claim it finds is
    // stranded by definition.
    //
    // Deliberately once, at start, and not on every tick. On a tick this would
    // race the drain running beside it and release a claim whose drainer is
    // mid-send, turning a delivery in progress into a redelivery.
    for (kind, released) in [
        (
            "events",
            cairn_store::spool::release_event_claims(&daemon.store).await,
        ),
        (
            "commands",
            cairn_store::spool::release_command_claims(&daemon.store).await,
        ),
    ] {
        match released {
            Ok(n) if n > 0 => tracing::info!(
                spool = kind,
                rows = n,
                "released claims a previous daemon left in flight"
            ),
            Ok(_) => {}
            Err(e) => tracing::debug!(spool = kind, error = %e, "could not release stale claims"),
        }
    }

    let mut backoff = WorkerBackoff::new();
    loop {
        let delay = if drain_typed_spools(&daemon).await {
            backoff.success();
            WORKER_TICK
        } else {
            backoff.failure()
        };
        tokio::time::sleep(delay).await;
    }
}

/// Drain only durable typed lanes. Legacy entity sync remains manually
/// callable until its handlers are removed; it is never background work.
pub(crate) async fn drain_typed_spools(d: &Daemon) -> bool {
    let _ = crate::capture::collect_capture_drops(d).await;
    let events = drain_event_spool(d, SPOOL_DRAIN_BATCH).await;
    if let Err(e) = &events {
        tracing::debug!(error = %e.message, "event spool drain deferred");
    }
    let commands = drain_command_spool(d, SPOOL_DRAIN_BATCH).await;
    if let Err(e) = &commands {
        tracing::debug!(error = %e.message, "command spool drain deferred");
    }
    events.is_ok() && commands.is_ok()
}

// ---------------------------------------------------------------------------
// Establishing and pulling the global namespaces (T101, T129 client half)
// ---------------------------------------------------------------------------

/// A stand-in instance id for a peer that has not reported one.
///
/// Deterministic in the configured endpoint, so the same server yields the same
/// lane on every start and across daemon restarts — a lane whose key moved on
/// restart would orphan whatever it held.
///
/// Not a guess at the server's real identity, and never treated as one: it
/// exists only so that a lane can be opened, held and reported before the server
/// is able to identify itself, and it is replaced the moment the server does.
/// Team knowledge binding still keys on the reported id, so a restored backup at
/// the same endpoint is still a different instance and still refused (FR-496) —
/// the provisional id is not what that check consults.
fn provisional_instance(url: &str) -> Uuid {
    let digest = cairn_core::digest(&format!("cairn-provisional-instance:{}", url.trim()));
    let mut bytes = [0u8; 16];
    for (slot, pair) in bytes.iter_mut().zip(digest.as_bytes().chunks(2)) {
        *slot = u8::from_str_radix(std::str::from_utf8(pair).unwrap_or("00"), 16).unwrap_or(0);
    }
    Uuid::from_bytes(bytes)
}

/// What a bounded, read-only peer-identity probe found (FR-792a).
///
/// Three outcomes rather than an `Option<Uuid>`, because the three decide
/// different reports and collapsing any two of them is how the defect this
/// exists to fix was written in the first place. "No peer known" is not one
/// state: an endpoint that did not answer is `server_unreachable`, and an
/// endpoint that was never configured is not blocked on the network at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PeerProbe {
    /// No endpoint or no credential is configured. Nothing was sent, and there
    /// is no peer for a queued row to be mismatched against.
    NotConfigured,
    /// The endpoint answered and named this instance.
    Peer(Uuid),
    /// The endpoint did not answer inside [`PEER_PROBE_DEADLINE`].
    Unreachable,
}

/// How long status waits for the endpoint to name itself (FR-792a).
///
/// **Stated, and much shorter than the drain's twenty seconds.** A drain is
/// background work and can afford to wait out a slow server; a status read is
/// somebody waiting at a terminal. The CLI allows a status exchange thirty
/// seconds, so this cannot be what makes one time out.
///
/// **Five seconds, and the first guess of two was measured wrong.** Exceeding
/// this deadline is reported as an unreachable endpoint, so the deadline decides
/// how readily status calls a *healthy* server unreachable — and that is a false
/// report, which is worse than a slow one. At two seconds, a 200-repetition
/// stress of the cross-daemon lifecycle scenario on a host at load ~25 reported
/// `server_unreachable` against a server that was up and answering in 9 runs of
/// 132: the endpoint was fine and the probe simply lost its race for CPU. CI
/// runners are small and run the whole suite in parallel, so they sit in exactly
/// that regime.
///
/// Loopback to a live server is a sub-millisecond round trip; five seconds is
/// therefore three orders of magnitude of headroom for scheduling noise, while
/// still bounding the read for a person who is waiting. It is not a fix for a
/// server that is genuinely gone — that connection is refused immediately and
/// never approaches the deadline.
pub(crate) const PEER_PROBE_DEADLINE: Duration = Duration::from_millis(5_000);

/// Ask the configured endpoint who it is, changing nothing (FR-792a, FR-792b).
///
/// **Why status takes its own sample.** FR-792 asks for the reason delivery is
/// not progressing, which is a claim about now, and the two things that decide
/// it — whether the endpoint answers, and which deployment answers — were both
/// read from this process's memory of an earlier delivery attempt. That memory
/// does not survive the daemon being replaced, and the daemon is replaced
/// routinely: `supervise` exits a daemon within one tick of another owning its
/// socket. So the daemon that watched a replacement server appear was reliably
/// gone by the time an operator asked what was wrong, and the survivor, having
/// observed nothing, fell back to the store's own binding and reported that
/// nothing was wrong — with the whole backlog queued for a deployment that no
/// longer exists.
///
/// **What makes it read-only.** Everything durable is untouched by
/// construction, not by care: this function reads the credential snapshot, does
/// one `GET`, and returns. It never calls `establish_global_namespaces`, so no
/// lane is opened and the `team:*` lane that *is* the binding cannot move
/// (FR-495/FR-496, D438). It touches no cursor, no spool row, no claim, no
/// attempt counter and no event state, because it calls nothing that can. It
/// does not go through [`AuthenticatedContext::acquire`], which would be the
/// tempting reuse: that path demands a proven account, waits twenty seconds and
/// exists to be the front door for work, and a status read is none of those.
///
/// The instance is parsed exactly as `acquire` parses it, provisional
/// substitution included, so a probe and a drain can never disagree about who
/// is answering.
pub(crate) async fn probe_peer_instance(d: &Daemon) -> PeerProbe {
    // One read, so the endpoint and the token describe one credential by
    // construction rather than by two reads happening to agree.
    let (url, token) = {
        let creds = d.server.read().await;
        (creds.url.clone(), creds.token.clone())
    };
    // Not "unreachable": there is nothing to reach. An unconfigured store is
    // not blocked on the network, and saying it was would send someone to
    // check a server they never named.
    let (Some(base), Some(token)) = (url, token) else {
        return PeerProbe::NotConfigured;
    };
    let base = base.trim_end_matches('/').to_string();
    let Ok(http) = reqwest::Client::builder()
        .timeout(PEER_PROBE_DEADLINE)
        .build()
    else {
        return PeerProbe::Unreachable;
    };
    let response = http
        .get(format!("{base}/api/version"))
        .bearer_auth(&token)
        .send()
        .await;
    // Any answer at all is the endpoint being there. A refusal still identifies
    // a reachable deployment, and an unparseable body from a reachable server is
    // the provisional-instance case rather than an outage — the same
    // substitution `acquire` makes, so the two agree.
    let peer = match response {
        Ok(r) => {
            let body: serde_json::Value = r.json().await.unwrap_or(serde_json::Value::Null);
            body.get("server_instance_id")
                .and_then(|v| v.as_str())
                .and_then(|s| Uuid::parse_str(s).ok())
                .unwrap_or_else(|| provisional_instance(&base))
        }
        Err(_) => return PeerProbe::Unreachable,
    };
    // Telemetry only (FR-792c). Recorded because it is genuinely useful when
    // reconstructing what a daemon saw, and it decides nothing: the report is
    // built from the sample just taken, not from this.
    {
        let mut observed = d.last_observed_instance.write().await;
        let previous = *observed;
        *observed = Some(peer);
        if previous != Some(peer) {
            tracing::info!(
                target: "cairn::observation",
                previous = ?previous, observed = %peer, endpoint = %base,
                "a status probe found a different server instance"
            );
        }
    }
    PeerProbe::Peer(peer)
}

/// Read this token's account id from `GET /api/auth/me` and record it.
///
/// Best-effort: an unreachable server leaves whatever was already known, which
/// is the honest answer — the identity did not change because the network did.
/// Returns whether an identity is now known at all.
pub(crate) async fn learn_account_identity(d: &Daemon) -> bool {
    // The *generation* the question is asked under, not the credential's
    // contents (FR-604). `GET /api/auth/me` means "who is this token", so the
    // answer belongs to the credential that asked — and comparing contents
    // afterwards cannot tell a credential that never changed from one switched
    // A → B → A while the server was answering. Both leave `token` and `url`
    // exactly as they were; only one of them has an answer that is still about
    // the current credential.
    let Ok(snapshot) = CredentialSnapshot::take(d).await else {
        return d.server.read().await.account_id.is_some();
    };
    let asked_under = snapshot.generation;
    let Ok(body) = snapshot.client.get("/api/auth/me").await else {
        return d.server.read().await.account_id.is_some();
    };
    let Some(id) = body
        .get("id")
        .and_then(|v| v.as_str())
        .and_then(|s| Uuid::parse_str(s).ok())
    else {
        return d.server.read().await.account_id.is_some();
    };

    // Commit through the one gateway, conditional on nothing having changed
    // since. The check runs inside the same write lock the assignment does, so
    // there is no window between deciding and committing (FR-605).
    let committed = d
        .mutate_credentials(|c| {
            if c.generation == asked_under {
                c.account_id = Some(id);
            }
        })
        .await;
    match committed {
        Ok(()) => {}
        Err(e) => {
            tracing::debug!(error = %e, "could not persist the server account id");
            return false;
        }
    }

    let creds = d.server.read().await;
    if creds.account_id != Some(id) {
        tracing::debug!(
            "discarding a learned account identity: the credential changed while \
             the server was answering"
        );
    }
    creds.account_id.is_some()
}

/// Make sure this store has a `personal:*` and a `team:*` lane, so the worker
/// has something to pull on.
///
/// **This is what makes a consume-only machine work** (FR-489,
/// `sync-namespaces.md` §5). Namespace discovery used to be
/// `outbox::known_namespaces` alone — the set of lanes with queued work — which
/// is empty on a machine that has never written personal or team knowledge of
/// its own. Such a machine would never pull, so a member who only ever reads
/// team guidance would never see an admin's ratification. Personal and team
/// knowledge are the first content a machine can legitimately only ever consume;
/// every earlier entity type could at least in principle be produced locally,
/// which is why this gap did not exist before.
///
/// The server instance id comes from `GET /api/version`, which already carries
/// it (FR-416) — there is no handshake to add. Without one there is no namespace
/// key to form, so a server below schema 3 establishes nothing and this returns
/// `None`: correct rather than degraded, since such a server has nowhere to put
/// either domain anyway.
///
/// Establishing also backfills. A user records personal notes before ever
/// linking a server, and those are precisely the ones they most want on their
/// second machine; without the backfill everything written before the link would
/// be stranded — recorded, recallable locally, permanently invisible elsewhere.
/// This mirrors [`backfill`], which does the same for a project's pre-link
/// history.
async fn establish_global_namespaces(d: &Daemon) -> Option<Uuid> {
    // A lane key names the owning account, so there is nothing to establish
    // until the account is known. A daemon that started with a token but no
    // recorded identity — the very first run after an upgrade, or one whose
    // config predates the field — learns it here.
    if d.server.read().await.account_id.is_none() && !learn_account_identity(d).await {
        return None;
    }

    // One credential read for the account, the endpoint and the peer's instance
    // (FR-597). These were four separate reads of `Daemon::server` with a network
    // call among them, and a lane key is built from two of them — so an account
    // switch landing in the middle opened a lane named for one account against a
    // server learned under another's token. That lane is durable: it is written
    // to `sync_cursor` and every later push and pull routes by it.
    let context = AuthenticatedContext::acquire(d).await.ok()?;
    let reported: Option<Uuid> = context
        .version
        .get("server_instance_id")
        .and_then(|v| v.as_str())
        .and_then(|s| Uuid::parse_str(s).ok());

    let owner = context.account;
    let provisional = provisional_instance(&context.base);
    // Identical to `reported.unwrap_or(provisional)` — the context already
    // made that substitution — and taken from it so the lane this establishes
    // and the lane every later operation admits are decided by one rule.
    let instance = context.peer_instance;

    // A server below schema 3 has no `server_instance` table and so reports no
    // id, and until now that meant no lane could be formed — which meant a
    // personal write against such a server was **never queued at all**. That is
    // not the behaviour §11a describes: it says content queued against a server
    // that cannot accept it is *held*, released automatically once the peer
    // supports it. Content that was never queued is not held; it is invisible,
    // and `cairn sync status` has nothing to report.
    //
    // So the lane opens under a provisional id derived from the configured
    // endpoint, and re-keys itself to the real id the moment the server reports
    // one. The endpoint is the right thing to derive from because it is exactly
    // what §11a's upgrade scenario holds fixed: "that peer is replaced by a
    // supporting server **at the same configured endpoint**".
    //
    // Re-keying moves the cursor, the backoff, the capability fingerprint and
    // every queued row, and touches no `idempotency_key` — so an entry that was
    // in flight across the re-key is still recognised as the same entry and
    // applies exactly once (FR-562).
    if reported.is_some() {
        // The spools move with the lane, and only here.
        //
        // A row is bound to the instance it was queued for and is never
        // rebound — that binding is what stops a replacement deployment
        // inheriting its predecessor's backlog (FR-791). This is the one
        // exception, and it is not an exception to the rule so much as the
        // same server finally able to say its own name: a peer below schema
        // 3 reports no instance, so its lane is keyed by an id derived from
        // the endpoint, and an in-place upgrade makes it start reporting a
        // real one (`sync-namespaces.md` §11a).
        //
        // Keyed on the *provisional* id, never on the URL. A different
        // deployment at the same address reports its own id and carries no
        // row bearing this provisional one, so it cannot be reached by this
        // statement.
        match cairn_store::spool::rebind_provisional_instance(&d.store, provisional, instance).await
        {
            Ok(n) if n > 0 => tracing::info!(
                rows = n,
                from = %provisional, to = %instance,
                "re-keyed spooled work from the provisional instance id to the reported one"
            ),
            Ok(_) => {}
            Err(e) => tracing::debug!(error = %e, "could not re-key spooled work"),
        }

        for (from, to) in [
            (
                SyncNamespace::Personal(provisional, owner),
                SyncNamespace::Personal(instance, owner),
            ),
            (
                SyncNamespace::Patterns(provisional, owner),
                SyncNamespace::Patterns(instance, owner),
            ),
            (
                SyncNamespace::Team(provisional),
                SyncNamespace::Team(instance),
            ),
        ] {
            let moved = outbox::rename_namespace(&d.store, &from.key(), &to.key())
                .await
                .unwrap_or(0);
            if let Err(e) = cursor::rename(&d.store, &from, &to).await {
                tracing::debug!(error = %e, "could not re-key a provisional lane");
            } else if moved > 0 {
                tracing::info!(
                    from = %from.key(), to = %to.key(), rows = moved,
                    "re-keyed a provisional lane now that the server reported its instance"
                );
            }
        }
    }

    // A lane key is durable routing state: it names an account and a server, and
    // every later push and pull is decided by it. Writing one derived from a
    // credential this machine no longer holds would outlive the mistake by as
    // long as the store does, so the context is checked before anything is
    // written rather than after (FR-604).
    if !context.still_current(d).await {
        tracing::debug!("not establishing lanes: the credential changed while probing");
        return None;
    }

    let personal = SyncNamespace::Personal(instance, owner);
    let team = SyncNamespace::Team(instance);
    // Opened with the personal lane and on the same terms. A pattern is a
    // personal-domain record (FR-708c), so the account that may read the one may
    // read the other, and a store holding two identities' personal knowledge side
    // by side holds two identities' patterns the same way.
    let patterns = SyncNamespace::Patterns(instance, owner);

    // **A store may hold several `personal:*` lanes and exactly one `team:*`
    // lane** (D438, FR-495, FR-496).
    //
    // The asymmetry is the design: personal knowledge is partitioned by owning
    // account, so two identities coexist; team knowledge is a claim about one
    // server's ratification history, and blending two deployments' guidance is
    // what FR-496 forbids. That refusal is implemented by
    // `bind_team_server_instance_tx`, which asks "which instance is this store's
    // team corpus bound to?" by reading the recorded `team:*` lane — so opening a
    // second one makes the question ambiguous, and the answer became whichever
    // row the query happened to return first. Relinking to a second server then
    // silently merged its guidance into a corpus bound to the first.
    //
    // So the second lane is never opened. The store keeps pulling team knowledge
    // from the instance it is bound to, and a genuine move to a different server
    // is an explicit act (a fresh store, or an unlink) rather than a side effect
    // of `cairn auth token set`.
    let already_bound = cursor::established(&d.store)
        .await
        .unwrap_or_default()
        .into_iter()
        .find_map(|ns| match ns {
            SyncNamespace::Team(existing) => Some(existing),
            _ => None,
        });
    let team_is_ours = match already_bound {
        Some(existing) if existing != instance => {
            tracing::warn!(
                bound_to = %existing, now_linked_to = %instance,
                "this store's team knowledge belongs to another server instance; \
                 not opening a second team lane (FR-496)"
            );
            false
        }
        _ => true,
    };

    let lanes: Vec<&SyncNamespace> = if team_is_ours {
        vec![&personal, &patterns, &team]
    } else {
        vec![&personal, &patterns]
    };
    for namespace in lanes {
        if let Err(e) = cursor::establish(&d.store, namespace).await {
            tracing::debug!(namespace = %namespace.key(), error = %e, "could not establish namespace");
            return None;
        }
    }

    // Both backfills are idempotent by the outbox's own key, so running them on
    // every establish costs one query per row and enqueues nothing twice.
    // Adoption before backfill: notes written before this machine knew who it was
    // become this account's, and the backfill then queues them like any other row
    // it owns (FR-608). Without this they stay owned by nobody — recallable here
    // and invisible everywhere else, which is local-first without the second half
    // of the promise.
    match cairn_store::global::adopt_unattributed_personal(&d.store, owner).await {
        Ok(n) if n > 0 => tracing::info!(
            adopted = n,
            "personal knowledge written before this machine signed in now belongs \
             to the signed-in account"
        ),
        Ok(_) => {}
        Err(e) => tracing::debug!(error = %e, "unattributed personal knowledge not adopted"),
    }
    match cairn_store::global::enqueue_personal_backlog(&d.store, owner).await {
        Ok(n) if n > 0 => tracing::info!(
            queued = n,
            "queued personal knowledge written before this link"
        ),
        Ok(_) => {}
        Err(e) => tracing::debug!(error = %e, "personal backlog not queued"),
    }
    match cairn_store::global::enqueue_team_backlog(&d.store).await {
        Ok(n) if n > 0 => {
            tracing::info!(queued = n, "queued team proposals written before this link")
        }
        Ok(_) => {}
        Err(e) => tracing::debug!(error = %e, "team backlog not queued"),
    }

    Some(instance)
}

/// Pull one global namespace's changes and merge them into this store.
///
/// Returns how many rows landed. A row that fails to merge is counted as not
/// landed and the cursor still advances past the page: the alternative is a
/// single unmergeable row wedging the lane forever, and every merge here is
/// idempotent by id, so a row that becomes mergeable later arrives again on the
/// next full pull rather than being lost. A *transport* failure returns `Err`
/// and does not advance the cursor, which is the case that must retry.
/// One read of the credential, yielding both the generation it was taken under
/// and a client that speaks with it (FR-604).
///
/// [`AuthenticatedContext`] needs an account and so cannot serve the one
/// operation whose job is to learn one. This is the part of it that does not:
/// enough to ask a question and to know afterwards whether the credential that
/// asked is still the credential this machine holds.
///
/// **The generation and the client come from the same read.** Taking them
/// separately is the ABA hole in a different place: a snapshot of the old
/// credential, a request sent with the new one, and a comparison afterwards that
/// finds the old value back in place and concludes nothing happened — committing
/// an answer about one account while another's token is stored.
struct CredentialSnapshot {
    generation: u64,
    client: Client,
}

impl CredentialSnapshot {
    async fn take(d: &Daemon) -> Result<CredentialSnapshot, WireError> {
        let (generation, url, token) = {
            let creds = d.server.read().await;
            (creds.generation, creds.url.clone(), creds.token.clone())
        };
        let base = url.ok_or_else(|| {
            WireError::new(
                codes::NOT_LINKED,
                "no server configured; run `cairn auth token set`",
            )
        })?;
        let token = token.ok_or_else(|| {
            WireError::new(
                codes::UNAUTHORIZED,
                "no API token; run `cairn auth token set`",
            )
        })?;
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .build()
            .map_err(|e| WireError::new(codes::SERVER_UNAVAILABLE, e.to_string()))?;
        Ok(CredentialSnapshot {
            generation,
            client: Client {
                base: base.trim_end_matches('/').to_string(),
                token,
                http,
            },
        })
    }
}

/// The one proven answer to "who is acting, against which server, with what
/// standing" — resolved once, and used from the start of a server-global
/// operation to its end (FR-604 through FR-607).
///
/// **This exists because the same five facts kept being resolved separately.**
/// The endpoint, the credential, the account, the server instance and the
/// caller's project membership were each fetched by whichever path needed them,
/// at whatever moment it needed them — and every review round found another pair
/// that could disagree: an account read before a token, a token read again after
/// a network call, an actor looked up after the mutation it was meant to
/// authorize, a membership answered from a local `linked` flag rather than from
/// the server. Each was fixed where it was found, and the next round found
/// another. They were not seven defects; they were one missing abstraction,
/// seven times.
///
/// So the facts are gathered together, under one lock acquisition and one
/// `GET /api/version`, and every decision an operation makes reads *this* rather
/// than asking again. An operation either runs entirely inside one context or
/// refuses. There is no fallback identity, no second credential read, and no
/// local proxy for a server's answer.
///
/// The `generation` is what makes "still the same credential" answerable at all.
/// Comparing token and endpoint cannot distinguish a credential that never
/// changed from one switched away and back while a request was in flight; a
/// counter that only increases can. Anything that commits a result derived from
/// this context checks it first — see [`still_current`](Self::still_current).
struct AuthenticatedContext {
    /// The credential generation this context was taken under.
    generation: u64,
    /// The account this operation is authenticated as. Never a fallback: a
    /// context cannot be acquired without one (FR-603).
    account: Uuid,
    client: Client,
    /// The instance the peer reports, or the provisional id derived from the
    /// endpoint when it reports none (a server below schema 3) — the same
    /// substitution `establish_global_namespaces` makes, so the two agree
    /// without a special case.
    peer_instance: Uuid,
    /// `GET /api/version`'s body, kept so a drain's capability refresh reads the
    /// response this context already paid for rather than fetching it again
    /// under a credential that may since have changed.
    version: serde_json::Value,
    /// The endpoint this context authenticated against.
    base: String,
    /// The projects the server says this account belongs to, fetched at most
    /// once and only if something asks (FR-607).
    ///
    /// Membership is the server's fact, and every local stand-in for it has been
    /// wrong in a way that mattered: `project.linked` says this machine once
    /// linked a project, which is a fact about this machine's past and not about
    /// whether the account now holding the token may act in that project.
    ///
    /// `None` inside the cell is **"the question could not be answered"**, which
    /// is not the same claim as "this account belongs to nothing". Both hold the
    /// batch and both are right to, but only one of them is a fact about the
    /// account — and a drain that reported the second when it meant the first
    /// wrote `no authorization project for this account` into `last_error` about
    /// an account that belongs to several. Kept apart so the hold can say which
    /// it was; what the drain *does* is unchanged.
    memberships: tokio::sync::OnceCell<Option<Vec<Uuid>>>,
}

impl AuthenticatedContext {
    async fn acquire(d: &Daemon) -> Result<AuthenticatedContext, WireError> {
        // One read. The generation, the account, the token and the endpoint come
        // out of the same lock acquisition, so they describe one credential by
        // construction rather than by four reads happening to agree.
        let (generation, account, url, token) = {
            let creds = d.server.read().await;
            (
                creds.generation,
                creds.account_id,
                creds.url.clone(),
                creds.token.clone(),
            )
        };
        // **No fallback identity** (FR-603). Substituting the machine's local id
        // when the account was unknown meant every global operation had an
        // account to route by even when nobody had authenticated — and the one it
        // had named something no server has ever issued. Without a proven account
        // there is no lane to act on, nothing to attribute, and no question this
        // operation is entitled to answer.
        let account = account.ok_or_else(|| {
            WireError::new(
                codes::UNAUTHORIZED,
                "the authenticated account is not known yet; global synchronization \
                 is held until it is",
            )
        })?;
        let base = url.ok_or_else(|| {
            WireError::new(
                codes::NOT_LINKED,
                "no server configured; run `cairn auth token set`",
            )
        })?;
        let token = token.ok_or_else(|| {
            WireError::new(
                codes::UNAUTHORIZED,
                "no API token; run `cairn auth token set`",
            )
        })?;
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .build()
            .map_err(|e| WireError::new(codes::SERVER_UNAVAILABLE, e.to_string()))?;
        let base = base.trim_end_matches('/').to_string();
        let client = Client {
            base: base.clone(),
            token,
            http,
        };

        let version = client.get("/api/version").await?;
        let peer_instance = version
            .get("server_instance_id")
            .and_then(|v| v.as_str())
            .and_then(|s| Uuid::parse_str(s).ok())
            .unwrap_or_else(|| provisional_instance(&base));
        // Remembered for FR-792's sake and for nothing else: the spool report
        // needs the instance *answering*, and this is the only place the daemon
        // learns it. Recording it adopts nothing — the binding is the `team:*`
        // lane and a mismatching peer is still refused (FR-791).
        {
            let mut observed = d.last_observed_instance.write().await;
            let previous = *observed;
            *observed = Some(peer_instance);
            if previous != Some(peer_instance) {
                tracing::info!(
                    target: "cairn::observation",
                    previous = ?previous, observed = %peer_instance, endpoint = %base,
                    "the endpoint reported a different server instance"
                );
            }
        }

        Ok(AuthenticatedContext {
            generation,
            account,
            client,
            peer_instance,
            version,
            base,
            memberships: tokio::sync::OnceCell::new(),
        })
    }

    /// Whether the credential this context was taken under is still the stored
    /// one (FR-604).
    ///
    /// Checked before committing anything derived from this context. A context
    /// that has gone stale has not necessarily produced a wrong answer — it has
    /// produced an answer about a credential this machine no longer holds, which
    /// is not an answer anyone asked for.
    async fn still_current(&self, d: &Daemon) -> bool {
        d.server.read().await.generation == self.generation
    }

    /// The projects this account belongs to, as the server reports them.
    ///
    /// Fetched once per context and cached, so an operation that asks twice gets
    /// one answer rather than two that might differ.
    async fn memberships(&self) -> Option<&Vec<Uuid>> {
        self.memberships
            .get_or_init(|| async {
                let body = self.client.get("/api/projects").await.ok()?;
                let rows = body.get("projects").and_then(|v| v.as_array())?;
                let mut ids: Vec<Uuid> = rows
                    .iter()
                    .filter_map(|p| p.get("id").and_then(|v| v.as_str()))
                    .filter_map(|s| Uuid::parse_str(s).ok())
                    .collect();
                ids.sort();
                Some(ids)
            })
            .await
            .as_ref()
    }

    /// Whether this account is a member of `server_project_id`, per the server.
    ///
    /// An unanswerable question is not a membership. Fail-closed here is the
    /// same answer the previous empty-vector fallback gave, and it is now the
    /// answer on purpose rather than by coincidence.
    async fn is_member_of(&self, server_project_id: Uuid) -> bool {
        self.memberships()
            .await
            .is_some_and(|mine| mine.contains(&server_project_id))
    }

    /// Whether this operation may act on `namespace` at all — for pushing and for
    /// pulling alike, since a lane admitted for one and refused for the other is
    /// how the push side came to have no instance check.
    ///
    /// A `personal:*` key names both the owning account and the server instance,
    /// and both must match. A `team:*` key names only the instance, because one
    /// server has one team corpus that every account on it shares (FR-496); which
    /// account may push *into* it is a question about the queued row's author,
    /// answered by the claim (FR-594), not about the lane.
    fn admits(&self, namespace: &SyncNamespace) -> bool {
        match namespace {
            SyncNamespace::Personal(instance, owner) | SyncNamespace::Patterns(instance, owner) => {
                *owner == self.account && self.is_this_peer(*instance)
            }
            SyncNamespace::Team(instance) => self.is_this_peer(*instance),
            // Project lanes are authorized by membership and have their own
            // drain; nothing here should be routing one.
            SyncNamespace::Project(_) => false,
        }
    }

    /// Whether `instance` names the peer this context is talking to — **exactly**
    /// (FR-601, `sync-namespaces.md` §1b).
    ///
    /// This briefly also accepted the provisional id derived from the endpoint, so
    /// that a lane opened against a server below schema 3 could keep working once
    /// that peer was upgraded in place and began reporting a real id (§11a). It
    /// bought upgrade liveness with the isolation FR-495 and FR-496 are for: an
    /// endpoint is not an identity, so a deployment *replaced* or restored from
    /// backup at the same URL — a different server, with a different corpus —
    /// matched the same provisional id and inherited the previous server's team
    /// lane. "Same URL" and "same server" are not the same claim, and only the
    /// second one licenses merging two deployments' guidance.
    ///
    /// The liveness that needed the loophole is now provided where it belongs:
    /// [`establish_global_namespaces`] re-keys a provisional lane to the reported
    /// id, and the worker runs it on its own cadence rather than only when a store
    /// has no global lanes at all. Establishment decides identity; operations
    /// require it.
    fn is_this_peer(&self, instance: Uuid) -> bool {
        instance == self.peer_instance
    }

    fn refuse(&self, namespace: &SyncNamespace, verb: &str) {
        tracing::debug!(
            lane = %namespace.key(), peer = %self.peer_instance, account = %self.account,
            "not {verb}: this lane belongs to another account or another server instance"
        );
    }
}

/// Whether this daemon, as currently authenticated, may synchronize `namespace`.
///
/// **The routing invariant for every global lane, in one place** (FR-567,
/// FR-593). A `personal:*` key names the account that owns the rows in it, and
/// this machine has standing to push or pull that lane only while it is
/// authenticated as that account. After `cairn auth token set` moves a store to a
/// second account, the first account's lane is still recorded here — that is the
/// design, since a store legitimately holds several identities' personal
/// knowledge (§10) — and it must simply sit still.
///
/// This began as an inline check inside `sync_now` and nowhere else, which is why
/// it is a function now. The background worker builds its own target list from
/// the outbox and from `sync_cursor` and had no such check, so the guarantee held
/// for exactly as long as a user only ever synchronized by hand: on the worker's
/// next tick — every thirty seconds, unprompted — A's personal lane was drained
/// and pulled under B's credentials. A rule enforced at one of two call sites is
/// not enforced.
///
/// `team:*` is deliberately not filtered here. A store binds to one server's team
/// corpus and every account on that server reads the same corpus (FR-496), so
/// there is no per-identity team lane to hold back. What *is* per-identity about
/// team knowledge is who authored a queued proposal, and that is enforced where
/// the proposal is claimed rather than by refusing the whole lane — see
/// [`drain_global`].
///
/// **This is a pre-filter, and no longer the guarantee.** It reads the account
/// on its own, so between building a target list and acting on one the answer can
/// go stale — which is the window FR-597 describes. The refusal that counts is
/// [`AuthenticatedContext::admits`], inside the operation, against a credential that
/// cannot change underneath it. Keeping this one is still worth it: it stops the
/// worker from opening an operation, and therefore a request, for a lane it
/// already knows is not ours.
async fn may_sync_lane(d: &Daemon, namespace: &SyncNamespace) -> bool {
    let Some(account) = d.account_identity().await else {
        // Nothing global is ours to touch until an account is proven (FR-603).
        return false;
    };
    match namespace {
        // Both owner-partitioned lanes answer the same question, because a
        // server-held pattern is a personal-domain record owned by one account
        // (FR-708d): a lane naming somebody else's account is never ours to
        // pull, whatever it carries.
        SyncNamespace::Personal(_, owner) | SyncNamespace::Patterns(_, owner) => *owner == account,
        SyncNamespace::Team(_) | SyncNamespace::Project(_) => true,
    }
}

/// Every global lane this store may synchronize as the account it currently
/// holds, established first so a freshly authenticated store has lanes to
/// return — **and the ones it may not**.
///
/// Both entry points — `cairn sync now` and the background worker — route through
/// [`may_sync_lane`], so neither can acquire a lane the other would refuse.
///
/// The second half is the part that used to be dropped on the floor. A lane
/// refused by [`may_sync_lane`] and a lane that does not exist are the same
/// absence from a list of lanes to act on, and they are not the same fact: one
/// is a store holding another identity's knowledge exactly as §10 intends, and
/// the other is a store that never established a lane at all. The caller reports
/// them, so "sync now did nothing" can say which.
async fn global_lane_targets(d: &Daemon) -> (Vec<SyncNamespace>, Vec<String>) {
    let _ = establish_global_namespaces(d).await;
    let (mut syncable, mut withheld) = (Vec::new(), Vec::new());
    for namespace in cursor::established(&d.store).await.unwrap_or_default() {
        if matches!(namespace, SyncNamespace::Project(_)) {
            continue;
        }
        if may_sync_lane(d, &namespace).await {
            syncable.push(namespace);
        } else {
            withheld.push(namespace.key());
        }
    }
    (syncable, withheld)
}

/// What one pulled row's merge attempt means for the pull cursor.
///
/// `bool` was not enough, and the difference between its two false cases is a
/// difference between a delay and an outage. A merge that failed *this time*
/// must hold the cursor, or the row is never requested again and is lost on
/// this device permanently. A row that can never be decoded at all must not
/// hold it, or one such row stops the lane for every row behind it, forever —
/// which is not a hypothetical: a `team_knowledge` row whose `writer_id` is not
/// a UUID cannot be turned into a `SyncedTeamKnowledge` by any amount of
/// retrying, and while the cursor waited for it the same page was re-applied on
/// every pull cycle. That is how a stale page got a second, third and
/// thousandth chance to overwrite a locally-recorded retirement.
pub(crate) enum Merged {
    /// The row landed in the store.
    Landed,
    /// The row cannot be decoded, and no later attempt would decode it
    /// differently. Reported at `warn` where it is dropped, because a silently
    /// discarded record is the one outcome nobody can investigate.
    Undecodable,
    /// The row did not land this time — a transient store failure, or a refusal
    /// that a change of circumstances would lift. The cursor waits for it.
    Deferred,
}

/// Report one permanently undecodable pulled row and drop it.
///
/// The id is logged as the raw wire value rather than a parsed one, because the
/// id is sometimes the field that failed to parse and "which row" is the whole
/// value of the line.
fn undecodable(lane: &str, row: &serde_json::Value, field: &str) -> Merged {
    tracing::warn!(
        lane,
        id = %row.get("id").map(ToString::to_string).unwrap_or_else(|| "absent".to_string()),
        field,
        "dropping a pulled row that cannot be decoded; the cursor moves past it"
    );
    Merged::Undecodable
}

async fn pull_global(d: &Daemon, namespace: &SyncNamespace) -> Result<usize, WireError> {
    // **A lane only ever pulls as the account it names, from the instance it
    // names** (FR-495, FR-496, FR-597). Both halves come from one credential
    // read — see [`AuthenticatedContext`], which is where this check used to be written
    // out inline against a separately-read client.
    let context = AuthenticatedContext::acquire(d).await?;
    if !context.admits(namespace) {
        context.refuse(namespace, "pulling");
        return Ok(0);
    }
    let c = &context.client;

    let since = cursor::pull_cursor(&d.store, namespace)
        .await
        .map_err(storage_err)?;

    let (path, array) = match namespace {
        SyncNamespace::Personal(..) => ("/api/sync/changes/personal", "personal"),
        SyncNamespace::Team(_) => ("/api/sync/changes/team", "team"),
        SyncNamespace::Patterns(..) => ("/api/sync/changes/patterns", "patterns"),
        // `project:*` has its own puller with its own entity types.
        SyncNamespace::Project(_) => return Ok(0),
    };
    let path = match &since {
        Some(cursor) => format!("{path}?since={}", urlencode(cursor)),
        None => path.to_string(),
    };
    let body = c.get(&path).await?;

    let rows = body
        .get(array)
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut landed = 0usize;
    let mut dropped = 0usize;
    let mut all_merged = true;
    for row in &rows {
        let merged = match namespace {
            SyncNamespace::Personal(_, owner) => merge_pulled_personal(d, *owner, row).await,
            SyncNamespace::Team(instance) => merge_pulled_team(d, *instance, row).await,
            SyncNamespace::Patterns(_, owner) => merge_pulled_pattern(d, *owner, row).await,
            // Unreachable: this function returns above for a project lane,
            // which has its own puller and its own entity types. Deferred
            // rather than dropped, so if it ever became reachable the failure
            // would be a stalled lane and not a discarded record.
            SyncNamespace::Project(_) => Merged::Deferred,
        };
        match merged {
            Merged::Landed => landed += 1,
            // Counted, not held against the cursor. Already reported at `warn`
            // by whoever decided it, with the row id.
            Merged::Undecodable => dropped += 1,
            Merged::Deferred => all_merged = false,
        }
    }

    // **The cursor moves only when the whole page landed.**
    //
    // A merge can fail for a reason that has nothing to do with the row: a
    // concurrent foreground write turning into a transient SQLite error is the
    // ordinary case. Advancing anyway meant the next pull asked for changes
    // *after* the page, so the failed row was never requested again — and
    // because a pull is the only way it can arrive, the record was lost on this
    // device permanently, silently, with the lane reporting success.
    //
    // Holding the cursor re-delivers the whole page next time. That is cheap and
    // safe: every merge here is idempotent by id, and content is written once and
    // never rewritten, so a row that already landed is a no-op the second time.
    // Re-reading a page is the right price for never dropping one.
    //
    // **"Cheap and safe" was only ever true of a page that eventually lands.**
    // A row that can never be decoded holds the cursor forever, and the same
    // page is then re-applied on every pull cycle for the life of the store:
    // the re-reading stops being a price and becomes a repeated write. That is
    // measurable damage rather than a wasted request — a stale page re-applied
    // once a cycle will eventually land on the far side of a local transition
    // and erase who performed it (FR-457). So [`Merged::Undecodable`] does not
    // hold the cursor; it is logged with the row id and stepped over, and only
    // a transient failure waits.
    //
    // One class of never-mergeable row was already argued away here and the
    // argument was too narrow. A row whose server instance does not match this
    // store's team binding (FR-496) indeed cannot reach this loop, because
    // `pull_global` refuses such a lane before reading a single row. That says
    // nothing about a row whose *own fields* do not parse — a `writer_id` that
    // is not a UUID, an unparseable `created_at`, a `state` outside the
    // vocabulary — and one of those is exactly what wedged a real lane.
    // **A cursor is a position in one caller's feed, and the `team:*` feed is
    // caller-dependent** (FR-592, `contracts/sync-namespaces.md` §1a).
    //
    // A pending proposal reaches its author and any admin and nobody else, so
    // "everything after this cursor" means something different once the caller's
    // view widens. A `personal:*` lane cannot hit this — its key already carries
    // the owning account, so a second identity gets a second lane and a second
    // cursor. `team:*` deliberately has no identity in its key, because a store
    // binds to exactly one server's team corpus (FR-496), so the view it was
    // reading has to be recorded beside the cursor instead.
    //
    // When the server reports a different view from the one the stored cursor was
    // built under — a member promoted to admin, or this machine now
    // authenticating as someone else — the cursor is discarded rather than
    // advanced, and the next pull walks the lane from the beginning. This page's
    // rows still merge: they are real, and every team merge is idempotent by id,
    // so re-reading them next cycle costs a request and changes nothing. What
    // must not happen is advancing past rows that were invisible a moment ago and
    // are visible now.
    //
    // A server that reports no `visibility` at all is one that predates this
    // field; there is nothing to compare, so the cursor behaves as it did before
    // and no lane is reset on every pull.
    let reported_visibility = body.get("visibility").and_then(|v| v.as_str());
    if let Some(reported) = reported_visibility {
        let stored = cursor::visibility_context(&d.store, namespace)
            .await
            .map_err(storage_err)?;
        if stored.as_deref() != Some(reported) {
            // A lane with no cursor yet is already reading from the beginning, so
            // there is nothing stale to discard and this page's cursor is
            // trustworthy — record the view and let it advance. Only a lane that
            // *has* a position built under some other view has to start over.
            // (A store upgraded from before this field has a position and no
            // recorded view, which is exactly a view it cannot vouch for.)
            let stale_position = since.is_some();

            if stale_position {
                // Order matters: the cursor is cleared before the new context is
                // recorded, so a failure between the two leaves the lane looking
                // stale and it resets again next cycle. The reverse order could
                // record the new view over a cursor that never got cleared.
                cursor::clear_pull_cursor(&d.store, namespace)
                    .await
                    .map_err(storage_err)?;
            }
            cursor::set_visibility_context(&d.store, namespace, reported)
                .await
                .map_err(storage_err)?;
            if stale_position {
                tracing::info!(
                    namespace = %namespace.key(),
                    "re-reading this lane from the beginning: the caller's view of it changed"
                );
                return Ok(landed);
            }
        }
    }

    if all_merged {
        if let Some(cursor) = body.get("cursor").and_then(|v| v.as_str()) {
            cursor::set_pull_cursor(&d.store, namespace, cursor)
                .await
                .map_err(storage_err)?;
        }
    } else {
        tracing::warn!(
            namespace = %namespace.key(),
            landed,
            dropped,
            of = rows.len(),
            "holding the pull cursor: a row in the page may still merge later"
        );
    }
    Ok(landed)
}

/// Applicability facts out of a pulled row.
///
/// A fact whose `kind` is outside the closed `language | tool` vocabulary is
/// dropped rather than guessed at, the same way the server's own ingest does:
/// inventing a kind here to carry the value through would be a second, looser
/// vocabulary living beside the closed one.
fn pulled_applicability(row: &serde_json::Value) -> Vec<ApplicabilityFact> {
    row.get("applicability")
        .and_then(|v| v.as_array())
        .map(|facts| {
            facts
                .iter()
                .filter_map(|f| {
                    let kind: ApplicabilityKind =
                        f.get("kind").and_then(|v| v.as_str())?.parse().ok()?;
                    Some(ApplicabilityFact {
                        kind,
                        value: f.get("value").and_then(|v| v.as_str())?.to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn pulled_uuid(row: &serde_json::Value, field: &str) -> Option<Uuid> {
    row.get(field)
        .and_then(|v| v.as_str())
        .and_then(|s| Uuid::parse_str(s).ok())
}

fn pulled_time(row: &serde_json::Value, field: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    row.get(field)
        .and_then(|v| v.as_str())
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|t| t.with_timezone(&chrono::Utc))
}

/// One pulled personal row into this store.
///
/// The owner is this daemon's own identity, not a field of the payload: the
/// route this row came from returns only the caller's own personal knowledge, so
/// trusting a payload field would be accepting a claim the transport already
/// answered — and answering it twice, differently, is how a record ends up filed
/// under the wrong identity.
/// Land one pulled personal row under the account whose lane delivered it.
///
/// **The owner comes from the lane key, not from `owner_identity`.** The two
/// agree in the ordinary case and are not the same thing: a lane key is fixed
/// when the lane is established, while `owner_identity` is whatever this daemon
/// currently believes it is authenticated as — and that can change underneath a
/// pull (a token set for a second account, or a stale id invalidated by
/// [`forget_account_identity`]). Reading it here would attribute one account's
/// rows to whoever happened to be current when the page landed, which is the
/// same partition-crossing this lane key exists to prevent (FR-567, FR-568).
/// A lane that names an account is the authority on whose rows it carries.
async fn merge_pulled_personal(d: &Daemon, owner: Uuid, row: &serde_json::Value) -> Merged {
    let Some(id) = pulled_uuid(row, "id") else {
        return undecodable("personal", row, "id");
    };
    let Some(writer_id) = pulled_uuid(row, "writer_id") else {
        return undecodable("personal", row, "writer_id");
    };
    let Some(created_at) = pulled_time(row, "created_at") else {
        return undecodable("personal", row, "created_at");
    };
    let knowledge_type: MemoryType = row
        .get("knowledge_type")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse().ok())
        .unwrap_or(MemoryType::Fact);

    let incoming = cairn_store::global::SyncedPersonalKnowledge {
        id,
        owner_user_id: owner,
        knowledge_type,
        content: row
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        topic_key: row
            .get("topic_key")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        value_key: row
            .get("value_key")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        applicability: pulled_applicability(row),
        writer_id,
        writer_seq: row.get("writer_seq").and_then(|v| v.as_i64()).unwrap_or(0),
        created_at,
        superseded_by_id: pulled_uuid(row, "superseded_by_id"),
        forgotten_at: pulled_time(row, "forgotten_at"),
    };

    match cairn_store::global::merge_synced_personal(&d.store, incoming).await {
        Ok(_) => Merged::Landed,
        Err(e) => {
            tracing::debug!(personal = %id, error = %e, "a pulled personal row did not merge");
            Merged::Deferred
        }
    }
}

/// One pulled pattern row into this store's cache.
///
/// **The owner comes from the lane, never from the row** — the same rule
/// `merge_pulled_personal` states just above, and it binds harder here. A
/// server-held pattern is visible only to its owner (FR-708d), so a row that
/// could name its own owner would be a row that could name somebody else's, and
/// the cache would hold a pattern this account is not entitled to read. The lane
/// key already carries the account whose feed this is; that is the authority.
///
/// The cached row is not authority either way. Losing it loses nothing the
/// server accepted (FR-703), and the merge that writes it lets the server
/// correct what is already there (FR-712a).
async fn merge_pulled_pattern(d: &Daemon, owner: Uuid, row: &serde_json::Value) -> Merged {
    let Some(pattern_id) = pulled_uuid(row, "pattern_id") else {
        return undecodable("patterns", row, "pattern_id");
    };
    let Some(created_at) = pulled_time(row, "created_at") else {
        return undecodable("patterns", row, "created_at");
    };
    let Some(updated_at) = pulled_time(row, "updated_at") else {
        return undecodable("patterns", row, "updated_at");
    };
    let text = |field: &str| {
        row.get(field)
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string()
    };
    let strings = |field: &str| {
        row.get(field)
            .and_then(serde_json::Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(serde_json::Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    };

    let incoming = cairn_store::global::SyncedPattern {
        pattern_id,
        owner_user_id: owner,
        title: text("title"),
        problem: text("problem"),
        root_cause: text("root_cause"),
        approach: text("approach"),
        constraints: strings("constraints"),
        applicability: strings("applicability"),
        content_key: text("content_key"),
        created_at,
        updated_at,
        forgotten_at: pulled_time(row, "forgotten_at"),
    };

    match cairn_store::global::merge_synced_pattern(&d.store, incoming).await {
        Ok(()) => Merged::Landed,
        Err(e) => {
            tracing::debug!(pattern = %pattern_id, error = %e, "a pulled pattern row did not merge");
            Merged::Deferred
        }
    }
}

/// One pulled team row into this store.
///
/// `merge_synced_team` refuses a row from a server instance other than the one
/// this store's team corpus is already bound to, and that refusal is not a
/// transport failure: it means the operator pointed this store at a different
/// deployment, and blending two servers' ratification histories is exactly what
/// must not happen silently (`sync-namespaces.md` §10). It is logged and the row
/// is skipped, so the lane keeps working for everything else.
pub(crate) async fn merge_pulled_team(
    d: &Daemon,
    instance: Uuid,
    row: &serde_json::Value,
) -> Merged {
    let Some(id) = pulled_uuid(row, "id") else {
        return undecodable("team", row, "id");
    };
    // The row that wedged a real lane. `writer_id` is a `TEXT` column on the
    // server and a `Uuid` on the mirror, so a value some other client invented
    // is unrepresentable here — and no retry changes that. It is dropped with
    // its id said out loud, and the lane keeps moving.
    let Some(writer_id) = pulled_uuid(row, "writer_id") else {
        return undecodable("team", row, "writer_id");
    };
    let Some(created_at) = pulled_time(row, "created_at") else {
        return undecodable("team", row, "created_at");
    };
    let Some(proposed_by_user_id) = pulled_uuid(row, "proposed_by_user_id") else {
        return undecodable("team", row, "proposed_by_user_id");
    };
    let Ok(state) = row
        .get("state")
        .and_then(|v| v.as_str())
        .unwrap_or("proposed")
        .parse::<TeamState>()
    else {
        return undecodable("team", row, "state");
    };
    let knowledge_type: MemoryType = row
        .get("knowledge_type")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse().ok())
        .unwrap_or(MemoryType::Fact);

    let incoming = cairn_store::global::SyncedTeamKnowledge {
        id,
        knowledge_type,
        content: row
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        topic_key: row
            .get("topic_key")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        value_key: row
            .get("value_key")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        applicability: pulled_applicability(row),
        state,
        proposed_by_user_id,
        ratified_by_user_id: pulled_uuid(row, "ratified_by_user_id"),
        ratified_at: pulled_time(row, "ratified_at"),
        writer_id,
        writer_seq: row.get("writer_seq").and_then(|v| v.as_i64()).unwrap_or(0),
        created_at,
        superseded_by_id: pulled_uuid(row, "superseded_by_id"),
        retired_by_user_id: pulled_uuid(row, "retired_by_user_id"),
        retired_at: pulled_time(row, "retired_at"),
        // **The version the server ordered this page by** (FR-457). Absent
        // when the peer predates the field, which the merge treats as "this
        // page cannot be ordered, so it applies" — see `merge_synced_team`.
        // Not defaulted to `created_at` or to now: a fabricated version is
        // worse than none, because none is honest about what is not known.
        server_changed_at: pulled_time(row, "changed_at"),
        // **The monotonic version, which is what actually orders two pages**
        // (FR-456, FR-457, FR-465). `changed_at` above is the server's
        // `GREATEST` over lifecycle columns stamped with transaction-start
        // `now()`, so a retirement can leave it byte-for-byte where the
        // preceding ratification left it — two states, one value. `revision`
        // comes from a sequence and every server-side write advances it.
        //
        // Absent when the peer is below server migration 5, which the merge
        // treats as "order this page by `changed_at` as before" — not as
        // revision zero. Read with `as_i64` so a JSON `null` and a missing key
        // are the same answer.
        server_revision: row.get("revision").and_then(|v| v.as_i64()),
    };

    match cairn_store::global::merge_synced_team(&d.store, instance, incoming).await {
        Ok(_) => Merged::Landed,
        Err(e) => {
            tracing::debug!(team = %id, error = %e, "a pulled team row did not merge");
            Merged::Deferred
        }
    }
}

pub(crate) struct Client {
    base: String,
    token: String,
    http: reqwest::Client,
}

pub(crate) async fn client(d: &Daemon) -> Result<Client, WireError> {
    let creds = d.server.read().await.clone();
    let base = creds.url.ok_or_else(|| {
        WireError::new(
            codes::NOT_LINKED,
            "no server configured; run `cairn auth token set`",
        )
    })?;
    let token = creds.token.ok_or_else(|| {
        WireError::new(
            codes::UNAUTHORIZED,
            "no API token; run `cairn auth token set`",
        )
    })?;
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|e| WireError::new(codes::SERVER_UNAVAILABLE, e.to_string()))?;
    Ok(Client {
        base: base.trim_end_matches('/').to_string(),
        token,
        http,
    })
}

impl Client {
    pub(crate) async fn post(
        &self,
        path: &str,
        body: &serde_json::Value,
    ) -> Result<serde_json::Value, WireError> {
        let response = self
            .http
            .post(format!("{}{path}", self.base))
            .bearer_auth(&self.token)
            .json(body)
            .send()
            .await
            .map_err(unreachable_err)?;
        decode(response).await
    }

    /// POST, distinguishing a **server answer** from a **transport failure**.
    ///
    /// `post` collapses the two: a refusal and an unreachable server both come
    /// back as `Err(WireError)`, and the drain that used them could not tell a
    /// `409 unsupported_kind` from a dropped connection. It spent an attempt on
    /// a row an upgrade would have delivered, and retried a permanent refusal
    /// forever. The difference is not cosmetic, so it is in the type.
    ///
    /// Any HTTP response at all — success or refusal — is a server answer. Only
    /// a failure to get one is transport.
    async fn post_for_outcome(
        &self,
        path: &str,
        body: &serde_json::Value,
    ) -> Result<ServerAnswer, WireError> {
        let response = self
            .http
            .post(format!("{}{path}", self.base))
            .bearer_auth(&self.token)
            .json(body)
            .send()
            .await
            .map_err(unreachable_err)?;
        let status = response.status();
        let body: serde_json::Value = response.json().await.unwrap_or(serde_json::Value::Null);
        if status.is_success() {
            return Ok(ServerAnswer::Ok);
        }
        // The structured code, kept: it is what tells a deferral from a
        // permanent refusal, and losing it is what made every refusal look
        // alike. A response with no code still yields one, because a status
        // with no body is still the server having answered.
        let code = body
            .get("error")
            .and_then(|e| e.get("code"))
            .and_then(|c| c.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| match status.as_u16() {
                401 => "unauthorized".to_string(),
                403 => "forbidden".to_string(),
                // A 5xx is the server failing rather than refusing, so it is
                // transient by code as well as by status.
                s if (500..600).contains(&s) => "server_error".to_string(),
                s => format!("http_{s}"),
            });
        Ok(ServerAnswer::Refused { code })
    }

    pub(crate) async fn get(&self, path: &str) -> Result<serde_json::Value, WireError> {
        let response = self
            .http
            .get(format!("{}{path}", self.base))
            .bearer_auth(&self.token)
            .send()
            .await
            .map_err(unreachable_err)?;
        decode(response).await
    }

    /// `PATCH /api/admin/users/{id}` is the only route this daemon calls with
    /// this verb, but it earns its own method rather than an inline
    /// `reqwest::Client` call so it shares `decode`'s error mapping with
    /// `post`/`get` above.
    async fn patch(
        &self,
        path: &str,
        body: &serde_json::Value,
    ) -> Result<serde_json::Value, WireError> {
        let response = self
            .http
            .patch(format!("{}{path}", self.base))
            .bearer_auth(&self.token)
            .json(body)
            .send()
            .await
            .map_err(unreachable_err)?;
        decode(response).await
    }

    /// `DELETE /api/projects/{id}/members` is this daemon's only `DELETE`
    /// with a body (T063) — a body on `DELETE` is unusual but valid HTTP,
    /// and axum's route for it already expects one (`api.rs`'s
    /// `MemberBody` extractor on `remove_member`).
    async fn delete(
        &self,
        path: &str,
        body: &serde_json::Value,
    ) -> Result<serde_json::Value, WireError> {
        let response = self
            .http
            .delete(format!("{}{path}", self.base))
            .bearer_auth(&self.token)
            .json(body)
            .send()
            .await
            .map_err(unreachable_err)?;
        decode(response).await
    }
}

fn unreachable_err(e: reqwest::Error) -> WireError {
    WireError::new(codes::SERVER_UNAVAILABLE, e.to_string())
}

async fn decode(response: reqwest::Response) -> Result<serde_json::Value, WireError> {
    let status = response.status();
    let body: serde_json::Value = response.json().await.unwrap_or(serde_json::Value::Null);
    if status.is_success() {
        return Ok(body);
    }
    let code = body
        .get("error")
        .and_then(|e| e.get("code"))
        .and_then(|c| c.as_str())
        .unwrap_or(if status.as_u16() == 403 {
            "forbidden"
        } else {
            "internal"
        })
        .to_string();
    let message = body
        .get("error")
        .and_then(|e| e.get("message"))
        .and_then(|m| m.as_str())
        .unwrap_or("server rejected the request")
        .to_string();
    Err(WireError::new(&code, message))
}

/// Store the API token and remember the server URL (D10).
///
/// The file is 0600 on Unix. Windows has no mode bits to set, so there it
/// inherits the privacy of the user-profile directory it sits in; see
/// `cairn_core::paths::token_path`.
pub async fn set_token(d: &Daemon, token: &str, server_url: Option<String>) -> Reply {
    cairn_core::paths::ensure_home()
        .map_err(|e| WireError::new(codes::STORAGE_UNAVAILABLE, e.to_string()))?;

    // **One transition** (FR-610). The token file, the endpoint, the account
    // identity, the generation, the config and the in-memory credential all move
    // together or none of them do.
    //
    // This was three steps — clear the identity, write the token file, then
    // change the credential — and the gaps between them were reachable. A
    // concurrent `GET /api/auth/me` answered under the *old* token could commit
    // between the first and the third, restoring the account that had just been
    // cleared; the third step then wrote the new token beside it, and neither
    // step had done anything wrong on its own. Doing it in one mutation removes
    // the window rather than narrowing it: the clear and the change are the same
    // write, and a lookup that snapshotted the old generation can no longer
    // commit at all.
    //
    // Re-setting the *same* credential is not a change and keeps the identity.
    // That case is common and offline-friendly (`cairn auth token set` re-run
    // from a script), and invalidating there would strand a user's own personal
    // rows every time they re-applied a token they already held.
    let trimmed = token.trim().to_string();
    let requested_url = server_url.clone();
    d.mutate_credentials(move |c| {
        let changed = c.token.as_deref() != Some(trimmed.as_str())
            || requested_url
                .as_ref()
                .is_some_and(|u| c.url.as_ref() != Some(u));
        c.token = Some(trimmed.clone());
        if let Some(url) = requested_url.clone() {
            c.url = Some(url);
        }
        if changed {
            // A different credential may name a different account, so the
            // recorded identity stops being evidence of anything (FR-591).
            c.account_id = None;
        }
    })
    .await
    .map_err(|e| WireError::new(codes::STORAGE_UNAVAILABLE, e.to_string()))?;
    let url = d.server.read().await.url.clone();

    // Learn which account this token belongs to, and persist it. Personal
    // knowledge is partitioned by the owning account (FR-567, FR-568), so this
    // is not a nicety: without it every identity this machine ever holds shares
    // one pool of rows, and relinking to a second server would merge two
    // people's-worth of notes with no way to separate them afterwards.
    //
    // Persisted rather than re-fetched, because a daemon that restarts offline
    // must still know which identity it holds — falling back to the local id
    // would silently reassign every existing row.
    learn_account_identity(d).await;

    // Establish the two global lanes now rather than waiting for the worker's
    // next establish window (up to `PULL_INTERVAL_SECONDS`). `cairn auth token
    // set` is the moment a user expects their personal knowledge to start
    // moving, and a lane that does not exist yet cannot pull. Failure here is
    // not an error for this command: authenticating succeeded, and the worker
    // will try again on its own schedule.
    let established = establish_global_namespaces(d).await.is_some();

    Ok(json!({
        "token_stored": true,
        "server_url": url,
        "global_namespaces_established": established,
    }))
}

/// Select the one shared project this repository already belongs to.
///
/// Exactly one match is selected; zero and more-than-one are **refused**, not
/// guessed (FR-425). Guessing among memberships the caller already holds is
/// still a decision only the human should make when it is ambiguous — the
/// single-match case is safe *because* it is unambiguous, not because
/// auto-selection is safe in general.
async fn auto_link(d: &Daemon, r: &Resolved) -> Reply {
    let Some(remote) = r
        .project
        .repository_remote
        .as_deref()
        .filter(|s| !s.is_empty())
    else {
        return Err(WireError::new(
            codes::INVALID_REQUEST,
            "this repository has no remote, so there is nothing to match a shared project \
             against; pass --project <id> or --create",
        ));
    };

    let c = client(d).await?;
    let found = c
        .get(&format!(
            "/api/projects/lookup?remote={}",
            urlencode(remote)
        ))
        .await?;
    let candidates: Vec<&serde_json::Value> = found
        .get("projects")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().collect())
        .unwrap_or_default();

    match candidates.as_slice() {
        [only] => {
            let id = only
                .get("id")
                .and_then(|v| v.as_str())
                .and_then(|s| Uuid::parse_str(s).ok())
                .ok_or_else(|| {
                    WireError::new(codes::SERVER_UNAVAILABLE, "lookup returned no project id")
                })?;
            // Attach locally, exactly as `--project <id>` would. No grant call:
            // lookup already proved the membership by returning the row.
            attach(d, r, id).await
        }
        [] => Err(WireError::new(
            codes::NOT_FOUND,
            "no shared project matches this repository and you are not a member of one. \
             Ask an admin or an existing member to add you (`cairn project member add`), \
             or pass --create to make a new shared project, or --project <id> if you \
             already know it",
        )),
        many => {
            let listed: Vec<String> = many
                .iter()
                .map(|p| {
                    format!(
                        "{} ({})",
                        p.get("id").and_then(|v| v.as_str()).unwrap_or("?"),
                        p.get("name").and_then(|v| v.as_str()).unwrap_or("?")
                    )
                })
                .collect();
            Err(WireError::new(
                codes::AMBIGUOUS_SESSION,
                format!(
                    "{} shared projects match this repository's remote and you are a member \
                     of all of them: {}. Specify one with --project <id>",
                    many.len(),
                    listed.join(", ")
                ),
            ))
        }
    }
}

/// Whether this machine holds a credential, and for which server.
///
/// The token itself is never returned — only whether one exists and where it
/// points, which is what someone asking "am I signed in?" actually needs.
pub async fn auth_status(d: &Daemon) -> Reply {
    let creds = d.server.read().await;
    Ok(json!({
        "authenticated": creds.token.is_some(),
        "server_url": creds.url,
        "token_path": cairn_core::paths::token_path().display().to_string(),
    }))
}

/// `GET /api/auth/me`: this account's id, role and status, verified fresh
/// against the server on every call (T121, FR-464).
///
/// This is the one route an authority decision may be made from. Nothing in
/// this daemon caches a role locally and trusts it later — an authority
/// claim checked against a stale local copy is not checked at all, which is
/// exactly the gap FR-464's own comment on the server's `me` handler names.
/// Every caller of this function inherits its failure mode too: an
/// unreachable server or a missing credential surfaces as this same
/// `Err`, not as an empty or default role.
pub async fn auth_me(d: &Daemon) -> Reply {
    let c = client(d).await?;
    c.get("/api/auth/me").await
}

pub async fn logout(d: &Daemon) -> Reply {
    // One transition, token file included (FR-610). Removing the file separately
    // meant a failed config write left the credential gone from disk and the
    // account identity still recorded beside it — a machine with no token that
    // still believes it is somebody.
    d.mutate_credentials(|c| {
        c.account_id = None;
        c.token = None;
    })
    .await
    .map_err(|e| {
        WireError::new(
            codes::STORAGE_UNAVAILABLE,
            format!("could not clear the stored credential, so it was left unchanged: {e}"),
        )
    })?;
    Ok(json!({ "token_stored": false }))
}

/// `POST /api/auth/password` (FR-405, `contracts/identity-administration.md`
/// §5). Self-service: the caller changes its own password with whatever
/// credential it is already holding, including a `must_change_password`
/// account's temporary one — this is the one route that stays reachable
/// while that flag is set.
pub async fn change_password(d: &Daemon, new_password: &str) -> Reply {
    let c = client(d).await?;
    c.post(
        "/api/auth/password",
        &json!({ "new_password": new_password }),
    )
    .await
}

// ---------------------------------------------------------------------------
// Administration (`contracts/identity-administration.md` §2, §2a, §9).
//
// Every account operation the CLI knows only by email; the server's routes
// are addressed by row id. This is where that gap is closed — by asking
// `GET /api/admin/users` first — rather than by the CLI ever learning or
// holding a uuid for an account.
// ---------------------------------------------------------------------------

/// Find one account by email, case-insensitively (emails are stored
/// lower-cased, `crates/cairn-server/src/auth.rs:354`), from the one route
/// that lists them all (FR-411).
async fn find_user(c: &Client, email: &str) -> Result<serde_json::Value, WireError> {
    let needle = email.trim().to_lowercase();
    let listed = c.get("/api/admin/users").await?;
    listed
        .get("users")
        .and_then(|v| v.as_array())
        .and_then(|users| {
            users
                .iter()
                .find(|u| u.get("email").and_then(|e| e.as_str()) == Some(needle.as_str()))
        })
        .cloned()
        .ok_or_else(|| WireError::not_found(format!("no account with email {email}")))
}

/// `POST /api/admin/users` (FR-401). The temporary password in the response
/// is shown to the caller exactly once — there is no route that reads it back
/// (FR-403).
pub async fn admin_user_create(d: &Daemon, email: &str, display_name: &str) -> Reply {
    let c = client(d).await?;
    c.post(
        "/api/admin/users",
        &json!({ "email": email, "display_name": display_name }),
    )
    .await
}

/// `GET /api/admin/users`: every account, its role and its status (FR-411).
pub async fn admin_user_list(d: &Daemon) -> Reply {
    let c = client(d).await?;
    c.get("/api/admin/users").await
}

/// `PATCH /api/admin/users/{id}`: promote, demote, disable or enable one
/// account (FR-402, FR-408, FR-412), addressed by email.
pub async fn admin_user_patch(
    d: &Daemon,
    email: &str,
    role: Option<ServerRole>,
    status: Option<UserStatus>,
) -> Reply {
    let c = client(d).await?;
    let target = find_user(&c, email).await?;
    let id = target.get("id").and_then(|v| v.as_str()).ok_or_else(|| {
        WireError::new(codes::SERVER_UNAVAILABLE, "server returned no account id")
    })?;
    let mut body = json!({});
    if let Some(role) = role {
        body["role"] = json!(role.as_str());
    }
    if let Some(status) = status {
        body["status"] = json!(status.as_str());
    }
    c.patch(&format!("/api/admin/users/{id}"), &body).await
}

/// `POST /api/admin/users/{id}/reset-password` (FR-553–FR-559). The target's
/// current `status` rides along in the reply — read from the same lookup that
/// resolved the email — so the CLI can say when a reset landed on an account
/// that remains disabled (FR-558) without a second round trip.
pub async fn admin_reset_password(d: &Daemon, email: &str) -> Reply {
    let c = client(d).await?;
    let target = find_user(&c, email).await?;
    let id = target.get("id").and_then(|v| v.as_str()).ok_or_else(|| {
        WireError::new(codes::SERVER_UNAVAILABLE, "server returned no account id")
    })?;
    let mut reset = c
        .post(&format!("/api/admin/users/{id}/reset-password"), &json!({}))
        .await?;
    if let Some(object) = reset.as_object_mut() {
        object
            .entry("email")
            .or_insert_with(|| target.get("email").cloned().unwrap_or(json!(email)));
        object
            .entry("status")
            .or_insert_with(|| target.get("status").cloned().unwrap_or(json!(null)));
    }
    Ok(reset)
}

// ---------------------------------------------------------------------------
// Team knowledge lifecycle (`contracts/global-memory.md` §5b, T121, T133).
//
// Ratification and retirement are administrator-only, and that authorization
// is the server's alone: each route below is gated by the server's own
// admin-only extractor, the same shape `admin_user_patch` already trusts for
// account administration. This daemon makes no local role decision in front
// of it — see `crates/cairnd/src/handlers.rs`'s `team_ratify`/`team_retire`
// for why, and for what happens to the local store once the server confirms.
// ---------------------------------------------------------------------------

/// `POST /api/team/{id}/ratify` (T121, T133). Compare-and-swap on the
/// entry's expected state, refusing by naming its actual one — the same
/// discipline the local store's own `ratify_team` (T119) keeps, mirrored
/// here because the server is where this transition is actually authorized.
/// Ratify on the server **and** report the actor that did it (FR-606).
///
/// The caller needs both, and they must be the same account: the server decides
/// whether this actor may ratify, and the local row then records who did. Those
/// were two resolutions — a client built from one read of the credential for the
/// request, and `owner_identity` consulted afterwards for the local write — so a
/// token switch between them recorded one account as having made a decision the
/// server had authorized for another. Determining the actor *after* the remote
/// mutation is the ordering error; returning it from the call that used it is the
/// fix.
pub async fn team_ratify_remote(
    d: &Daemon,
    id: Uuid,
    supersedes: Option<Uuid>,
) -> Result<(serde_json::Value, Uuid), WireError> {
    let context = AuthenticatedContext::acquire(d).await?;
    let mut body = json!({});
    if let Some(sup) = supersedes {
        body["supersedes"] = json!(sup);
    }
    let reply = context
        .client
        .post(&format!("/api/team/{id}/ratify"), &body)
        .await?;
    stale_if_changed(&context, d, "ratification").await?;
    Ok((reply, context.account))
}

/// Whether the authenticated account is a member of this project, per the server
/// (FR-607), together with that account.
///
/// The one caller is promotion, which needs both: who is promoting, and whether
/// they have standing in the project the memory came from. Both come from one
/// context, so the answer cannot be about a different account than the record it
/// authorizes.
///
/// Replaces `r.project.linked` — "this machine once linked this project" — which
/// is a fact about the machine's past standing in for a fact about the caller's
/// present authorization. A store linked long ago by one account, now
/// authenticated as another, reported the second account as a member of a project
/// it may never have belonged to, and team promotion's non-member check (check 5)
/// passed on that.
pub async fn promoter_standing(d: &Daemon, server_project_id: Option<Uuid>) -> (Uuid, bool) {
    // **Who is acting is a local fact; what they may do is the peer's.** These
    // are separated deliberately, and getting them the same way was a defect of
    // its own: taking both from an [`AuthenticatedContext`] meant an unreachable
    // server produced "no account", so an offline *personal* promotion — which
    // needs no server at all — filed its record under the unattributed owner even
    // though this machine knew perfectly well who it was.
    //
    // The account comes from the credential this machine holds, which is knowable
    // without a network. Membership does not, and an unreachable server is not a
    // yes: a team promotion refuses rather than guessing, which is the same
    // fail-closed answer as being genuinely unauthorized (FR-607).
    let Some(account) = d.account_identity().await else {
        return (cairn_core::domain::UNATTRIBUTED_OWNER, false);
    };
    let member = match server_project_id {
        Some(id) => match AuthenticatedContext::acquire(d).await {
            Ok(context) => context.is_member_of(id).await,
            Err(_) => false,
        },
        // A project that has never been shared with a server has no membership to
        // check, and nothing can be promoted out of it to a team that cannot see
        // it either.
        None => false,
    };
    (account, member)
}

/// Refuse to record a decision whose authorization was granted to a credential
/// this machine no longer holds (FR-604).
///
/// The server authorized *an account*, and the local row is about to say that
/// account decided this. If the credential changed while the request was in
/// flight, the two halves would describe different people — so the local half
/// does not happen, and the caller is told rather than left with a divergence it
/// cannot see. The server-side effect stands; it was authorized when it was made.
async fn stale_if_changed(
    context: &AuthenticatedContext,
    d: &Daemon,
    what: &str,
) -> Result<(), WireError> {
    if context.still_current(d).await {
        return Ok(());
    }
    Err(WireError::new(
        codes::UNAUTHORIZED,
        format!(
            "the signed-in account changed while this {what} was in flight; it was \
             applied on the server but not recorded locally — run `cairn sync now`"
        ),
    ))
}

/// `POST /api/team/{id}/retire` (T121, T133). Same admin gate and
/// compare-and-swap shape as [`team_ratify_remote`].
/// As [`team_ratify_remote`], for retirement, and for the same reason (FR-606).
/// Take the server's answer for a team entry the local CAS could not apply.
///
/// **The gap this closes.** `ratify` and `retire` write locally with a
/// compare-and-swap on the state they expect, and when that swap loses they
/// return the server's answer and leave the local row exactly as it was. The
/// caller is told the transition happened — it did, on the server — while this
/// device goes on showing `proposed` for guidance the whole deployment is now
/// following, and `retired_by_user_id` stays empty on the very machine that
/// retired it.
///
/// The swap loses for ordinary reasons: the ratification that made the row
/// authoritative had not landed locally yet, or a pull re-merged it in between.
/// Neither is an error, and neither is a reason to keep a stale copy — under
/// FR-712a the local row is a cache and the server's answer is the correct
/// content for it. This is that rule applied to the one path that predates it.
///
/// **Failure refuses, and no longer reports success.** What stood here said the
/// failure was logged and swallowed, because "the transition already happened on
/// the server, so refusing the caller now would report a failure that did not
/// occur; the next pull repairs the row." Both halves of that were wrong in the
/// same direction. The command's whole job on this path is to record the
/// transition locally, so a caller told `ok` when nothing was written is told
/// the opposite of what happened — and the local half of FR-457 ("who acted,
/// inspectable after the transition") is exactly what did not get recorded. The
/// next pull *may* repair the row; it may also be the pull that overwrote it,
/// and it is not something the caller can see either way.
///
/// So this reports what [`stale_if_changed`] reports for the same shape of
/// half-completed write: the server's effect stands, this device recorded
/// nothing, and `cairn sync now` is the way to reconcile. A refusal naming the
/// server-side success is strictly more information than a success naming
/// nothing.
pub(crate) async fn adopt_team_answer(
    d: &Daemon,
    reply: &serde_json::Value,
) -> Result<(), WireError> {
    let row = reply.get("entry").unwrap_or(reply);
    // **Not `merge_pulled_team`**, which was the first attempt and could never
    // have worked. That function builds a whole `SyncedTeamKnowledge` and needs
    // `writer_id`, `created_at` and the content; a transition reply carries the
    // id, the new state, who acted and when, and nothing else. So the merge
    // failed on every call, logged a line nobody read, and left exactly the
    // stale row this function exists to repair.
    //
    // A reply this device cannot read is a reply this device cannot apply, which
    // is the same outcome as a failed write and is reported as one. Returning
    // silently here was the other half of the same hole.
    let Some(state) = row
        .get("state")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<cairn_core::domain::TeamState>().ok())
    else {
        return Err(not_recorded_locally(
            "the server's answer could not be read",
        ));
    };
    let actor = ["retired_by_user_id", "ratified_by_user_id"]
        .iter()
        .find_map(|k| row.get(k).and_then(|v| v.as_str()))
        .and_then(|s| Uuid::parse_str(s).ok());
    let at = ["retired_at", "ratified_at"]
        .iter()
        .find_map(|k| row.get(k).and_then(|v| v.as_str()));
    // The monotonic version this transition was assigned, straight from the
    // reply that made it — see [`team_transition_version`], which extracts the
    // same two facts for the swap path. Absent from a server below server
    // migration 5, which leaves the adoption ordered by `at` as before.
    let revision = row.get("revision").and_then(|v| v.as_i64());
    if let Err(e) =
        cairn_store::global::adopt_team_transition(&d.store, id_of(row), state, actor, at, revision)
            .await
    {
        tracing::warn!(error = %e, "the server's team answer did not apply locally");
        return Err(not_recorded_locally(&e.to_string()));
    }
    Ok(())
}

/// The refusal for a transition the server made and this device did not record.
///
/// Worded as [`stale_if_changed`]'s is, because it is the same situation
/// reached by a different route: the server-side effect stands and was
/// authorized, the local record does not exist, and the caller needs to know
/// which of the two it is holding.
fn not_recorded_locally(why: &str) -> WireError {
    WireError::new(
        codes::STORAGE_UNAVAILABLE,
        format!(
            "the transition was applied on the server but not recorded locally ({why}) — run `cairn sync now`"
        ),
    )
}

/// The server version a transition reply carries, if any.
///
/// **Extraction only — the write belongs in the transition's own transaction.**
/// The row's version and the transition itself have to be recorded together:
/// written as two transactions, there is a window in which the row already
/// holds the new actor while `server_changed_at` still names the previous
/// page's version, and a page fetched before the transition is admitted through
/// [`cairn_store::global::merge_synced_team`]'s guard and erases the actor —
/// the same defect, through a much narrower door. So this hands the value to
/// `retire_team_at_version` / `ratify_team_at_version` and writes nothing
/// itself.
///
/// **Both halves, and the revision is the one that decides.** `revision` is
/// `team_knowledge.revision` as the server assigned it to this very write — a
/// sequence value taken at statement time, monotonic in the order the writes
/// happened. `changed_at` is reconstructed from the transition's own timestamp,
/// which is also the server's older ordering key for the row after it
/// (`GREATEST(created_at, ratified_at, retired_at, superseded_at)`), so a reply
/// saying "retired at T" is a reply saying "this row's `changed_at` is now T".
///
/// The timestamp is kept because a server below server migration 5 sends no
/// revision and must still work; it cannot replace one, because those columns
/// are stamped with transaction-start `now()` and a retirement can therefore
/// leave that `GREATEST` exactly where the preceding ratification left it.
///
/// A half the reply did not carry, or that this store cannot read, is `None`
/// and leaves that mark alone rather than guessing.
pub(crate) fn team_transition_version(
    reply: &serde_json::Value,
) -> cairn_store::global::ServerVersion {
    let row = reply.get("entry").unwrap_or(reply);
    cairn_store::global::ServerVersion {
        changed_at: ["retired_at", "ratified_at"]
            .iter()
            .find_map(|k| pulled_time(row, k)),
        revision: row.get("revision").and_then(|v| v.as_i64()),
    }
}

/// The id a transition reply names.
fn id_of(row: &serde_json::Value) -> Uuid {
    row.get("id")
        .and_then(|v| v.as_str())
        .and_then(|s| Uuid::parse_str(s).ok())
        .unwrap_or_else(Uuid::nil)
}

pub async fn team_retire_remote(
    d: &Daemon,
    id: Uuid,
) -> Result<(serde_json::Value, Uuid), WireError> {
    let context = AuthenticatedContext::acquire(d).await?;
    let reply = context
        .client
        .post(&format!("/api/team/{id}/retire"), &json!({}))
        .await?;
    stale_if_changed(&context, d, "retirement").await?;
    Ok((reply, context.account))
}

// ---------------------------------------------------------------------------
// Shared-project membership (`contracts/identity-administration.md` §9a,
// T063). Every route the server exposes here is addressed by user id
// (`api.rs`'s `MemberBody`, deliberately — see its own doc comment on why an
// email-addressed grant route would be an enumeration oracle); this is where
// that is closed the same way [`admin_user_patch`] closes it for accounts,
// by asking `GET /api/admin/users` first rather than the CLI ever learning
// or holding a uuid.
// ---------------------------------------------------------------------------

/// `POST /api/projects/{id}/members` — grant membership by email (T063,
/// FR-418, FR-419).
pub async fn project_member_add(d: &Daemon, project_id: Uuid, email: &str) -> Reply {
    let c = client(d).await?;
    let target = find_user(&c, email).await?;
    let user_id = target.get("id").and_then(|v| v.as_str()).ok_or_else(|| {
        WireError::new(codes::SERVER_UNAVAILABLE, "server returned no account id")
    })?;
    c.post(
        &format!("/api/projects/{project_id}/members"),
        &json!({ "user_id": user_id }),
    )
    .await
}

/// `DELETE /api/projects/{id}/members` — revoke membership by email (T063,
/// FR-420, FR-421). Same email-to-id resolution as [`project_member_add`].
pub async fn project_member_remove(d: &Daemon, project_id: Uuid, email: &str) -> Reply {
    let c = client(d).await?;
    let target = find_user(&c, email).await?;
    let user_id = target.get("id").and_then(|v| v.as_str()).ok_or_else(|| {
        WireError::new(codes::SERVER_UNAVAILABLE, "server returned no account id")
    })?;
    c.delete(
        &format!("/api/projects/{project_id}/members"),
        &json!({ "user_id": user_id }),
    )
    .await
}

/// `GET /api/projects/{id}/members` — the full membership list (T063,
/// FR-427). No email resolution needed: the server already returns email
/// and display name alongside each member's id.
pub async fn project_member_list(d: &Daemon, project_id: Uuid) -> Reply {
    let c = client(d).await?;
    c.get(&format!("/api/projects/{project_id}/members")).await
}

/// Opt a project into sharing.
///
/// `create` mints a shared project; `server_project_id` joins one. With
/// neither, remote-based candidates are *offered* for the user to confirm —
/// never applied silently (FR-064, D14).
pub async fn link(d: &Daemon, cwd: &str, server_project_id: Option<Uuid>, create: bool) -> Reply {
    let r = d.resolve(cwd).await?;

    // No arguments, already linked: a question, not an instruction — "am I
    // linked?" — answered entirely from local state, before any server is
    // contacted.
    if server_project_id.is_none() && !create && r.project.server_project_id.is_some() {
        return link_status(d, &r).await;
    }

    // No arguments, not yet linked: attempt safe auto-link (FR-424, FR-425,
    // D14). This is the cloned-repository case — a teammate who has been granted
    // membership out of band runs `cairn link` in a fresh clone and expects it to
    // find the project.
    //
    // Safe because of what it draws from, not because auto-selection is
    // inherently safe: `GET /api/projects/lookup` returns **only** projects the
    // caller is already a member of (server `api.rs`, membership join), so the
    // candidate set cannot contain anything the caller was not already entitled
    // to. There is no membership-granting call on this path at all — the deleted
    // join route was exactly that, and this replaces it with a *selection* among
    // rows the caller already holds.
    if server_project_id.is_none() && !create {
        // Only when a server is actually configured. Bare `link` on a machine
        // with no credential is still a question — "am I linked?" — and must be
        // answered from local state, exactly as it was before auto-link existed.
        // Reaching for the network here turned a local status query into a
        // connection error, which is a worse answer to a question the store can
        // answer on its own.
        if d.server.read().await.token.is_none() {
            return link_status(d, &r).await;
        }
        return auto_link(d, &r).await;
    }

    let c = client(d).await?;

    let target = match (server_project_id, create) {
        // Linking a project the caller is already a member of is a *local*
        // attach, and now says so.
        //
        // It used to `POST /api/projects/{id}/join`, and that route was removed
        // as a security fix: it granted membership to anyone who could name a
        // project UUID, and `GET /api/projects/lookup` handed those UUIDs out
        // for any git remote. Confirming an existing membership needs no grant —
        // `GET /api/projects` already returns exactly the caller's own
        // memberships — so the check moves here and the server grants nothing.
        //
        // A non-member now gets a refusal naming what to do about it, rather
        // than silently becoming a member.
        (Some(id), _) => {
            let mine = c.get("/api/projects").await?;
            let is_member =
                mine.get("projects")
                    .and_then(|v| v.as_array())
                    .is_some_and(|projects| {
                        projects.iter().any(|p| {
                            p.get("id").and_then(|v| v.as_str()) == Some(id.to_string().as_str())
                        })
                    });
            if !is_member {
                return Err(WireError::new(
                    codes::UNAUTHORIZED,
                    format!(
                        "you are not a member of project {id}; ask a member to add you, \
                         then run this again"
                    ),
                ));
            }
            id
        }
        (None, true) => {
            let body = json!({
                "name": r.project.name,
                "repository_remote": r.project.repository_remote,
            });
            let created = c.post("/api/projects", &body).await?;
            created
                .get("id")
                .and_then(|v| v.as_str())
                .and_then(|s| Uuid::parse_str(s).ok())
                .ok_or_else(|| {
                    WireError::new(codes::SERVER_UNAVAILABLE, "server returned no project id")
                })?
        }
        // Handled above, before the client was built. Returned as an error
        // rather than `unreachable!`: this is a daemon serving other
        // sessions, and a refactor that lets this arm be reached should cost
        // one failed request, not the process.
        (None, false) => {
            return Err(WireError::invalid(
                "bare `link` is answered from local state; this is a bug",
            ));
        }
    };

    attach(d, &r, target).await
}

/// Record the local project as linked to `target` and seed the outbox.
///
/// Shared by the explicit `--project`, `--create` and auto-link paths so all
/// three attach identically. Auto-link in particular must be indistinguishable
/// from the explicit form once the target is chosen — the whole claim is that it
/// only *chooses*, and choosing differently is the only thing it does.
async fn attach(d: &Daemon, r: &Resolved, target: Uuid) -> Reply {
    let project = repo::link_project(&d.store, r.project.id, target)
        .await
        .map_err(storage_err)?;

    // Seed the queue with what already exists locally, so linking an
    // established project shares its history rather than only its future.
    backfill(d, &project).await?;

    Ok(json!({
        "linked": true,
        "project": ProjectSummary::from(&project),
        "server_project_id": target,
    }))
}

/// How long bare `cairn link` will wait on a server for candidate projects.
///
/// Short on purpose. The answer it is really giving — linked or not — comes
/// from the local row, so an unreachable server must cost a moment rather
/// than the shared client's full 20 seconds.
const CANDIDATE_LOOKUP_BUDGET: Duration = Duration::from_secs(3);

/// Answer bare `cairn link`: am I linked, and if not, what could I join?
///
/// Whether this project is linked is local state, so the answer comes from
/// the project row and never from the network (C1). This used to report
/// `linked: false` unconditionally — so a linked project was told it was not
/// linked and pointed at `cairn link --create`, which would have made a
/// second shared project for a repository that already had one, while `cairn
/// status` read the same row and said the opposite. It also used to fail
/// outright with `no server configured` on a machine that simply had not
/// stored one, for a question that needs no server to answer.
async fn link_status(d: &Daemon, r: &crate::state::Resolved) -> Reply {
    match (r.project.linked, r.project.server_project_id) {
        (true, Some(target)) => Ok(json!({
            "linked": true,
            "project": ProjectSummary::from(&r.project),
            "server_project_id": target,
            "hint": "already linked; run `cairn unlink` to stop sharing, \
                     or `cairn link --project <id>` to join a different one",
        })),

        // Linked to nothing. The schema permits the pair to disagree and
        // nothing in this codebase writes it, so reaching here means the row
        // was damaged. Reporting "not linked" would put us straight back to
        // contradicting `cairn status`, which reads the same row and reports
        // linked; say what is actually wrong instead.
        (true, None) => Err(WireError::new(
            codes::STORAGE_UNAVAILABLE,
            "this project is marked linked but records no shared project id; \
             run `cairn unlink` and link it again",
        )),

        // Not linked. Candidates are a convenience that needs a server, but
        // the answer itself does not: a machine with no server configured
        // still gets a truthful "not linked" rather than an error.
        //
        // A *configured but unreachable* server is the case that bites. The
        // shared client allows 20s, and spending that on a question answered
        // from the local row would make a nonsense of calling this offline —
        // so the lookup gets its own short budget and the answer goes out
        // with an empty list when it expires.
        (false, _) => {
            let candidates = match client(d).await {
                Ok(c) => {
                    let remote = r.project.repository_remote.clone().unwrap_or_default();
                    let path = format!("/api/projects/lookup?remote={}", urlencode(&remote));
                    tokio::time::timeout(CANDIDATE_LOOKUP_BUDGET, c.get(&path))
                        .await
                        .unwrap_or_else(|_| Ok(json!({ "projects": [] })))
                        .unwrap_or_else(|_| json!({ "projects": [] }))
                }
                Err(_) => json!({ "projects": [] }),
            };
            // Discovery hint only. The user picks (D14).
            Ok(json!({
                "linked": false,
                "candidates": candidates.get("projects").cloned().unwrap_or(json!([])),
                "hint": "run `cairn link --create` for a new shared project, \
                         or `cairn link --project <id>` to join one",
            }))
        }
    }
}

fn urlencode(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '.' | '~' => c.to_string(),
            other => format!("%{:02X}", other as u32),
        })
        .collect()
}

/// Queue everything already stored for a newly linked project.
async fn backfill(d: &Daemon, project: &Project) -> Result<(), WireError> {
    let policy = outbox::SyncPolicy::from_project(project);
    // The same immediate write transaction the store uses everywhere, so
    // queuing work never loses a race with capture (FR-047).
    let mut tx = cairn_store::tx::begin(&d.store, "backfill")
        .await
        .map_err(storage_err)?;

    outbox::enqueue(
        &mut *tx,
        policy,
        project.id,
        OutboxEntityType::Project,
        project.id,
        OutboxOperation::Upsert,
        &outbox::project_payload(project),
    )
    .await
    .map_err(storage_err)?;
    cairn_store::tx::commit(tx, "backfill")
        .await
        .map_err(storage_err)?;

    for s in repo::list_sessions(&d.store, project.id)
        .await
        .map_err(storage_err)?
    {
        enqueue_one(
            d,
            policy,
            project.id,
            OutboxEntityType::Session,
            s.id,
            outbox::session_payload(&s),
        )
        .await?;
        for h in repo::handoffs_for_session(&d.store, s.id)
            .await
            .map_err(storage_err)?
        {
            enqueue_one(
                d,
                policy,
                project.id,
                OutboxEntityType::Handoff,
                h.id,
                outbox::handoff_payload(&h),
            )
            .await?;
        }
    }
    for m in shared_memories(d, project.id).await? {
        enqueue_one(d, policy, project.id, OutboxEntityType::Memory, m.id, {
            // No transaction is open here, so a pooled connection is taken
            // for the read. A payload that cannot be enriched still syncs
            // its Feature 001 shape rather than being dropped.
            let mut conn = d
                .store
                .pool()
                .acquire()
                .await
                .map_err(|e| WireError::new(codes::STORAGE_UNAVAILABLE, e.to_string()))?;
            outbox::memory_payload_for(&mut conn, &m)
                .await
                .unwrap_or_else(|_| outbox::memory_payload(&m))
        })
        .await?;
    }
    Ok(())
}

/// Memories eligible for sharing. `local_only` never leaves the machine.
async fn shared_memories(d: &Daemon, project_id: Uuid) -> Result<Vec<Memory>, WireError> {
    let q = MemoryQuery {
        limit: Some(50),
        ..Default::default()
    };
    let results = cairn_store::search::search(
        &d.store,
        project_id,
        &q,
        &cairn_store::search::SearchContext::default(),
    )
    .await
    .map_err(storage_err)?;

    let mut out = Vec::new();
    for r in results.into_iter().filter(|r| !r.local_only) {
        if let Ok(m) = repo::memory(&d.store, r.id).await {
            out.push(m);
        }
    }
    Ok(out)
}

async fn enqueue_one(
    d: &Daemon,
    policy: outbox::SyncPolicy,
    project_id: Uuid,
    entity_type: OutboxEntityType,
    entity_id: Uuid,
    payload: serde_json::Value,
) -> Result<(), WireError> {
    // The same immediate write transaction the store uses everywhere, so
    // queuing work never loses a race with capture (FR-047).
    let mut tx = cairn_store::tx::begin(&d.store, "enqueue_one")
        .await
        .map_err(storage_err)?;
    outbox::enqueue(
        &mut *tx,
        policy,
        project_id,
        entity_type,
        entity_id,
        OutboxOperation::Upsert,
        &payload,
    )
    .await
    .map_err(storage_err)?;
    cairn_store::tx::commit(tx, "backfill")
        .await
        .map_err(storage_err)?;
    Ok(())
}

pub async fn status(d: &Daemon, cwd: &str) -> Reply {
    let r = d.resolve(cwd).await?;
    let (pending, failed) = outbox::counts(&d.store, r.project.id)
        .await
        .map_err(storage_err)?;
    let payload = SyncStatusPayload {
        linked: r.project.linked,
        server_project_id: r.project.server_project_id,
        server_url: d.server.read().await.url.clone(),
        pending,
        failed,
        last_success_at: cursor::last_success_at(&d.store, &SyncNamespace::Project(r.project.id))
            .await
            .map_err(storage_err)?,
        failures: outbox::failures(&d.store, r.project.id)
            .await
            .map_err(storage_err)?,
        degradation: degradation(d, r.project.id).await,
    };
    Ok(serde_json::to_value(payload).unwrap_or(json!({})))
}

/// What this project is holding back, and why (T112, FR-415).
///
/// `None` when nothing is blocked, so an ordinary deployment reports nothing
/// and the field costs a reader nothing. When something is blocked the answer
/// names the gap and says the work will be delivered automatically — a count
/// with no explanation would read as data loss.
pub async fn degradation(d: &Daemon, project_id: Uuid) -> Option<SyncDegradation> {
    let items = outbox::blocked(&d.store, project_id).await.ok()?;
    if items.is_empty() {
        return None;
    }
    let capability = cursor::server_capability(&d.store, &SyncNamespace::Project(project_id))
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| UNKNOWN_CAPABILITY.to_string());

    let mut missing: Vec<String> = items
        .iter()
        .filter_map(|i| {
            ENTITY_CAPABILITIES
                .iter()
                .find(|(entity, _)| *entity == i.entity_type)
                .map(|(_, needs)| needs.join(" or "))
        })
        .collect();
    missing.sort();
    missing.dedup();

    let (pending, _) = outbox::counts(&d.store, project_id).await.ok()?;
    Some(SyncDegradation {
        blocked: items.len() as i64,
        server_capability: capability,
        note: format!(
            "{} item(s) are waiting for this server to gain {}. Everything else \
             syncs normally ({pending} queued), nothing has been lost, and the \
             retained work is delivered automatically once the server is upgraded.",
            items.len(),
            missing.join(", ")
        ),
        missing_capabilities: missing,
    })
}

/// Drain the outbox, then pull shared records produced by other members.
pub async fn sync_now(d: &Daemon, cwd: &str) -> Reply {
    let r = d.resolve(cwd).await?;
    if !r.project.linked {
        // An unlinked project sends nothing, ever (FR-053, SC-010).
        return Err(WireError::new(
            codes::NOT_LINKED,
            "this project is not linked; run `cairn link`",
        ));
    }
    let server_project_id = r
        .project
        .server_project_id
        .ok_or_else(|| WireError::new(codes::NOT_LINKED, "linked project has no server id"))?;

    // **A credential that no longer belongs to this project is a state, not a
    // failed command** (FR-595).
    //
    // The project lane is one of three this command drains, and it is the only
    // one whose failure used to end the whole call: the global loop below
    // deliberately ignores a lane's error, while this `?` returned before the
    // loop was reached. A store linked as A and then authenticated as B is
    // exactly that case — B is not a member of A's project, the batch route
    // refuses it, and B's own personal and team knowledge then never moved
    // either, because the command stopped one line above where they are sent.
    //
    // Only a refusal. Any other error still ends the call, because a store that
    // cannot reach its own server has nothing useful to say about the rest.
    let mut project_refused = false;
    let (mut applied, mut duplicate, mut rejected) =
        match drain(d, r.project.id, server_project_id).await {
            Ok(counts) => counts,
            Err(e) if e.code == codes::FORBIDDEN => {
                tracing::info!(
                    project = %r.project.id,
                    "this account may not push this project's work; \
                     draining the account's own lanes instead (FR-595)"
                );
                project_refused = true;
                (0, 0, 0)
            }
            Err(e) => return Err(e),
        };
    let mut pulled = if project_refused {
        0
    } else {
        pull(d, r.project.id, server_project_id).await.unwrap_or(0)
    };

    // Not a success for a lane that was refused: recording one would mark this
    // project synchronized as of now, and nothing of it was sent.
    if rejected == 0 && !project_refused {
        cursor::record_success(&d.store, &SyncNamespace::Project(r.project.id))
            .await
            .map_err(storage_err)?;
    }

    // Every lane, not only this project's. `cairn sync now` is what a user runs
    // when they want their machine caught up *now*, and answering only for the
    // project lane meant personal and team knowledge moved solely on the
    // background worker's 30-second cadence — so "sync now" was true of one
    // third of what the command is named after, and a user who ran it and then
    // checked the other machine would reasonably conclude sync was broken.
    //
    // Lanes are established first, because a store authenticated since the last
    // establish window has none yet and there would be nothing to drain.
    // **Per lane, and said rather than inferred from a zero** (FR-792's rule
    // applied to the command that does the delivering). Four of the five ways a
    // global lane can move nothing are a delivery that did not happen, and the
    // aggregate counts below cannot tell any of them from an empty queue — nor
    // from a lane this command never looked at, which is its own answer and the
    // one that is invisible without this. A test or an operator asking "why did
    // my proposal not go out" reads this; before it, there was nothing to read.
    let mut lanes: Vec<serde_json::Value> = Vec::new();
    let (syncable, withheld) = global_lane_targets(d).await;
    for namespace in syncable {
        let key = namespace.key();
        let (drained, error) = match drain_global(d, &namespace).await {
            Ok(drained) => {
                applied += drained.applied;
                duplicate += drained.duplicate;
                rejected += drained.rejected;
                (Some(drained), None)
            }
            // Still not fatal to the command — one lane's unreachable server
            // says nothing about the next lane — but no longer discarded
            // either: a push that failed is not a push that found nothing.
            Err(e) => (None, Some(e.message)),
        };
        let lane_pulled = pull_global(d, &namespace).await.unwrap_or(0);
        pulled += lane_pulled;
        lanes.push(json!({
            "namespace": key,
            "applied": drained.as_ref().map(|d| d.applied).unwrap_or(0),
            "duplicate": drained.as_ref().map(|d| d.duplicate).unwrap_or(0),
            "rejected": drained.as_ref().map(|d| d.rejected).unwrap_or(0),
            "pulled": lane_pulled,
            "hold": drained
                .as_ref()
                .map(|d| d.hold)
                .unwrap_or(LaneHold::None)
                .as_str(),
            "error": error,
        }));
    }

    Ok(json!({
        "applied": applied,
        "duplicate": duplicate,
        "rejected": rejected,
        "pulled": pulled,
        // Said rather than inferred from a zero: "this project's work stayed
        // put because this credential may not push it" and "there was nothing
        // to push" are different answers.
        "project_forbidden": project_refused,
        // The account every lane above was routed and filtered by. `null` means
        // this machine could not establish who it is, which is the one state in
        // which *every* global lane is skipped and the list above is empty for a
        // reason that has nothing to do with any lane (FR-603).
        "account": d.account_identity().await.map(|a| a.to_string()),
        "lanes": lanes,
        // Lanes this store holds and this credential may not act on. Reported
        // because "the lane was skipped" and "the lane had nothing" are
        // different answers and both used to render as silence (FR-593).
        "lanes_withheld": withheld,
    }))
}

async fn drain(
    d: &Daemon,
    project_id: Uuid,
    server_project_id: Uuid,
) -> Result<(usize, usize, usize), WireError> {
    // One drain at a time in this process. Claiming makes concurrent drains
    // correct; this makes them orderly, so `cairn sync now` returns having
    // emptied the queue rather than having emptied its own share of it.
    let _drain_guard = d.sync_drain.lock().await;

    // Once per drain cycle, not once per item and not once per tick with an
    // empty queue: the probe is cheap, but a request per row against a server
    // that just refused everything is exactly the futile traffic `blocked`
    // exists to avoid (FR-418).
    let capability = refresh_capability(d, &SyncNamespace::Project(project_id)).await;

    let (mut applied, mut duplicate, mut rejected, mut blocked) = (0, 0, 0, 0);
    let mut connection: Option<Client> = None;

    // Once this store has begun migrating, its project *knowledge* belongs to
    // the migration's transfer path and stops going out through this one. Work
    // session and handoff tracking are untouched and keep syncing as before.
    let excluded: &[&str] = if legacy_writes_are_open(d).await {
        &[]
    } else {
        KNOWLEDGE_BEARING
    };

    loop {
        let batch = outbox::claim_excluding(&d.store, project_id, excluded, BATCH)
            .await
            .map_err(storage_err)?;
        if batch.is_empty() {
            break;
        }

        // Built only once there is something to send, so a queue that turns out
        // to be empty still costs no credentials and no request (SC-010).
        if connection.is_none() {
            match client(d).await {
                Ok(c) => connection = Some(c),
                Err(e) => {
                    release(d, &batch, &e.message).await?;
                    return Err(e);
                }
            }
        }
        let c = connection.as_ref().expect("a client was just built");

        let items: Vec<SyncItem> = batch.iter().map(|(_, item)| item.clone()).collect();
        let body = serde_json::to_value(SyncBatch {
            project_id: server_project_id,
            items,
        })
        .unwrap_or(json!({}));

        let response = match c.post("/api/sync/batch", &body).await {
            Ok(v) => v,
            Err(e) => {
                // Transient: release the claim and try again later.
                release(d, &batch, &e.message).await?;
                return Err(e);
            }
        };

        let parsed: SyncBatchResponse = match serde_json::from_value(response) {
            Ok(parsed) => parsed,
            Err(e) => {
                // An unreadable response says nothing about what was applied.
                // Releasing is safe because redelivery is a `duplicate`.
                let err = WireError::new(codes::SERVER_UNAVAILABLE, e.to_string());
                release(d, &batch, &err.message).await?;
                return Err(err);
            }
        };

        for (row_id, item) in &batch {
            let result = parsed
                .results
                .iter()
                .find(|r| r.idempotency_key == item.idempotency_key);
            match result.map(|r| r.status) {
                Some(SyncItemStatus::Applied) => {
                    outbox::mark_delivered(&d.store, *row_id)
                        .await
                        .map_err(storage_err)?;
                    applied += 1;
                }
                Some(SyncItemStatus::Duplicate) => {
                    outbox::mark_delivered(&d.store, *row_id)
                        .await
                        .map_err(storage_err)?;
                    duplicate += 1;
                }
                Some(SyncItemStatus::Rejected) => {
                    let error = result.and_then(|r| r.error.as_ref());
                    let msg = error
                        .map(|e| e.message.clone())
                        .unwrap_or_else(|| "rejected".into());

                    // Two kinds of "no", and they must not share a state.
                    //
                    // A **content** rejection is permanent: an observation
                    // identifier where none may go will never become
                    // acceptable, and retaining it would turn a privacy refusal
                    // into a pending delivery. A **capability** rejection says
                    // the server cannot hold this *yet*; failing it strands
                    // work that an upgrade would deliver, which is the
                    // behaviour this corrects (FR-415, FR-418, D81).
                    match error.map(|e| e.code.as_str()) {
                        Some(code) if codes::CAPABILITY_REFUSALS.contains(&code) => {
                            outbox::mark_blocked(&d.store, *row_id, code, &capability, &msg)
                                .await
                                .map_err(storage_err)?;
                            blocked += 1;
                        }
                        _ => {
                            // Permanent. Surfaced with its identity, not
                            // retried forever.
                            outbox::mark_failed(&d.store, *row_id, &msg)
                                .await
                                .map_err(storage_err)?;
                            rejected += 1;
                        }
                    }
                }
                None => {
                    outbox::mark_retryable(&d.store, *row_id, "no result for item")
                        .await
                        .map_err(storage_err)?;
                }
            }
        }
        if batch.len() < BATCH as usize {
            break;
        }
    }
    if blocked > 0 {
        tracing::info!(
            project = %project_id, blocked, capability = %capability,
            "work retained for a server that cannot hold it yet"
        );
    }
    Ok((applied, duplicate, rejected))
}

/// Any one linked project's server id, to authenticate a personal/team push
/// through (T100).
///
/// `POST /api/sync/batch` (`crates/cairn-server/src/sync.rs`) still requires a
/// `project_id` on every request, including one carrying only project-less
/// `personal_knowledge`/`team_knowledge` items: the server's `apply_item`
/// checks membership on it (`auth::require_member`) and then dispatches by the
/// item's own `entity_type`, never by that project id — for the
/// `"personal_knowledge" | "team_knowledge"` arm the project id is an
/// authorization context only, not an attribution. Any project this account
/// belongs to satisfies it. A store with no linked project at all has nothing
/// to authenticate a personal or team push through yet.
/// A server project the **currently authenticated account** is a member of, for
/// the `project_id` that `POST /api/sync/batch` authorizes against (FR-595).
///
/// A global batch carries no project — a personal or team row belongs to none —
/// but the route still needs one, because project membership is what it checks a
/// caller against. This picked the first locally linked project, and "locally
/// linked" is a fact about this machine's past, not about who is holding the
/// token now: a store linked as A and then authenticated as B offered A's
/// project, the route refused a caller who is not a member of it, and **every**
/// global push failed — personal and team both, silently, for as long as B stayed
/// logged in. Nothing in the local store can distinguish the two cases, because
/// membership is not local state.
///
/// So it comes from the context's memberships, which are the server's answer for
/// this account, intersected with what this machine has linked so an established
/// local project is preferred over an unrelated one the account happens to belong
/// to. Ordering is by id so the choice is stable across calls and across devices.
///
/// `None` means this account is a member of no project the route would accept,
/// and the drain holds its work rather than sending a batch that cannot be
/// authorized.
async fn authorization_project(context: &AuthenticatedContext, d: &Daemon) -> Option<Uuid> {
    let mine = context.memberships().await?;
    if mine.is_empty() {
        return None;
    }
    let linked: std::collections::HashSet<Uuid> = repo::list_projects(&d.store)
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|p| p.linked)
        .filter_map(|p| p.server_project_id)
        .collect();

    mine.iter()
        .find(|id| linked.contains(id))
        .or_else(|| mine.first())
        .copied()
}

// ---------------------------------------------------------------------------
// The shared spool drain primitive (T039)
// ---------------------------------------------------------------------------
//
// One drain shape for two spools. The event spool and the command spool differ
// in what they claim and where they post it, and in nothing else that matters
// here: both claim in order under an exact account, both get per-item outcomes
// back, both have to tell a permanent refusal from a transient failure and from
// a version the server cannot hold yet, and both settle every claimed row
// before returning.
//
// Written once because the interesting part is the *settling*, and settling is
// where a second implementation goes wrong quietly. A row claimed and not
// settled is a row in flight until its lease expires — recoverable, but it
// looks like progress while nothing is happening.

/// Whether the server answered at all, and what it said if it refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ServerAnswer {
    Ok,
    Refused { code: String },
}

/// What one delivered item's outcome was, in the vocabulary both spools share.
///
/// Four outcomes, and the third is the one that needs a name of its own. A
/// *permanent refusal* and a *version the server cannot hold yet* are both a
/// "no" from the server, and treating them alike either strands work an upgrade
/// would deliver or retries forever something that will never be accepted
/// (FR-772, FR-774, FR-775).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ItemOutcome {
    /// The server stored it, or already had it. A `duplicate` is a success:
    /// it is what the retry was for (FR-770, FR-786).
    Delivered,
    /// Permanent. Never retried, and it stays visible (FR-772, FR-784).
    Refused,
    /// The server cannot hold this contract version or kind yet. Deferred, not
    /// failed: an upgrade delivers it (FR-775).
    Deferred,
    /// Transport, or a response that said nothing about this item. Retried
    /// under the spool's backoff.
    Transient,
}

/// What a drain pass did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct DrainReport {
    pub delivered: usize,
    pub refused: usize,
    pub deferred: usize,
    pub transient: usize,
}

impl DrainReport {
    fn record(&mut self, outcome: ItemOutcome) {
        match outcome {
            ItemOutcome::Delivered => self.delivered += 1,
            ItemOutcome::Refused => self.refused += 1,
            ItemOutcome::Deferred => self.deferred += 1,
            ItemOutcome::Transient => self.transient += 1,
        }
    }

    /// Every row the pass settled, which must equal every row it claimed.
    ///
    /// The invariant a drain is easiest to get wrong: a claimed row that is
    /// neither delivered nor released is in flight until its lease expires, and
    /// for that minute it looks like progress while nothing is happening.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn settled(&self) -> usize {
        self.delivered + self.refused + self.deferred + self.transient
    }
}

/// Map a server's per-item error code to an outcome.
///
/// The deferral set is the existing capability-refusal set plus the two the
/// event contract adds. Sharing the set rather than restating it is the point:
/// a code the sync boundary defers and this one fails would be the drift
/// FR-760 forbids for rejection classes, moved to the delivery path.
pub(crate) fn outcome_for(code: Option<&str>) -> ItemOutcome {
    match code {
        None => ItemOutcome::Transient,
        Some(code)
            if codes::CAPABILITY_REFUSALS.contains(&code)
                || code == "contract_version_unsupported"
                || code == "unsupported_kind" =>
        {
            ItemOutcome::Deferred
        }
        // The server failed rather than refused. Transient, and it consumes an
        // attempt like any other transport-class failure — a 500 is not a
        // statement about the request.
        Some("server_error") | Some("storage_unavailable") => ItemOutcome::Transient,
        Some(_) => ItemOutcome::Refused,
    }
}

/// Settle one claimed spool row according to its outcome.
///
/// A `Deferred` row is released back to `pending` with a backoff rather than
/// being marked `refused`: the server will accept it after an upgrade, and
/// burning its attempt budget on a deferral would eventually declare an
/// upgradeable row permanently undeliverable.
async fn settle_event(
    d: &Daemon,
    event_id: uuid::Uuid,
    outcome: ItemOutcome,
    reason: &str,
) -> Result<(), WireError> {
    use cairn_store::spool;
    match outcome {
        ItemOutcome::Delivered => spool::mark_event_delivered(&d.store, event_id).await,
        ItemOutcome::Refused => spool::mark_event_refused(&d.store, event_id, reason).await,
        // Deferral costs no attempt. Routing it through the failure path was
        // the defect this replaces: `attempts` increments at claim time, so
        // every probe of an old server spent one, and a long enough old-server
        // period drove an upgradeable row to `retry_exhausted`.
        ItemOutcome::Deferred => {
            spool::mark_event_deferred(&d.store, event_id, spool::DEFERRED_AWAITING_CAPABILITY)
                .await
        }
        ItemOutcome::Transient => spool::mark_event_failed(&d.store, event_id, reason).await,
    }
    .map_err(storage_err)
}

async fn settle_command(
    d: &Daemon,
    command_id: uuid::Uuid,
    outcome: ItemOutcome,
    reason: &str,
) -> Result<(), WireError> {
    use cairn_store::spool;
    match outcome {
        ItemOutcome::Delivered => spool::mark_command_delivered(&d.store, command_id).await,
        ItemOutcome::Refused => spool::mark_command_refused(&d.store, command_id, reason).await,
        ItemOutcome::Deferred => {
            spool::mark_command_deferred(&d.store, command_id, spool::DEFERRED_AWAITING_CAPABILITY)
                .await
        }
        ItemOutcome::Transient => spool::mark_command_failed(&d.store, command_id, reason).await,
    }
    .map_err(storage_err)
}

/// Drain the event spool once, in claim order, settling every claimed row.
///
/// **Every claimed row is settled before this returns, including on the error
/// paths.** A claimed row that is neither delivered nor released is in flight
/// until its lease expires — recoverable, but for a minute it looks like
/// progress while nothing is happening, and a drain that returned early on a
/// transport error used to leave exactly that.
///
/// The account and the server come from one credential read, so a switch mid-
/// drain cannot route as one identity and authenticate as another (FR-597), and
/// rows stay bound to the account that authored them (FR-790).
pub(crate) async fn drain_event_spool(d: &Daemon, limit: i64) -> Result<DrainReport, WireError> {
    use cairn_store::spool;
    let _drain_guard = d.sync_drain.lock().await;
    // Acquiring the context reads `/api/version`, so its failure is where an
    // outage first stops this drain. It is only returned, never recorded: what
    // status reports about reachability comes from status's own bounded sample
    // (FR-792a), because a flag left in this process's memory is gone the next
    // time a daemon is replaced — which is exactly when an operator asks.
    let context = AuthenticatedContext::acquire(d).await?;

    let claimed = spool::claim_events(&d.store, context.account, context.peer_instance, limit)
        .await
        .map_err(storage_err)?;
    let mut report = DrainReport::default();
    if claimed.is_empty() {
        return Ok(report);
    }

    let events: Vec<serde_json::Value> = claimed
        .iter()
        .map(|c| serde_json::to_value(&c.event).unwrap_or(serde_json::Value::Null))
        .collect();
    let body = serde_json::json!({
        "contract_version": cairn_core::event::CONTRACT_VERSION,
        "events": events,
    });

    let response = match context.client.post("/api/events/batch", &body).await {
        Ok(response) => response,
        Err(e) => {
            // Transport. Every claimed row is released with a backoff rather
            // than left in flight, because the alternative is a minute of
            // apparent progress after a failure that already happened.
            for c in &claimed {
                settle_event(d, c.event_id, ItemOutcome::Transient, "transport").await?;
                report.record(ItemOutcome::Transient);
            }
            return Err(e);
        }
    };

    let results = response
        .get("results")
        .and_then(|r| r.as_array())
        .cloned()
        .unwrap_or_default();
    for c in &claimed {
        let found = results
            .iter()
            .find(|r| r.get("event_id").and_then(|v| v.as_str()) == Some(&c.event_id.to_string()));
        // An item with no result in the response is transient, not delivered.
        // Assuming success for a silence would mark a row delivered that the
        // server may never have seen.
        let outcome = match found.and_then(|r| r.get("status").and_then(|s| s.as_str())) {
            Some("accepted") | Some("duplicate") => ItemOutcome::Delivered,
            Some("rejected") => {
                outcome_for(found.and_then(|r| r.get("reason").and_then(|s| s.as_str())))
            }
            _ => ItemOutcome::Transient,
        };
        let reason = found
            .and_then(|r| r.get("reason").and_then(|s| s.as_str()))
            .unwrap_or("no result for item");
        settle_event(d, c.event_id, outcome, reason).await?;
        report.record(outcome);
    }
    Ok(report)
}

/// Drain the command spool once, in scope order.
///
/// Ordering is the difference from the event drain and it is enforced by the
/// claim, not here: a supersede queued after its target has to reach the server
/// after it, and `claim_commands` will not hand out a row whose scope has an
/// earlier unsettled one.
pub(crate) async fn drain_command_spool(d: &Daemon, limit: i64) -> Result<DrainReport, WireError> {
    use cairn_store::spool;
    let _drain_guard = d.sync_drain.lock().await;
    let context = AuthenticatedContext::acquire(d).await?;

    let claimed = spool::claim_commands(&d.store, context.account, context.peer_instance, limit)
        .await
        .map_err(storage_err)?;
    let mut report = DrainReport::default();

    // One at a time, in the order claimed. Batching would deliver a scope's
    // commands concurrently and lose the ordering the claim just established.
    for c in &claimed {
        // **The local project id is not the server's, and only the server's
        // means anything on the wire.**
        //
        // `projects.id` is this store's own identifier and `server_project_id`
        // is the shared one; linking records the second without adopting it,
        // because a project can be re-linked and the local rows must keep
        // pointing at something stable. A command spooled with the local id and
        // posted verbatim named a project the server has never heard of, so
        // every project-scoped command queued under server authority was
        // undeliverable — and the refusal it drew was classified as permanent,
        // which turned an addressing mistake into the user's instruction being
        // dropped.
        //
        // Translated here rather than at the point the command is queued: at
        // queue time the project may not be linked yet, and burning the wrong id
        // into a durable row would outlive the mistake.
        let envelope = match resolve_command_project(d, c).await {
            CommandRoute::Ready(envelope) => envelope,
            CommandRoute::NotLinked => {
                // Deferred, not refused. An unlinked project sends nothing
                // (FR-053), but linking it later is an ordinary thing to do and
                // the command should survive to be delivered then. A deferral
                // spends no attempt budget, which is what stops a long unlinked
                // period driving the row to `retry_exhausted`.
                settle_command(d, c.command_id, ItemOutcome::Deferred, "project_not_linked")
                    .await?;
                report.record(ItemOutcome::Deferred);
                continue;
            }
        };
        let (outcome, reason) = match context
            .client
            .post_for_outcome(COMMAND_ENVELOPE_PATH, &envelope)
            .await
        {
            // A structured refusal from the server is **not** a transport
            // failure, and conflating them was the defect this replaces: a
            // `409 unsupported_kind` read as transport spent an attempt on a
            // row an upgrade would have delivered, and a `400` read as
            // transport retried a refusal forever.
            Ok(ServerAnswer::Ok) => (ItemOutcome::Delivered, "accepted".to_string()),
            Ok(ServerAnswer::Refused { code }) => (outcome_for(Some(&code)), code),
            Err(_) => (ItemOutcome::Transient, "transport".to_string()),
        };
        settle_command(d, c.command_id, outcome, &reason).await?;
        report.record(outcome);
        if outcome == ItemOutcome::Transient {
            // Stop the pass. The next command in this scope must not be
            // attempted before this one settles, and a server that is not
            // answering will not answer the next one either.
            break;
        }
    }
    Ok(report)
}

/// A claimed command's envelope, or the reason it cannot be addressed yet.
enum CommandRoute {
    Ready(serde_json::Value),
    /// The command names a project this store has not linked, so there is no
    /// server identifier to address it by.
    NotLinked,
}

/// Build one command's envelope, translating the project it names.
///
/// The only place a local project id becomes a server project id. A command
/// naming no project needs no translation and is always `Ready`.
async fn resolve_command_project(
    d: &Daemon,
    command: &cairn_store::spool::SpooledCommand,
) -> CommandRoute {
    let Some(local) = command.project_id else {
        return CommandRoute::Ready(command_envelope(command, None));
    };
    match repo::project(&d.store, local).await {
        Ok(project) => match project.server_project_id {
            Some(server_project_id) if project.linked => {
                CommandRoute::Ready(command_envelope(command, Some(server_project_id)))
            }
            _ => CommandRoute::NotLinked,
        },
        // A project row that is gone cannot be linked either, and the answer is
        // the same: hold the command rather than refuse it. Deleting a project
        // locally is not the user withdrawing an instruction about it.
        Err(e) => {
            tracing::debug!(project = %local, error = %e, "a queued command names an unknown project");
            CommandRoute::NotLinked
        }
    }
}

/// What one queued command needs to say on the wire.
///
/// Everything a command is, in one object: its deterministic identity, its
/// kind, whatever it targets, and its intent. The first version of this drain
/// posted `payload` alone to a path derived from the kind, which lost the
/// `command_id` — so nothing was idempotent — and named several paths the
/// server does not serve, so nothing arrived either. Both were the same
/// mistake: the wire form did not carry the command.
///
/// The account is **not** here. It comes from the credential the request is
/// made with, and there is deliberately no field for it: a daemon that could
/// name an account could attribute one identity's writes to another
/// (Principle XI).
fn command_envelope(
    command: &cairn_store::spool::SpooledCommand,
    server_project_id: Option<uuid::Uuid>,
) -> serde_json::Value {
    use cairn_store::spool::CommandKind;
    // What the command applies to. A project for the commands that create
    // within one, a record for the commands that act on one, neither for the
    // account-scoped domains — which is why both are optional rather than one
    // widened field that means different things per kind.
    let (project_id, target_id) = match command.kind {
        CommandKind::Remember | CommandKind::Relate => (server_project_id, None),
        CommandKind::Supersede
        | CommandKind::Reinforce
        | CommandKind::Pin
        | CommandKind::Forget
        | CommandKind::PersonalForget
        | CommandKind::PatternForget => (
            server_project_id,
            command
                .payload
                .get("target_id")
                .and_then(|v| v.as_str())
                .and_then(|s| uuid::Uuid::parse_str(s).ok()),
        ),
        CommandKind::PersonalCreate
        | CommandKind::TeamPropose
        | CommandKind::PatternPromote
        | CommandKind::VerificationRun
        | CommandKind::VerificationAttestation => (None, None),
    };
    serde_json::json!({
        "command_id": command.command_id,
        "kind": command.kind.as_str(),
        "project_id": project_id,
        "target_id": target_id,
        "payload": command.payload,
    })
}

/// The one route every queued command is delivered to.
const COMMAND_ENVELOPE_PATH: &str = "/api/commands";

/// [`drain`], for a `personal:*`/`team:*` namespace (T093, T100, T106, T107).
///
/// Same claim → send → record-outcome shape as `drain`, over
/// [`outbox::claim_namespace`] instead of the project-scoped [`outbox::claim`]
/// — personal and team rows carry no `project_id` for that one to match
/// against. The two refusal paths (§4a) are unchanged from `drain`: a
/// capability refusal (`409 unknown_entity_type`) still calls
/// [`outbox::mark_blocked`], and an ingest content refusal (`422
/// content_rejected`, not in [`codes::CAPABILITY_REFUSALS`]) still falls to
/// [`outbox::mark_failed`] — permanent, never `blocked`, never throttling this
/// namespace's backoff, because the outcome only ever reads as
/// [`NamespaceOutcome::Transient`] when the *request itself* failed, never
/// when an item in a successful response was refused.
/// Whether the pre-005 dual-authority write path may still carry knowledge
/// from this store.
///
/// **Only while the store has not begun migrating.** Once `authority_mode`
/// leaves `feature_004`, the migration owns the transfer of durable knowledge
/// and this path must stop competing with it. Two things go wrong when it does
/// not, and both were observed rather than imagined: the worker delivers a
/// legacy row with its un-normalized keys while the migration is re-keying,
/// and the pull then merges that server copy back over the corrected local
/// row — so `topic_key` reverts and the collision detection SC-750 measures
/// silently stops working against exactly the corpus it is about.
///
/// It is also what the server will say anyway once the fleet cuts over: these
/// same shapes are refused with `upgrade_required`, and a store that has
/// migrated has no business asking. Stopping here means a migrated store stops
/// emitting them rather than learning not to from a refusal.
///
/// A store that cannot answer is treated as still `feature_004`: that is the
/// path that works without a server, and guessing the other way would strand
/// queued work on a store that never migrates.
async fn legacy_writes_are_open(d: &Daemon) -> bool {
    cairn_store::authority::mode(&d.store)
        .await
        .map(|m| m == cairn_store::authority::AuthorityMode::Feature004)
        .unwrap_or(true)
}

/// The entity types the migration owns once it has begun (`migration-cutover.md`
/// §3.1, §4.2). The same list the server refuses after cutover, and
/// deliberately so: the two must not diverge.
const KNOWLEDGE_BEARING: &[&str] = &[
    "memory",
    "memory_relation",
    "personal_knowledge",
    "personal_knowledge_relation",
    "team_knowledge",
    "team_knowledge_relation",
];

/// Why a global lane moved nothing, when it moved nothing.
///
/// **`applied 0` had five meanings and no way to tell them apart.** A lane this
/// store has begun migrating past, a lane this credential may not touch, a lane
/// whose only queued work belongs to a logged-out identity, a batch held because
/// the account belongs to no project the route would authorize, and a lane with
/// an empty queue all reported the same two words. Four of those are a delivery
/// that did not happen; the fifth is nothing to deliver. Principle X does not
/// let a report say "nothing happened" when what it means is "I declined to act
/// and did not say so".
///
/// It is also what makes the FR-594 hold *observable*: a proposal held for its
/// absent author and a proposal silently skipped look identical from outside,
/// and only one of them is the behaviour that requirement asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LaneHold {
    /// Nothing was withheld. Either work went out, or the lane's queue is empty
    /// for this account and for everyone else.
    None,
    /// This store has begun migrating, so the pre-005 write path is closed and
    /// the migration owns the transfer (FR-877).
    Migrating,
    /// The credential does not admit this lane: another account's `personal:*`,
    /// or a lane bound to another server instance (FR-495, FR-496, FR-598).
    NotAdmitted,
    /// Rows are queued here and none of them are this account's to send
    /// (FR-594). The lane is *held*, not idle, and it moves the moment its
    /// author is authenticated again.
    AnotherAuthor,
    /// The authenticated account belongs to no project `POST /api/sync/batch`
    /// would authorize, so the batch went back to `pending` (FR-595).
    NoAuthorizationProject,
    /// The server could not be asked which projects this account belongs to, so
    /// the batch went back to `pending` for a reason that is about the network
    /// and not about the account.
    ///
    /// The drain does the same thing in both cases and should: the rows are not
    /// at fault either way. What differs is what a person reading
    /// `cairn sync now` is told, and "this account belongs to no project" about
    /// an account that belongs to several sends them to look at memberships
    /// instead of at the server.
    MembershipUnknown,
}

impl LaneHold {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            LaneHold::None => "none",
            LaneHold::Migrating => "migrating",
            LaneHold::NotAdmitted => "not_admitted",
            LaneHold::AnotherAuthor => "another_author",
            LaneHold::NoAuthorizationProject => "no_authorization_project",
            LaneHold::MembershipUnknown => "membership_unknown",
        }
    }
}

/// What one global lane's drain did, and what it withheld.
pub(crate) struct GlobalDrain {
    pub(crate) applied: usize,
    pub(crate) duplicate: usize,
    pub(crate) rejected: usize,
    pub(crate) hold: LaneHold,
}

impl GlobalDrain {
    fn held(hold: LaneHold) -> Self {
        GlobalDrain {
            applied: 0,
            duplicate: 0,
            rejected: 0,
            hold,
        }
    }
}

async fn drain_global(d: &Daemon, namespace: &SyncNamespace) -> Result<GlobalDrain, WireError> {
    // A `personal:*` or `team:*` lane carries nothing but knowledge, so the
    // whole lane stops once this store has begun migrating.
    if !legacy_writes_are_open(d).await {
        return Ok(GlobalDrain::held(LaneHold::Migrating));
    }

    // Same single-drainer discipline `drain` uses, and the same lock: claiming
    // is what makes two concurrent drains correct, this is what keeps them
    // orderly, and there is no reason a project drain and a global drain
    // running at once would be more correct interleaved than serialized.
    let _drain_guard = d.sync_drain.lock().await;

    // **Pushing is bound to the lane's instance exactly as pulling is**
    // (FR-598). Only `pull_global` checked, so after `cairn auth token set`
    // moved a store to a second deployment, this function went on posting
    // `team:<A>` and `personal:<A>:*` rows at server B. Nothing reported it: a
    // push that the peer accepts looks like a successful delivery, and the rows
    // were marked delivered against a server that was never supposed to receive
    // them. The pull-side repair that added the check for reading did not add it
    // for writing, which is the asymmetry [`AuthenticatedContext::admits`] now removes by
    // answering for both.
    //
    // Acquiring the context is also the credential snapshot (FR-597): the account
    // this drain filters rows by and the token it sends them with come from one
    // read, so a switch mid-drain cannot route as A while authenticating as B.
    let context = AuthenticatedContext::acquire(d).await?;
    if !context.admits(namespace) {
        context.refuse(namespace, "pushing");
        return Ok(GlobalDrain::held(LaneHold::NotAdmitted));
    }

    let capability = capability_from(&context.version, d, namespace).await;
    let key = namespace.key();

    // Only rows this account authored (FR-594). A `team:*` lane is shared by
    // every account on the server, so an undelivered proposal written as A would
    // otherwise be pushed once B logs in — and the server, right to distrust
    // payload identity, would record B as its proposer. See
    // [`outbox::claim_namespace_for_author`] for why the filter belongs in the
    // claim. Taken from the context, not re-read, for the reason above.
    let author = context.account;

    // Resolved on the first non-empty batch, not up front. The namespace's
    // pending count includes rows held for another account's author, so a lane
    // whose only queued work belongs to a logged-out identity reaches this
    // function on every tick with nothing it may send — and asking the server
    // which projects this account belongs to in order to send nothing is a
    // request every thirty seconds, forever.
    let mut auth_project: Option<Uuid> = None;

    let (mut applied, mut duplicate, mut rejected, mut blocked) = (0, 0, 0, 0);
    let mut hold = LaneHold::None;

    loop {
        let batch = outbox::claim_namespace_for_author(&d.store, &key, author, BATCH)
            .await
            .map_err(storage_err)?;
        if batch.is_empty() {
            // **An empty claim is two different states** (FR-594). A lane with
            // nothing queued and a lane whose whole queue belongs to an account
            // that is not signed in both claim nothing, and only the second is a
            // held delivery. Asked once, and only when the claim came back
            // empty, so an ordinary idle lane pays one local `COUNT` and a busy
            // one pays nothing.
            if applied + duplicate + rejected == 0 {
                let (pending, _) = outbox::counts_namespace(&d.store, &key)
                    .await
                    .map_err(storage_err)?;
                if pending > 0 {
                    hold = LaneHold::AnotherAuthor;
                    tracing::info!(
                        namespace = %key, account = %author, pending,
                        "holding this lane's queued work: none of it was authored \
                         by the authenticated account"
                    );
                }
            }
            break;
        }

        // The context's client, not a fresh one: the account this batch was
        // claimed for and the token it is sent with must be the same read
        // (FR-597).
        let c = &context.client;

        let mut membership_known = true;
        if auth_project.is_none() {
            membership_known = context.memberships().await.is_some();
            auth_project = authorization_project(&context, d).await;
        }
        let Some(project_id) = auth_project else {
            // Nothing this batch could be authorized against. The rows go back to
            // `pending` rather than counting as failures: the account will belong
            // to a project, or a different account will log in, and neither is
            // this row's fault.
            // **`info`, not `debug`.** This is a delivery that silently did not
            // happen: the rows go back to `pending`, the drain reports success,
            // and nothing else anywhere says the queue did not move. A default
            // log level that omits the one line naming the reason is the same
            // silence FR-792 exists to remove.
            let (reason, why) = if membership_known {
                (
                    LaneHold::NoAuthorizationProject,
                    "no authorization project for this account",
                )
            } else {
                (
                    LaneHold::MembershipUnknown,
                    "could not read this account's project membership from the server",
                )
            };
            tracing::info!(
                namespace = %key, account = %author, hold = reason.as_str(),
                "holding this batch: {why}"
            );
            release(d, &batch, why).await?;
            hold = reason;
            break;
        };

        let items: Vec<SyncItem> = batch.iter().map(|(_, item)| item.clone()).collect();
        let body = serde_json::to_value(SyncBatch { project_id, items }).unwrap_or(json!({}));

        let response = match c.post("/api/sync/batch", &body).await {
            Ok(v) => v,
            Err(e) => {
                release(d, &batch, &e.message).await?;
                return Err(e);
            }
        };

        let parsed: SyncBatchResponse = match serde_json::from_value(response) {
            Ok(parsed) => parsed,
            Err(e) => {
                let err = WireError::new(codes::SERVER_UNAVAILABLE, e.to_string());
                release(d, &batch, &err.message).await?;
                return Err(err);
            }
        };

        for (row_id, item) in &batch {
            let result = parsed
                .results
                .iter()
                .find(|r| r.idempotency_key == item.idempotency_key);
            match result.map(|r| r.status) {
                Some(SyncItemStatus::Applied) => {
                    outbox::mark_delivered(&d.store, *row_id)
                        .await
                        .map_err(storage_err)?;
                    applied += 1;
                }
                Some(SyncItemStatus::Duplicate) => {
                    outbox::mark_delivered(&d.store, *row_id)
                        .await
                        .map_err(storage_err)?;
                    duplicate += 1;
                }
                Some(SyncItemStatus::Rejected) => {
                    let error = result.and_then(|r| r.error.as_ref());
                    let msg = error
                        .map(|e| e.message.clone())
                        .unwrap_or_else(|| "rejected".into());

                    // §4a's two refusals, exactly as `drain` branches them:
                    // capability (409, recoverable, held) vs content (422,
                    // permanent, never blocked) — decided by the typed `code`,
                    // never by matching on `msg`.
                    match error.map(|e| e.code.as_str()) {
                        Some(code) if codes::CAPABILITY_REFUSALS.contains(&code) => {
                            outbox::mark_blocked(&d.store, *row_id, code, &capability, &msg)
                                .await
                                .map_err(storage_err)?;
                            blocked += 1;
                        }
                        _ => {
                            outbox::mark_failed(&d.store, *row_id, &msg)
                                .await
                                .map_err(storage_err)?;
                            rejected += 1;
                        }
                    }
                }
                None => {
                    outbox::mark_retryable(&d.store, *row_id, "no result for item")
                        .await
                        .map_err(storage_err)?;
                }
            }
        }
        if batch.len() < BATCH as usize {
            break;
        }
    }
    if blocked > 0 {
        tracing::info!(
            namespace = %key, blocked, capability = %capability,
            "work retained for a server that cannot hold it yet"
        );
    }
    Ok(GlobalDrain {
        applied,
        duplicate,
        rejected,
        hold,
    })
}

/// Ask the server what it can hold, and release anything it now can (T111,
/// T106, T107).
///
/// Returns the capability as an opaque string, which is what a blocked row
/// records so a person can see *what* it is waiting for.
///
/// A server that answers without `capabilities` is a server from before the
/// field existed, and its silence is the answer: it can hold none of this. That
/// is why there is no probe endpoint and no negotiation — `GET /api/version`
/// already existed, and adding to it additively meant an old server needed no
/// change at all (D81).
///
/// **Namespace-generic (§11a).** The one probe implementation serves
/// `project:*`, `personal:*` and `team:*` alike: it reads `capabilities`
/// (never resends a held item — FR-561, the distinction §11a insists on), and
/// on a change it releases *this namespace's own* `blocked` rows
/// (`outbox::release_blocked_namespace`) with their original idempotency key
/// intact (FR-562) and records the fingerprint under this namespace's own
/// `sync_cursor` row (`cairn_store::cursor`) — never another namespace's.
async fn refresh_capability(d: &Daemon, namespace: &SyncNamespace) -> String {
    let Ok(client) = client(d).await else {
        // Offline. Whatever was last known still describes the server better
        // than nothing does.
        return last_known_capability(d, namespace).await;
    };
    let Ok(body) = client.get("/api/version").await else {
        return last_known_capability(d, namespace).await;
    };
    capability_from(&body, d, namespace).await
}

async fn last_known_capability(d: &Daemon, namespace: &SyncNamespace) -> String {
    cursor::server_capability(&d.store, namespace)
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| UNKNOWN_CAPABILITY.to_string())
}

/// As [`refresh_capability`], over a `/api/version` body already in hand.
///
/// A global drain holds one: [`AuthenticatedContext`] fetched it to learn the peer's
/// instance. Reading that same response rather than issuing a second one is not
/// only a saved request — two reads are two chances to observe two different
/// servers, which is exactly what snapshotting the credential exists to prevent
/// (FR-597).
async fn capability_from(
    body: &serde_json::Value,
    d: &Daemon,
    namespace: &SyncNamespace,
) -> String {
    let schema = body
        .get("schema_version")
        .and_then(|v| v.as_i64())
        .unwrap_or(1);
    let mut names: Vec<String> = body
        .get("capabilities")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str())
                .map(|s| s.to_string())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    let capability = format!("schema={schema};capabilities={}", names.join(","));

    let previous = cursor::server_capability(&d.store, namespace)
        .await
        .ok()
        .flatten();
    if previous.as_deref() == Some(capability.as_str()) {
        return capability;
    }

    // The capability changed. Anything the server can now hold goes back into
    // the ordinary queue with its original idempotency key, and the ordinary
    // drain — the one about to run — delivers it. Nothing here sends anything
    // itself, so there is no second delivery path to keep exactly-once
    // (FR-562, SC-331's precedent restated for schema 3).
    let releasable: Vec<OutboxEntityType> = ENTITY_CAPABILITIES
        .iter()
        // Every capability the type can wait on must be present. Releasing a
        // memory on `memory_subject_identity` alone would put an attested one
        // back in front of a server that still has no column for it.
        .filter(|(_, needs)| needs.iter().all(|need| names.iter().any(|n| n == need)))
        .map(|(entity, _)| *entity)
        .collect();
    match outbox::release_blocked_namespace(&d.store, &namespace.key(), &releasable).await {
        Ok(n) if n > 0 => tracing::info!(
            namespace = %namespace.key(), released = n, capability = %capability,
            "the server gained a capability; retained work returns to the queue"
        ),
        Ok(_) => {}
        Err(e) => tracing::warn!(error = %e, "could not release retained work"),
    }
    let _ = cursor::set_server_capability(&d.store, namespace, &capability).await;
    capability
}

/// What a server has never answered about.
const UNKNOWN_CAPABILITY: &str = "schema=unknown;capabilities=";

/// The capabilities each retainable entity type may be waiting for.
///
/// A `memory` lists **two**, because a schema-1 server refuses one by field
/// rather than by type and there is more than one field it can refuse on: a
/// subject identity, or a verification. Either is enough to hold the memory
/// back, and it is released when the server can hold whichever it carries.
///
/// A memory is retained whole rather than sent stripped: delivering a claim
/// without the thing that makes it comparable, or without what established it,
/// is worse than delivering it a migration later.
const ENTITY_CAPABILITIES: &[(OutboxEntityType, &[&str])] = &[
    (OutboxEntityType::MemoryRelation, &["memory_relations"]),
    (
        OutboxEntityType::Memory,
        &["memory_subject_identity", "memory_verification"],
    ),
    // Feature 004 (FR-498, FR-522). A server that predates schema 3 causes only
    // these four entity types to be held — never the project namespace, which is
    // what per-namespace backoff exists to guarantee.
    (OutboxEntityType::PersonalKnowledge, &["personal_knowledge"]),
    (
        OutboxEntityType::PersonalKnowledgeRelation,
        &["personal_knowledge"],
    ),
    (OutboxEntityType::TeamKnowledge, &["team_knowledge"]),
    (OutboxEntityType::TeamKnowledgeRelation, &["team_knowledge"]),
];

/// Hand a claimed batch back to the queue after a transient failure.
///
/// Without this an interrupted send would leave rows claimed until the claim
/// went stale, which is correct but needlessly slow when the drainer is still
/// alive and simply could not reach the server.
async fn release(d: &Daemon, batch: &[(Uuid, SyncItem)], error: &str) -> Result<(), WireError> {
    for (id, _) in batch {
        outbox::mark_retryable(&d.store, *id, error)
            .await
            .map_err(storage_err)?;
    }
    Ok(())
}

/// Pull shared records other members produced, so local search and context
/// include a teammate's memory (FR-056).
async fn pull(d: &Daemon, project_id: Uuid, server_project_id: Uuid) -> Result<usize, WireError> {
    let c = client(d).await?;
    let since = cursor::pull_cursor(&d.store, &SyncNamespace::Project(project_id))
        .await
        .map_err(storage_err)?;
    let path = match &since {
        Some(since_cursor) => format!(
            "/api/sync/changes?project_id={server_project_id}&since={}",
            urlencode(since_cursor)
        ),
        None => format!("/api/sync/changes?project_id={server_project_id}"),
    };
    let body = c.get(&path).await?;

    let memories = body
        .get("memories")
        .and_then(|m| m.as_array())
        .cloned()
        .unwrap_or_default();
    let mut count = 0;
    for m in &memories {
        if import_memory(d, project_id, m).await.is_ok() {
            count += 1;
        }
    }

    // Memories first, then the decisions about them: a relation whose memory has
    // not arrived is held and retried rather than dropped, and importing in this
    // order means it usually does not have to be.
    for r in body
        .get("relations")
        .and_then(|v| v.as_array())
        .unwrap_or(&Vec::new())
    {
        match import_relation(d, project_id, r).await {
            Placement::Placed => count += 1,
            Placement::AwaitingParent(waiting_on) => {
                hold_for_a_later_pull(d, project_id, "relation", &relation_key(r), r, waiting_on)
                    .await;
            }
            Placement::Unusable => {}
        }
    }

    count += replay_deferred(d, project_id).await;

    if let Some(next_cursor) = body.get("cursor").and_then(|c| c.as_str()) {
        cursor::set_pull_cursor(&d.store, &SyncNamespace::Project(project_id), next_cursor)
            .await
            .map_err(storage_err)?;
    }
    Ok(count)
}

/// Insert a teammate's memory locally, read-only.
///
/// It carries provenance references; the observations behind it stayed on their
/// machine, which is the whole point (FR-055).
async fn import_memory(
    d: &Daemon,
    project_id: Uuid,
    value: &serde_json::Value,
) -> Result<(), WireError> {
    let id = value
        .get("id")
        .and_then(|v| v.as_str())
        .and_then(|s| Uuid::parse_str(s).ok())
        .ok_or_else(|| WireError::invalid("shared memory without an id"))?;

    // A memory this store already holds is **not** skipped. `import_memory`
    // never overwrites a local row — `INSERT OR IGNORE` is the whole rule — but
    // a peer re-sends a memory precisely when something shareable about it
    // changed, and the one such thing is its verification. Returning early here
    // meant a peer's later check never arrived, so `remote_cairn` and
    // `remote_attested` could not occur (FR-368, SC-329).
    let content = value
        .get("content")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let kind: MemoryType = value
        .get("type")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse().ok())
        .unwrap_or(MemoryType::Fact);
    let scope: MemoryScope = value
        .get("scope")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse().ok())
        .unwrap_or(MemoryScope::Project);
    let scope_key = value
        .get("scope_key")
        .and_then(|v| v.as_str())
        .unwrap_or(&project_id.to_string())
        .to_string();
    let origin = value
        .get("provenance")
        .and_then(|p| p.get("session_id"))
        .and_then(|v| v.as_str())
        .and_then(|s| Uuid::parse_str(s).ok())
        .unwrap_or_else(new_id);

    // The subject identity the sender proposed travels with the row. Without it
    // the proposal arrives free-form, no subject read can ever see it, and a
    // value another machine proposed for a subject this machine already holds
    // is invisible rather than corroborating or conflicting — which is the
    // whole of US7 (FR-411).
    let str_of = |k: &str| value.get(k).and_then(|v| v.as_str());
    repo::import_memory(
        &d.store,
        repo::ImportedMemory {
            id,
            project_id,
            kind,
            scope,
            scope_key: &scope_key,
            content,
            origin_session_id: origin,
            topic_key: str_of("topic_key"),
            value_key: str_of("value_key"),
            importance: str_of("importance")
                .and_then(|s| s.parse().ok())
                .unwrap_or(Importance::Normal),
            effective_from: str_of("effective_from"),
        },
    )
    .await
    .map_err(|e| WireError::new(codes::STORAGE_UNAVAILABLE, e.to_string()))?;

    import_verification(d, id, value).await;

    // The arriving proposal changes what this subject's members are, so the
    // counts derived from them are rebuilt rather than assumed unchanged.
    let _ = cairn_store::knowledge::rebuild_reinforcement(&d.store, id).await;
    Ok(())
}

/// Record what a peer said about a memory's verification, wearing the peer's
/// badge (FR-368, FR-370, SC-329).
///
/// `cairn` → `remote_cairn`, `attested` → `remote_attested`. The sender's value
/// is **never** stored verbatim. "Verified here" is a claim only the local
/// machine can make, and an imported verification counts towards neither local
/// readiness nor promotion — it is rendered as verified *elsewhere*, with the
/// peer's authority named.
///
/// Without this an attested claim from a peer would arrive as
/// `{state: verified, basis: ["test_outcome"]}` and be rendered exactly like a
/// peer that had really run the tests.
async fn import_verification(d: &Daemon, memory_id: Uuid, value: &serde_json::Value) {
    let Some(verification) = value.get("verification") else {
        return;
    };
    let state = verification.get("state").and_then(|v| v.as_str());
    let Some(state) = state else { return };

    // A run this machine recorded outranks anything a peer says about the same
    // memory. Records win over derived state (FR-478), and a verification run
    // is a durable local record.
    //
    // Without this a memory this machine checked itself came back from the
    // server wearing `remote_cairn`: it had been pushed, and the pull applied
    // the peer's badge over the local one. The state stayed `verified`, so
    // nothing looked wrong — but the authority decides two things, and both
    // then refused it. Its own project could no longer promote it, and it no
    // longer counted towards local readiness, on the strength of a check this
    // machine had run.
    if !cairn_store::evidence::runs_for_memory(&d.store, memory_id)
        .await
        .unwrap_or_default()
        .is_empty()
    {
        let _ = cairn_store::evidence::rebuild_verification(&d.store, memory_id).await;
        return;
    }

    let authority = match verification.get("authority").and_then(|v| v.as_str()) {
        Some("cairn") => Some("remote_cairn"),
        Some("attested") => Some("remote_attested"),
        // A peer relaying a third machine's authority is not something this
        // machine can act on, so it is not recorded as an authority at all.
        _ => None,
    };

    // What a peer says is input, not truth, and this is the one place a state
    // reaches the row without passing through `rebuild_verification`.
    //
    // Two rules that function enforces have to hold here as well, because the
    // column carries no CHECK and this is the trust boundary:
    //
    //   * a state outside the enum is not storable at all — a malformed or
    //     older peer must not be able to invent one;
    //   * `verified` with no authority is not a pair Cairn may hold (FR-370).
    //     A peer that sends one is telling us it was verified without saying
    //     what verified it, and the honest local answer is `unverified`. Left
    //     as-is it rendered as a bare `verified`, re-emitted itself to the next
    //     peer through `summary`, and — having no local runs and no `remote_*`
    //     authority to recognise — was silently rewritten by the next
    //     `doctor --rebuild-derived` anyway.
    //
    // An authority without `verified` is dropped for the same reason: authority
    // says what established the state, and nothing established a state that is
    // not `verified`.
    let Ok(state) = state.parse::<cairn_core::VerificationState>() else {
        tracing::debug!(%memory_id, state, "ignored an unrecognised imported verification state");
        return;
    };
    let (state, authority) = match (state, authority) {
        (cairn_core::VerificationState::Verified, None) => {
            tracing::debug!(%memory_id, "a peer sent `verified` with no authority; storing unverified");
            (cairn_core::VerificationState::Unverified, None)
        }
        (cairn_core::VerificationState::Verified, some) => {
            (cairn_core::VerificationState::Verified, some)
        }
        (other, _) => (other, None),
    };
    let state = state.as_str();

    let _ = sqlx::query(
        "UPDATE memories
            SET verification = ?2, verification_authority = ?3,
                last_verified_at = COALESCE(?4, last_verified_at)
          WHERE id = ?1",
    )
    .bind(memory_id.to_string())
    .bind(state)
    .bind(authority)
    .bind(
        verification
            .get("last_verified_at")
            .and_then(|v| v.as_str()),
    )
    .execute(d.store.pool())
    .await;
}

/// What became of one pulled record.
///
/// The distinction that matters is between a record that cannot be placed
/// **yet** and one that can never be placed. The first is held and replayed;
/// the second is discarded, because retrying it forever would be a leak with no
/// outcome (#44).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Placement {
    /// Imported.
    Placed,
    /// The parent it names has not arrived. Carries the missing parent, so a
    /// project waiting on one record can say what it is waiting for.
    AwaitingParent(Uuid),
    /// Malformed, or refused by the store. There is nothing to retry.
    Unusable,
}

/// How many held records one pull replays.
///
/// A backlog must not turn a single pull into unbounded work; whatever does not
/// fit is offered again on the next pull, oldest wait first.
const DEFERRED_REPLAY_BATCH: i64 = 500;

/// Retry the records earlier pulls could not place.
///
/// Run after the fresh page has been imported, so a relation held since an
/// earlier pull is placed as soon as the memory it names lands. Relations wait
/// on memories, so one pass is enough and there is no ordering to get right.
async fn replay_deferred(d: &Daemon, project_id: Uuid) -> usize {
    let held = match repo::deferred_records(&d.store, project_id, DEFERRED_REPLAY_BATCH).await {
        Ok(h) => h,
        Err(e) => {
            tracing::warn!(error = %e, "could not read the records held for a later pull");
            return 0;
        }
    };

    let mut placed = 0;
    for record in held {
        let outcome = match serde_json::from_str::<serde_json::Value>(&record.payload) {
            Ok(value) => match record.kind.as_str() {
                "relation" => import_relation(d, project_id, &value).await,
                _ => Placement::Unusable,
            },
            // A payload that cannot be parsed can never be placed.
            Err(_) => Placement::Unusable,
        };

        match outcome {
            Placement::Placed => {
                release_held_record(d, project_id, &record, "it landed").await;
                placed += 1;
            }
            Placement::Unusable => {
                release_held_record(d, project_id, &record, "it can never be placed").await
            }
            // Still waiting. Recorded rather than retried in silence, so a
            // parent that never arrives is visible in the store.
            Placement::AwaitingParent(_) => {
                if let Err(e) = repo::note_deferred_attempt(
                    &d.store,
                    project_id,
                    &record.kind,
                    &record.record_key,
                )
                .await
                {
                    tracing::warn!(error = %e, "could not record a held record's attempt");
                }
            }
        }
    }
    placed
}

/// Stop holding a record, because it landed or never can.
async fn release_held_record(
    d: &Daemon,
    project_id: Uuid,
    record: &cairn_store::repo::DeferredRecord,
    reason: &'static str,
) {
    if let Err(e) =
        repo::clear_deferred_record(&d.store, project_id, &record.kind, &record.record_key).await
    {
        tracing::warn!(error = %e, "could not release a held record");
        return;
    }
    tracing::debug!(
        kind = %record.kind, key = %record.record_key,
        waiting_since = %record.first_seen_at, attempts = record.attempts, reason,
        "released a held record"
    );
}

/// Hold a record the fresh page could not place.
async fn hold_for_a_later_pull(
    d: &Daemon,
    project_id: Uuid,
    kind: &str,
    record_key: &str,
    value: &serde_json::Value,
    waiting_on: Uuid,
) {
    if let Err(e) = repo::defer_pulled_record(
        &d.store,
        project_id,
        kind,
        record_key,
        &value.to_string(),
        &waiting_on.to_string(),
    )
    .await
    {
        tracing::warn!(
            error = %e, kind, record_key,
            "could not hold a record whose parent has not arrived; it is lost"
        );
    }
}

/// Import a reconciliation decision.
///
/// `INSERT OR IGNORE` on the normalized primary key, then re-derive. This is the
/// correction research B2 found: today `import_memory` returns early when the
/// row exists, so a supersession decided on another machine never lands. The
/// *decision* is what travels, and deriving from it fixes the defect without
/// introducing row overwriting (D67, R5).
async fn import_relation(d: &Daemon, project_id: Uuid, value: &serde_json::Value) -> Placement {
    let uuid = |k: &str| {
        value
            .get(k)
            .and_then(|v| v.as_str())
            .and_then(|s| Uuid::parse_str(s).ok())
    };
    let (Some(from), Some(to)) = (uuid("from_memory_id"), uuid("to_memory_id")) else {
        return Placement::Unusable;
    };
    let kind = value
        .get("kind")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let basis = value
        .get("basis")
        .and_then(|v| v.as_str())
        .unwrap_or("explicit_user");

    let (Ok(kind), Ok(basis)) = (kind.parse(), basis.parse()) else {
        return Placement::Unusable;
    };

    // A relation whose memory has not arrived is held rather than dropped: the
    // foreign key would refuse it, and it is replayed after every later pull
    // until the memory lands.
    //
    // It used to be dropped outright, on the claim that "the next pull carries
    // it again" — a promise the cursor does not keep. The cursor is a timestamp
    // and the server re-sends a record only when the record itself changes, so
    // a relation older than the page's newest row, whose memory falls in the
    // next page, was lost permanently (#44).
    for parent in [from, to] {
        if repo::memory(&d.store, parent).await.is_err() {
            tracing::debug!(
                project = %project_id, %from, %to, waiting_on = %parent,
                "holding a relation whose memory has not arrived yet"
            );
            return Placement::AwaitingParent(parent);
        }
    }

    let _ = cairn_store::knowledge::record_relation(
        &d.store,
        cairn_store::knowledge::NewRelation {
            project_id,
            from,
            to,
            kind,
            decided_by_session: uuid("decided_by_session").unwrap_or_else(new_id),
            basis,
            // Stripped on the wire, and correctly absent here.
            basis_evidence_id: None,
            rationale: None,
        },
    )
    .await;

    // The decision changed what is canonical, so the derived state is rebuilt
    // from the records rather than patched.
    //
    // Supersession is rebuilt per project, because one `supersedes` relation
    // can move a whole chain. Reinforcement is rebuilt per **memory** — it is
    // keyed by memory id, and passing the project id here silently rebuilt
    // nothing at all, leaving an imported `reinforces` uncounted.
    let _ = cairn_store::knowledge::rebuild_supersession(&d.store, project_id).await;
    for endpoint in [to, from] {
        let _ = cairn_store::knowledge::rebuild_reinforcement(&d.store, endpoint).await;
    }
    Placement::Placed
}

/// The identity of a relation as the wire carries it.
///
/// Relations have no `id` on the wire; `(from, to, kind)` is the primary key
/// `memory_relations` is declared with, so it is the relation's identity here
/// too. A relation the server sends again replaces its held copy rather than
/// adding a second row.
fn relation_key(value: &serde_json::Value) -> String {
    let field = |k: &str| value.get(k).and_then(|v| v.as_str()).unwrap_or_default();
    format!(
        "{}:{}:{}",
        field("from_memory_id"),
        field("to_memory_id"),
        field("kind")
    )
}

// ---------------------------------------------------------------------------
// Migration eligibility (T141; FR-864a, FR-867b)
//
// Who is allowed to hand a legacy row to the server, decided as two pure
// functions so the rule can be read, tested and mutated on its own rather than
// inferred from a `WHERE` clause several call sites away.
//
// This is not a second claim path. `outbox::claim_namespace_for_author` still
// does the claiming; these say which claim a namespace calls for, and name the
// reason when a row is not eligible so `--status` can report it individually
// (contract §4.3) instead of leaving it silently pending.
// ---------------------------------------------------------------------------

/// Why a queued legacy row is, or is not, this account's to drain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Eligibility {
    /// Claimable by this account, through the claim its namespace calls for.
    Eligible,
    /// A global row with no author recorded against it.
    ///
    /// Drained by **no one's** migration. A missing author is not a wildcard:
    /// treating it as one would deliver the row under whichever account happens
    /// to be signed in during migration, which is the misattribution
    /// `outbox.rs` records as introduced and fixed twice already. The row stays
    /// pending and is reported.
    NoRecordedAuthor,
    /// A global row authored by somebody else. Held, not refused — it goes out
    /// unchanged the moment its own author resumes their migration.
    AuthorMismatch { recorded: Uuid },
}

/// Whether `account` may drain a queued row in `namespace` authored by
/// `authored_by`.
///
/// **A `project:*` row carries no author and needs none.** Its authorization is
/// membership of the project, which the server checks on arrival; the local
/// CHECK on `outbox` requires exactly that split, so a project row with an
/// author is as impossible as a global row without one. Reading the namespace
/// rather than the entity type is deliberate: the namespace is the column the
/// claim itself keys on, so this cannot disagree with what the claim will do.
pub fn legacy_row_eligibility(
    namespace: &str,
    authored_by: Option<Uuid>,
    account: Uuid,
) -> Eligibility {
    if namespace.starts_with("project:") {
        return Eligibility::Eligible;
    }
    match authored_by {
        Some(a) if a == account => Eligibility::Eligible,
        Some(recorded) => Eligibility::AuthorMismatch { recorded },
        None => Eligibility::NoRecordedAuthor,
    }
}

/// Why a legacy pattern is, or is not, deliverable by this account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PatternEligibility {
    /// A persisted claim by this account exists. Both values come from the
    /// claim, never recomputed from the credential active at retry time.
    Eligible {
        pattern_id: Uuid,
        content_key: String,
    },
    /// Nobody has claimed it. It stays local and is reported individually;
    /// the active account can never substitute for a missing claim.
    OwnerUnclaimed,
    /// Claimed by a different account. Reported until that account resumes its
    /// own migration, and never re-keyed to this one.
    AuthorMismatch { owner: Uuid },
}

/// Whether `account` may deliver the pattern behind `claim`.
///
/// The whole point of taking the persisted claim rather than the local pattern
/// is that a credential switch must not change the answer's *identity*: a
/// claimed row keeps the `pattern_id` its claim recorded, so no sequence of
/// sign-ins can produce a second owner or a second canonical pattern.
pub fn pattern_eligibility(
    claim: Option<&cairn_store::migrate::PatternClaim>,
    account: Uuid,
) -> PatternEligibility {
    match claim {
        None => PatternEligibility::OwnerUnclaimed,
        Some(c) if c.owner_user_id == account => PatternEligibility::Eligible {
            pattern_id: c.pattern_id,
            content_key: c.content_key.clone(),
        },
        Some(c) => PatternEligibility::AuthorMismatch {
            owner: c.owner_user_id,
        },
    }
}

