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

type Reply = Result<serde_json::Value, WireError>;

/// Drain the outbox automatically, forever (FR-056, D9).
///
/// `cairn sync now` stays available as an explicit trigger, but it is not the
/// only one: work queued while the server was unreachable is delivered when it
/// comes back, with no manual step. Transient failures back off; permanent
/// rejections are already recorded as `failed` by `drain` and are not retried.
pub async fn run_worker(daemon: std::sync::Arc<Daemon>) {
    let mut backoff = BACKOFF_MIN;
    loop {
        tokio::time::sleep(WORKER_TICK).await;

        let projects = match repo::list_projects(&daemon.store).await {
            Ok(p) => p,
            Err(e) => {
                tracing::debug!(error = %e, "sync worker could not list projects");
                continue;
            }
        };

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
                continue;
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

/// Store the API token 0600 and remember the server URL (D10).
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
        (None, false) => {
            // Discovery hint only. The user picks (D14).
            let remote = r.project.repository_remote.clone().unwrap_or_default();
            let candidates = c
                .get(&format!(
                    "/api/projects/lookup?remote={}",
                    urlencode(&remote)
                ))
                .await
                .unwrap_or_else(|_| json!({ "projects": [] }));
            return Ok(json!({
                "linked": false,
                "candidates": candidates.get("projects").cloned().unwrap_or(json!([])),
                "hint": "run `cairn link --create` for a new shared project, \
                         or `cairn link --project <id>` to join one",
            }));
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
        enqueue_one(
            d,
            policy,
            project.id,
            OutboxEntityType::Memory,
            m.id,
            outbox::memory_payload(&m),
        )
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
    };
    Ok(serde_json::to_value(payload).unwrap_or(json!({})))
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

    let (mut applied, mut duplicate, mut rejected) = (0, 0, 0);
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
                    // Permanent. Surfaced with its identity, not retried forever.
                    let msg = result
                        .and_then(|r| r.error.as_ref())
                        .map(|e| e.message.clone())
                        .unwrap_or_else(|| "rejected".into());
                    outbox::mark_failed(&d.store, *row_id, &msg)
                        .await
                        .map_err(storage_err)?;
                    rejected += 1;
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
    Ok((applied, duplicate, rejected))
}

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

    if repo::memory(&d.store, id).await.is_ok() {
        return Ok(());
    }
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

    sqlx::query(
        "INSERT OR IGNORE INTO memories
            (id, project_id, type, scope, scope_key, content, state, superseded_by_id,
             origin_session_id, local_only, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'active', NULL, ?7, 0, ?8, ?8)",
    )
    .bind(id.to_string())
    .bind(project_id.to_string())
    .bind(kind.as_str())
    .bind(scope.as_str())
    .bind(scope_key)
    .bind(content)
    .bind(origin.to_string())
    .bind(chrono::Utc::now().to_rfc3339())
    .execute(d.store.pool())
    .await
    .map_err(|e| WireError::new(codes::STORAGE_UNAVAILABLE, e.to_string()))?;
    Ok(())
}
