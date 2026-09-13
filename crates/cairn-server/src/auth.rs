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
use cairn_core::domain::{ServerRole, UserStatus};
use rand::RngCore;
use sqlx::PgPool;
use uuid::Uuid;

pub const COOKIE_NAME: &str = "cairn_session";
const SESSION_DAYS: i64 = 30;

/// The shortest password the environment-defined admin may carry. Registration
/// applies the same floor to accounts created over HTTP.
pub const MIN_PASSWORD_LEN: usize = 8;

/// The authenticated caller, and its standing on this server.
///
/// Role and status travel with the identity rather than being looked up again
/// per route. A route that had to remember to check `status` is a route that can
/// forget to, and `FR-410` says a disabled account is refused "by any means" —
/// which is only true if the refusal happens where the identity is established.
#[derive(Debug, Clone, Copy)]
pub struct CurrentUser {
    pub id: Uuid,
    pub role: ServerRole,
    pub status: UserStatus,
    /// While set, this account may reach the password-change route and nothing
    /// else (FR-407).
    pub must_change_password: bool,
}

impl CurrentUser {
    pub fn is_admin(&self) -> bool {
        self.role == ServerRole::Admin
    }
}

impl FromRequestParts<AppState> for CurrentUser {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        // Bearer token first: that is how the daemon authenticates.
        let id = if let Some(token) = bearer_token(parts) {
            match user_for_api_token(&state.pool, &token, state.schema_version).await? {
                Some(user) => user,
                // Revoked, expired, and never-existed are one answer on
                // purpose: a distinguishable refusal tells the holder of a
                // stale token that it was once valid for this server, which is
                // an oracle about both the server's history and the account
                // (FR-585).
                None => return Err(ApiError::unauthorized("invalid API token")),
            }
        } else if let Some(cookie) = session_cookie(parts) {
            match user_for_web_session(&state.pool, &cookie).await? {
                Some(user) => user,
                None => return Err(ApiError::unauthorized("sign in or supply an API token")),
            }
        } else {
            return Err(ApiError::unauthorized("sign in or supply an API token"));
        };

        let standing = standing_of(&state.pool, id, state.schema_version).await?;
        // A token minted before the account was disabled is refused here, which
        // is what makes revocation-at-disable a belt rather than the only line
        // of defence: even a token the revoking transaction somehow missed
        // cannot authenticate a disabled account.
        if standing.status == UserStatus::Disabled {
            return Err(ApiError::unauthorized("this account is disabled"));
        }
        Ok(standing)
    }
}

/// One account's standing, read once per request.
async fn standing_of(pool: &PgPool, id: Uuid, schema_version: i64) -> ApiResult<CurrentUser> {
    // All three columns arrive with migration 3. A deployment held back for a
    // staged rollout is a supported configuration, and selecting them
    // unconditionally made every request to a schema-2 server fail at the
    // extractor — before any route ran.
    //
    // Below schema 3 the answer is the one this server gave before roles
    // existed: every account is an active member with nothing outstanding.
    // There is no way for it to be otherwise, because there is nowhere to
    // record it.
    if schema_version < 3 {
        let exists: Option<(Uuid,)> = sqlx::query_as("SELECT id FROM users WHERE id = $1")
            .bind(id)
            .fetch_optional(pool)
            .await?;
        if exists.is_none() {
            return Err(ApiError::unauthorized("invalid API token"));
        }
        return Ok(CurrentUser {
            id,
            role: ServerRole::Member,
            status: UserStatus::Active,
            must_change_password: false,
        });
    }

    let row: Option<(String, String, bool)> =
        sqlx::query_as("SELECT role, status, must_change_password FROM users WHERE id = $1")
            .bind(id)
            .fetch_optional(pool)
            .await?;
    // A credential that resolves to no row is not a credential. This happens
    // only if the account was deleted between minting and use, and the honest
    // answer is the same as any other invalid credential.
    let Some((role, status, must_change_password)) = row else {
        return Err(ApiError::unauthorized("invalid API token"));
    };
    Ok(CurrentUser {
        id,
        role: role.parse().unwrap_or(ServerRole::Member),
        status: status.parse().unwrap_or(UserStatus::Disabled),
        must_change_password,
    })
}

/// An authenticated caller who has nothing outstanding (FR-407).
///
/// **This, not `CurrentUser`, is what an ordinary route should take.**
/// `CurrentUser` establishes *who* is calling and deliberately does not enforce
/// the password-change gate, because exactly one route must remain reachable
/// while the gate is up — the password change itself. Every other route wants
/// this type, and the compiler is what remembers: a new route added later gets
/// the gate by writing `SettledUser` in its parameter list, and gets it wrong
/// only by explicitly asking for the ungated type instead.
#[derive(Debug, Clone, Copy)]
pub struct SettledUser(pub CurrentUser);

impl SettledUser {
    pub fn id(&self) -> Uuid {
        self.0.id
    }
    /// This account's role, as the extractor already resolved it.
    ///
    /// Below schema 3 it is always `Member`, because there is nowhere to record
    /// anything else — see [`standing_of`].
    pub fn role(&self) -> ServerRole {
        self.0.role
    }
    /// This account's status, as the extractor already resolved it.
    pub fn status(&self) -> UserStatus {
        self.0.status
    }
}

impl FromRequestParts<AppState> for SettledUser {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let user = CurrentUser::from_request_parts(parts, state).await?;
        if user.must_change_password {
            return Err(password_change_required());
        }
        Ok(SettledUser(user))
    }
}

/// An authenticated caller who is also an administrator (FR-402).
///
/// A separate extractor rather than a check inside each handler: the type is the
/// authorization. A route that takes `AdminUser` cannot be reached by a member,
/// and a route that forgot to check cannot compile if it declared the right
/// parameter.
#[derive(Debug, Clone, Copy)]
pub struct AdminUser(pub CurrentUser);

impl AdminUser {
    /// Who is acting. Administration is auditable by nature — "an account was
    /// created" is not a useful log line without it.
    pub fn id(&self) -> Uuid {
        self.0.id
    }
}

impl FromRequestParts<AppState> for AdminUser {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let user = CurrentUser::from_request_parts(parts, state).await?;
        // The password-change gate applies to administrators too: an account
        // holding a temporary credential authenticates to the change route and
        // nothing else, whatever its role (FR-407).
        if user.must_change_password {
            return Err(password_change_required());
        }
        if !user.is_admin() {
            return Err(ApiError::forbidden("this action requires an administrator"));
        }
        Ok(AdminUser(user))
    }
}

/// The refusal every route other than the password change returns while an
/// account still owes a password change (FR-407).
pub fn password_change_required() -> ApiError {
    ApiError::new(
        axum::http::StatusCode::FORBIDDEN,
        "password_change_required",
        "change this account's password before doing anything else",
    )
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

/// A one-time password an operator can read out loud and a user can type.
///
/// Same CSPRNG as `random_token`, truncated: a bearer token is 32 bytes because
/// nothing has to type it, while this is typed once and then replaced. 12 hex
/// characters is 48 bits, which is far short of a bearer token — and correct
/// here, because this credential authenticates to exactly one route, is
/// invalidated by the change it exists to permit, and can be reset by an
/// administrator at any time.
pub fn temporary_password() -> String {
    random_token()[..12].to_string()
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

/// Whether [`ensure_admin`] created the account or found it already there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdminOutcome {
    Created,
    Updated,
}

impl AdminOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            AdminOutcome::Created => "created",
            AdminOutcome::Updated => "updated",
        }
    }
}

/// Define the operator's account from the environment.
///
/// A fresh deployment has no users, and `/api/auth/register` is the only route
/// that makes one — which leaves nobody able to sign in and open registration
/// to whoever reaches the server first. Naming the account in the environment
/// closes both gaps.
///
/// The environment is the source of truth, so this re-applies the password on
/// every start: rotating it means editing the variable and restarting. Running
/// twice with an unchanged password is still a write, but not a change.
pub async fn ensure_admin(
    pool: &PgPool,
    email: &str,
    display_name: &str,
    password: &str,
) -> anyhow::Result<(Uuid, AdminOutcome)> {
    let email = email.trim().to_lowercase();
    if email.is_empty() {
        anyhow::bail!("CAIRN_ADMIN_EMAIL is empty");
    }
    if !email.contains('@') {
        anyhow::bail!("CAIRN_ADMIN_EMAIL is not an email address");
    }
    if password.chars().count() < MIN_PASSWORD_LEN {
        anyhow::bail!("CAIRN_ADMIN_PASSWORD must be at least {MIN_PASSWORD_LEN} characters");
    }

    let hash = hash_password(password).map_err(|e| anyhow::anyhow!(e.message))?;

    // `xmax = 0` distinguishes the inserted row from the updated one: an INSERT
    // leaves no deleting transaction behind, an UPDATE does. It is the only way
    // to tell the two apart from a single upsert.
    // `role` and `status` are restored, not merely set on insert (FR-539).
    //
    // This is the break-glass path, and it only works if it restores *authority*
    // as well as the password. An operator who demoted or disabled the last
    // administrator has no supported API left to recover through; without these
    // two assignments a restart would hand them a working password on an account
    // that still cannot administer anything.
    //
    // `must_change_password` is deliberately forced false (FR-540). The
    // environment re-establishes this password on every start, so a forced
    // change would be reverted by the next restart — an unbreakable loop rather
    // than a security measure.
    // Below schema 3 there are no standing columns to restore — and nothing that
    // could have demoted or disabled the account either, so the seed reduces to
    // what it always was. Selecting them unconditionally made a held-back
    // deployment fail to *start* when an operator supplied the environment
    // account, which is the one configuration that exists to prevent lockout.
    let standing_columns: bool = sqlx::query_scalar(
        "SELECT EXISTS (
             SELECT 1 FROM information_schema.columns
              WHERE table_name = 'users' AND column_name = 'role'
         )",
    )
    .fetch_one(pool)
    .await?;

    let sql = if standing_columns {
        "INSERT INTO users (id, email, display_name, password_hash,
                            role, status, must_change_password)
         VALUES ($1, $2, $3, $4, 'admin', 'active', false)
         ON CONFLICT (email) DO UPDATE
             SET password_hash        = EXCLUDED.password_hash,
                 display_name         = EXCLUDED.display_name,
                 role                 = 'admin',
                 status               = 'active',
                 must_change_password = false
         RETURNING id, (xmax = 0) AS inserted"
    } else {
        "INSERT INTO users (id, email, display_name, password_hash)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (email) DO UPDATE
             SET password_hash = EXCLUDED.password_hash,
                 display_name  = EXCLUDED.display_name
         RETURNING id, (xmax = 0) AS inserted"
    };
    let (id, inserted): (Uuid, bool) = sqlx::query_as(sql)
        .bind(Uuid::now_v7())
        .bind(&email)
        .bind(display_name)
        .bind(&hash)
        .fetch_one(pool)
        .await?;

    Ok((
        id,
        if inserted {
            AdminOutcome::Created
        } else {
            AdminOutcome::Updated
        },
    ))
}

/// Create an account. The only way one comes into existence, besides
/// `ensure_admin`'s environment-defined account.
///
/// Not reachable over HTTP by design — see `api::routes`. The validation here
/// is the same the removed registration route performed; what changed is who
/// can invoke it.
pub async fn create_user(
    pool: &PgPool,
    email: &str,
    display_name: &str,
    password: &str,
    must_change_password: bool,
) -> anyhow::Result<(Uuid, String)> {
    let email = email.trim().to_lowercase();
    if email.is_empty() || !email.contains('@') {
        anyhow::bail!("email is not an email address");
    }
    if password.chars().count() < MIN_PASSWORD_LEN {
        anyhow::bail!("password must be at least {MIN_PASSWORD_LEN} characters");
    }
    let hash = hash_password(password).map_err(|e| anyhow::anyhow!(e.message))?;
    let id = Uuid::now_v7();
    // `must_change_password` arrives with migration 3, and a deployment held back
    // for a staged rollout is a supported configuration — an operator
    // provisioning an account on one must not be met with a column error. The
    // column is checked rather than the schema version because that is the fact
    // the statement actually depends on.
    let has_change_flag: bool = sqlx::query_scalar(
        "SELECT EXISTS (
             SELECT 1 FROM information_schema.columns
              WHERE table_name = 'users' AND column_name = 'must_change_password'
         )",
    )
    .fetch_one(pool)
    .await?;

    let result = if has_change_flag {
        sqlx::query(
            "INSERT INTO users (id, email, display_name, password_hash, must_change_password)
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(id)
        .bind(&email)
        .bind(display_name)
        .bind(hash)
        .bind(must_change_password)
        .execute(pool)
        .await
    } else {
        // Pre-schema-3: there is no flag to set, and nothing reads one. An
        // account created here is immediately usable, which is the only
        // behaviour available and the one that was correct before this feature.
        sqlx::query(
            "INSERT INTO users (id, email, display_name, password_hash) VALUES ($1, $2, $3, $4)",
        )
        .bind(id)
        .bind(&email)
        .bind(display_name)
        .bind(hash)
        .execute(pool)
        .await
    };
    match result {
        Ok(_) => Ok((id, email)),
        Err(sqlx::Error::Database(e)) if e.is_unique_violation() => {
            anyhow::bail!("that email already has an account")
        }
        Err(e) => Err(e.into()),
    }
}

async fn user_for_api_token(
    pool: &PgPool,
    token: &str,
    schema_version: i64,
) -> ApiResult<Option<Uuid>> {
    let hash = hash_token(token);
    // Expiry sits alongside revocation in one predicate, deliberately: the two
    // then produce the *same* answer at the same place, so no caller can tell
    // them apart. A distinguishable refusal would tell whoever holds a stale
    // token that it was once valid for this server (FR-585, SC-452).
    //
    // `expires_at` arrives with migration 3, so a held-back deployment has only
    // the revocation half. That is not a weakening: with no column, no token can
    // carry an expiry, so the predicate it would satisfy is vacuous.
    let sql = if schema_version >= 3 {
        "SELECT user_id FROM api_tokens
          WHERE token_hash = $1
            AND revoked_at IS NULL
            AND (expires_at IS NULL OR expires_at > now())"
    } else {
        "SELECT user_id FROM api_tokens WHERE token_hash = $1 AND revoked_at IS NULL"
    };
    let row: Option<(Uuid,)> = sqlx::query_as(sql).bind(&hash).fetch_optional(pool).await?;

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

// ---------------------------------------------------------------------------
// Feature 005 — per-domain resolution and ownership (FR-768-FR-769a, FR-834,
// FR-846a)
// ---------------------------------------------------------------------------

use cairn_core::domain::{KnowledgeDomain, Readership, Reference};

// The four items below are the read side of this boundary and have no caller
// until retrieval lands (US2/US5, T093-T124). They are written here, with the
// ingest guards they belong beside, because the rule they encode is one rule:
// a reference is resolved per domain, and a personal record is owner-only
// whatever asks for it. Splitting the write side from the read side would
// invite two answers to that question. The suppression is scoped to these four
// and comes off the moment retrieval calls them.

/// Whether a reference may be shown to a reader, and — when it may not — the
/// only thing the caller is allowed to do about it.
///
/// Two outcomes, and the second is the interesting one. A reference the reader
/// may not see is **withheld entirely**, not rendered as an opaque handle
/// (FR-846a). An opaque handle still discloses that the record exists, and a
/// briefing spans three domains, so a project member counting handles or
/// noticing a gap in a rank sequence could enumerate a colleague's personal
/// knowledge without ever reading a word of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum Visibility {
    /// The reader may see this reference and resolve it.
    Visible,
    /// The reader may not. The caller must drop it — not blank it, not
    /// placeholder it, not leave a numbered gap where it was.
    Withheld,
}

impl Visibility {
    #[allow(dead_code)]
    pub fn is_visible(&self) -> bool {
        matches!(self, Visibility::Visible)
    }
}

/// What the server knows about the reader, once, so a resolution loop does not
/// re-query membership per reference.
///
/// Assembled from the authenticated credential (FR-769). Nothing here is read
/// from a request body, and there is deliberately no constructor that takes a
/// user id as a plain argument from a handler: the only way to build one is
/// from a [`CurrentUser`] the extractor produced.
#[derive(Debug, Clone)]
pub struct ReaderContext {
    user_id: Uuid,
    /// Projects this reader belongs to.
    projects: std::collections::BTreeSet<Uuid>,
}

impl ReaderContext {
    /// Load the reader's memberships from the authenticated account.
    pub async fn load(pool: &PgPool, user: &CurrentUser) -> ApiResult<Self> {
        let rows: Vec<(Uuid,)> =
            sqlx::query_as("SELECT project_id FROM project_members WHERE user_id = $1")
                .bind(user.id)
                .fetch_all(pool)
                .await?;
        Ok(Self {
            user_id: user.id,
            projects: rows.into_iter().map(|(p,)| p).collect(),
        })
    }

    #[allow(dead_code)]
    pub fn user_id(&self) -> Uuid {
        self.user_id
    }

    pub fn is_member_of(&self, project_id: Uuid) -> bool {
        self.projects.contains(&project_id)
    }
}

/// Whether this reader may see this reference, resolved per domain.
///
/// Per domain because there is no single table to ask. Project knowledge is in
/// `memories` and is checked against membership; personal knowledge is in
/// `personal_knowledge` and is checked against ownership; team knowledge is in
/// `team_knowledge` and is checked against the server's team, with `proposed`
/// rows additionally requiring author-or-administrator. A `PatternRef` resolves
/// against `shared_patterns`, whose visibility is owner-only — the table name
/// describes where it is stored, not who can see it (data-model.md §6.2).
///
/// **Shared project membership does not widen a personal record.** An
/// administrator's standing is over team guidance, not over a colleague's
/// private notes, so `AdminUser` gets no exemption here and is not a parameter.
#[allow(dead_code)]
pub async fn reference_visibility(
    pool: &PgPool,
    reader: &ReaderContext,
    reference: Reference,
) -> ApiResult<Visibility> {
    let visible = match reference {
        Reference::Knowledge(k) => match k.domain {
            KnowledgeDomain::Project => {
                // The project is a property of the row, not of the request: a
                // caller naming a project it is not a member of must not be
                // able to make the reference visible by saying so.
                let owner: Option<(Uuid,)> =
                    sqlx::query_as("SELECT project_id FROM memories WHERE id = $1")
                        .bind(k.id)
                        .fetch_optional(pool)
                        .await?;
                owner.is_some_and(|(project,)| reader.is_member_of(project))
            }
            KnowledgeDomain::Personal => {
                let owner: Option<(Uuid,)> = sqlx::query_as(
                    "SELECT owner_user_id FROM personal_knowledge
                      WHERE id = $1 AND forgotten_at IS NULL",
                )
                .bind(k.id)
                .fetch_optional(pool)
                .await?;
                owner.is_some_and(|(owner,)| owner == reader.user_id)
            }
            KnowledgeDomain::Team => {
                // Team knowledge is server-wide, so any authenticated account
                // may read a ratified or retired row. A `proposed` row is a
                // proposal and not yet guidance, so it is visible to its
                // author only until an administrator acts on it (FR-825).
                let row: Option<(String, Uuid)> = sqlx::query_as(
                    "SELECT state, proposed_by_user_id FROM team_knowledge WHERE id = $1",
                )
                .bind(k.id)
                .fetch_optional(pool)
                .await?;
                match row {
                    None => false,
                    Some((state, author)) => state != "proposed" || author == reader.user_id,
                }
            }
        },
        Reference::Pattern(p) => {
            let owner: Option<(Uuid,)> = sqlx::query_as(
                "SELECT owner_user_id FROM shared_patterns
                  WHERE pattern_id = $1 AND forgotten_at IS NULL",
            )
            .bind(p.0)
            .fetch_optional(pool)
            .await?;
            owner.is_some_and(|(owner,)| owner == reader.user_id)
        }
    };
    Ok(if visible {
        Visibility::Visible
    } else {
        Visibility::Withheld
    })
}

/// Keep only the references this reader may see, dropping the rest.
///
/// Dropping, not marking. The returned list carries no evidence that anything
/// was removed — no gap, no count, no placeholder — because the existence of a
/// withheld record is itself the disclosure FR-846a forbids.
#[allow(dead_code)]
pub async fn visible_references(
    pool: &PgPool,
    reader: &ReaderContext,
    references: &[Reference],
) -> ApiResult<Vec<Reference>> {
    let mut kept = Vec::new();
    for reference in references {
        if reference_visibility(pool, reader, *reference)
            .await?
            .is_visible()
        {
            kept.push(*reference);
        }
    }
    Ok(kept)
}

/// What a session establishes about the event that names it.
///
/// The project is **derived** from the session and never asserted by the
/// caller. A caller that could name the project could attribute its events to a
/// project it has nothing to do with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionBinding {
    pub project_id: Uuid,
    pub owner_user_id: Uuid,
}

/// Why a session could not establish a binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionBindingError {
    /// The caller cannot resolve this session at all: either it does not
    /// exist, or it exists in a project the caller does not belong to.
    ///
    /// **Deliberately one answer for both**, and this is the part that is easy
    /// to get wrong. A **request-level** `403` for non-membership and a
    /// per-item `session_not_found` for an unknown id are two visibly different
    /// responses, so a caller who could tell them apart could enumerate which
    /// session ids exist across the whole server, one guess at a time — which
    /// is exactly what answering per item was supposed to prevent (FR-894a,
    /// `contracts/safe-events.md` §7.1 step 6).
    ///
    /// A member who genuinely mistypes a session id therefore gets a `403`
    /// rather than a narrower message. That is the bluntness the guarantee
    /// costs, and it is worth it: the alternative is a probe oracle.
    Unresolvable,
    /// The caller is a member of the session's project, but the session is not
    /// theirs.
    ///
    /// Per-item, because a member already knows this project's sessions exist —
    /// they can list them — so nothing crosses a trust boundary here. What is
    /// refused is *attribution*: consolidation must not produce knowledge that
    /// credits a colleague's authorship to work they never did (FR-769a).
    NotOwned,
}

/// Resolve and authorize the session an event names (FR-769, FR-769a).
///
/// An event names its session, and a session identifier is body data. Two
/// separate things are checked here and both are required:
///
/// - the caller is a **member of the session's project**, which is a
///   request-level `403` if not; and
/// - the session **belongs to the authenticated account**, which is a per-item
///   refusal if not.
///
/// The second is the one FR-769a exists for. Without it a project member could
/// submit well-formed events naming a colleague's session, and consolidation
/// would produce durable knowledge attributing a colleague's authorship to work
/// they never did — the same class of defect as falsified proposal attribution,
/// moved to a different key. Membership alone does not cover it, because the
/// colleague is a member too.
///
/// A session whose `user_id` is NULL fails. It predates account binding, so
/// nobody can be established as its owner, and "nobody owns it" is not the same
/// as "you own it" (FR-764, Principle XI).
pub async fn bind_session(
    pool: &PgPool,
    reader: &ReaderContext,
    session_id: Uuid,
) -> ApiResult<Result<SessionBinding, SessionBindingError>> {
    let row: Option<(Uuid, Option<Uuid>)> =
        sqlx::query_as("SELECT project_id, user_id FROM sessions WHERE id = $1")
            .bind(session_id)
            .fetch_optional(pool)
            .await?;
    let Some((project_id, owner)) = row else {
        // Indistinguishable from non-membership, on purpose. See
        // `SessionBindingError::Unresolvable`.
        return Ok(Err(SessionBindingError::Unresolvable));
    };
    if !reader.is_member_of(project_id) {
        return Ok(Err(SessionBindingError::Unresolvable));
    }
    match owner {
        Some(owner) if owner == reader.user_id => Ok(Ok(SessionBinding {
            project_id,
            owner_user_id: owner,
        })),
        // Includes a session whose `user_id` is NULL: it predates account
        // binding, so nobody can be established as its owner, and "nobody owns
        // it" is not "you own it" (FR-764, Principle XI).
        _ => Ok(Err(SessionBindingError::NotOwned)),
    }
}

/// The readership a domain declares, so a caller can state it without
/// re-deriving it.
#[allow(dead_code)]
pub fn declared_readership(reference: Reference) -> Readership {
    reference.readership()
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

    #[test]
    fn a_malformed_hash_does_not_verify_any_password() {
        // Garbage must not panic, just return false.
        assert!(!verify_password("anything", "not-a-valid-hash"));
        assert!(!verify_password("anything", ""));
        assert!(!verify_password("", "$argon2id$garbage"));
    }

    #[test]
    fn random_tokens_are_unique() {
        let a = random_token();
        let b = random_token();
        assert_ne!(a, b, "two tokens must not collide");
    }

    #[test]
    fn hash_token_is_deterministic_and_distinct() {
        let t = "test-token-value";
        assert_eq!(hash_token(t), hash_token(t), "same token → same hash");
        assert_ne!(
            hash_token("a"),
            hash_token("b"),
            "different tokens → different hashes"
        );
    }
}
