//! Opt-in synchronization with the Cairn server (FR-053 – FR-058, D9, D14).
//!
//! Local → server for what this machine produced, plus read access to shared
//! records produced by others. Delivery is idempotent, offline is normal, and
//! an unlinked project never produces a request.

use crate::state::{storage_err, Daemon, Resolved};
use cairn_core::domain::*;
use cairn_core::wire::*;
use cairn_store::{cursor, outbox, repo};
use serde_json::json;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use uuid::Uuid;

const BATCH: i64 = 100;

/// How often the background worker checks whether any namespace has work due.
///
/// This is the *check* cadence, not the *pull* cadence (§5,
/// `contracts/sync-namespaces.md`) — see [`PULL_INTERVAL_SECONDS`], which is
/// the interval that actually paces requests to the server.
const WORKER_TICK: Duration = Duration::from_millis(500);
/// Backoff after a transient failure: doubles to a ceiling, then holds.
///
/// Applied **per namespace** (D427, FR-497): each namespace this daemon
/// services keeps its own [`NamespaceClock`], so a `project:*` namespace
/// backing off from a rate limit never slows `personal:*` or `team:*`'s own
/// retry timing, and vice versa. Before this feature `run_worker` kept one
/// `Duration` shared by the whole loop body — the process-global backoff
/// `contracts/sync-namespaces.md` §4 names as the defect this replaces.
const BACKOFF_MIN: Duration = Duration::from_millis(500);
const BACKOFF_MAX: Duration = Duration::from_secs(30);
/// How often a namespace holding **only** retained work asks the server
/// whether it has been upgraded (FR-418, FR-561, §11a).
///
/// Slower than the worker tick on purpose. There is nothing to send, so this is
/// a single small request every few seconds rather than one every half second —
/// and noticing an upgrade a few seconds late costs nothing, while never
/// noticing it costs the whole promise. Also per namespace, for the same
/// reason backoff is: a `team:*` namespace blocked on a missing capability
/// re-probes on its own schedule, never delaying `project:*`'s own probe or
/// drain (§11a, Invariant 16).
const CAPABILITY_PROBE: Duration = Duration::from_secs(5);
/// How often each namespace pulls, independent of whether it has anything
/// pending to push (FR-489, FR-589, §5).
///
/// **The fix for the conditional-pull defect.** `pull` used to run only after
/// a successful `drain`, which itself only ran when the outbox held pending or
/// blocked work — so a consume-only machine (one that only *reads* team or
/// personal knowledge and never writes any of its own) never called `pull` in
/// the background at all. The fix moves `pull` outside that gate entirely and
/// paces it with this interval instead of `WORKER_TICK`: without an interval
/// of its own, "the pull-due timer has elapsed" would be true on *every* tick,
/// and each namespace would poll the server twice a second forever — three
/// namespaces, six requests per second per machine, whether or not anything
/// changed. `SC-412` asserts against this exact number (twice this interval),
/// so a passing test does not depend on landing inside a single window.
const PULL_INTERVAL_SECONDS: u64 = 30;

type Reply = Result<serde_json::Value, WireError>;

/// One namespace's independent backoff, probe and pull scheduling (D427,
/// FR-489, FR-497, §4, §5, §11a).
///
/// `run_worker` keeps one of these per namespace `key()` rather than the single
/// `backoff: Duration` and `last_probe: Instant` it used to hold for the whole
/// process — that sharing is exactly the process-global backoff
/// `contracts/sync-namespaces.md` §4 names as the defect this replaces. This
/// state lives only for the worker task's lifetime; `sync_cursor.backoff_until`
/// (`cairn_store::cursor`) is the durable counterpart a fresh process consults
/// before it has doubled anything of its own.
struct NamespaceClock {
    backoff: Duration,
    /// Not attempted again before this instant. A transient failure pushes it
    /// forward by `backoff`; success resets it to "now", so a healthy
    /// namespace is never held back by a backoff it does not have.
    retry_after: Instant,
    last_probe: Instant,
    last_pull: Instant,
}

impl NamespaceClock {
    /// Due for everything immediately — a namespace seen for the first time,
    /// or a daemon that just started next to an already-upgraded server,
    /// should not wait out a full interval before its first attempt.
    fn due_now(now: Instant) -> Self {
        Self {
            backoff: BACKOFF_MIN,
            retry_after: now,
            last_probe: now - CAPABILITY_PROBE,
            last_pull: now - Duration::from_secs(PULL_INTERVAL_SECONDS),
        }
    }

    fn probe_due(&self, now: Instant) -> bool {
        now.duration_since(self.last_probe) >= CAPABILITY_PROBE
    }

    fn pull_due(&self, now: Instant) -> bool {
        now.duration_since(self.last_pull) >= Duration::from_secs(PULL_INTERVAL_SECONDS)
    }

    /// Record that this namespace just pulled.
    ///
    /// Separate from [`Self::record`], which folds in an *outcome*, because the
    /// two answer different questions: `record` decides when to retry after a
    /// failure, this decides when the next scheduled pull is due. Conflating
    /// them is what left `last_pull` never advancing — `record` ran on every
    /// tick and touched only the backoff, so `pull_due` stayed true forever and
    /// `WORKER_TICK` became the pull frequency. Three namespaces polling twice a
    /// second, indefinitely, is precisely the unbounded poll
    /// `PULL_INTERVAL_SECONDS` exists to prevent (`sync-namespaces.md` §5), and
    /// backoff does not save it because these requests succeed.
    ///
    /// Marked whether the pull succeeded or not. A failed pull is still an
    /// attempt, and retrying it sooner is `retry_after`'s job — the backoff
    /// clock — not this one's.
    fn mark_pulled(&mut self, now: Instant) {
        self.last_pull = now;
    }

    /// Record that this namespace just re-read the server's capabilities. See
    /// [`Self::mark_pulled`].
    fn mark_probed(&mut self, now: Instant) {
        self.last_probe = now;
    }

    /// Fold one attempt's outcome in. `Ok` clears the backoff entirely rather
    /// than merely not-doubling it: a namespace that just succeeded is exactly
    /// as eligible as one that has never failed (Invariant 2).
    fn record(&mut self, now: Instant, outcome: NamespaceOutcome) {
        match outcome {
            NamespaceOutcome::Transient => {
                self.retry_after = now + self.backoff;
                self.backoff = (self.backoff * 2).min(BACKOFF_MAX);
            }
            NamespaceOutcome::Ok => {
                self.backoff = BACKOFF_MIN;
                self.retry_after = now;
            }
        }
    }
}

/// What one namespace's attempt this tick came to — the input
/// [`NamespaceClock::record`] folds into that namespace's own backoff, and
/// nobody else's (Invariant 2, FR-488).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NamespaceOutcome {
    Ok,
    Transient,
}

/// One lane this daemon services (§1, D426).
///
/// `Global` covers both `personal:*` and `team:*`: both are project-less and
/// driven the same way from here — [`drain_global`] dispatches on the
/// namespace's own key, and neither needs anything `Project` carries.
enum NamespaceTarget {
    Project {
        project_id: Uuid,
        server_project_id: Uuid,
    },
    Global(SyncNamespace),
}

impl NamespaceTarget {
    fn key(&self) -> String {
        match self {
            NamespaceTarget::Project { project_id, .. } => {
                SyncNamespace::Project(*project_id).key()
            }
            NamespaceTarget::Global(ns) => ns.key(),
        }
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
pub async fn run_worker(daemon: std::sync::Arc<Daemon>) {
    let mut clocks: HashMap<String, NamespaceClock> = HashMap::new();
    let mut establish_clock = NamespaceClock::due_now(Instant::now());
    loop {
        tokio::time::sleep(WORKER_TICK).await;

        let projects = match repo::list_projects(&daemon.store).await {
            Ok(p) => p,
            Err(e) => {
                tracing::debug!(error = %e, "sync worker could not list projects");
                continue;
            }
        };

        let mut targets: Vec<NamespaceTarget> = projects
            .iter()
            .filter(|p| p.linked)
            .filter_map(|p| {
                p.server_project_id
                    .map(|server_project_id| NamespaceTarget::Project {
                        project_id: p.id,
                        server_project_id,
                    })
            })
            .collect();

        // `personal:*`/`team:*` namespaces this store has ever queued work
        // for. Discovered from the outbox rather than assumed absent: a
        // namespace can hold queued work before its first successful pull
        // (`outbox::known_namespaces`'s own reasoning), and a namespace with
        // nothing queued yet and no pull route to try (§5's fix is stated for
        // `project:*`; a personal/team pull endpoint is a later addition) is
        // not worth inventing a target for.
        // Every global lane this store knows about, from two sources that
        // answer different questions. The outbox answers "what has work
        // queued", which is what a lane needs to *push*. `sync_cursor` answers
        // "what lanes exist at all", which is what a lane needs to *pull* — and
        // a consume-only machine has the second and not the first. Taking only
        // the first is the defect §5 describes: it made the pull unreachable on
        // exactly the machines that had nothing but pulling to do.
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        if let Ok(known) = outbox::known_namespaces(&daemon.store).await {
            for key in known {
                if let Some(ns) = parse_global_namespace(&key) {
                    if seen.insert(key) {
                        targets.push(NamespaceTarget::Global(ns));
                    }
                }
            }
        }
        if let Ok(established) = cursor::established(&daemon.store).await {
            for ns in established {
                if matches!(ns, SyncNamespace::Project(_)) {
                    continue;
                }
                if seen.insert(ns.key()) {
                    targets.push(NamespaceTarget::Global(ns));
                }
            }
        }

        // A store that is linked and authenticated but has never established
        // its global lanes gets them here, on the pull cadence rather than every
        // tick: the probe is one `GET /api/version`, and doing it twice a second
        // forever would be the unbounded poll §5 warns about.
        if targets
            .iter()
            .all(|t| matches!(t, NamespaceTarget::Project { .. }))
            && establish_clock.pull_due(Instant::now())
        {
            establish_clock.mark_pulled(Instant::now());
            let _ = establish_global_namespaces(&daemon).await;
        }

        let now = Instant::now();
        for target in &targets {
            let key = target.key();
            let due = {
                let clock = clocks
                    .entry(key.clone())
                    .or_insert_with(|| NamespaceClock::due_now(now));
                now >= clock.retry_after
            };
            if !due {
                // This namespace's own backoff has not elapsed yet — and only
                // this namespace's: every other target in this same tick is
                // still evaluated against its own clock (Invariant 2, FR-488).
                continue;
            }

            let outcome = match target {
                NamespaceTarget::Project {
                    project_id,
                    server_project_id,
                } => {
                    let clock = clocks.get_mut(&key).expect("just inserted above");
                    process_project_namespace(&daemon, *project_id, *server_project_id, clock, now)
                        .await
                }
                NamespaceTarget::Global(ns) => {
                    let clock = clocks.get_mut(&key).expect("just inserted above");
                    process_global_namespace(&daemon, ns, clock, now).await
                }
            };
            clocks
                .get_mut(&key)
                .expect("just inserted above")
                .record(now, outcome);
        }
    }
}

/// Drain (if due) and pull (on its own interval) one linked project.
async fn process_project_namespace(
    d: &Daemon,
    project_id: Uuid,
    server_project_id: Uuid,
    clock: &mut NamespaceClock,
    now: Instant,
) -> NamespaceOutcome {
    let mut transient = false;

    let (pending, _) = match outbox::counts(&d.store, project_id).await {
        Ok(c) => c,
        // A local read failure is not a reason to punish this namespace's
        // retry timing — it says nothing about the server at all.
        Err(_) => return NamespaceOutcome::Ok,
    };
    let blocked = if pending == 0 {
        outbox::blocked_count(&d.store, project_id)
            .await
            .unwrap_or(0)
    } else {
        0
    };
    let probe_due = clock.probe_due(now);

    // Nothing queued and nothing blocked worth re-probing: no request, no
    // credentials needed, no noise (unchanged from before this feature).
    if pending > 0 || (blocked > 0 && probe_due) {
        // The drain re-reads `GET /api/version` on every run, so the probe
        // clock advances whenever a drain happens — not only when the
        // `probe_due` gate is what let it happen. The gate exists to stop a
        // namespace holding *only* blocked work from re-reading capabilities on
        // every tick; it is not the sole occasion on which a read occurs.
        clock.mark_probed(now);
        match drain(d, project_id, server_project_id).await {
            Ok((applied, duplicate, rejected)) => {
                if applied + duplicate > 0 {
                    tracing::info!(
                        project = %project_id, applied, duplicate, rejected,
                        "background sync delivered queued work"
                    );
                }
                if rejected == 0 {
                    let _ =
                        cursor::record_success(&d.store, &SyncNamespace::Project(project_id)).await;
                }
            }
            Err(e) => {
                transient = true;
                tracing::debug!(project = %project_id, error = %e, "sync deferred");
            }
        }
    }

    // T094 fix (FR-489, Invariant 3): pull runs on its own interval,
    // unconditionally — never gated on `pending == 0`. A project that only
    // ever consumes shared records still gets them.
    if clock.pull_due(now) {
        clock.mark_pulled(now);
        if pull(d, project_id, server_project_id).await.is_err() {
            transient = true;
        }
    }

    if transient {
        NamespaceOutcome::Transient
    } else {
        NamespaceOutcome::Ok
    }
}

/// Drain (if due) and pull (if due) a personal or team namespace.
///
/// **The pull is unconditional** — not gated on there being anything pending or
/// blocked to push first (FR-489, `sync-namespaces.md` §5). That gating is the
/// defect this feature had to correct in the project lane, and it bites harder
/// here: personal and (especially) team knowledge is the first content a machine
/// can legitimately only ever consume, so a member who never proposes anything
/// would otherwise never learn that an admin ratified something.
async fn process_global_namespace(
    d: &Daemon,
    namespace: &SyncNamespace,
    clock: &mut NamespaceClock,
    now: Instant,
) -> NamespaceOutcome {
    let mut transient = false;
    let key = namespace.key();

    let (pending, _) = outbox::counts_namespace(&d.store, &key)
        .await
        .unwrap_or((0, 0));
    let blocked = if pending == 0 {
        outbox::blocked_count_namespace(&d.store, &key)
            .await
            .unwrap_or(0)
    } else {
        0
    };
    let probe_due = clock.probe_due(now);

    if pending > 0 || (blocked > 0 && probe_due) {
        // The drain re-reads `GET /api/version` on every run, so the probe
        // clock advances whenever a drain happens — not only when the
        // `probe_due` gate is what let it happen. The gate exists to stop a
        // namespace holding *only* blocked work from re-reading capabilities on
        // every tick; it is not the sole occasion on which a read occurs.
        clock.mark_probed(now);
        match drain_global(d, namespace).await {
            Ok((applied, duplicate, rejected)) => {
                if applied + duplicate > 0 {
                    tracing::info!(
                        namespace = %key, applied, duplicate, rejected,
                        "background sync delivered queued global knowledge"
                    );
                }
                if rejected == 0 {
                    let _ = cursor::record_success(&d.store, namespace).await;
                }
            }
            Err(e) => {
                transient = true;
                tracing::debug!(namespace = %key, error = %e, "global sync deferred");
            }
        }
    }

    if clock.pull_due(now) {
        clock.mark_pulled(now);
        match pull_global(d, namespace).await {
            Ok(landed) if landed > 0 => {
                tracing::info!(namespace = %key, landed, "pulled global knowledge");
            }
            Ok(_) => {}
            Err(e) => {
                transient = true;
                tracing::debug!(namespace = %key, error = %e, "global pull deferred");
            }
        }
    }

    if transient {
        NamespaceOutcome::Transient
    } else {
        NamespaceOutcome::Ok
    }
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

/// Read this token's account id from `GET /api/auth/me` and record it.
///
/// Best-effort: an unreachable server leaves whatever was already known, which
/// is the honest answer — the identity did not change because the network did.
/// Returns whether an identity is now known at all.
async fn learn_account_identity(d: &Daemon) -> bool {
    let Ok(client) = client(d).await else {
        return d.server.read().await.account_id.is_some();
    };
    let Ok(body) = client.get("/api/auth/me").await else {
        return d.server.read().await.account_id.is_some();
    };
    let Some(id) = body
        .get("id")
        .and_then(|v| v.as_str())
        .and_then(|s| Uuid::parse_str(s).ok())
    else {
        return d.server.read().await.account_id.is_some();
    };

    d.server.write().await.account_id = Some(id);
    let mut config = d.config.write().await;
    if config.server_account_id != Some(id) {
        config.server_account_id = Some(id);
        if let Err(e) = config.save() {
            tracing::debug!(error = %e, "could not persist the server account id");
        }
    }
    true
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
    let url = d.server.read().await.url.clone()?;
    let client = client(d).await.ok()?;
    let body = client.get("/api/version").await.ok()?;
    let reported: Option<Uuid> = body
        .get("server_instance_id")
        .and_then(|v| v.as_str())
        .and_then(|s| Uuid::parse_str(s).ok());

    let owner = d.owner_identity().await;
    let provisional = provisional_instance(&url);
    let instance = reported.unwrap_or(provisional);

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
        for (from, to) in [
            (
                SyncNamespace::Personal(provisional, owner),
                SyncNamespace::Personal(instance, owner),
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

    let personal = SyncNamespace::Personal(instance, owner);
    let team = SyncNamespace::Team(instance);

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
        vec![&personal, &team]
    } else {
        vec![&personal]
    };
    for namespace in lanes {
        if let Err(e) = cursor::establish(&d.store, namespace).await {
            tracing::debug!(namespace = %namespace.key(), error = %e, "could not establish namespace");
            return None;
        }
    }

    // Both backfills are idempotent by the outbox's own key, so running them on
    // every establish costs one query per row and enqueues nothing twice.
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
async fn pull_global(d: &Daemon, namespace: &SyncNamespace) -> Result<usize, WireError> {
    let c = client(d).await?;

    // **A lane only ever pulls from the instance it names** (FR-495, FR-496).
    //
    // The lane key carries a server instance id, and until this check existed
    // that id was treated as a fact about where the rows came from — while the
    // HTTP client points at whatever server the current token belongs to. After
    // `cairn auth token set` moved a store to a second deployment, the old
    // `team:<A>` lane happily pulled server B and merged B's ratified guidance
    // into a corpus bound to A, labelled as A's. `merge_synced_team` could not
    // catch it: it was handed the lane's instance, which matched by construction.
    //
    // Confirming the peer here is the only place the two can be compared, and it
    // is one `GET /api/version` — the same endpoint the drain already polls for
    // capabilities every cycle.
    let lane_instance = match namespace {
        SyncNamespace::Personal(instance, _) | SyncNamespace::Team(instance) => Some(*instance),
        SyncNamespace::Project(_) => None,
    };
    if let Some(expected) = lane_instance {
        let url = d.server.read().await.url.clone().unwrap_or_default();
        // A peer that reports no instance is one below schema 3, and the lane it
        // corresponds to is the provisional one derived from the endpoint — the
        // same identity `establish_global_namespaces` would have used, so the two
        // agree without a special case.
        let reported = match c.get("/api/version").await {
            Ok(body) => body
                .get("server_instance_id")
                .and_then(|v| v.as_str())
                .and_then(|s| Uuid::parse_str(s).ok())
                .unwrap_or_else(|| provisional_instance(&url)),
            // Offline: nothing to pull anyway, and refusing here would be
            // indistinguishable from a mismatch.
            Err(e) => return Err(e),
        };
        if reported != expected {
            tracing::debug!(
                lane = %namespace.key(), peer = %reported,
                "not pulling: this lane belongs to a different server instance"
            );
            return Ok(0);
        }
    }

    let since = cursor::pull_cursor(&d.store, namespace)
        .await
        .map_err(storage_err)?;

    let (path, array) = match namespace {
        SyncNamespace::Personal(..) => ("/api/sync/changes/personal", "personal"),
        SyncNamespace::Team(_) => ("/api/sync/changes/team", "team"),
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
    for row in &rows {
        let merged = match namespace {
            SyncNamespace::Personal(..) => merge_pulled_personal(d, row).await,
            SyncNamespace::Team(instance) => merge_pulled_team(d, *instance, row).await,
            SyncNamespace::Project(_) => false,
        };
        if merged {
            landed += 1;
        }
    }

    if let Some(cursor) = body.get("cursor").and_then(|v| v.as_str()) {
        cursor::set_pull_cursor(&d.store, namespace, cursor)
            .await
            .map_err(storage_err)?;
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
async fn merge_pulled_personal(d: &Daemon, row: &serde_json::Value) -> bool {
    let Some(id) = pulled_uuid(row, "id") else {
        return false;
    };
    let Some(writer_id) = pulled_uuid(row, "writer_id") else {
        return false;
    };
    let Some(created_at) = pulled_time(row, "created_at") else {
        return false;
    };
    let knowledge_type: MemoryType = row
        .get("knowledge_type")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse().ok())
        .unwrap_or(MemoryType::Fact);

    let incoming = cairn_store::global::SyncedPersonalKnowledge {
        id,
        owner_user_id: d.owner_identity().await,
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
        Ok(_) => true,
        Err(e) => {
            tracing::debug!(personal = %id, error = %e, "a pulled personal row did not merge");
            false
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
async fn merge_pulled_team(d: &Daemon, instance: Uuid, row: &serde_json::Value) -> bool {
    let Some(id) = pulled_uuid(row, "id") else {
        return false;
    };
    let Some(writer_id) = pulled_uuid(row, "writer_id") else {
        return false;
    };
    let Some(created_at) = pulled_time(row, "created_at") else {
        return false;
    };
    let Some(proposed_by_user_id) = pulled_uuid(row, "proposed_by_user_id") else {
        return false;
    };
    let Ok(state) = row
        .get("state")
        .and_then(|v| v.as_str())
        .unwrap_or("proposed")
        .parse::<TeamState>()
    else {
        return false;
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
        retired_at: pulled_time(row, "retired_at"),
    };

    match cairn_store::global::merge_synced_team(&d.store, instance, incoming).await {
        Ok(_) => true,
        Err(e) => {
            tracing::debug!(team = %id, error = %e, "a pulled team row did not merge");
            false
        }
    }
}

/// Recover a `personal:*`/`team:*` namespace from the plain string
/// `outbox::known_namespaces` returns.
///
/// `SyncNamespace` has no public parser (`key()` is one-way, by design — it is
/// a cursor key, not a wire format), so this reads the same three shapes
/// `key()` produces rather than adding one to `cairn_core` (out of this task's
/// file ownership). `project:*` rows are excluded: `run_worker` already builds
/// project targets from `repo::list_projects`, which is the authoritative
/// source for a project's *current* `server_project_id` — parsing it back out
/// of a namespace string here would risk drifting from that if a project were
/// ever re-linked to a different server project.
fn parse_global_namespace(key: &str) -> Option<SyncNamespace> {
    if let Some(rest) = key.strip_prefix("personal:") {
        let (instance, user) = rest.split_once(':')?;
        return Some(SyncNamespace::Personal(
            Uuid::parse_str(instance).ok()?,
            Uuid::parse_str(user).ok()?,
        ));
    }
    if let Some(rest) = key.strip_prefix("team:") {
        return Some(SyncNamespace::Team(Uuid::parse_str(rest).ok()?));
    }
    None
}

struct Client {
    base: String,
    token: String,
    http: reqwest::Client,
}

async fn client(d: &Daemon) -> Result<Client, WireError> {
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
    async fn post(
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

    async fn get(&self, path: &str) -> Result<serde_json::Value, WireError> {
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
    let path = cairn_core::paths::token_path();
    std::fs::write(&path, token.trim())
        .map_err(|e| WireError::new(codes::STORAGE_UNAVAILABLE, e.to_string()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }

    let mut creds = d.server.write().await;
    creds.token = Some(token.trim().to_string());
    if let Some(url) = server_url {
        creds.url = Some(url.clone());
        let mut config = d.config.write().await;
        config.server_url = Some(url);
        config
            .save()
            .map_err(|e| WireError::new(codes::STORAGE_UNAVAILABLE, e.to_string()))?;
    }
    let url = creds.url.clone();
    drop(creds);

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
    let _ = std::fs::remove_file(cairn_core::paths::token_path());
    d.server.write().await.token = None;
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
pub async fn team_ratify_remote(d: &Daemon, id: Uuid, supersedes: Option<Uuid>) -> Reply {
    let c = client(d).await?;
    let mut body = json!({});
    if let Some(sup) = supersedes {
        body["supersedes"] = json!(sup);
    }
    c.post(&format!("/api/team/{id}/ratify"), &body).await
}

/// `POST /api/team/{id}/retire` (T121, T133). Same admin gate and
/// compare-and-swap shape as [`team_ratify_remote`].
pub async fn team_retire_remote(d: &Daemon, id: Uuid) -> Reply {
    let c = client(d).await?;
    c.post(&format!("/api/team/{id}/retire"), &json!({})).await
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
            ))
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

    for t in repo::list_tasks(&d.store, project.id, None)
        .await
        .map_err(storage_err)?
    {
        enqueue_one(
            d,
            policy,
            project.id,
            OutboxEntityType::Task,
            t.id,
            outbox::task_payload(&t),
        )
        .await?;
    }
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

    let (mut applied, mut duplicate, mut rejected) =
        drain(d, r.project.id, server_project_id).await?;
    let mut pulled = pull(d, r.project.id, server_project_id).await.unwrap_or(0);

    if rejected == 0 {
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
    let _ = establish_global_namespaces(d).await;
    let lanes = cursor::established(&d.store).await.unwrap_or_default();
    for namespace in lanes {
        if matches!(namespace, SyncNamespace::Project(_)) {
            continue;
        }
        // Another identity's lane is held on this store and not synchronized:
        // this machine is not authenticated as that account, so it has no
        // standing to push or pull for it (FR-567).
        if let SyncNamespace::Personal(_, user) = namespace {
            if user != d.owner_identity().await {
                continue;
            }
        }
        if let Ok((a, dup, rej)) = drain_global(d, &namespace).await {
            applied += a;
            duplicate += dup;
            rejected += rej;
        }
        pulled += pull_global(d, &namespace).await.unwrap_or(0);
    }

    Ok(json!({
        "applied": applied,
        "duplicate": duplicate,
        "rejected": rejected,
        "pulled": pulled,
    }))
}

/// Deliver queued work, in batches, until this drainer has nothing left.
///
/// Rows are *claimed* before they are sent (`outbox::claim`), so a drain running
/// at the same time as this one works on a disjoint set rather than re-sending
/// the same rows. A transient failure releases the claim; a permanent rejection
/// records the row `failed` (FR-056, FR-058).
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

    loop {
        let batch = outbox::claim(&d.store, project_id, BATCH)
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
async fn any_linked_project(d: &Daemon) -> Option<Uuid> {
    let projects = repo::list_projects(&d.store).await.ok()?;
    projects
        .into_iter()
        .find(|p| p.linked)
        .and_then(|p| p.server_project_id)
}

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
async fn drain_global(
    d: &Daemon,
    namespace: &SyncNamespace,
) -> Result<(usize, usize, usize), WireError> {
    // Same single-drainer discipline `drain` uses, and the same lock: claiming
    // is what makes two concurrent drains correct, this is what keeps them
    // orderly, and there is no reason a project drain and a global drain
    // running at once would be more correct interleaved than serialized.
    let _drain_guard = d.sync_drain.lock().await;

    let capability = refresh_capability(d, namespace).await;
    let key = namespace.key();

    let Some(auth_project) = any_linked_project(d).await else {
        return Ok((0, 0, 0));
    };

    let (mut applied, mut duplicate, mut rejected, mut blocked) = (0, 0, 0, 0);
    let mut connection: Option<Client> = None;

    loop {
        let batch = outbox::claim_namespace(&d.store, &key, BATCH)
            .await
            .map_err(storage_err)?;
        if batch.is_empty() {
            break;
        }

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
            project_id: auth_project,
            items,
        })
        .unwrap_or(json!({}));

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
    Ok((applied, duplicate, rejected))
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
        return cursor::server_capability(&d.store, namespace)
            .await
            .ok()
            .flatten()
            .unwrap_or_else(|| UNKNOWN_CAPABILITY.to_string());
    };
    let Ok(body) = client.get("/api/version").await else {
        return cursor::server_capability(&d.store, namespace)
            .await
            .ok()
            .flatten()
            .unwrap_or_else(|| UNKNOWN_CAPABILITY.to_string());
    };

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
    (OutboxEntityType::TaskCriterion, &["task_criteria"]),
    (OutboxEntityType::TaskBlocker, &["task_blockers"]),
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

    // Tasks before their criteria, for the same reason memories come before
    // their relations: a criterion naming a task this store does not have is
    // held rather than invented, and importing in this order means it usually
    // does not have to be.
    for t in body
        .get("tasks")
        .and_then(|v| v.as_array())
        .unwrap_or(&Vec::new())
    {
        if import_task(d, project_id, t).await {
            count += 1;
        }
    }

    // Criteria and blockers upsert by stable id, so two machines that changed
    // different criteria offline both land — neither overwrites the other.
    let id_key = |v: &serde_json::Value| {
        v.get("id")
            .and_then(|x| x.as_str())
            .unwrap_or_default()
            .to_string()
    };
    for c in body
        .get("criteria")
        .and_then(|v| v.as_array())
        .unwrap_or(&Vec::new())
    {
        match import_criterion(d, c).await {
            Placement::Placed => count += 1,
            Placement::AwaitingParent(waiting_on) => {
                hold_for_a_later_pull(d, project_id, "criterion", &id_key(c), c, waiting_on).await;
            }
            Placement::Unusable => {}
        }
    }
    for b in body
        .get("blockers")
        .and_then(|v| v.as_array())
        .unwrap_or(&Vec::new())
    {
        match import_blocker(d, b).await {
            Placement::Placed => count += 1,
            Placement::AwaitingParent(waiting_on) => {
                hold_for_a_later_pull(d, project_id, "blocker", &id_key(b), b, waiting_on).await;
            }
            Placement::Unusable => {}
        }
    }

    // Records earlier pulls could not place. Replayed after the fresh page, so
    // a parent that arrived in *this* page releases what was waiting on it
    // without waiting for another pull (#44).
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
/// earlier pull is placed as soon as the memory it names lands. Nothing here
/// depends on another held record — relations wait on memories and criteria and
/// blockers wait on tasks, and neither is itself deferred — so one pass is
/// enough and there is no ordering to get right.
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
                "criterion" => import_criterion(d, &value).await,
                "blocker" => import_blocker(d, &value).await,
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

/// Import one criterion that arrived from a peer.
async fn import_criterion(d: &Daemon, value: &serde_json::Value) -> Placement {
    let uuid = |k: &str| {
        value
            .get(k)
            .and_then(|v| v.as_str())
            .and_then(|s| Uuid::parse_str(s).ok())
    };
    let (Some(id), Some(task_id)) = (uuid("id"), uuid("task_id")) else {
        return Placement::Unusable;
    };
    // A criterion for a task that has not arrived is held, not invented — and
    // held durably, for the reason the relation case is (#44): the cursor does
    // not offer it again.
    if repo::task(&d.store, task_id).await.is_err() {
        tracing::debug!(
            criterion_id = %id, %task_id,
            "holding a criterion whose task has not arrived yet"
        );
        return Placement::AwaitingParent(task_id);
    }
    let str_of = |k: &str| value.get(k).and_then(|v| v.as_str()).unwrap_or_default();
    let (Ok(state), Ok(verification)) = (str_of("state").parse(), str_of("verification").parse())
    else {
        return Placement::Unusable;
    };

    let stored = cairn_store::criteria::import_criterion(
        &d.store,
        id,
        task_id,
        value.get("ordinal").and_then(|v| v.as_i64()).unwrap_or(1),
        str_of("label"),
        str_of("text"),
        state,
        verification,
        value
            .get("deleted")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
    )
    .await
    .is_ok();
    if stored {
        Placement::Placed
    } else {
        Placement::Unusable
    }
}

/// Import one blocker that arrived from a peer.
async fn import_blocker(d: &Daemon, value: &serde_json::Value) -> Placement {
    let uuid = |k: &str| {
        value
            .get(k)
            .and_then(|v| v.as_str())
            .and_then(|s| Uuid::parse_str(s).ok())
    };
    let (Some(id), Some(task_id)) = (uuid("id"), uuid("task_id")) else {
        return Placement::Unusable;
    };
    // Held rather than dropped, for the same reason a criterion is (#44).
    if repo::task(&d.store, task_id).await.is_err() {
        tracing::debug!(
            blocker_id = %id, %task_id,
            "holding a blocker whose task has not arrived yet"
        );
        return Placement::AwaitingParent(task_id);
    }
    let str_of = |k: &str| value.get(k).and_then(|v| v.as_str()).unwrap_or_default();
    let Ok(state) = str_of("state").parse() else {
        return Placement::Unusable;
    };

    let stored = cairn_store::criteria::import_blocker(
        &d.store,
        id,
        task_id,
        str_of("description"),
        state,
        uuid("opened_by_session").unwrap_or_else(Uuid::nil),
        uuid("cleared_by_session"),
        value
            .get("deleted")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
    )
    .await
    .is_ok();
    if stored {
        Placement::Placed
    } else {
        Placement::Unusable
    }
}

/// Insert a peer's task locally.
///
/// The title, goal and status are the peer's; everything derived stays this
/// machine's. `local_revision` is never transmitted and never overwritten — it
/// is a private concurrency token (D80) — and the `acceptance_criteria`
/// projection is rebuilt from the criteria rows that arrive separately rather
/// than copied, so it cannot disagree with them.
async fn import_task(d: &Daemon, project_id: Uuid, value: &serde_json::Value) -> bool {
    let Some(id) = value
        .get("id")
        .and_then(|v| v.as_str())
        .and_then(|s| Uuid::parse_str(s).ok())
    else {
        return false;
    };
    let str_of = |k: &str| value.get(k).and_then(|v| v.as_str()).unwrap_or_default();
    let status = match str_of("status") {
        "" => "todo",
        other => other,
    };
    cairn_store::criteria::import_task(
        &d.store,
        id,
        project_id,
        str_of("title"),
        str_of("goal"),
        status,
        value
            .get("deleted")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
    )
    .await
    .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::ServerCredentials;
    use crate::testsupport as fx;

    // -------------------------------------------------------------------
    // `NamespaceClock` — per-namespace backoff, probe and pull scheduling
    // (T093, T094, T106, T107)
    // -------------------------------------------------------------------

    /// The core claim of T093: two namespaces' clocks are two independent
    /// `Duration`s, not one shared value. A transient failure recorded against
    /// one must not move the other's `retry_after` at all — the same guarantee
    /// `contracts/sync-namespaces.md` §4 states as "a `project:*` namespace
    /// hitting the server's rate limit backs off on its own schedule while
    /// `personal:*` and `team:*` continue retrying at `BACKOFF_MIN` on theirs."
    #[test]
    fn a_transient_failure_on_one_namespace_never_moves_another_namespaces_clock() {
        let now = Instant::now();
        let mut struggling = NamespaceClock::due_now(now);
        let healthy = NamespaceClock::due_now(now);

        struggling.record(now, NamespaceOutcome::Transient);
        struggling.record(now, NamespaceOutcome::Transient);

        assert!(
            struggling.retry_after > now,
            "a namespace with two transient failures must not be immediately eligible again"
        );
        assert!(
            struggling.backoff > BACKOFF_MIN,
            "backoff must have doubled at least once"
        );
        // The namespace that never failed is exactly as eligible as it was at
        // creation — nothing about the other namespace's struggle reached it.
        assert_eq!(healthy.retry_after, now);
        assert_eq!(healthy.backoff, BACKOFF_MIN);
    }

    /// Backoff doubles on repeated failure and is capped at `BACKOFF_MAX`,
    /// then a single success clears it back to `BACKOFF_MIN` outright — not
    /// merely halved, so a namespace that just recovered is as eligible as one
    /// that never failed (Invariant 2).
    #[test]
    fn backoff_doubles_to_a_ceiling_and_a_success_clears_it_entirely() {
        let now = Instant::now();
        let mut clock = NamespaceClock::due_now(now);

        for _ in 0..10 {
            clock.record(now, NamespaceOutcome::Transient);
        }
        assert_eq!(
            clock.backoff, BACKOFF_MAX,
            "backoff must not exceed the ceiling"
        );

        clock.record(now, NamespaceOutcome::Ok);
        assert_eq!(
            clock.backoff, BACKOFF_MIN,
            "a success must clear backoff outright"
        );
        assert_eq!(
            clock.retry_after, now,
            "a successful namespace is immediately eligible again"
        );
    }

    /// A fresh clock is due for its probe and its pull immediately — a
    /// namespace seen for the first time, or a daemon that just restarted,
    /// must not wait a full interval before its first attempt (FR-489,
    /// Invariant 3).
    /// The pull and probe clocks actually advance.
    ///
    /// They did not. `record` folded in an outcome and touched only the backoff,
    /// so nothing in production ever moved `last_pull` or `last_probe`: both
    /// predicates stayed true from the first tick onward and `WORKER_TICK`
    /// became the pull frequency — three namespaces issuing six requests a
    /// second, forever, against a server that answers every one of them
    /// successfully so backoff never engages. The interval constant existed and
    /// described nothing.
    ///
    /// Falsified by removing either `mark_` call from the processing functions,
    /// or by folding them back into `record`.
    #[test]
    fn marking_a_pull_or_a_probe_is_what_advances_its_clock() {
        let now = Instant::now();
        let mut clock = NamespaceClock::due_now(now);
        assert!(clock.pull_due(now) && clock.probe_due(now));

        clock.mark_pulled(now);
        clock.mark_probed(now);
        assert!(
            !clock.pull_due(now) && !clock.probe_due(now),
            "marking did not advance the clock"
        );

        // An outcome is a different thing and must not reset either one: a
        // namespace that just succeeded is eligible to *retry* immediately, and
        // is not thereby due for another scheduled pull.
        clock.record(now, NamespaceOutcome::Ok);
        assert!(
            !clock.pull_due(now),
            "recording a successful outcome made the namespace due for another pull"
        );
        clock.record(now, NamespaceOutcome::Transient);
        assert!(
            !clock.pull_due(now),
            "recording a transient failure made the namespace due for another pull"
        );

        assert!(clock.pull_due(now + Duration::from_secs(PULL_INTERVAL_SECONDS)));
        assert!(clock.probe_due(now + CAPABILITY_PROBE));
    }

    #[test]
    fn a_fresh_clock_is_due_for_probe_and_pull_immediately() {
        let now = Instant::now();
        let clock = NamespaceClock::due_now(now);
        assert!(clock.probe_due(now));
        assert!(clock.pull_due(now));
    }

    /// T094, the conditional-pull fix, at the unit level: the pull-due timer
    /// is `PULL_INTERVAL_SECONDS`, not `WORKER_TICK` — a tick that has not
    /// covered the interval yet must not read as pull-due, or every namespace
    /// would poll the server on every 500ms tick forever (§5's exact
    /// objection to "just move the call out of the `pending == 0` guard").
    #[test]
    fn pull_is_not_due_again_before_the_interval_elapses() {
        let start = Instant::now();
        let mut clock = NamespaceClock::due_now(start);
        clock.last_pull = start; // as if a pull just happened

        let one_tick_later = start + WORKER_TICK;
        assert!(
            !clock.pull_due(one_tick_later),
            "a single worker tick must not be enough to make the next pull due"
        );

        let after_the_interval = start + Duration::from_secs(PULL_INTERVAL_SECONDS);
        assert!(clock.pull_due(after_the_interval));
    }

    // -------------------------------------------------------------------
    // `parse_global_namespace`
    // -------------------------------------------------------------------

    #[test]
    fn parse_global_namespace_round_trips_personal_and_team_keys() {
        let personal = SyncNamespace::Personal(new_id(), new_id());
        let team = SyncNamespace::Team(new_id());

        assert_eq!(parse_global_namespace(&personal.key()), Some(personal));
        assert_eq!(parse_global_namespace(&team.key()), Some(team));
    }

    /// `project:*` keys are deliberately not recovered here — `run_worker`
    /// builds project targets from `repo::list_projects`, the authoritative
    /// source, not by reparsing a namespace string.
    #[test]
    fn parse_global_namespace_never_recovers_a_project_namespace() {
        let project = SyncNamespace::Project(new_id());
        assert_eq!(parse_global_namespace(&project.key()), None);
    }

    #[test]
    fn parse_global_namespace_rejects_garbage() {
        assert_eq!(parse_global_namespace("nonsense"), None);
        assert_eq!(parse_global_namespace("personal:not-a-uuid:also-not"), None);
        assert_eq!(parse_global_namespace("team:not-a-uuid"), None);
    }

    /// A linked project must report the link it has.
    ///
    /// This is the regression this release is named for: bare `cairn link`
    /// answered `linked: false` unconditionally, so a linked project was told
    /// it was not linked and pointed at `cairn link --create` — which would
    /// have made a second shared project for a repository that already had
    /// one — while `cairn status`, reading the same row, said the opposite.
    /// A peer cannot put a state on a row that Cairn refuses to derive.
    ///
    /// `import_verification` is the one path a verification reaches a row
    /// without passing `rebuild_verification`, the column carries no CHECK, and
    /// what a peer sends is input rather than truth. Two pairs must not be
    /// storable: a state outside the enum, and `verified` with no authority —
    /// which rendered as a bare `verified` against FR-370, re-emitted itself to
    /// the next peer, and was silently rewritten by the next rebuild anyway.
    #[tokio::test]
    async fn an_imported_verification_cannot_invent_a_state_or_drop_its_authority() {
        let d = fx::daemon().await;
        let p = fx::project(&d, "importing", None).await;
        let s = fx::session(&d, &p, "peer").await;

        let stored = |d: &Daemon, id: Uuid| {
            let store = d.store.clone();
            async move {
                sqlx::query_as::<_, (String, Option<String>)>(
                    "SELECT verification, verification_authority FROM memories WHERE id = ?1",
                )
                .bind(id.to_string())
                .fetch_one(store.pool())
                .await
                .expect("row")
            }
        };

        let make = |content: &'static str| {
            let store = d.store.clone();
            let (project, session) = (p.id, s.id);
            async move {
                cairn_store::repo::create_memory(
                    &store,
                    cairn_store::repo::NewMemory::free_form(
                        project,
                        cairn_core::MemoryType::Fact,
                        cairn_core::MemoryScope::Project,
                        &project.to_string(),
                        content,
                        session,
                        false,
                        &[],
                    ),
                    cairn_store::outbox::SyncPolicy {
                        linked: false,
                        server_project_id: None,
                    },
                )
                .await
                .expect("memory")
                .id
            }
        };

        // `verified` with nothing standing behind it.
        let bare = make("A peer said this was verified.").await;
        import_verification(
            &d,
            bare,
            &serde_json::json!({ "verification": { "state": "verified" } }),
        )
        .await;
        assert_eq!(
            stored(&d, bare).await,
            ("unverified".to_string(), None),
            "a peer stored `verified` with no authority"
        );

        // A state that is not a state.
        let bogus = make("A peer invented a state for this.").await;
        import_verification(
            &d,
            bogus,
            &serde_json::json!({ "verification": { "state": "extremely_verified" } }),
        )
        .await;
        assert_eq!(
            stored(&d, bogus).await,
            ("unverified".to_string(), None),
            "a peer stored a state outside the enum"
        );

        // The ordinary case still lands, wearing the imported badge.
        let good = make("A peer really did check this.").await;
        import_verification(
            &d,
            good,
            &serde_json::json!({ "verification": { "state": "verified", "authority": "cairn" } }),
        )
        .await;
        assert_eq!(
            stored(&d, good).await,
            ("verified".to_string(), Some("remote_cairn".to_string())),
            "an honest imported verification did not land"
        );
    }

    /// A relation whose memory has not arrived is held and later placed, not
    /// dropped (#44).
    ///
    /// The pull cursor is a timestamp over `updated_at` and the server offers a
    /// record again only when the record itself changes, so "the next pull
    /// carries it again" was never true. One page of 500 memories reaches it:
    /// the cursor pins to that page's newest row, and a relation older than
    /// that row whose memory falls in the *next* page was lost permanently.
    #[tokio::test]
    async fn a_relation_whose_memory_has_not_arrived_is_held_until_it_does() {
        let d = fx::daemon().await;
        let p = fx::project(&d, "held-relation", None).await;
        let s = fx::session(&d, &p, "peer").await;

        let make = |content: &'static str| {
            let store = d.store.clone();
            let (project, session) = (p.id, s.id);
            async move {
                cairn_store::repo::create_memory(
                    &store,
                    cairn_store::repo::NewMemory::free_form(
                        project,
                        cairn_core::MemoryType::Fact,
                        cairn_core::MemoryScope::Project,
                        &project.to_string(),
                        content,
                        session,
                        false,
                        &[],
                    ),
                    cairn_store::outbox::SyncPolicy {
                        linked: false,
                        server_project_id: None,
                    },
                )
                .await
                .expect("memory")
                .id
            }
        };

        let present = make("This memory arrived in the first page.").await;
        // The memory this relation names has not arrived, and will not until a
        // later page.
        let absent = Uuid::now_v7();
        let wire = serde_json::json!({
            "from_memory_id": absent.to_string(),
            "to_memory_id": present.to_string(),
            "kind": "supersedes",
            "basis": "explicit_agent",
            "decided_by_session": s.id.to_string(),
        });

        assert_eq!(
            import_relation(&d, p.id, &wire).await,
            Placement::AwaitingParent(absent),
            "the relation was not recognised as waiting on its memory"
        );

        // The fresh-page path holds it, which is what the cursor cannot do.
        hold_for_a_later_pull(&d, p.id, "relation", &relation_key(&wire), &wire, absent).await;
        assert_eq!(
            repo::deferred_count(&d.store, p.id).await.expect("count"),
            1,
            "the relation was dropped rather than held"
        );

        // Replaying while the memory is still missing keeps holding it, and
        // records the attempt rather than retrying in silence.
        assert_eq!(replay_deferred(&d, p.id).await, 0);
        assert_eq!(
            repo::deferred_count(&d.store, p.id).await.expect("count"),
            1,
            "a record still waiting on its parent must stay held"
        );
        let held = repo::deferred_records(&d.store, p.id, 10)
            .await
            .expect("held");
        assert_eq!(held[0].attempts, 1, "the attempt was not recorded");
        assert_eq!(held[0].waiting_on, absent.to_string());

        // The memory arrives in a later page, through the same importer the
        // real pull uses, exactly as the ordering failure has it.
        import_memory(
            &d,
            p.id,
            &serde_json::json!({
                "id": absent.to_string(),
                "type": "fact",
                "scope": "project",
                "scope_key": p.id.to_string(),
                "content": "This memory arrived in a later page.",
                "provenance": { "session_id": s.id.to_string() },
            }),
        )
        .await
        .expect("the later page's memory did not import");

        assert_eq!(
            replay_deferred(&d, p.id).await,
            1,
            "the held relation was not placed once its memory arrived"
        );
        assert_eq!(
            repo::deferred_count(&d.store, p.id).await.expect("count"),
            0,
            "a placed record must stop being held"
        );

        let stored = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM memory_relations
              WHERE from_memory_id = ?1 AND to_memory_id = ?2 AND kind = 'supersedes'",
        )
        .bind(absent.to_string())
        .bind(present.to_string())
        .fetch_one(d.store.pool())
        .await
        .expect("query");
        assert_eq!(stored, 1, "the relation never reached the store");
    }

    /// A criterion whose task has not arrived is held, then placed (#44).
    #[tokio::test]
    async fn a_criterion_whose_task_has_not_arrived_is_held_until_it_does() {
        let d = fx::daemon().await;
        let p = fx::project(&d, "held-criterion", None).await;

        let task_id = Uuid::now_v7();
        let criterion_id = Uuid::now_v7();
        let wire = serde_json::json!({
            "id": criterion_id.to_string(),
            "task_id": task_id.to_string(),
            "ordinal": 1,
            "label": "C1",
            "text": "The daemon starts in the worktree it was asked about.",
            "state": "pending",
            "verification": "unverified",
        });

        assert_eq!(
            import_criterion(&d, &wire).await,
            Placement::AwaitingParent(task_id),
            "the criterion was not recognised as waiting on its task"
        );
        hold_for_a_later_pull(
            &d,
            p.id,
            "criterion",
            &criterion_id.to_string(),
            &wire,
            task_id,
        )
        .await;
        assert_eq!(replay_deferred(&d, p.id).await, 0);

        // The task arrives on a later page.
        assert!(
            import_task(
                &d,
                p.id,
                &serde_json::json!({
                    "id": task_id.to_string(),
                    "title": "Fix the daemon's working directory",
                    "goal": "Start where asked.",
                    "status": "todo",
                }),
            )
            .await,
            "the task fixture did not import"
        );

        assert_eq!(
            replay_deferred(&d, p.id).await,
            1,
            "the held criterion was not placed once its task arrived"
        );
        assert_eq!(
            repo::deferred_count(&d.store, p.id).await.expect("count"),
            0
        );
        let stored = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM task_criteria WHERE id = ?1 AND task_id = ?2",
        )
        .bind(criterion_id.to_string())
        .bind(task_id.to_string())
        .fetch_one(d.store.pool())
        .await
        .expect("query");
        assert_eq!(stored, 1, "the criterion never reached the store");
    }

    /// A record that can never be placed is released, not retried forever.
    ///
    /// Holding is for a parent that has not arrived *yet*. A payload nothing can
    /// parse has no parent to wait for, and keeping it would be a leak with no
    /// outcome.
    #[tokio::test]
    async fn a_record_that_can_never_be_placed_stops_being_held() {
        let d = fx::daemon().await;
        let p = fx::project(&d, "unusable", None).await;

        repo::defer_pulled_record(
            &d.store,
            p.id,
            "relation",
            "nonsense",
            "{ this is not json",
            &Uuid::now_v7().to_string(),
        )
        .await
        .expect("defer");

        assert_eq!(replay_deferred(&d, p.id).await, 0);
        assert_eq!(
            repo::deferred_count(&d.store, p.id).await.expect("count"),
            0,
            "an unplaceable record is still being held"
        );
    }

    /// A record the server sends again replaces its held copy.
    #[tokio::test]
    async fn re_sending_a_held_record_does_not_pile_up_rows() {
        let d = fx::daemon().await;
        let p = fx::project(&d, "resent", None).await;
        let task_id = Uuid::now_v7();
        let wire = serde_json::json!({ "id": "c", "task_id": task_id.to_string() });

        for _ in 0..3 {
            hold_for_a_later_pull(&d, p.id, "criterion", "c", &wire, task_id).await;
        }
        assert_eq!(
            repo::deferred_count(&d.store, p.id).await.expect("count"),
            1,
            "a re-sent record was held more than once"
        );
    }

    #[tokio::test]
    async fn link_status_reports_an_existing_link() {
        let d = fx::daemon().await;
        let p = fx::project(&d, "linked", Some("github.com/example/linked")).await;
        let target = Uuid::now_v7();
        repo::link_project(&d.store, p.id, target)
            .await
            .expect("link");

        let v = link_status(&d, &fx::resolved(&fx::reload(&d, p.id).await))
            .await
            .expect("a linked project answers");

        assert_eq!(v["linked"], true);
        assert_eq!(v["server_project_id"], target.to_string());
        assert!(
            v["hint"].as_str().unwrap_or_default().contains("unlink"),
            "the hint should offer the way out, not the way in: {v}"
        );
    }

    /// And it must answer with no server and no token stored.
    ///
    /// Whether a project is linked is local state, so reading it must never
    /// need the network (C1, FR-045). Before the fix this failed outright with
    /// `no server configured`.
    #[tokio::test]
    async fn link_status_answers_offline_for_an_unlinked_project() {
        let d = fx::daemon().await;
        let p = fx::project(&d, "solo", Some("github.com/example/solo")).await;

        let v = link_status(&d, &fx::resolved(&p))
            .await
            .expect("an unlinked project answers without a server");

        assert_eq!(v["linked"], false);
        assert_eq!(
            v["candidates"].as_array().map(Vec::len),
            Some(0),
            "candidates need a server; with none stored the list is empty"
        );
    }

    /// A *configured but unreachable* server must not change the answer.
    ///
    /// The shared client allows 20s. Spending that on a question answered from
    /// the local row would make nonsense of calling this offline-capable, so
    /// the candidate lookup carries its own short budget and the answer goes
    /// out with an empty list when it expires. Port 1 refuses immediately,
    /// which exercises the same path without spending the budget.
    #[tokio::test]
    async fn an_unreachable_server_still_yields_a_truthful_answer() {
        let d = fx::daemon_with(
            cairn_core::CairnConfig::default(),
            ServerCredentials {
                url: Some("http://127.0.0.1:1".to_string()),
                token: Some("irrelevant".to_string()),
                // No account identity: this daemon has never reached a server,
                // which is the situation under test.
                account_id: None,
            },
        )
        .await;
        let p = fx::project(&d, "offline", Some("github.com/example/offline")).await;

        let started = std::time::Instant::now();
        let v = link_status(&d, &fx::resolved(&p))
            .await
            .expect("an unreachable server is not an error here");

        assert_eq!(v["linked"], false);
        assert_eq!(v["candidates"].as_array().map(Vec::len), Some(0));
        assert!(
            started.elapsed() < CANDIDATE_LOOKUP_BUDGET,
            "a refused connection should not spend the lookup budget, took {:?}",
            started.elapsed()
        );
    }

    /// `linked = 1` with no `server_project_id` is a damaged row.
    ///
    /// The schema permits the pair to disagree. Answering "not linked" would
    /// reintroduce the exact contradiction with `cairn status` that this area
    /// was fixed for, so the daemon names the problem instead.
    #[tokio::test]
    async fn a_damaged_row_is_reported_rather_than_called_unlinked() {
        let d = fx::daemon().await;
        let p = fx::project(&d, "damaged", None).await;
        sqlx::query("UPDATE projects SET linked = 1, server_project_id = NULL WHERE id = ?1")
            .bind(p.id.to_string())
            .execute(d.store.pool())
            .await
            .expect("damage the row");

        let err = link_status(&d, &fx::resolved(&fx::reload(&d, p.id).await))
            .await
            .expect_err("a damaged row must not be reported as a clean answer");

        assert_eq!(err.code, codes::STORAGE_UNAVAILABLE);
        assert!(
            err.message.contains("no shared project id"),
            "the error should say what is actually wrong: {}",
            err.message
        );
    }

    /// `unlink` leaves `server_project_id` set, so the pair disagrees the other
    /// way round. That direction is *not* damage — it is a project that used to
    /// be shared — and must read as simply unlinked.
    #[tokio::test]
    async fn a_project_that_was_unlinked_reads_as_unlinked() {
        let d = fx::daemon().await;
        let p = fx::project(&d, "formerly", None).await;
        repo::link_project(&d.store, p.id, Uuid::now_v7())
            .await
            .expect("link");
        repo::unlink_project(&d.store, p.id).await.expect("unlink");

        let reloaded = fx::reload(&d, p.id).await;
        assert!(
            reloaded.server_project_id.is_some(),
            "precondition: unlink keeps the id, which is what makes this case real"
        );

        let v = link_status(&d, &fx::resolved(&reloaded))
            .await
            .expect("an unlinked project answers");
        assert_eq!(v["linked"], false);
    }

    #[test]
    fn urlencode_escapes_what_a_remote_can_contain() {
        assert_eq!(
            urlencode("github.com/example/repo"),
            "github.com%2Fexample%2Frepo"
        );
        assert_eq!(urlencode("a b"), "a%20b");
        assert_eq!(urlencode("plain-name_1.git"), "plain-name_1.git");
    }
}
