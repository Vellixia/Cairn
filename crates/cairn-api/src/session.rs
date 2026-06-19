//! Cookie-based admin session.
//!
//! The dashboard uses an httpOnly cookie instead of a JWT in `Authorization`. The cookie is signed
//! (HMAC-SHA256) using the same `CAIRN_SECRET_KEY` as device tokens so a single secret covers both
//! auth surfaces. The signed payload includes the admin's `generation` counter — when the admin
//! rotates their password, the generation bumps and every previously-issued cookie is rejected by
//! [`Session::verify`].
//!
//! Cookie format: `<base64url(payload)>.<hex(hmac_sha256(payload))>`.
//! Wire format: the same string is what gets written into the `Set-Cookie` and `Cookie` headers.
//!
//! Sliding TTL: every call to [`SessionSigner::verify`] returns the original payload plus a hint
//! about whether the session has consumed more than half its TTL — the caller can choose to
//! re-issue a fresh cookie on the same response. This is what keeps long-lived sessions from
//! expiring under the user without weakening the absolute maximum lifetime.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use chrono::Utc;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::time::Duration;

use cairn_core::AdminRole;

pub const COOKIE_NAME: &str = "cairn_session";

/// Wire payload. Kept small and JSON-only so the base64 blob stays under the cookie size limit.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionPayload {
    /// Admin username.
    pub u: String,
    /// Admin record generation counter at sign time.
    pub g: u64,
    /// Issued-at, seconds since epoch.
    pub iat: i64,
    /// Expires-at, seconds since epoch.
    pub exp: i64,
    /// Coarse role. Today always `Admin`.
    pub role: AdminRole,
}

impl SessionPayload {
    pub fn new(username: String, generation: u64, ttl: Duration) -> Self {
        let now = Utc::now().timestamp();
        Self {
            u: username,
            g: generation,
            iat: now,
            exp: now + ttl.as_secs() as i64,
            role: AdminRole::Admin,
        }
    }

    /// True if more than half the TTL has been consumed. Used by callers to decide whether to
    /// re-issue a fresh cookie (sliding extension).
    pub fn is_more_than_half_consumed(&self) -> bool {
        let total = (self.exp - self.iat).max(1);
        let left = (self.exp - Utc::now().timestamp()).max(0);
        // left * 2 <= total  ⇔  consumed >= total/2  (sliding extension triggers at or past midpoint)
        (left as u128) * 2 <= (total as u128)
    }

    pub fn is_expired(&self) -> bool {
        Utc::now().timestamp() >= self.exp
    }
}

/// Result of [`SessionSigner::verify`].
#[derive(Debug, Clone)]
pub struct VerifiedSession {
    pub payload: SessionPayload,
    /// True when the caller's generation has advanced past the cookie's `g`. The session is
    /// already rejected in this case (returned as `Err`), so this field is only set on success.
    pub fresh: bool,
}

/// HMAC-SHA256 signer/verifier for session cookies. Stateless and cheap to clone.
#[derive(Clone)]
pub struct SessionSigner {
    secret: Vec<u8>,
}

impl SessionSigner {
    pub fn new(secret: Vec<u8>) -> Self {
        Self { secret }
    }

    /// Sign the payload and return the cookie value (base64url + dot + hex HMAC).
    pub fn sign(&self, payload: &SessionPayload) -> String {
        let json = serde_json::to_vec(payload).expect("session payload serializes");
        let body = URL_SAFE_NO_PAD.encode(&json);
        let mac = self.mac(&body);
        format!("{body}.{mac}")
    }

    /// Verify a cookie value. Returns `Err` for any failure: malformed base64, bad MAC, bad JSON,
    /// expired payload, or generation mismatch (caller-supplied `current_generation`).
    pub fn verify(
        &self,
        cookie_value: &str,
        current_generation: u64,
    ) -> Result<VerifiedSession, SessionError> {
        let (body, mac_hex) = cookie_value
            .split_once('.')
            .ok_or(SessionError::Malformed)?;
        let expected = self.mac(body);
        if !constant_time_eq(expected.as_bytes(), mac_hex.as_bytes()) {
            return Err(SessionError::BadSignature);
        }
        let bytes = URL_SAFE_NO_PAD
            .decode(body.as_bytes())
            .map_err(|_| SessionError::Malformed)?;
        let payload: SessionPayload = serde_json::from_slice(&bytes).map_err(SessionError::Decode)?;
        if payload.is_expired() {
            return Err(SessionError::Expired);
        }
        if payload.g != current_generation {
            return Err(SessionError::GenerationMismatch {
                cookie: payload.g,
                current: current_generation,
            });
        }
        Ok(VerifiedSession { payload, fresh: true })
    }

    fn mac(&self, body: &str) -> String {
        let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(&self.secret)
            .expect("HMAC accepts any key length");
        mac.update(body.as_bytes());
        hex::encode(mac.finalize().into_bytes())
    }
}

/// Error categories for [`SessionSigner::verify`]. Kept narrow so middleware can translate.
#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("malformed session cookie")]
    Malformed,
    #[error("bad signature")]
    BadSignature,
    #[error("session decode failed: {0}")]
    Decode(serde_json::Error),
    #[error("session expired")]
    Expired,
    #[error("session generation mismatch (cookie={cookie}, current={current})")]
    GenerationMismatch { cookie: u64, current: u64 },
}

/// Constant-time string comparison; falls back to a length check first because `subtle` isn't in
/// the dependency set, and a length check leaks at most one bit per attempt.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Build a `Set-Cookie` header value for a fresh session. Centralized here so the attributes are
/// applied consistently and can be tweaked in one place.
pub fn build_set_cookie(value: &str, ttl: Duration, secure: bool) -> String {
    let max_age = ttl.as_secs();
    let mut s = format!(
        "{COOKIE_NAME}={value}; Path=/; HttpOnly; SameSite=Strict; Max-Age={max_age}"
    );
    if secure {
        s.push_str("; Secure");
    }
    s
}

/// Build a `Set-Cookie` header that clears the cookie.
pub fn build_clear_cookie(secure: bool) -> String {
    let mut s = format!("{COOKIE_NAME}=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0");
    if secure {
        s.push_str("; Secure");
    }
    s
}

/// Read the `cairn_session` value out of a `Cookie` header. Returns `None` when the header is
/// absent or the cookie isn't present.
pub fn extract_cookie(cookie_header: Option<&str>) -> Option<&str> {
    let header = cookie_header?;
    for part in header.split(';') {
        let part = part.trim();
        if let Some(rest) = part.strip_prefix(&format!("{COOKIE_NAME}=")) {
            return Some(rest);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signer() -> SessionSigner {
        SessionSigner::new(b"a-32-byte-secret-for-session-tests!!!".to_vec())
    }

    #[test]
    fn sign_then_verify_roundtrip() {
        let s = signer();
        let p = SessionPayload::new("admin".into(), 1, Duration::from_secs(3600));
        let cookie = s.sign(&p);
        let v = s.verify(&cookie, 1).unwrap();
        assert_eq!(v.payload, p);
    }

    #[test]
    fn tampered_cookie_is_rejected() {
        let s = signer();
        let cookie = s.sign(&SessionPayload::new("admin".into(), 1, Duration::from_secs(60)));
        let mut bad = cookie.clone();
        let last = bad.pop().unwrap();
        bad.push(if last == '0' { '1' } else { '0' });
        assert!(matches!(s.verify(&bad, 1), Err(SessionError::BadSignature)));
    }

    #[test]
    fn expired_cookie_is_rejected() {
        let s = signer();
        let mut p = SessionPayload::new("admin".into(), 1, Duration::from_secs(60));
        p.exp = Utc::now().timestamp() - 10;
        p.iat = p.exp - 60;
        let cookie = s.sign(&p);
        assert!(matches!(s.verify(&cookie, 1), Err(SessionError::Expired)));
    }

    #[test]
    fn generation_mismatch_is_rejected() {
        let s = signer();
        let cookie = s.sign(&SessionPayload::new("admin".into(), 1, Duration::from_secs(60)));
        assert!(matches!(
            s.verify(&cookie, 2),
            Err(SessionError::GenerationMismatch { cookie: 1, current: 2 })
        ));
    }

    #[test]
    fn malformed_cookie_is_rejected() {
        let s = signer();
        assert!(matches!(s.verify("not-a-cookie", 1), Err(SessionError::Malformed)));
    }

    #[test]
    fn sliding_window_reports_half_consumed() {
        let mut p = SessionPayload::new("admin".into(), 1, Duration::from_secs(3600));
        p.iat = Utc::now().timestamp() - 1800;
        p.exp = p.iat + 3600;
        assert!(p.is_more_than_half_consumed());
    }

    #[test]
    fn extract_cookie_parses_header() {
        let h = "other=foo; cairn_session=abc.def; trail=bar";
        assert_eq!(extract_cookie(Some(h)), Some("abc.def"));
        assert_eq!(extract_cookie(None), None);
        assert_eq!(extract_cookie(Some("")), None);
        assert_eq!(extract_cookie(Some("nothing=here")), None);
    }

    #[test]
    fn set_cookie_attributes_match_design() {
        let v = build_set_cookie("p", Duration::from_secs(60), true);
        assert!(v.contains("HttpOnly"));
        assert!(v.contains("SameSite=Strict"));
        assert!(v.contains("Secure"));
        assert!(v.contains("Max-Age=60"));
        let v2 = build_set_cookie("p", Duration::from_secs(60), false);
        assert!(!v2.contains("Secure"));
    }

    #[test]
    fn clear_cookie_zeroes_max_age() {
        let v = build_clear_cookie(true);
        assert!(v.contains("Max-Age=0"));
    }
}
