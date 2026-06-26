//! JWT-based device-token authentication.
//!
//! Device tokens are signed JWTs (HS256). The backend stores only token metadata (id, name,
//! scope, created_at); the bearer value itself is never persisted. This aligns the implementation
//! with the auth architecture promised in `docs/PLAN.md`.

use cairn_core::TokenScope;
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
    /// Token scope (admin/write/read).
    scope: String,
    /// Optional expiration timestamp (seconds since epoch).
    #[serde(skip_serializing_if = "Option::is_none")]
    exp: Option<i64>,
}

/// Decoded token metadata returned by `verify`.
#[derive(Debug, Clone)]
pub struct TokenInfo {
    pub id: String,
    pub scope: TokenScope,
}

/// Minimum HS256 secret length in bytes. RFC 2104 recommends the key length equal the hash
/// output (32 bytes for SHA-256); shorter keys meaningfully weaken the HMAC.
pub const MIN_SECRET_LEN: usize = 32;

/// Error type for token operations.
#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("missing or invalid signing secret")]
    MissingSecret,
    #[error(
        "CAIRN_SECRET_KEY is too short ({len} bytes); HS256 requires at least {MIN_SECRET_LEN} \
         bytes - generate one with `openssl rand -base64 48` and set it in .env"
    )]
    WeakSecret { len: usize },
    #[error("token decode failed: {0}")]
    Decode(#[from] jsonwebtoken::errors::Error),
    #[error("unknown token scope: {0}")]
    UnknownScope(String),
}

/// Signs and verifies device-token JWTs.
#[derive(Clone)]
pub struct TokenSigner {
    secret: Vec<u8>,
}

impl TokenSigner {
    /// Build a signer from a raw secret. Returns an error if the secret is empty or shorter
    /// than [`MIN_SECRET_LEN`] bytes.
    pub fn new(secret: Vec<u8>) -> Result<Self, AuthError> {
        if secret.is_empty() {
            return Err(AuthError::MissingSecret);
        }
        if secret.len() < MIN_SECRET_LEN {
            return Err(AuthError::WeakSecret { len: secret.len() });
        }
        Ok(Self { secret })
    }

    /// Issue a signed JWT for the given token metadata.
    pub fn mint(
        &self,
        token_id: &str,
        name: &str,
        scope: TokenScope,
        expires_at: Option<chrono::DateTime<Utc>>,
    ) -> String {
        let claims = Claims {
            jti: token_id.to_string(),
            sub: name.to_string(),
            iat: Utc::now().timestamp(),
            scope: scope.as_str().to_string(),
            exp: expires_at.map(|dt| dt.timestamp()),
        };
        jsonwebtoken::encode(
            &jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS256),
            &claims,
            &jsonwebtoken::EncodingKey::from_secret(&self.secret),
        )
        .expect("HS256 encoding cannot fail with a valid secret")
    }

    /// Verify a bearer token and return the token id + scope if valid.
    pub fn verify(&self, token: &str) -> Result<TokenInfo, AuthError> {
        let mut validation = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::HS256);
        validation.leeway = 60;
        // If exp is present, validate it; if absent, skip exp validation.
        validation.validate_exp = true;
        validation.required_spec_claims.remove("exp");
        let decoded = jsonwebtoken::decode::<Claims>(
            token,
            &jsonwebtoken::DecodingKey::from_secret(&self.secret),
            &validation,
        )?;
        let scope = decoded
            .claims
            .scope
            .parse::<TokenScope>()
            .map_err(|_| AuthError::UnknownScope(decoded.claims.scope.clone()))?;
        Ok(TokenInfo {
            id: decoded.claims.jti,
            scope,
        })
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

    fn signer() -> TokenSigner {
        TokenSigner::new(b"test-secret-at-least-32-bytes-long!!".to_vec()).unwrap()
    }

    #[test]
    fn mint_and_verify_roundtrip() {
        let s = signer();
        let jwt = s.mint("id-123", "laptop", TokenScope::Write, None);
        let info = s.verify(&jwt).unwrap();
        assert_eq!(info.id, "id-123");
        assert_eq!(info.scope, TokenScope::Write);
    }

    #[test]
    fn tampered_token_fails() {
        let s = signer();
        let mut jwt = s.mint("id-123", "laptop", TokenScope::Write, None);
        jwt.push('x');
        assert!(s.verify(&jwt).is_err());
    }

    #[test]
    fn wrong_secret_fails() {
        let s1 = TokenSigner::new(b"secret-one-is-long-enough-1234567".to_vec()).unwrap();
        let s2 = TokenSigner::new(b"secret-two-is-long-enough-7890123".to_vec()).unwrap();
        let jwt = s1.mint("id-123", "laptop", TokenScope::Write, None);
        assert!(s2.verify(&jwt).is_err());
    }

    #[test]
    fn expired_token_is_rejected() {
        let s = signer();
        let past = Utc::now() - chrono::Duration::hours(1);
        let jwt = s.mint("id-123", "laptop", TokenScope::Write, Some(past));
        assert!(s.verify(&jwt).is_err());
    }

    #[test]
    fn future_expiry_is_accepted() {
        let s = signer();
        let future = Utc::now() + chrono::Duration::hours(1);
        let jwt = s.mint("id-123", "laptop", TokenScope::Read, Some(future));
        let info = s.verify(&jwt).unwrap();
        assert_eq!(info.scope, TokenScope::Read);
    }

    #[test]
    fn scope_is_preserved() {
        let s = signer();
        for scope in [TokenScope::Admin, TokenScope::Write, TokenScope::Read] {
            let jwt = s.mint("id", "dev", scope, None);
            let info = s.verify(&jwt).unwrap();
            assert_eq!(info.scope, scope);
        }
    }

    #[test]
    fn empty_secret_is_rejected() {
        assert!(matches!(
            TokenSigner::new(Vec::new()),
            Err(AuthError::MissingSecret)
        ));
    }

    #[test]
    fn short_secret_is_rejected() {
        let short = b"too-short".to_vec();
        assert!(
            matches!(
                TokenSigner::new(short),
                Err(AuthError::WeakSecret { len: 9 })
            ),
            "expected WeakSecret error for 9-byte secret"
        );
        let boundary = vec![b'a'; MIN_SECRET_LEN - 1];
        assert!(
            matches!(
                TokenSigner::new(boundary),
                Err(AuthError::WeakSecret { len }) if len == MIN_SECRET_LEN - 1
            ),
            "expected WeakSecret error for secret one byte below minimum"
        );
    }

    #[test]
    fn minimum_length_secret_is_accepted() {
        let ok = vec![b'k'; MIN_SECRET_LEN];
        assert!(TokenSigner::new(ok).is_ok());
    }
}
