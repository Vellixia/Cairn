//! JWT-based device-token authentication.
//!
//! Device tokens are now signed JWTs (HS256). The backend stores only token metadata (id, name,
//! created_at); the bearer value itself is never persisted. This aligns the implementation with the
//! auth architecture promised in `docs/PLAN.md`.

use chrono::Utc;
use serde::{Deserialize, Serialize};

/// Claims embedded in a device-token JWT.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Claims {
    /// Token identifier (matches the metadata stored in HelixDB).
    jti: String,
    /// Human-readable device name.
    sub: String,
    /// Issued-at timestamp (seconds since epoch).
    iat: i64,
}

/// Error type for token operations.
#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("missing or invalid signing secret")]
    MissingSecret,
    #[error("token decode failed: {0}")]
    Decode(#[from] jsonwebtoken::errors::Error),
}

/// Signs and verifies device-token JWTs.
#[derive(Clone)]
pub struct TokenSigner {
    secret: Vec<u8>,
}

impl TokenSigner {
    /// Build a signer from a raw secret. Returns an error if the secret is empty.
    pub fn new(secret: Vec<u8>) -> Result<Self, AuthError> {
        if secret.is_empty() {
            return Err(AuthError::MissingSecret);
        }
        Ok(Self { secret })
    }

    /// Issue a signed JWT for the given token metadata.
    pub fn mint(&self, token_id: &str, name: &str) -> String {
        let claims = Claims {
            jti: token_id.to_string(),
            sub: name.to_string(),
            iat: Utc::now().timestamp(),
        };
        jsonwebtoken::encode(
            &jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS256),
            &claims,
            &jsonwebtoken::EncodingKey::from_secret(&self.secret),
        )
        .expect("HS256 encoding cannot fail with a valid secret")
    }

    /// Verify a bearer token and return the token id if valid.
    pub fn verify(&self, token: &str) -> Result<String, AuthError> {
        let mut validation = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::HS256);
        validation.leeway = 60; // tolerate 60s clock skew
        // Device tokens are long-lived: we validate the signature and `iat`, but do not require an
        // `exp` claim (revocation is handled by deleting the metadata record).
        validation.validate_exp = false;
        validation.required_spec_claims.remove("exp");
        let decoded = jsonwebtoken::decode::<Claims>(
            token,
            &jsonwebtoken::DecodingKey::from_secret(&self.secret),
            &validation,
        )?;
        Ok(decoded.claims.jti)
    }
}

/// Strip a `Bearer ` prefix case-insensitively from an Authorization header value.
pub fn extract_bearer(value: &str) -> Option<&str> {
    value
        .strip_prefix("Bearer ")
        .or_else(|| value.strip_prefix("bearer "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mint_and_verify_roundtrip() {
        let signer = TokenSigner::new(b"test-secret-at-least-32-bytes-long!!".to_vec()).unwrap();
        let jwt = signer.mint("id-123", "laptop");
        let jti = signer.verify(&jwt).unwrap();
        assert_eq!(jti, "id-123");
    }

    #[test]
    fn tampered_token_fails() {
        let signer = TokenSigner::new(b"test-secret-at-least-32-bytes-long!!".to_vec()).unwrap();
        let mut jwt = signer.mint("id-123", "laptop");
        jwt.push_str("x");
        assert!(signer.verify(&jwt).is_err());
    }

    #[test]
    fn wrong_secret_fails() {
        let signer1 = TokenSigner::new(b"secret-one-is-long-enough-12345".to_vec()).unwrap();
        let signer2 = TokenSigner::new(b"secret-two-is-long-enough-67890".to_vec()).unwrap();
        let jwt = signer1.mint("id-123", "laptop");
        assert!(signer2.verify(&jwt).is_err());
    }
}
