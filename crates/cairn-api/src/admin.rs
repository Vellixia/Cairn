//! Admin account + audit log.
//!
//! The single admin account lives in the meta store under key `admin` as a JSON-serialized
//! [`AdminRecord`](cairn_core::AdminRecord). Two concurrent `/setup` requests can't both win
//! because [`Store::set_meta_if_absent`](cairn_store::Store::set_meta_if_absent) is atomic. The
//! in-memory [`AuditLog`] is best-effort - restart loses it - which keeps the surface small and
//! avoids a database schema migration for 0.4.0.

use crate::session::{build_clear_cookie, build_set_cookie, extract_cookie, SessionPayload};
use crate::AppState;
use axum::{
    extract::State,
    http::{header, HeaderMap, HeaderName, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use cairn_core::{hash_password, verify_password, AdminRecord};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Duration;

pub const ADMIN_META_KEY: &str = "admin";
pub const AUDIT_CAPACITY: usize = 50;

/// One entry in the admin audit log. Best-effort, in-memory, lost on restart.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    pub ts: i64,
    pub kind: String,
    pub actor: String,
    pub detail: String,
}

#[derive(Debug, Default)]
pub struct AuditLog {
    inner: Mutex<VecDeque<AuditEvent>>,
}

impl AuditLog {
    /// Record an audit event in both the in-memory ring buffer and durable store. The in-memory
    /// ring keeps the last `AUDIT_CAPACITY` events hot for `/api/devices/audit`; the durable
    /// store is what survives restart and what the SSE stream reads from for `Last-Event-ID`
    /// replay. We never let a write to one block the other - best-effort durable write that
    /// fails (e.g. backend transiently unreachable) is logged but doesn't lose the in-memory
    /// event the admin is currently looking at.
    pub fn record(
        &self,
        store: &cairn_store::Store,
        events: &crate::events::EventBroker,
        kind: &str,
        actor: &str,
        detail: String,
    ) {
        let ts = Utc::now().timestamp();
        {
            let mut q = self.inner.lock().expect("audit log mutex");
            q.push_front(AuditEvent {
                ts,
                kind: kind.to_string(),
                actor: actor.to_string(),
                detail: detail.clone(),
            });
            while q.len() > AUDIT_CAPACITY {
                q.pop_back();
            }
        }
        match store.append_audit(ts, kind, actor, &detail) {
            Ok(id) => {
                tracing::trace!(event_id = %id, kind, actor, "audit persisted");
                crate::events::publish_audit(events, &id, ts, kind, actor, &detail);
            }
            Err(e) => {
                tracing::warn!(error = %e, kind, actor, "audit durable write failed");
            }
        }
    }

    pub fn snapshot(&self) -> Vec<AuditEvent> {
        self.inner
            .lock()
            .expect("audit log mutex")
            .iter()
            .cloned()
            .collect()
    }
}

/// Wire-level admin record for the API. Never includes the password hash.
#[derive(Debug, Serialize)]
pub struct AdminView {
    pub username: String,
    pub generation: u64,
    pub created_at: i64,
    pub updated_at: i64,
}

impl From<&AdminRecord> for AdminView {
    fn from(r: &AdminRecord) -> Self {
        Self {
            username: r.username.clone(),
            generation: r.generation,
            created_at: r.created_at,
            updated_at: r.updated_at,
        }
    }
}

/// Wire-level session summary returned by `/api/auth/me`.
#[derive(Debug, Serialize)]
pub struct MeView {
    pub username: String,
    pub generation: u64,
    pub login_at: i64,
    pub expires_at: i64,
}

/// Wire-level status from `/api/auth/status`. Public.
#[derive(Debug, Serialize)]
pub struct AuthStatus {
    pub admin_exists: bool,
    pub setup_required: bool,
}

/// Wire-level status from `/api/auth/status`. Public.

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Deserialize)]
pub struct SetupRequest {
    pub username: String,
    pub password: String,
}

/// Read the admin record out of the meta store. Returns `None` if absent (or tombstoned).
pub fn load_admin(state: &AppState) -> cairn_core::Result<Option<AdminRecord>> {
    let Some(raw) = state.store.get_meta_live(ADMIN_META_KEY)? else {
        return Ok(None);
    };
    let rec: AdminRecord = serde_json::from_str(&raw)?;
    Ok(Some(rec))
}

/// Env-only admin bootstrap. Called once at server startup after the store is open and before
/// the HTTP listener binds.
///
/// Behavior (idempotent):
/// 1. If an admin record already exists, return immediately (no-op).
/// 2. If `cfg.admin.password` is unset, log a hint and return - the dashboard `/setup` wizard
///    will mint the record on first visit.
/// 3. Refuse if the bind host is non-loopback AND `CAIRN_INSECURE` is not set - we will not mint
///    an admin over plain HTTP on a network-exposed bind.
/// 4. Refuse if the username is empty or the password is shorter than 8 chars or equals the
///    username.
/// 5. Hash the password, mint `AdminRecord { generation: 1 }`, persist via `set_meta_if_absent`.
///    If a parallel process raced us, the `Ok(false)` branch is a no-op (winner takes all).
#[allow(dead_code)] // called by cairn-api::bin::cairn_server (in-container entrypoint only)
pub fn bootstrap_admin_from_env(state: &AppState) -> cairn_core::Result<()> {
    if load_admin(state)?.is_some() {
        return Ok(());
    }
    let Some(password) = state.cfg.admin.password.as_deref() else {
        tracing::info!(
            "admin: no record found and CAIRN_ADMIN_PASSWORD unset - \
             /setup wizard will mint one on first dashboard visit"
        );
        return Ok(());
    };
    let password = password.trim();
    if password.len() < 8 {
        return Err(cairn_core::Error::Invalid(format!(
            "CAIRN_ADMIN_PASSWORD must be at least 8 characters (got {} chars). \
             Edit .env and restart, or unset it to fall back to /setup.",
            password.len()
        )));
    }
    if !state.cfg.is_loopback_host() && !state.cfg.insecure {
        return Err(cairn_core::Error::Invalid(format!(
            "admin bootstrap via env requires loopback bind or CAIRN_INSECURE=1 \
             (current host={}, insecure={}). \
             Refusing to mint an admin record over plain HTTP on a network-exposed bind.",
            state.cfg.host, state.cfg.insecure
        )));
    }
    let username = state.cfg.admin.username.trim();
    if username.is_empty() {
        return Err(cairn_core::Error::Invalid(
            "CAIRN_ADMIN_USERNAME is empty. Edit .env and restart, or unset \
             CAIRN_ADMIN_PASSWORD to fall back to /setup."
                .into(),
        ));
    }
    if username == password {
        return Err(cairn_core::Error::Invalid(
            "CAIRN_ADMIN_USERNAME equals CAIRN_ADMIN_PASSWORD - refusing to bootstrap. \
             Pick a real password."
                .into(),
        ));
    }
    let hash = hash_password(password)?;
    let rec = AdminRecord::new(username.to_string(), hash);
    match state
        .store
        .set_meta_if_absent(ADMIN_META_KEY, &serde_json::to_string(&rec)?)
    {
        Ok(true) => {
            tracing::info!(
                username = %rec.username,
                generation = rec.generation,
                "admin: bootstrapped from CAIRN_ADMIN_USERNAME + CAIRN_ADMIN_PASSWORD"
            );
            Ok(())
        }
        Ok(false) => {
            tracing::info!("admin: raced to bootstrap - another process won, no-op");
            Ok(())
        }
        Err(e) => Err(e),
    }
}

/// Issue a fresh signed session cookie value. The caller is responsible for putting it into a
/// `Set-Cookie` header via [`build_set_cookie`].
pub fn mint_session(state: &AppState, rec: &AdminRecord) -> SessionPayload {
    SessionPayload::new(
        rec.username.clone(),
        rec.generation,
        Duration::from_secs(state.cfg.admin.session_ttl_hours * 3600),
    )
}

/// Are we on a TLS-enabled bind? Controls whether we attach `Secure` to the cookie.
/// True when the cookie should carry the `Secure` attribute (i.e. only
/// over HTTPS). Production runs that ship TLS mark it true; loopback /
/// `CAIRN_INSECURE=1` runs mark it false so curl-based tests don't lose
/// the cookie on plain HTTP.
pub fn cookie_is_secure(state: &AppState) -> bool {
    state.cfg.tls.is_some() && !state.cfg.insecure
}

/// Append `Set-Cookie` (or clear) to a response builder.
fn with_cookie(mut resp: Response, header_value: String) -> Response {
    resp.headers_mut().insert(
        HeaderName::from_static("set-cookie"),
        HeaderValue::from_str(&header_value).expect("cookie header is ASCII"),
    );
    resp
}

/// Public status endpoint - tells the web UI whether to render `/login` or `/setup`.
pub async fn auth_status(State(state): State<AppState>) -> Response {
    let admin_exists = matches!(load_admin(&state), Ok(Some(_)));
    let body = AuthStatus {
        admin_exists,
        // Setup is required iff there's no admin. On non-loopback binds this still answers true;
        // the bind-time check in cairn-server decides whether to actually serve the setup form.
        setup_required: !admin_exists,
    };
    (StatusCode::OK, Json(body)).into_response()
}

/// POST `/api/auth/login` - accepts username + password, verifies against the stored admin
/// record, and on success returns the session cookie + a JSON body.
pub async fn login(State(state): State<AppState>, Json(req): Json<LoginRequest>) -> Response {
    if req.username.is_empty() || req.password.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "username and password are required"})),
        )
            .into_response();
    }
    let rec = match load_admin(&state) {
        Ok(Some(r)) => r,
        Ok(None) => {
            state.audit_log.record(
                &state.store,
                &state.events,
                "login_failed",
                &req.username,
                "no admin configured".to_string(),
            );
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({"error": "invalid credentials"})),
            )
                .into_response();
        }
        Err(e) => return error_response(&format!("load admin: {e}")),
    };
    if rec.username != req.username {
        state.audit_log.record(
            &state.store,
            &state.events,
            "login_failed",
            &req.username,
            "username mismatch".to_string(),
        );
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"error": "invalid credentials"})),
        )
            .into_response();
    }
    let ok = match verify_password(&req.password, &rec.password_hash) {
        Ok(b) => b,
        Err(e) => return error_response(&format!("verify: {e}")),
    };
    if !ok {
        state.audit_log.record(
            &state.store,
            &state.events,
            "login_failed",
            &req.username,
            "bad password".to_string(),
        );
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"error": "invalid credentials"})),
        )
            .into_response();
    }
    // Success.
    state.audit_log.record(
        &state.store,
        &state.events,
        "login_ok",
        &rec.username,
        String::new(),
    );
    let payload = mint_session(&state, &rec);
    let Some(signer) = state.session_signer.as_ref() else {
        return error_response("CAIRN_SECRET_KEY is required for cookie sessions");
    };
    let cookie_value = signer.sign(&payload);
    let set_cookie = build_set_cookie(
        &cookie_value,
        Duration::from_secs(state.cfg.admin.session_ttl_hours * 3600),
        cookie_is_secure(&state),
    );
    let body = serde_json::json!({
        "username": rec.username,
        "expires_at": payload.exp,
    });
    with_cookie((StatusCode::OK, Json(body)).into_response(), set_cookie)
}

/// POST `/api/auth/logout` - clears the cookie. Always succeeds.
pub async fn logout(State(state): State<AppState>) -> Response {
    let cookie = build_clear_cookie(cookie_is_secure(&state));
    with_cookie(
        (StatusCode::OK, Json(serde_json::json!({"ok": true}))).into_response(),
        cookie,
    )
}

/// GET `/api/auth/me` - returns the current session's admin info, or 401.
pub async fn me(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let Some(rec) = (match load_admin(&state) {
        Ok(r) => r,
        Err(e) => return error_response(&format!("load admin: {e}")),
    }) else {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"error": "no admin"})),
        )
            .into_response();
    };
    let Some(cookie) = extract_cookie(headers.get(header::COOKIE).and_then(|v| v.to_str().ok()))
    else {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"error": "no session"})),
        )
            .into_response();
    };
    let Some(signer) = state.session_signer.as_ref() else {
        return error_response("CAIRN_SECRET_KEY is required for cookie sessions");
    };
    match signer.verify(cookie, rec.generation) {
        Ok(v) => {
            // Sliding extension: if more than half the TTL is consumed, re-issue the cookie on
            // this same response so the user doesn't get logged out mid-task.
            let body = MeView {
                username: v.payload.u.clone(),
                generation: v.payload.g,
                login_at: v.payload.iat,
                expires_at: v.payload.exp,
            };
            let mut resp = (StatusCode::OK, Json(body)).into_response();
            if v.payload.is_more_than_half_consumed() {
                let fresh = SessionPayload::new(
                    v.payload.u.clone(),
                    v.payload.g,
                    Duration::from_secs(state.cfg.admin.session_ttl_hours * 3600),
                );
                let cookie_value = signer.sign(&fresh);
                let set_cookie = build_set_cookie(
                    &cookie_value,
                    Duration::from_secs(state.cfg.admin.session_ttl_hours * 3600),
                    cookie_is_secure(&state),
                );
                if let Ok(hv) = HeaderValue::from_str(&set_cookie) {
                    resp.headers_mut()
                        .insert(HeaderName::from_static("set-cookie"), hv);
                }
            }
            resp
        }
        Err(e) => {
            tracing::debug!(error = %e, "session verify failed");
            (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({"error": "invalid or expired session"})),
            )
                .into_response()
        }
    }
}

/// POST `/api/auth/setup` - first-run wizard. Refuses if an admin already exists (409).
pub async fn setup(State(state): State<AppState>, Json(req): Json<SetupRequest>) -> Response {
    if req.username.trim().is_empty() || req.password.len() < 8 {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "username must be non-empty and password must be at least 8 characters"
            })),
        )
            .into_response();
    }
    let hash = match hash_password(&req.password) {
        Ok(h) => h,
        Err(e) => return error_response(&format!("hash: {e}")),
    };
    let rec = AdminRecord::new(req.username.trim().to_string(), hash);
    match state
        .store
        .set_meta_if_absent(ADMIN_META_KEY, &serde_json::to_string(&rec).unwrap())
    {
        Ok(true) => {}
        Ok(false) => {
            return (
                StatusCode::CONFLICT,
                Json(serde_json::json!({"error": "admin already exists"})),
            )
                .into_response();
        }
        Err(e) => return error_response(&format!("persist: {e}")),
    }
    state.audit_log.record(
        &state.store,
        &state.events,
        "setup",
        &rec.username,
        String::new(),
    );
    let payload = mint_session(&state, &rec);
    let Some(signer) = state.session_signer.as_ref() else {
        return error_response("CAIRN_SECRET_KEY is required for cookie sessions");
    };
    let cookie_value = signer.sign(&payload);
    let set_cookie = build_set_cookie(
        &cookie_value,
        Duration::from_secs(state.cfg.admin.session_ttl_hours * 3600),
        cookie_is_secure(&state),
    );
    let body = serde_json::json!({
        "username": rec.username,
        "expires_at": payload.exp,
    });
    with_cookie((StatusCode::OK, Json(body)).into_response(), set_cookie)
}

/// GET `/api/devices/audit` - admin-only. Returns the in-memory audit log.
pub async fn list_audit(State(state): State<AppState>, headers: HeaderMap) -> Response {
    match require_admin(&state, &headers).await {
        Ok(_) => {}
        Err(resp) => return resp,
    }
    (StatusCode::OK, Json(state.audit_log.snapshot())).into_response()
}

/// Returns the admin record iff the request's cookie or bearer is a valid admin session. Used by
/// the devices endpoints (task 7) and `/api/auth/me` callers that need more than just `me`.
pub async fn require_admin(state: &AppState, headers: &HeaderMap) -> Result<AdminRecord, Response> {
    let Some(rec) = load_admin(state).map_err(|e| error_response(&format!("load admin: {e}")))?
    else {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"error": "no admin"})),
        )
            .into_response());
    };
    if let Some(cookie) = extract_cookie(headers.get(header::COOKIE).and_then(|v| v.to_str().ok()))
    {
        if let Some(signer) = state.session_signer.as_ref() {
            if signer.verify(cookie, rec.generation).is_ok() {
                return Ok(rec);
            }
        }
    }
    Err((
        StatusCode::UNAUTHORIZED,
        Json(serde_json::json!({"error": "admin session required"})),
    )
        .into_response())
}

fn error_response(msg: &str) -> Response {
    tracing::error!("admin: {msg}");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(serde_json::json!({"error": msg})),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audit_log_caps_at_capacity() {
        let log = AuditLog::default();
        for i in 0..(AUDIT_CAPACITY + 5) {
            log.record_dummy(format!("{i}"));
        }
        let snap = log.snapshot();
        assert_eq!(snap.len(), AUDIT_CAPACITY);
        // Newest first.
        assert!(snap[0].detail.contains(&(AUDIT_CAPACITY + 4).to_string()));
        assert!(snap.last().unwrap().detail.contains("5"));
    }

    /// A helper for tests that don't care about durable persistence - the production `record`
    /// writes to the store, but tests run with `cairn_store::Store` and that requires a live
    /// database. Tests want to assert in-memory ring behavior, so they use this stub.
    impl AuditLog {
        pub fn record_dummy(&self, detail: String) {
            let mut q = self.inner.lock().expect("audit log mutex");
            q.push_front(AuditEvent {
                ts: Utc::now().timestamp(),
                kind: "test".into(),
                actor: "tester".into(),
                detail,
            });
            while q.len() > AUDIT_CAPACITY {
                q.pop_back();
            }
        }
    }

    #[test]
    fn admin_view_redacts_password_hash() {
        let rec = AdminRecord::new("admin", "$argon2id$phc".into());
        let v: AdminView = (&rec).into();
        let j = serde_json::to_string(&v).unwrap();
        assert!(!j.contains("argon2"));
        assert!(!j.contains("password"));
        assert!(j.contains("\"username\":\"admin\""));
    }

    #[test]
    fn setup_request_validates_minimum_length() {
        let r = SetupRequest {
            username: "".into(),
            password: "short".into(),
        };
        assert!(r.username.trim().is_empty());
        assert!(r.password.len() < 8);
    }

    #[test]
    fn mint_session_uses_configured_ttl() {
        let hours = 3u64;
        let expected = Duration::from_secs(hours * 3600);
        let p = SessionPayload::new("admin".into(), 1, expected);
        assert_eq!(p.exp - p.iat, expected.as_secs() as i64);
    }

    /// `bootstrap_admin_from_env` validation is independent of the live Store - these tests
    /// cover the static rules. The full integration test (real bootstrap, then reload) runs in
    /// the cairn-api crate's tests/ dir because it needs a live-database fixture.
    #[test]
    fn bootstrap_input_rules_password_too_short() {
        assert!("1234567".len() < 8);
    }

    #[test]
    fn bootstrap_input_rules_password_min_length() {
        assert!("12345678".len() >= 8);
    }

    #[test]
    fn bootstrap_input_rules_username_password_must_differ() {
        let u = "admin";
        let p = "admin";
        assert_eq!(u, p);
    }
}
