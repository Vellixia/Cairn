//! Durable typed-spool delivery.

use crate::state::{Daemon, storage_err};
use cairn_core::wire::{WireError, codes};
use cairn_store::repo;
use std::time::Duration;
use uuid::Uuid;

const BATCH: i64 = 128;
const TICK: Duration = Duration::from_millis(500);
const MAX_BACKOFF: Duration = Duration::from_secs(30);

type Reply = Result<serde_json::Value, WireError>;

pub async fn run_worker(d: std::sync::Arc<Daemon>) {
    for result in [
        cairn_store::spool::release_event_claims(&d.store).await,
        cairn_store::spool::release_command_claims(&d.store).await,
    ] {
        if let Err(error) = result {
            tracing::debug!(%error, "could not release stale spool claim");
        }
    }
    let mut delay = TICK;
    loop {
        delay = if drain_typed_spools(&d).await {
            TICK
        } else {
            (delay * 2).min(MAX_BACKOFF)
        };
        tokio::time::sleep(delay).await;
    }
}

pub(crate) async fn drain_typed_spools(d: &Daemon) -> bool {
    let _ = crate::capture::collect_capture_drops(d).await;
    drain_events(d, BATCH).await.is_ok() && drain_commands(d, BATCH).await.is_ok()
}

pub(crate) struct Client {
    base: String,
    token: String,
    http: reqwest::Client,
}

pub(crate) async fn client(d: &Daemon) -> Result<Client, WireError> {
    let creds = d.server.read().await;
    Client::new(creds.url.as_deref(), creds.token.as_deref())
}

impl Client {
    fn new(url: Option<&str>, token: Option<&str>) -> Result<Self, WireError> {
        let base = url.ok_or_else(|| WireError::new(codes::NOT_LINKED, "no server configured"))?;
        let token = token.ok_or_else(|| WireError::new(codes::UNAUTHORIZED, "no API token"))?;
        Ok(Self {
            base: base.trim_end_matches('/').to_owned(),
            token: token.to_owned(),
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(20))
                .build()
                .map_err(|e| WireError::new(codes::SERVER_UNAVAILABLE, e.to_string()))?,
        })
    }

    pub(crate) async fn get(&self, path: &str) -> Reply {
        decode(
            self.http
                .get(format!("{}{path}", self.base))
                .bearer_auth(&self.token)
                .send()
                .await,
        )
        .await
    }

    pub(crate) async fn get_with_query(&self, path: &str, query: &[(String, String)]) -> Reply {
        let mut url = reqwest::Url::parse(&format!("{}{path}", self.base))
            .map_err(|error| WireError::new(codes::SERVER_UNAVAILABLE, error.to_string()))?;
        url.query_pairs_mut().extend_pairs(query);
        decode(self.http.get(url).bearer_auth(&self.token).send().await).await
    }

    pub(crate) async fn post(&self, path: &str, body: &serde_json::Value) -> Reply {
        decode(
            self.http
                .post(format!("{}{path}", self.base))
                .bearer_auth(&self.token)
                .json(body)
                .send()
                .await,
        )
        .await
    }

    async fn post_outcome(
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
            .map_err(unreachable)?;
        if response.status().is_success() {
            return Ok(ServerAnswer::Accepted);
        }
        let status = response.status();
        if status.is_server_error() {
            return Err(WireError::new(codes::SERVER_UNAVAILABLE, "server_error"));
        }
        let body: serde_json::Value = response.json().await.unwrap_or_default();
        let code = body
            .pointer("/error/code")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| format!("http_{}", status.as_u16()));
        Ok(ServerAnswer::Refused(code))
    }
}

async fn decode(response: Result<reqwest::Response, reqwest::Error>) -> Reply {
    let response = response.map_err(unreachable)?;
    let status = response.status();
    let body: serde_json::Value = response.json().await.unwrap_or_default();
    if status.is_success() {
        return Ok(body);
    }
    if status.is_server_error() {
        return Err(WireError::new(codes::SERVER_UNAVAILABLE, "server_error"));
    }
    Err(WireError::new(
        body.pointer("/error/code")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("server_rejected"),
        body.pointer("/error/message")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("server rejected request"),
    ))
}

fn unreachable(error: reqwest::Error) -> WireError {
    WireError::new(codes::SERVER_UNAVAILABLE, error.to_string())
}

struct Context {
    account: Uuid,
    peer: Uuid,
    client: Client,
}

async fn context(d: &Daemon) -> Result<Context, WireError> {
    let (account, client) = {
        let creds = d.server.read().await;
        let account = creds
            .account_id
            .ok_or_else(|| WireError::new(codes::UNAUTHORIZED, "server account unknown"))?;
        (
            account,
            Client::new(creds.url.as_deref(), creds.token.as_deref())?,
        )
    };
    // Typed operation endpoints own capability and authorization responses.
    // No `/api/version` preflight may leave a queue stuck behind an unrelated
    // 4xx. Fresh edge rows are unbound and use this stable local claim lane.
    let peer = Uuid::nil();
    Ok(Context {
        account,
        peer,
        client,
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Delivered,
    Refused,
    Deferred,
    Transient,
}

fn outcome(code: Option<&str>) -> Outcome {
    match code {
        Some(code)
            if codes::CAPABILITY_REFUSALS.contains(&code)
                || matches!(code, "contract_version_unsupported" | "unsupported_kind") =>
        {
            Outcome::Deferred
        }
        Some("server_error" | "storage_unavailable") | None => Outcome::Transient,
        Some(_) => Outcome::Refused,
    }
}

async fn settle_event(d: &Daemon, id: Uuid, state: Outcome, reason: &str) -> Result<(), WireError> {
    use cairn_store::spool;
    match state {
        Outcome::Delivered => spool::mark_event_delivered(&d.store, id).await,
        Outcome::Refused => spool::mark_event_refused(&d.store, id, reason).await,
        Outcome::Deferred => {
            spool::mark_event_deferred(&d.store, id, spool::DEFERRED_AWAITING_CAPABILITY).await
        }
        Outcome::Transient => spool::mark_event_failed(&d.store, id, reason).await,
    }
    .map_err(storage_err)
}

async fn settle_command(
    d: &Daemon,
    id: Uuid,
    state: Outcome,
    reason: &str,
) -> Result<(), WireError> {
    use cairn_store::spool;
    match state {
        Outcome::Delivered => spool::mark_command_delivered(&d.store, id).await,
        Outcome::Refused => spool::mark_command_refused(&d.store, id, reason).await,
        Outcome::Deferred => {
            spool::mark_command_deferred(&d.store, id, spool::DEFERRED_AWAITING_CAPABILITY).await
        }
        Outcome::Transient => spool::mark_command_failed(&d.store, id, reason).await,
    }
    .map_err(storage_err)
}

async fn drain_events(d: &Daemon, limit: i64) -> Result<(), WireError> {
    let _guard = d.sync_drain.lock().await;
    let context = context(d).await?;
    let rows = cairn_store::spool::claim_events(&d.store, context.account, context.peer, limit)
        .await
        .map_err(storage_err)?;
    if rows.is_empty() {
        return Ok(());
    }
    let mut sessions = Vec::new();
    for row in &rows {
        let project = repo::project(&d.store, row.project_id).await.map_err(storage_err)?;
        let Some(project_id) = project.server_project_id else { continue };
        let session = repo::session(&d.store, row.event.session_id).await.map_err(storage_err)?;
        sessions.push(serde_json::json!({
            "id": session.id,
            "project_id": project_id,
            "agent": session.agent,
            "branch": session.branch,
            "commit_sha": session.commit_sha,
        }));
    }
    let body = serde_json::json!({
        "contract_version": cairn_core::event::CONTRACT_VERSION,
        "sessions": sessions,
        "events": rows.iter().map(|row| &row.event).collect::<Vec<_>>(),
    });
    let response = match context.client.post("/api/events/batch", &body).await {
        Ok(response) => response,
        Err(error) => {
            let state = outcome(Some(&error.code));
            for row in rows {
                settle_event(d, row.event_id, state, &error.code).await?;
            }
            return Err(error);
        }
    };
    let results = response
        .get("results")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    for row in rows {
        let found = results.iter().find(|value| {
            value.get("event_id").and_then(serde_json::Value::as_str)
                == Some(&row.event_id.to_string())
        });
        let reason = found
            .and_then(|value| value.get("reason"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("no result");
        let state = match found
            .and_then(|value| value.get("status"))
            .and_then(serde_json::Value::as_str)
        {
            Some("accepted" | "duplicate") => Outcome::Delivered,
            Some("rejected") => outcome(Some(reason)),
            _ => Outcome::Transient,
        };
        settle_event(d, row.event_id, state, reason).await?;
    }
    Ok(())
}

async fn drain_commands(d: &Daemon, limit: i64) -> Result<(), WireError> {
    use cairn_store::spool;
    let _guard = d.sync_drain.lock().await;
    let context = context(d).await?;
    let rows = spool::claim_commands(&d.store, context.account, context.peer, limit)
        .await
        .map_err(storage_err)?;
    for row in rows {
        let project_id = match row.project_id {
            None => None,
            Some(local) => match repo::project(&d.store, local).await.map_err(storage_err)? {
                project if project.linked => project.server_project_id,
                _ => None,
            },
        };
        if row.project_id.is_some() && project_id.is_none() {
            settle_command(d, row.command_id, Outcome::Deferred, "project_not_linked").await?;
            continue;
        }
        let target_id = row
            .payload
            .get("target_id")
            .and_then(serde_json::Value::as_str)
            .and_then(|id| Uuid::parse_str(id).ok());
        let body = serde_json::json!({
            "command_id": row.command_id, "kind": row.kind.as_str(), "project_id": project_id,
            "target_id": target_id, "session_id": row.session_id, "payload": row.payload,
        });
        let (state, reason) = match context.client.post_outcome("/api/commands", &body).await {
            Ok(ServerAnswer::Accepted) => (Outcome::Delivered, "accepted".to_owned()),
            Ok(ServerAnswer::Refused(code)) => (outcome(Some(&code)), code),
            Err(_) => (Outcome::Transient, "transport".to_owned()),
        };
        settle_command(d, row.command_id, state, &reason).await?;
        if state == Outcome::Transient {
            break;
        }
    }
    Ok(())
}

enum ServerAnswer {
    Accepted,
    Refused(String),
}
