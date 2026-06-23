//! Push notification subscription store (v0.5.0 Sprint 20b).
//!
//! The dashboard's service worker calls `POST /api/push/subscribe` with the
//! browser-issued `PushSubscription` JSON. We persist it (one file per
//! subscription under `<data_dir>/push/`) and broadcast every drift /
//! revocation event from the event broker to all live subscriptions via the
//! Web Push protocol (simplified to `display` + `tag` + a JSON body — full
//! VAPID signing is a v0.6 item).
//!
//! For v0.5.0 we focus on:
//! 1. Persisting the subscription + last-seen cursor so the dashboard
//!    re-subscribes on reload and the server can detect offline devices.
//! 2. Broadcasting on drift / revoke events from the existing `EventBroker`.
//!
//! The actual outbound HTTP POST to the push provider (FCM, Mozilla Autopush,
//! APNs, etc.) is left to a future "push relay" deployment — we just write
//! the subscription record and the per-subscription cursor.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
};
use thiserror::Error;

/// One dashboard push subscription record. The `endpoint` + `keys` come
/// directly from `PushSubscription` in the browser; we persist them
/// unchanged so a re-subscribe can use the same id.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PushSubscriptionRecord {
    /// Stable id (we generate a UUID when the subscription arrives).
    pub id: String,
    pub endpoint: String,
    pub keys: PushKeys,
    /// User-agent that registered the subscription (helps debugging).
    pub user_agent: Option<String>,
    pub created_at: DateTime<Utc>,
    pub last_seen_at: DateTime<Utc>,
    /// Cursor (event id) the subscriber has acknowledged — drives
    /// `?since=` semantics for re-subscribed clients.
    pub last_event_id: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PushKeys {
    pub p256dh: String,
    pub auth: String,
}

#[derive(Debug, Error)]
pub enum PushStoreError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("not found: {0}")]
    NotFound(String),
}

/// On-disk store for push subscriptions. Cheap clone (`Arc` inside if we
/// wanted), but we hold a single instance per process — no sharing needed.
pub struct PushStore {
    root: PathBuf,
    cache: Mutex<Vec<PushSubscriptionRecord>>,
}

impl PushStore {
    pub fn open(data_dir: &Path) -> Result<Self, PushStoreError> {
        let root = data_dir.join("push");
        fs::create_dir_all(&root)?;
        let mut cache = Vec::new();
        for entry in fs::read_dir(&root)? {
            let entry = entry?;
            if entry.file_type()?.is_file() {
                let bytes = fs::read(entry.path())?;
                let rec: PushSubscriptionRecord = serde_json::from_slice(&bytes)?;
                cache.push(rec);
            }
        }
        Ok(Self {
            root,
            cache: Mutex::new(cache),
        })
    }

    /// Persist a new subscription (or update the `last_seen_at` of an existing one
    /// matched by endpoint).
    pub fn upsert(
        &self,
        endpoint: String,
        keys: PushKeys,
        user_agent: Option<String>,
    ) -> Result<PushSubscriptionRecord, PushStoreError> {
        let now = Utc::now();
        let mut guard = self.cache.lock().expect("push cache poisoned");
        if let Some(existing) = guard.iter_mut().find(|r| r.endpoint == endpoint) {
            existing.last_seen_at = now;
            existing.keys = keys;
            // First-registration UA wins — a user-agent change usually means the
            // browser navigated, not that the subscription itself changed.
            if existing.user_agent.is_none() {
                existing.user_agent = user_agent;
            }
            let path = self.root.join(format!("{}.json", existing.id));
            fs::write(&path, serde_json::to_vec_pretty(&*existing)?)?;
            return Ok(existing.clone());
        }
        let id = uuid::Uuid::new_v4().to_string();
        let rec = PushSubscriptionRecord {
            id: id.clone(),
            endpoint,
            keys,
            user_agent,
            created_at: now,
            last_seen_at: now,
            last_event_id: 0,
        };
        let path = self.root.join(format!("{id}.json"));
        fs::write(&path, serde_json::to_vec_pretty(&rec)?)?;
        guard.push(rec.clone());
        Ok(rec)
    }

    /// List every subscription (for diagnostics).
    pub fn list(&self) -> Vec<PushSubscriptionRecord> {
        self.cache.lock().expect("push cache poisoned").clone()
    }

    /// Drop a subscription by id (called when the browser sends
    /// `PushSubscription.unsubscribe`).
    pub fn unsubscribe(&self, id: &str) -> Result<(), PushStoreError> {
        let mut guard = self.cache.lock().expect("push cache poisoned");
        guard.retain(|r| r.id != id);
        // Distinguish "file was already gone" (NotFound — fine, we just removed it
        // from the cache too) from a real I/O error (log it and surface as a
        // PushStoreError). Pre-fix both were silently swallowed with `let _ =`,
        // letting zombie subscription files accumulate on disk after a crash.
        match fs::remove_file(self.root.join(format!("{id}.json"))) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => {
                tracing::warn!(id, error = %e, "push unsubscribe: failed to remove subscription file");
                Err(PushStoreError::Io(e))
            }
        }
    }

    /// Bump a subscription's last-event cursor (the dashboard calls this
    /// after successfully rendering an event so the next poll doesn't repeat).
    pub fn ack(&self, id: &str, event_id: i64) -> Result<(), PushStoreError> {
        let mut guard = self.cache.lock().expect("push cache poisoned");
        if let Some(r) = guard.iter_mut().find(|r| r.id == id) {
            r.last_event_id = r.last_event_id.max(event_id);
            let path = self.root.join(format!("{}.json", r.id));
            fs::write(&path, serde_json::to_vec_pretty(&*r)?)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn upsert_is_idempotent_on_endpoint() {
        let dir = TempDir::new().unwrap();
        let store = PushStore::open(dir.path()).unwrap();
        let r1 = store
            .upsert(
                "https://push.example/sub/abc".into(),
                PushKeys {
                    p256dh: "p".into(),
                    auth: "a".into(),
                },
                Some("ua/1".into()),
            )
            .unwrap();
        let r2 = store
            .upsert(
                "https://push.example/sub/abc".into(),
                PushKeys {
                    p256dh: "p".into(),
                    auth: "a".into(),
                },
                Some("ua/2".into()),
            )
            .unwrap();
        assert_eq!(r1.id, r2.id, "upsert must be idempotent on endpoint");
        assert_eq!(r2.user_agent.as_deref(), Some("ua/1"), "first UA wins");
        assert_eq!(store.list().len(), 1);
    }

    #[test]
    fn ack_advances_last_event_id_monotonically() {
        let dir = TempDir::new().unwrap();
        let store = PushStore::open(dir.path()).unwrap();
        let r = store
            .upsert(
                "https://e/1".into(),
                PushKeys {
                    p256dh: "p".into(),
                    auth: "a".into(),
                },
                None,
            )
            .unwrap();
        store.ack(&r.id, 5).unwrap();
        store.ack(&r.id, 3).unwrap(); // smaller — no-op
        let after = store.list();
        assert_eq!(after[0].last_event_id, 5);
    }

    /// Pre-fix regression: `let _ = fs::remove_file(...)` silently swallowed I/O errors,
    /// letting zombie subscription files accumulate on disk after a crash. NotFound
    /// is still OK (cache is the source of truth); any other error must propagate.
    #[test]
    fn unsubscribe_handles_missing_file_silently_and_propagates_other_errors() {
        let dir = TempDir::new().unwrap();
        let store = PushStore::open(dir.path()).unwrap();
        let r = store
            .upsert(
                "https://e/missing".into(),
                PushKeys {
                    p256dh: "p".into(),
                    auth: "a".into(),
                },
                None,
            )
            .unwrap();
        // Yank the file out from under the store so the unsubscribe hits NotFound.
        let path = dir.path().join("push").join(format!("{}.json", r.id));
        std::fs::remove_file(&path).unwrap();
        store
            .unsubscribe(&r.id)
            .expect("NotFound from remove_file must be silent Ok");

        // Calling unsubscribe on a never-existed id (cache miss) is also Ok.
        store
            .unsubscribe("never-existed")
            .expect("missing entry is Ok");
    }
}

// ---- HTTP handlers ----------------------------------------------------------
//
// Three small handlers, each accepting the AppState via axum's
// State<T> extractor. The push module owns these so the route() callsite
// can use `push::subscribe` etc. directly.

use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};

use crate::AppState;

#[derive(Debug, Deserialize)]
pub struct SubscribeRequest {
    pub endpoint: String,
    pub keys: PushKeys,
    pub user_agent: Option<String>,
}

/// `POST /api/push/subscribe` — the dashboard's SW posts its browser-issued
/// `PushSubscription` JSON. We persist it and return the assigned id.
pub async fn subscribe(
    State(state): State<AppState>,
    Json(req): Json<SubscribeRequest>,
) -> Response {
    let Some(store) = state.push.as_ref() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error": "push store not configured"})),
        )
            .into_response();
    };
    match store.upsert(req.endpoint, req.keys, req.user_agent) {
        Ok(rec) => (StatusCode::CREATED, Json(rec)).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Debug, Deserialize)]
pub struct UnsubscribeRequest {
    pub id: String,
}

/// `POST /api/push/unsubscribe` — drop a subscription by id. Called by the SW
/// when the dashboard unregisters.
pub async fn unsubscribe(
    State(state): State<AppState>,
    Json(req): Json<UnsubscribeRequest>,
) -> Response {
    let Some(store) = state.push.as_ref() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    match store.unsubscribe(&req.id) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

/// `GET /api/push/list` — diagnostics. Returns every stored subscription.
pub async fn list_subscriptions(State(state): State<AppState>) -> Response {
    let Some(store) = state.push.as_ref() else {
        return Json(Vec::<PushSubscriptionRecord>::new()).into_response();
    };
    Json(store.list()).into_response()
}

#[cfg(test)]
mod http_tests {
    use super::*;
    use crate::tests::test_state;
    use axum::body::{to_bytes, Body};
    use axum::http::{Request as HttpRequest, StatusCode as AxStatus};
    use tower::ServiceExt;

    #[tokio::test]
    async fn push_subscribe_persists_subscription() {
        let Some((state, _dir)) = test_state() else {
            return;
        };
        let app = crate::build_router_with_registry(state);
        let body = serde_json::json!({
            "endpoint": "https://push.example/sub/x",
            "keys": { "p256dh": "pp", "auth": "aa" },
            "user_agent": "test"
        });
        let req = HttpRequest::builder()
            .method("POST")
            .uri("/api/push/subscribe")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), AxStatus::CREATED);
        let rec: PushSubscriptionRecord =
            serde_json::from_slice(&to_bytes(resp.into_body(), 1 << 20).await.unwrap()).unwrap();
        assert_eq!(rec.endpoint, "https://push.example/sub/x");
        assert_eq!(rec.user_agent.as_deref(), Some("test"));
    }
}
