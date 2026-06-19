//! Single-admin account: Argon2id password hashing and the persistent record.
//!
//! Cairn's web dashboard is a one-admin console. The admin record lives in the same meta store as
//! every other piece of Cairn state (under the `admin` key), so there's no separate users table to
//! keep in sync. Argon2id is the OWASP-recommended password hash; we use the `argon2` crate's
//! default parameters (m=19456, t=2, p=1) which match the recommended settings as of argon2
//! 0.5.x. Each hash embeds its own salt + parameters, so verification is parameter-free.

use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};

use crate::Result;

/// Coarse role. Today Cairn has exactly one role — `Admin` — and device tokens. Kept as an enum so
/// we can extend later (read-only web viewer, etc.) without a schema migration.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AdminRole {
    #[default]
    Admin,
}

/// The persistent admin record. Stored at the `admin` meta key.
///
/// `generation` is bumped on every password change; signed sessions include the generation they
/// were created at so a rotated password immediately invalidates every existing cookie.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminRecord {
    pub username: String,
    /// Argon2id PHC-formatted hash (`$argon2id$v=19$m=...$t=...$p=...$salt$hash`).
    pub password_hash: String,
    /// Monotonic counter; bumped on every password change.
    pub generation: u64,
    /// UNIX timestamp seconds.
    pub created_at: i64,
    /// UNIX timestamp seconds.
    pub updated_at: i64,
    pub role: AdminRole,
}

impl AdminRecord {
    pub fn new(username: impl Into<String>, password_hash: String) -> Self {
        let now = chrono::Utc::now().timestamp();
        Self {
            username: username.into(),
            password_hash,
            generation: 1,
            created_at: now,
            updated_at: now,
            role: AdminRole::Admin,
        }
    }

    /// Bump the generation counter and `updated_at`. Returns the new record.
    pub fn rotate_password(&mut self, new_password_hash: String) {
        self.password_hash = new_password_hash;
        self.generation = self.generation.saturating_add(1);
        self.updated_at = chrono::Utc::now().timestamp();
    }
}

/// Hash `password` with Argon2id using the default parameters + a fresh OS-random salt. Returns
/// the PHC-formatted hash string.
pub fn hash_password(password: &str) -> Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    let hash = argon2
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| crate::Error::Other(format!("argon2 hash: {e}")))?;
    Ok(hash.to_string())
}

/// Verify `password` against a PHC-formatted Argon2id hash. Returns `Ok(true)` on match,
/// `Ok(false)` on mismatch, `Err` on a malformed stored hash.
pub fn verify_password(password: &str, phc: &str) -> Result<bool> {
    let parsed = PasswordHash::new(phc)
        .map_err(|e| crate::Error::Invalid(format!("argon2 hash parse: {e}")))?;
    Ok(Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_then_verify_round_trip() {
        let h = hash_password("correct horse battery staple").unwrap();
        assert!(verify_password("correct horse battery staple", &h).unwrap());
        assert!(!verify_password("wrong", &h).unwrap());
    }

    #[test]
    fn hash_is_argon2id_phc_format() {
        let h = hash_password("p").unwrap();
        assert!(h.starts_with("$argon2id$"));
    }

    #[test]
    fn rotate_password_invalidates_old_hash_and_bumps_generation() {
        let mut rec = AdminRecord::new("admin", hash_password("first").unwrap());
        let gen_before = rec.generation;
        rec.rotate_password(hash_password("second").unwrap());
        assert!(rec.generation > gen_before);
        assert!(!verify_password("first", &rec.password_hash).unwrap());
        assert!(verify_password("second", &rec.password_hash).unwrap());
    }

    #[test]
    fn verify_rejects_garbage_hash() {
        assert!(verify_password("p", "not-a-phc-string").is_err());
    }

    #[test]
    fn admin_record_serializes_round_trip() {
        let rec = AdminRecord::new("admin", hash_password("p").unwrap());
        let j = serde_json::to_string(&rec).unwrap();
        let back: AdminRecord = serde_json::from_str(&j).unwrap();
        assert_eq!(back.username, rec.username);
        assert_eq!(back.generation, rec.generation);
        assert!(verify_password("p", &back.password_hash).unwrap());
    }
}
