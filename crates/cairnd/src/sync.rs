//! Opt-in synchronization with the Cairn server (FR-053 – FR-058, D9, D14).
//!
//! Local → server for what this machine produced, plus read access to shared
//! records produced by others. Delivery is idempotent, offline is normal, and
//! an unlinked project never produces a request.

use crate::state::{storage_err, Daemon};
use cairn_core::domain::*;
use cairn_core::wire::*;
use cairn_store::{outbox, repo};
use serde_json::json;
use std::time::Duration;
use uuid::Uuid;

const BATCH: i64 = 100;

/// How often the background worker looks for queued work.
const WORKER_TICK: Duration = Duration::from_millis(500);
/// Backoff after a transient failure: doubles to a ceiling, then holds.
const BACKOFF_MIN: Duration = Duration::from_millis(500);
const BACKOFF_MAX: Duration = Duration::from_secs(30);
/// How often a project holding **only** retained work asks the server whether
/// it has been upgraded (FR-418).
///
/// Slower than the worker tick on purpose. There is nothing to send, so this is
/// a single small request every few seconds rather than one every half second —
/// and noticing an upgrade a few seconds late costs nothing, while never
/// noticing it costs the whole promise.
const CAPABILITY_PROBE: Duration = Duration::from_secs(5);

type Reply = Result<serde_json::Value, WireError>;

/// Drain the outbox automatically, forever (FR-056, D9).
///
/// `cairn sync now` stays available as an explicit trigger, but it is not the
/// only one: work queued while the server was unreachable is delivered when it
/// comes back, with no manual step. Transient failures back off; permanent
/// rejections are already recorded as `failed` by `drain` and are not retried.
pub async fn run_worker(daemon: std::sync::Arc<Daemon>) {
    let mut backoff = BACKOFF_MIN;
    // Due immediately, so a daemon starting up next to an already-upgraded
    // server does not wait an interval before noticing.
    let mut last_probe = std::time::Instant::now() - CAPABILITY_PROBE;
    loop {
        tokio::time::sleep(WORKER_TICK).await;

        let projects = match repo::list_projects(&daemon.store).await {
            Ok(p) => p,
            Err(e) => {
                tracing::debug!(error = %e, "sync worker could not list projects");
                continue;
            }
        };

        let probe_due = last_probe.elapsed() >= CAPABILITY_PROBE;
        let mut probed = false;
        let mut hit_transient = false;
        for project in projects.iter().filter(|p| p.linked) {
            let Some(server_project_id) = project.server_project_id else {
                continue;
            };

            // Nothing queued: no request, no credentials needed, no noise.
            let (pending, _) = match outbox::counts(&daemon.store, project.id).await {
                Ok(c) => c,
                Err(_) => continue,
            };
            if pending == 0 {
                // Retained work is **not** pending, and a project holding only
                // retained work would otherwise never enter a drain again — so
                // the capability probe would never run, and the upgrade this
                // machine is waiting for would never be noticed. "Delivered
                // automatically when the server is upgraded" is the promise
                // FR-418 makes and `sync status` repeats to the user; without
                // this it would need a manual `cairn sync now` to come true.
                //
                // Rarely, though. There is nothing to send, so this is one
                // small request per interval, not one per tick.
                let blocked = outbox::blocked_count(&daemon.store, project.id)
                    .await
                    .unwrap_or(0);
                if blocked == 0 || !probe_due {
                    continue;
                }
                probed = true;
            }

            match drain(&daemon, project.id, server_project_id).await {
                Ok((applied, duplicate, rejected)) => {
                    if applied + duplicate > 0 {
                        tracing::info!(
                            project = %project.id, applied, duplicate, rejected,
                            "background sync delivered queued work"
                        );
                    }
                    if rejected == 0 {
                        let _ = repo::record_sync_success(&daemon.store, project.id).await;
                    }
                    let _ = pull(&daemon, project.id, server_project_id).await;
                }
                Err(e) => {
                    // Offline, unauthenticated, or the server is down: keep the
                    // work queued and try again later.
                    hit_transient = true;
                    tracing::debug!(project = %project.id, error = %e, "sync deferred");
                }
            }
        }

        if probed {
            last_probe = std::time::Instant::now();
        }
        if hit_transient {
            tokio::time::sleep(backoff).await;
            backoff = (backoff * 2).min(BACKOFF_MAX);
        } else {
            backoff = BACKOFF_MIN;
        }
    }
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
    Ok(json!({ "token_stored": true, "server_url": creds.url }))
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

pub async fn logout(d: &Daemon) -> Reply {
    let _ = std::fs::remove_file(cairn_core::paths::token_path());
    d.server.write().await.token = None;
    Ok(json!({ "token_stored": false }))
}

/// Opt a project into sharing.
///
/// `create` mints a shared project; `server_project_id` joins one. With
/// neither, remote-based candidates are *offered* for the user to confirm —
/// never applied silently (FR-064, D14).
pub async fn link(d: &Daemon, cwd: &str, server_project_id: Option<Uuid>, create: bool) -> Reply {
    let r = d.resolve(cwd).await?;

    // No arguments is a question, not an instruction: "am I linked?". It is
    // answered entirely from local state, before any server is contacted.
    if server_project_id.is_none() && !create {
        return link_status(d, &r).await;
    }

    let c = client(d).await?;

    let target = match (server_project_id, create) {
        (Some(id), _) => {
            c.post(&format!("/api/projects/{id}/join"), &json!({}))
                .await?;
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
        last_success_at: repo::last_sync_success(&d.store, r.project.id)
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
    let capability = repo::server_capability(&d.store, project_id)
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

    let (applied, duplicate, rejected) = drain(d, r.project.id, server_project_id).await?;
    let pulled = pull(d, r.project.id, server_project_id).await.unwrap_or(0);

    if rejected == 0 {
        repo::record_sync_success(&d.store, r.project.id)
            .await
            .map_err(storage_err)?;
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
    let capability = refresh_capability(d, project_id).await;

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

/// Ask the server what it can hold, and release anything it now can (T111).
///
/// Returns the capability as an opaque string, which is what a blocked row
/// records so a person can see *what* it is waiting for.
///
/// A server that answers without `capabilities` is a server from before the
/// field existed, and its silence is the answer: it can hold none of this. That
/// is why there is no probe endpoint and no negotiation — `GET /api/version`
/// already existed, and adding to it additively meant an old server needed no
/// change at all (D81).
async fn refresh_capability(d: &Daemon, project_id: Uuid) -> String {
    let Ok(client) = client(d).await else {
        // Offline. Whatever was last known still describes the server better
        // than nothing does.
        return repo::server_capability(&d.store, project_id)
            .await
            .ok()
            .flatten()
            .unwrap_or_else(|| UNKNOWN_CAPABILITY.to_string());
    };
    let Ok(body) = client.get("/api/version").await else {
        return repo::server_capability(&d.store, project_id)
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

    let previous = repo::server_capability(&d.store, project_id)
        .await
        .ok()
        .flatten();
    if previous.as_deref() == Some(capability.as_str()) {
        return capability;
    }

    // The capability changed. Anything the server can now hold goes back into
    // the ordinary queue with its original idempotency key, and the ordinary
    // drain — the one about to run — delivers it. Nothing here sends anything
    // itself, so there is no second delivery path to keep exactly-once.
    let releasable: Vec<OutboxEntityType> = ENTITY_CAPABILITIES
        .iter()
        // Every capability the type can wait on must be present. Releasing a
        // memory on `memory_subject_identity` alone would put an attested one
        // back in front of a server that still has no column for it.
        .filter(|(_, needs)| needs.iter().all(|need| names.iter().any(|n| n == need)))
        .map(|(entity, _)| *entity)
        .collect();
    match outbox::release_blocked(&d.store, project_id, &releasable).await {
        Ok(n) if n > 0 => tracing::info!(
            project = %project_id, released = n, capability = %capability,
            "the server gained a capability; retained work returns to the queue"
        ),
        Ok(_) => {}
        Err(e) => tracing::warn!(error = %e, "could not release retained work"),
    }
    let _ = repo::set_server_capability(&d.store, project_id, &capability).await;
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
    let since = repo::pull_cursor(&d.store, project_id)
        .await
        .map_err(storage_err)?;
    let path = match &since {
        Some(cursor) => format!(
            "/api/sync/changes?project_id={server_project_id}&since={}",
            urlencode(cursor)
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
        import_relation(d, project_id, r).await;
        count += 1;
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
    for c in body
        .get("criteria")
        .and_then(|v| v.as_array())
        .unwrap_or(&Vec::new())
    {
        if import_criterion(d, c).await {
            count += 1;
        }
    }
    for b in body
        .get("blockers")
        .and_then(|v| v.as_array())
        .unwrap_or(&Vec::new())
    {
        if import_blocker(d, b).await {
            count += 1;
        }
    }

    if let Some(cursor) = body.get("cursor").and_then(|c| c.as_str()) {
        repo::set_pull_cursor(&d.store, project_id, cursor)
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

/// Import a reconciliation decision.
///
/// `INSERT OR IGNORE` on the normalized primary key, then re-derive. This is the
/// correction research B2 found: today `import_memory` returns early when the
/// row exists, so a supersession decided on another machine never lands. The
/// *decision* is what travels, and deriving from it fixes the defect without
/// introducing row overwriting (D67, R5).
async fn import_relation(d: &Daemon, project_id: Uuid, value: &serde_json::Value) {
    let uuid = |k: &str| {
        value
            .get(k)
            .and_then(|v| v.as_str())
            .and_then(|s| Uuid::parse_str(s).ok())
    };
    let (Some(from), Some(to)) = (uuid("from_memory_id"), uuid("to_memory_id")) else {
        return;
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
        return;
    };

    // A relation whose memory has not arrived is held rather than dropped: the
    // foreign key would refuse it, and the next pull carries it again.
    if repo::memory(&d.store, from).await.is_err() || repo::memory(&d.store, to).await.is_err() {
        return;
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
}

/// Import one criterion that arrived from a peer.
async fn import_criterion(d: &Daemon, value: &serde_json::Value) -> bool {
    let uuid = |k: &str| {
        value
            .get(k)
            .and_then(|v| v.as_str())
            .and_then(|s| Uuid::parse_str(s).ok())
    };
    let (Some(id), Some(task_id)) = (uuid("id"), uuid("task_id")) else {
        return false;
    };
    // A criterion for a task that has not arrived is held, not invented.
    if repo::task(&d.store, task_id).await.is_err() {
        return false;
    }
    let str_of = |k: &str| value.get(k).and_then(|v| v.as_str()).unwrap_or_default();
    let (Ok(state), Ok(verification)) = (str_of("state").parse(), str_of("verification").parse())
    else {
        return false;
    };

    cairn_store::criteria::import_criterion(
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
    .is_ok()
}

/// Import one blocker that arrived from a peer.
async fn import_blocker(d: &Daemon, value: &serde_json::Value) -> bool {
    let uuid = |k: &str| {
        value
            .get(k)
            .and_then(|v| v.as_str())
            .and_then(|s| Uuid::parse_str(s).ok())
    };
    let (Some(id), Some(task_id)) = (uuid("id"), uuid("task_id")) else {
        return false;
    };
    if repo::task(&d.store, task_id).await.is_err() {
        return false;
    }
    let str_of = |k: &str| value.get(k).and_then(|v| v.as_str()).unwrap_or_default();
    let Ok(state) = str_of("state").parse() else {
        return false;
    };

    cairn_store::criteria::import_blocker(
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
    .is_ok()
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

    /// A linked project must report the link it has.
    ///
    /// This is the regression this release is named for: bare `cairn link`
    /// answered `linked: false` unconditionally, so a linked project was told
    /// it was not linked and pointed at `cairn link --create` — which would
    /// have made a second shared project for a repository that already had
    /// one — while `cairn status`, reading the same row, said the opposite.
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
