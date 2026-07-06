//! Server-Sent Events (SSE) broker for real-time dashboard updates.
//!
//! The Cairn dashboard is a static Next.js export; pushing real-time state to it over WebSocket
//! would mean adding a second protocol. SSE is one-way (server -> browser), native to every
//! browser via [`EventSource`], and auto-reconnects - exactly what the Overview/Audit pages
//! need to replace their 5 s polling.
//!
//! ## Event broker
//! [`EventBroker`] is a `Send + Sync` in-process pub/sub. Every state-mutating API handler (login,
//! logout, checkpoint, rollback, memory add/edit/delete, drift event) calls [`EventBroker::publish`]
//! after the operation succeeds. Subscribers (the `/api/events` SSE handler) get a
//! `tokio::sync::broadcast` receiver and stream events as they arrive.
//!
//! ## Replay (`Last-Event-ID`)
//! Each event has a monotonically increasing `id` (allocated by the store's audit backend - same
//! source the audit log uses, so audit + live are consistent). On reconnect, the browser sends
//! `Last-Event-ID: <id>` and we replay anything newer from durable storage before resuming the
//! live stream.

use axum::{
    extract::State,
    http::{HeaderValue, StatusCode},
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse,
    },
};
use chrono::Utc;
use futures_core::Stream;
use serde::Serialize;
use std::{
    convert::Infallible,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::sync::broadcast;

use crate::AppState;
use cairn_store::AuditRecord;

/// SSE event kinds surfaced to the dashboard. New kinds are additive; old
/// ones stay forever so existing clients don't break.
pub const KIND_AUDIT: &str = "audit";
pub const KIND_MEMORY: &str = "memory";
pub const KIND_DRIFT: &str = "drift";
/// v0.8.0 Sprint 10: Projects/Documents/Automation pages went live via polling because nothing
/// published these three kinds - added so the whole dashboard is SSE-driven, not just
/// memory/drift/audit.
pub const KIND_PROJECT: &str = "project";
pub const KIND_DOCUMENT: &str = "document";
pub const KIND_CRON: &str = "cron";

/// Payload of an event published to subscribers. `id` is a unique, monotonic string suitable for
/// `id:` in SSE and `Last-Event-ID` on reconnect.
#[derive(Debug, Clone, Serialize)]
pub struct EventPayload {
    pub id: String,
    pub kind: &'static str,
    pub ts: i64,
    /// Free-form event-specific data, serialized as JSON for the browser.
    pub data: serde_json::Value,
}

impl EventPayload {
    fn audit(rec: &AuditRecord) -> Self {
        Self {
            id: rec.id.to_string(),
            kind: KIND_AUDIT,
            ts: rec.ts,
            data: serde_json::json!({
                "kind": rec.kind,
                "actor": rec.actor,
                "detail": rec.detail,
            }),
        }
    }
}

/// In-process pub/sub broker. Cheap to clone (the inner state is behind an `Arc`/`Mutex`).
#[derive(Clone)]
pub struct EventBroker {
    tx: broadcast::Sender<EventPayload>,
    /// Monotonic counter used to mint SSE ids for synthetic events (`stats-`, `drift-`).
    /// Seeded from the durable audit-log high-water mark at startup so a freshly published
    /// `stats-` event never collides with an audit-log id replayed to a reconnecting client.
    /// Pre-fix the counter was `Utc::now().timestamp_millis()` and two publishes in the
    /// same millisecond produced identical ids, breaking the SSE `Last-Event-ID` contract.
    next_synth_id: Arc<AtomicU64>,
}

impl Default for EventBroker {
    fn default() -> Self {
        Self::new(1024, 0)
    }
}

impl EventBroker {
    /// Build a broker with `capacity` slots for slow subscribers. Overflow is dropped (broadcast's
    /// "lagged" semantics - we accept dropped events on a slow client rather than block).
    /// `seed` is the initial value of the synthetic-event counter - pass `max_audit_event_id`
    /// from the durable audit log so live and replayed ids share the same namespace.
    pub fn new(capacity: usize, seed: u64) -> Self {
        let (tx, _rx) = broadcast::channel(capacity);
        Self {
            tx,
            next_synth_id: Arc::new(AtomicU64::new(seed)),
        }
    }

    /// Build a broker with no seed (synthetic ids start at 0). Mostly useful for tests.
    pub fn with_capacity(capacity: usize) -> Self {
        Self::new(capacity, 0)
    }

    /// Publish an event to all current subscribers. Returns the number of receivers that saw it.
    pub fn publish(&self, payload: EventPayload) -> usize {
        // `send` returns Err only when there are no receivers; treat that as 0 deliveries.
        self.tx.send(payload).unwrap_or(0)
    }

    /// Subscribe to the live stream. The receiver is cheaply cloneable for fan-out within a
    /// single SSE handler.
    pub fn subscribe(&self) -> broadcast::Receiver<EventPayload> {
        self.tx.subscribe()
    }

    /// Number of active subscribers (for tests + diagnostics).
    pub fn receiver_count(&self) -> usize {
        self.tx.receiver_count()
    }

    /// Mint the next monotonic synthetic-id suffix. Used by `publish_drift`
    /// and exposed to tests.
    pub(crate) fn next_synth_id(&self) -> u64 {
        self.next_synth_id.fetch_add(1, Ordering::Relaxed)
    }
}

/// SSE query params - supports `?since=<event-id>` for replay on reconnect.
#[derive(Debug, Default, serde::Deserialize)]
pub struct EventsQuery {
    #[serde(default)]
    pub since: Option<String>,
}

/// `GET /api/events` - server-sent event stream.
///
/// On connect, replays any audit events with id greater than `?since=<id>` (or `Last-Event-ID`)
/// from durable storage, then streams live events as they're published. Sends a 30 s heartbeat
/// so intermediaries (proxies, browser) don't drop the connection on idle.
pub async fn events(
    State(s): State<super::AppState>,
    axum::extract::Query(q): axum::extract::Query<EventsQuery>,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    // Backfill from durable storage when the client supplies a cursor. SSE spec puts the cursor
    // in the `Last-Event-ID` request header (axum has no constant for it, so we use a static).
    let since = q.since.clone().or_else(|| {
        headers
            .get("last-event-id")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string())
    });

    let history: Vec<EventPayload> = match backfill(&s, since.as_deref()) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("sse: audit backfill failed: {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                axum::Json(serde_json::json!({"error": e.to_string()})),
            )
                .into_response();
        }
    };

    let rx = s.events.subscribe();
    let stream = broadcast_stream(rx, history);

    let mut resp = Sse::new(stream)
        .keep_alive(KeepAlive::new().interval(Duration::from_secs(30)))
        .into_response();
    // Disable proxy buffering (nginx defaults to buffering SSE responses, which defeats the
    // real-time guarantee).
    resp.headers_mut()
        .insert("x-accel-buffering", HeaderValue::from_static("no"));
    resp
}

/// Build the SSE stream: first replay `history`, then forward live events until the client
/// disconnects (the receiver returns an error when the sender is dropped, which closes the stream).
fn broadcast_stream(
    rx: broadcast::Receiver<EventPayload>,
    history: Vec<EventPayload>,
) -> impl Stream<Item = Result<Event, Infallible>> {
    // A small adapter that drains a Vec, then pumps a broadcast receiver. The receiver is moved
    // into the closure so each subscriber gets its own.
    let history_iter = history.into_iter().peekable();
    let mut rx = rx;
    async_stream::stream! {
        // Replay first.
        for ev in history_iter {
            yield Ok(to_sse_event(&ev));
        }
        // Then live.
        loop {
            match rx.recv().await {
                Ok(ev) => yield Ok(to_sse_event(&ev)),
                Err(broadcast::error::RecvError::Lagged(_)) => continue, // drop & keep up
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    }
}

/// Translate an [`EventPayload`] into an axum SSE [`Event`]. The `id` is set so the browser
/// remembers it for `Last-Event-ID` on the next reconnect. `ts` is packed into the JSON data
/// (SSE has no standard timestamp field).
fn to_sse_event(p: &EventPayload) -> Event {
    let mut data = p.data.clone();
    if let Some(obj) = data.as_object_mut() {
        obj.insert("ts".into(), serde_json::json!(p.ts));
    }
    Event::default()
        .id(p.id.clone())
        .event(p.kind)
        .data(serde_json::to_string(&data).unwrap_or_default())
}

/// Read audit events with id greater than `since` (if given), up to `MAX_REPLAY`. Newest first
/// in the result so the SSE stream replays in chronological order from the client's POV.
#[doc(hidden)] // exposed only for integration tests in `lib.rs`
pub fn backfill(
    state: &AppState,
    since: Option<&str>,
) -> std::result::Result<Vec<EventPayload>, cairn_core::Error> {
    const MAX_REPLAY: usize = 500;
    let mut records = state.store.recent_audit(MAX_REPLAY, since)?;
    records.reverse(); // oldest first so the replay reads in order
    Ok(records.iter().map(EventPayload::audit).collect())
}
/// Publish a memory-related event (add/edit/delete/pin). `action` is "added" | "edited" |
/// "deleted" | "pinned".
pub fn publish_memory(broker: &EventBroker, action: &str, memory_id: &str) {
    broker.publish(EventPayload {
        id: format!("mem-{}-{}", action, memory_id),
        kind: KIND_MEMORY,
        ts: Utc::now().timestamp(),
        data: serde_json::json!({"action": action, "memory_id": memory_id}),
    });
}

/// Publish a drift event (verify flagged an edit as warn/danger).
pub fn publish_drift(broker: &EventBroker, path: &str, risk: &str) {
    broker.publish(EventPayload {
        id: format!("drift-{}", broker.next_synth_id()),
        kind: KIND_DRIFT,
        ts: Utc::now().timestamp(),
        data: serde_json::json!({"path": path, "risk": risk}),
    });
}

/// Publish a durable audit-log entry as a live event. `id` is the id already minted by
/// `Store::append_audit` (not a synthetic one) so a client whose last-seen event was an audit
/// entry can still resume replay correctly via `Last-Event-ID` - `backfill` reads from the same
/// durable audit log this id came from.
pub fn publish_audit(
    broker: &EventBroker,
    id: &str,
    ts: i64,
    kind: &str,
    actor: &str,
    detail: &str,
) {
    broker.publish(EventPayload {
        id: id.to_string(),
        kind: KIND_AUDIT,
        ts,
        data: serde_json::json!({
            "kind": kind,
            "actor": actor,
            "detail": detail,
        }),
    });
}

/// Publish a project-registry event (create or `last_active` touch via `/api/projects/upsert`).
pub fn publish_project(broker: &EventBroker, project_id: &str) {
    broker.publish(EventPayload {
        id: format!("project-{}-{}", project_id, broker.next_synth_id()),
        kind: KIND_PROJECT,
        ts: Utc::now().timestamp(),
        data: serde_json::json!({"project_id": project_id}),
    });
}

/// Publish a document event. `action` is "ingested" | "deleted".
pub fn publish_document(broker: &EventBroker, action: &str, document_id: &str) {
    broker.publish(EventPayload {
        id: format!("doc-{}-{}-{}", action, document_id, broker.next_synth_id()),
        kind: KIND_DOCUMENT,
        ts: Utc::now().timestamp(),
        data: serde_json::json!({"action": action, "document_id": document_id}),
    });
}

/// Publish a cron-run-completed event. Fired from [`crate::cron::run_job_now`] so a manual
/// `POST /api/cron/run/:job` and a scheduled tick both show up live, matching how they already
/// share one history.
pub fn publish_cron(broker: &EventBroker, job: &str, outcome: &str) {
    broker.publish(EventPayload {
        id: format!("cron-{}-{}", job, broker.next_synth_id()),
        kind: KIND_CRON,
        ts: Utc::now().timestamp(),
        data: serde_json::json!({"job": job, "outcome": outcome}),
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pre-fix regression: synthetic event ids minted from `timestamp_millis()`
    /// collided when two publishes landed in the same millisecond, breaking
    /// the SSE `Last-Event-ID` contract. The fix moved id minting to an
    /// `AtomicU64` counter (`next_synth_id`).
    #[test]
    fn synthetic_event_ids_are_monotonic_under_burst() {
        let broker = EventBroker::with_capacity(8);
        // Use Option so we can distinguish "first call returns 0" from "monotonic violation".
        let mut prev: Option<u64> = None;
        for _ in 0..100 {
            let id = broker.next_synth_id();
            match prev {
                None => assert_eq!(id, 0, "first id must be the seed value"),
                Some(p) => assert!(id > p, "ids must strictly increase under burst"),
            }
            prev = Some(id);
        }
    }

    #[test]
    fn publish_drift_uses_counter_prefix() {
        let broker = EventBroker::with_capacity(4);
        let mut rx = broker.subscribe();
        publish_drift(&broker, "/x.rs", "warn");
        let ev = rx.try_recv().unwrap();
        assert!(ev.id.starts_with("drift-"), "got id={}", ev.id);
        let n: u64 = ev.id.trim_start_matches("drift-").parse().unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn seeded_counter_starts_above_audit_high_water() {
        let broker = EventBroker::new(8, 42);
        assert_eq!(broker.next_synth_id(), 42);
        assert_eq!(broker.next_synth_id(), 43);
    }
}
