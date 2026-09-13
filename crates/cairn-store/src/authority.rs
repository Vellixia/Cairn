//! Which side this store treats as authoritative for durable knowledge
//! (`data-model.md` §5, `contracts/migration-cutover.md` §1, FR-876).
//!
//! One row, `authority_mode`, moving `feature_004` → `migrating` →
//! `server_authoritative`. It is the store's own answer and not the server's:
//! a client may reach `server_authoritative` locally before its server cuts
//! over (FR-876a) or after (FR-876d), so a single global flag could not express
//! the state either side is actually in.
//!
//! Read by every explicit knowledge mutation, because the mode decides whether
//! a write is a local durable record or a command the server will accept or
//! refuse (FR-712).

use crate::{rows, Result, Store, StoreError};
use std::fmt;
use std::str::FromStr;

/// Where this store believes durable knowledge lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorityMode {
    /// Feature 004's arrangement: the local store is authoritative and writes
    /// are local records that sync afterwards.
    Feature004,
    /// Migration is under way. Knowledge is being handed over and the store is
    /// in neither end state.
    Migrating,
    /// The server owns durable knowledge. A local mutation is a **request**,
    /// never a local write the server later discovers (FR-712).
    ServerAuthoritative,
}

impl AuthorityMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            AuthorityMode::Feature004 => "feature_004",
            AuthorityMode::Migrating => "migrating",
            AuthorityMode::ServerAuthoritative => "server_authoritative",
        }
    }

    /// Whether an explicit knowledge mutation must become a command.
    ///
    /// True only in the end state. During `migrating` the local path still
    /// stands: migration is moving what already exists, and turning new writes
    /// into commands half way through would leave a store whose recent
    /// knowledge went one way and whose older knowledge went another.
    pub fn commands_are_authoritative(&self) -> bool {
        matches!(self, AuthorityMode::ServerAuthoritative)
    }
}

impl fmt::Display for AuthorityMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for AuthorityMode {
    type Err = StoreError;
    fn from_str(s: &str) -> Result<Self> {
        match s {
            "feature_004" => Ok(AuthorityMode::Feature004),
            "migrating" => Ok(AuthorityMode::Migrating),
            "server_authoritative" => Ok(AuthorityMode::ServerAuthoritative),
            other => Err(StoreError::Corrupt(format!("authority mode {other}"))),
        }
    }
}

/// This store's current mode.
///
/// The row is seeded by migration 8, so its absence is corruption rather than a
/// default worth inventing: guessing `feature_004` would let a store that had
/// cut over quietly resume writing local durable records.
pub async fn mode(store: &Store) -> Result<AuthorityMode> {
    let found: Option<String> = sqlx::query_scalar("SELECT mode FROM authority_mode WHERE id = 1")
        .fetch_optional(store.pool())
        .await?;
    found
        .ok_or_else(|| StoreError::Corrupt("authority_mode has no row".into()))?
        .parse()
}

/// Move this store to a new mode.
///
/// Forward only, and refused otherwise. The sequence is a migration, and a
/// store that could go back to `feature_004` after cutting over would start
/// authoring local records the server already holds — the canonical fork
/// FR-787 forbids.
pub async fn set_mode(store: &Store, next: AuthorityMode) -> Result<()> {
    let current = mode(store).await?;
    let ordered = |m: AuthorityMode| match m {
        AuthorityMode::Feature004 => 0,
        AuthorityMode::Migrating => 1,
        AuthorityMode::ServerAuthoritative => 2,
    };
    if ordered(next) < ordered(current) {
        return Err(StoreError::Corrupt(format!(
            "authority mode cannot move from {current} back to {next}"
        )));
    }
    sqlx::query("UPDATE authority_mode SET mode = ?1, changed_at = ?2 WHERE id = 1")
        .bind(next.as_str())
        .bind(rows::now_text())
        .execute(store.pool())
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn store() -> Store {
        Store::open_memory().await.expect("store")
    }

    #[tokio::test]
    async fn a_fresh_store_starts_where_feature_004_left_it() {
        let s = store().await;
        assert_eq!(mode(&s).await.unwrap(), AuthorityMode::Feature004);
        assert!(!mode(&s).await.unwrap().commands_are_authoritative());
    }

    #[tokio::test]
    async fn the_mode_moves_forward_and_refuses_to_move_back() {
        let s = store().await;
        set_mode(&s, AuthorityMode::Migrating).await.unwrap();
        set_mode(&s, AuthorityMode::ServerAuthoritative)
            .await
            .unwrap();
        assert!(mode(&s).await.unwrap().commands_are_authoritative());

        // Going back would let a cut-over store author local records the server
        // already holds, which is the canonical fork FR-787 forbids.
        for backwards in [AuthorityMode::Migrating, AuthorityMode::Feature004] {
            assert!(
                set_mode(&s, backwards).await.is_err(),
                "the mode went backwards to {backwards}"
            );
        }
        assert_eq!(mode(&s).await.unwrap(), AuthorityMode::ServerAuthoritative);
    }

    #[tokio::test]
    async fn only_the_end_state_makes_commands_authoritative() {
        // Migration is moving what already exists. Turning new writes into
        // commands half way through would leave a store whose recent knowledge
        // went one way and whose older knowledge went another.
        assert!(!AuthorityMode::Feature004.commands_are_authoritative());
        assert!(!AuthorityMode::Migrating.commands_are_authoritative());
        assert!(AuthorityMode::ServerAuthoritative.commands_are_authoritative());
    }
}
