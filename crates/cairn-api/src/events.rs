//! Server-Sent Events (SSE) broker for real-time dashboard updates.
//!
//! The Cairn dashboard is a static Next.js export; pushing real-time state to it over WebSocket
//! would mean adding a second protocol. SSE is one-way (server → browser), native to every
//! browser via [`EventSource`], and auto-reconnects — exactly what the Overview/Audit pages
//! need to replace their 5 s polling.
//!
//! ## Event broker
//! [`EventBroker`] is a `Send + Sync` in-process pub/sub. Every state-mutating API handler (login,
//! logout, checkpoint, rollback, memory add/edit/delete, drift event) calls [`EventBroker::publish`]
//! after the operation succeeds. Subscribers (the `/api/events` SSE handler) get a
//! `tokio::sync::broadcast` receiver and stream events as they arrive.
//!
//! ## Replay (`Last-Event-ID`)
//! Each event has a monotonically increasing `id` (allocated by the store's audit backend — same
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
use std::{convert::Infallible, sync::Arc, time::Duration};
use tokio::sync::broadcast;

use crate::AppState;
use cairn_store::AuditRecord;

/// SSE event kinds surfaced to the dashboard. New kinds are additive; old ones stay forever.
#[allow(dead_code)] // Exposed for future sprint handlers; the constant itself is the wire contract.
pub const KIND_AUDIT: &str = "audit";
#[allow(dead_code)]
pub const KIND_STATS: &str = "stats";
#[allow(dead_code)]
pub const KIND_MEMORY: &str = "memory";
#[allow(dead_code)]
pub const KIND_CHECKPOINT: &str = "checkpoint";
#[allow(dead_code)]
pub const KIND_DRIFT: &str = "drift";

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
}

impl Default for EventBroker {
    fn default() -> Self {
        Self::new(1024)
    }
}

impl EventBroker {
    /// Build a broker with `capacity` slots for slow subscribers. Overflow is dropped (broadcast's
    /// "lagged" semantics — we accept dropped events on a slow client rather than block).
    pub fn new(capacity: usize) -> Self {
        let (tx, _rx) = broadcast::channel(capacity);
        Self { tx }
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
}

/// SSE query params — supports `?since=<event-id>` for replay on reconnect.
#[derive(Debug, Default, serde::Deserialize)]
pub struct EventsQuery {
    #[serde(default)]
    pub since: Option<String>,
}

/// `GET /api/events` — server-sent event stream.
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
fn backfill(
    state: &AppState,
    since: Option<&str>,
) -> std::result::Result<Vec<EventPayload>, cairn_core::Error> {
    const MAX_REPLAY: usize = 500;
    let mut records = state.store.recent_audit(MAX_REPLAY, since)?;
    records.reverse(); // oldest first so the replay reads in order
    Ok(records.iter().map(EventPayload::audit).collect())
}

/// Test-only wrapper for `backfill` — used by integration tests that need to assert on the
/// replay set without spinning up an HTTP server.
#[doc(hidden)]
#[allow(dead_code)]
pub fn test_backfill(
    state: &AppState,
    since: Option<&str>,
) -> std::result::Result<Vec<EventPayload>, cairn_core::Error> {
    backfill(state, since)
}

/// Convenience for handlers that just want to publish a stats-changed event without writing the
/// full payload inline.
#[allow(dead_code)]
pub fn publish_stats(broker: &EventBroker) {
    broker.publish(EventPayload {
        id: format!("stats-{}", Utc::now().timestamp_millis()),
        kind: KIND_STATS,
        ts: Utc::now().timestamp(),
        data: serde_json::json!({}),
    });
}

/// Publish a memory-related event (add/edit/delete/pin). `action` is "added" | "edited" |
/// "deleted" | "pinned".
#[allow(dead_code)]
pub fn publish_memory(broker: &EventBroker, action: &str, memory_id: &str) {
    broker.publish(EventPayload {
        id: format!("mem-{}-{}", action, memory_id),
        kind: KIND_MEMORY,
        ts: Utc::now().timestamp(),
        data: serde_json::json!({"action": action, "memory_id": memory_id}),
    });
}

/// Publish a checkpoint/rollback event.
#[allow(dead_code)]
pub fn publish_checkpoint(broker: &EventBroker, action: &str, id: &str, files: usize) {
    broker.publish(EventPayload {
        id: format!("cp-{}-{}", action, id),
        kind: KIND_CHECKPOINT,
        ts: Utc::now().timestamp(),
        data: serde_json::json!({"action": action, "id": id, "files": files}),
    });
}

/// Publish a drift event (verify flagged an edit as warn/danger).
#[allow(dead_code)]
pub fn publish_drift(broker: &EventBroker, path: &str, risk: &str) {
    broker.publish(EventPayload {
        id: format!("drift-{}", Utc::now().timestamp_millis()),
        kind: KIND_DRIFT,
        ts: Utc::now().timestamp(),
        data: serde_json::json!({"path": path, "risk": risk}),
    });
}

/// Wrap an [`Arc<EventBroker>`] in a closure-friendly helper for tests that want to assert on
/// delivery.
#[allow(dead_code)]
pub type BrokerRef = Arc<EventBroker>;
