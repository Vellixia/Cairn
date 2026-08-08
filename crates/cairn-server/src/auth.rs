//! Authentication and membership (FR-054, FR-057, D10).
//!
//! Two audiences resolve to the same user: the web UI through a session cookie,
//! and the daemon through a personal API token. Passwords are argon2; tokens
//! are stored hashed so a database read cannot impersonate anyone.

use crate::error::{ApiError, ApiResult};
use crate::AppState;
use argon2::password_hash::{
    rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString,
};
use argon2::Argon2;
use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use rand::RngCore;
use sqlx::PgPool;
use uuid::Uuid;

pub const COOKIE_NAME: &str = "cairn_session";
const SESSION_DAYS: i64 = 30;

/// The authenticated caller.
#[derive(Debug, Clone, Copy)]
pub struct CurrentUser {
    pub id: Uuid,
}

impl FromRequestParts<AppState> for CurrentUser {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        // Bearer token first: that is how the daemon authenticates.
        if let Some(token) = bearer_token(parts) {
            if let Some(user) = user_for_api_token(&state.pool, &token).await? {
                return Ok(CurrentUser { id: user });
            }
            return Err(ApiError::unauthorized("invalid API token"));
        }
        if let Some(cookie) = session_cookie(parts) {
            if let Some(user) = user_for_web_session(&state.pool, &cookie).await? {
                return Ok(CurrentUser { id: user });
            }
        }
        Err(ApiError::unauthorized("sign in or supply an API token"))
    }
}

fn bearer_token(parts: &Parts) -> Option<String> {
    parts
        .headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
}

fn session_cookie(parts: &Parts) -> Option<String> {
    let raw = parts
        .headers
        .get(axum::http::header::COOKIE)?
        .to_str()
        .ok()?;
    raw.split(';')
        .filter_map(|c| c.trim().split_once('='))
        .find(|(k, _)| *k == COOKIE_NAME)
        .map(|(_, v)| v.to_string())
}

/// Tokens are compared by hash, never by plaintext.
pub fn hash_token(token: &str) -> String {
    cairn_core::digest(token)
}

pub fn random_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}

pub fn hash_password(password: &str) -> ApiResult<String> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| ApiError::internal(e.to_string()))
}

pub fn verify_password(password: &str, hash: &str) -> bool {
    match PasswordHash::new(hash) {
        Ok(parsed) => Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok(),
        Err(_) => false,
    }
}

async fn user_for_api_token(pool: &PgPool, token: &str) -> ApiResult<Option<Uuid>> {
    let hash = hash_token(token);
    let row: Option<(Uuid,)> = sqlx::query_as(
        "SELECT user_id FROM api_tokens WHERE token_hash = $1 AND revoked_at IS NULL",
    )
    .bind(&hash)
    .fetch_optional(pool)
    .await?;

    if let Some((user_id,)) = row {
        sqlx::query("UPDATE api_tokens SET last_used_at = now() WHERE token_hash = $1")
            .bind(&hash)
            .execute(pool)
            .await?;
        return Ok(Some(user_id));
    }
    Ok(None)
}

async fn user_for_web_session(pool: &PgPool, token: &str) -> ApiResult<Option<Uuid>> {
    let row: Option<(Uuid,)> = sqlx::query_as(
        "SELECT user_id FROM web_sessions WHERE token_hash = $1 AND expires_at > now()",
    )
    .bind(hash_token(token))
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|(id,)| id))
}

pub async fn create_web_session(pool: &PgPool, user_id: Uuid) -> ApiResult<String> {
    let token = random_token();
    sqlx::query(
        "INSERT INTO web_sessions (token_hash, user_id, expires_at)
         VALUES ($1, $2, now() + ($3 || ' days')::interval)",
    )
    .bind(hash_token(&token))
    .bind(user_id)
    .bind(SESSION_DAYS.to_string())
    .execute(pool)
    .await?;
    Ok(token)
}

pub async fn destroy_web_session(pool: &PgPool, token: &str) -> ApiResult<()> {
    sqlx::query("DELETE FROM web_sessions WHERE token_hash = $1")
        .bind(hash_token(token))
        .execute(pool)
        .await?;
    Ok(())
}

/// Membership guard. Every project-scoped route calls this before anything
/// else; a non-member gets `403`, not an empty list (FR-057).
pub async fn require_member(pool: &PgPool, project_id: Uuid, user_id: Uuid) -> ApiResult<()> {
    let member: Option<(Uuid,)> = sqlx::query_as(
        "SELECT user_id FROM project_members WHERE project_id = $1 AND user_id = $2",
    )
    .bind(project_id)
    .bind(user_id)
    .fetch_optional(pool)
    .await?;
    match member {
        Some(_) => Ok(()),
        None => Err(ApiError::forbidden("you are not a member of this project")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passwords_verify_only_against_themselves() {
        let hash = hash_password("correct horse battery staple").unwrap();
        assert!(verify_password("correct horse battery staple", &hash));
        assert!(!verify_password("wrong", &hash));
        assert!(hash.starts_with("$argon2"), "argon2 is required (D10)");
    }

    #[test]
    fn tokens_are_stored_hashed() {
        let token = random_token();
        assert_eq!(token.len(), 64);
        let hash = hash_token(&token);
        assert_ne!(hash, token);
        assert_eq!(hash, hash_token(&token), "hashing is stable");
    }
}
