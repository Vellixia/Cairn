//! Per-namespace synchronization state (`sync_cursor`, D426, D427).
//!
//! `sync_meta` (`crates/cairn-store/migrations/0001_init.sql`) was keyed
//! `project_id TEXT PRIMARY KEY` — one row, one pull cursor, one backoff clock,
//! per project. That shape cannot hold `personal:*` or `team:*` state at all,
//! and even restricted to `project:*` it forced every namespace sharing one
//! process to retry on the same clock (`contracts/sync-namespaces.md` §4).
//!
//! `sync_cursor` fixes both: it is keyed by the namespace's own `key()`
//! (`project:<uuid>` | `personal:<instance>:<user>` | `team:<instance>`,
//! `cairn_core::domain::SyncNamespace`), so advancing or backing off one
//! namespace writes exactly one row and never touches another (Invariant 1,
//! FR-487). `sync_meta` itself is untouched by this module — migration 0007
//! backfills it into `sync_cursor` once, and nothing here reads it again.

use crate::{rows, Result, Store};
use cairn_core::domain::SyncNamespace;
use chrono::{DateTime, Utc};
use sqlx::Row;
use uuid::Uuid;

/// Establish a namespace's row, so the drain/pull loop can see it exists.
///
/// **This is what makes a consume-only lane work at all** (FR-489,
/// `sync-namespaces.md` §5). Namespace discovery used to come from
/// `outbox::known_namespaces` alone — the set of namespaces with queued work —
/// which is empty on a machine that has never written personal or team
/// knowledge of its own. Such a machine would never pull, so an admin's
/// ratification would never reach the member who only ever reads team guidance.
/// Personal and team knowledge are the first content a machine can legitimately
/// only ever consume; every earlier entity type could at least in principle be
/// produced locally, which is why this gap did not exist before.
///
/// Idempotent: called on every link and every authentication, and a namespace
/// already established keeps its cursor, its backoff and its last-known server
/// capability rather than being reset to a fresh lane.
pub async fn establish(store: &Store, namespace: &SyncNamespace) -> Result<()> {
    sqlx::query(
        "INSERT INTO sync_cursor (namespace) VALUES (?1) ON CONFLICT(namespace) DO NOTHING",
    )
    .bind(namespace.key())
    .execute(store.pool())
    .await?;
    Ok(())
}

/// Move one namespace's state to a new key, keeping everything it holds.
///
/// Used once, for one purpose: a lane opened against a server that had not yet
/// told this client its instance id is re-keyed the moment it does (see
/// `crates/cairnd/src/sync.rs`'s `establish_global_namespaces`). The cursor, the
/// backoff and the last-known capability all belong to the same peer before and
/// after, so they move rather than being reset — a re-keyed lane that started
/// over would re-pull the peer's whole history.
///
/// A no-op when `from` does not exist. When `to` already exists, `from` is
/// dropped rather than merged: `to` is the authoritative key, and its cursor is
/// the one derived from the id the server actually reported.
pub async fn rename(store: &Store, from: &SyncNamespace, to: &SyncNamespace) -> Result<()> {
    if from.key() == to.key() {
        return Ok(());
    }
    // **The destination wins a collision, and `UPDATE OR REPLACE` gets that
    // backwards.**
    //
    // `OR REPLACE` resolves a primary-key conflict by *deleting the destination*
    // and letting the source take its place — so a store that already held a
    // real `team:<instance>` lane and also a provisional one (the endpoint stopped
    // reporting its instance for a while, then recovered) would have its
    // authoritative pull cursor, backoff and capability state replaced by the
    // provisional row's. The visible cost is a full-history replay from a cursor
    // that had already advanced; the doc above promised the opposite.
    //
    // So: rename only into a key that is free, and otherwise drop the source. The
    // destination is derived from the id the server actually reported, which is
    // the better answer by construction.
    let occupied: Option<i64> =
        sqlx::query_scalar("SELECT 1 FROM sync_cursor WHERE namespace = ?1")
            .bind(to.key())
            .fetch_optional(store.pool())
            .await?;
    if occupied.is_none() {
        sqlx::query("UPDATE sync_cursor SET namespace = ?2 WHERE namespace = ?1")
            .bind(from.key())
            .bind(to.key())
            .execute(store.pool())
            .await?;
    }
    sqlx::query("DELETE FROM sync_cursor WHERE namespace = ?1")
        .bind(from.key())
        .execute(store.pool())
        .await?;
    Ok(())
}

/// Every namespace this store knows about, whether or not it has queued work.
///
/// Rows whose key does not parse are skipped rather than reported: the only way
/// one exists is a hand-edited database, and a lane nobody can name is a lane
/// nobody can drain — there is nothing a caller could do with the news.
pub async fn established(store: &Store) -> Result<Vec<SyncNamespace>> {
    let keys: Vec<String> =
        sqlx::query_scalar("SELECT namespace FROM sync_cursor ORDER BY namespace")
            .fetch_all(store.pool())
            .await?;
    Ok(keys.iter().filter_map(|k| parse(k)).collect())
}

/// Recover a [`SyncNamespace`] from its own [`SyncNamespace::key`].
///
/// `key()` is one-way by intent — it is a cursor key, not a wire format — but
/// `sync_cursor` stores only the key, so reading the table back requires
/// exactly one parser. This is it, and it lives beside the table it reads
/// rather than being duplicated per caller.
pub fn parse(key: &str) -> Option<SyncNamespace> {
    if let Some(rest) = key.strip_prefix("project:") {
        return Some(SyncNamespace::Project(Uuid::parse_str(rest).ok()?));
    }
    if let Some(rest) = key.strip_prefix("personal:") {
        let (instance, user) = rest.split_once(':')?;
        return Some(SyncNamespace::Personal(
            Uuid::parse_str(instance).ok()?,
            Uuid::parse_str(user).ok()?,
        ));
    }
    if let Some(rest) = key.strip_prefix("team:") {
        return Some(SyncNamespace::Team(Uuid::parse_str(rest).ok()?));
    }
    None
}

/// The pull position for one namespace, or `None` before its first successful
/// pull — exactly the state a fresh `personal:*` or `team:*` namespace starts
/// in, mirroring how a newly linked project has no `project:*` row yet either.
pub async fn pull_cursor(store: &Store, namespace: &SyncNamespace) -> Result<Option<String>> {
    let row = sqlx::query("SELECT pull_cursor FROM sync_cursor WHERE namespace = ?1")
        .bind(namespace.key())
        .fetch_optional(store.pool())
        .await?;
    Ok(row.and_then(|r| r.try_get::<Option<String>, _>("pull_cursor").ok().flatten()))
}

/// Advance one namespace's pull cursor.
///
/// `INSERT ... ON CONFLICT` touches only the `pull_cursor` column of this one
/// namespace's row — a `personal:*` cursor advancing has no effect on
/// `project:*` or `team:*`'s rows because they are different primary keys in
/// this table, not different columns of one row (Invariant 1, FR-487).
pub async fn set_pull_cursor(store: &Store, namespace: &SyncNamespace, cursor: &str) -> Result<()> {
    sqlx::query(
        "INSERT INTO sync_cursor (namespace, pull_cursor) VALUES (?1, ?2)
         ON CONFLICT(namespace) DO UPDATE SET pull_cursor = ?2",
    )
    .bind(namespace.key())
    .bind(cursor)
    .execute(store.pool())
    .await?;
    Ok(())
}

/// Forget one namespace's pull position, so the next pull re-reads the lane
/// from the beginning.
///
/// Used when the *meaning* of the stored cursor has lapsed rather than when a
/// pull failed — see `visibility_context`. A held cursor re-delivers one page;
/// this re-delivers the lane.
pub async fn clear_pull_cursor(store: &Store, namespace: &SyncNamespace) -> Result<()> {
    sqlx::query("UPDATE sync_cursor SET pull_cursor = NULL WHERE namespace = ?1")
        .bind(namespace.key())
        .execute(store.pool())
        .await?;
    Ok(())
}

/// The visibility context under which this namespace's cursor was last
/// advanced, as the server itself reported it.
///
/// See the column comment in `0007_collaborative_global_memory.sql` for why a
/// `team:*` cursor needs this and a `personal:*` cursor does not.
pub async fn visibility_context(
    store: &Store,
    namespace: &SyncNamespace,
) -> Result<Option<String>> {
    let row = sqlx::query("SELECT visibility_context FROM sync_cursor WHERE namespace = ?1")
        .bind(namespace.key())
        .fetch_optional(store.pool())
        .await?;
    Ok(row.and_then(|r| {
        r.try_get::<Option<String>, _>("visibility_context")
            .ok()
            .flatten()
    }))
}

pub async fn set_visibility_context(
    store: &Store,
    namespace: &SyncNamespace,
    context: &str,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO sync_cursor (namespace, visibility_context) VALUES (?1, ?2)
         ON CONFLICT(namespace) DO UPDATE SET visibility_context = ?2",
    )
    .bind(namespace.key())
    .bind(context)
    .execute(store.pool())
    .await?;
    Ok(())
}

pub async fn last_success_at(
    store: &Store,
    namespace: &SyncNamespace,
) -> Result<Option<DateTime<Utc>>> {
    let row = sqlx::query("SELECT last_success_at FROM sync_cursor WHERE namespace = ?1")
        .bind(namespace.key())
        .fetch_optional(store.pool())
        .await?;
    match row {
        None => Ok(None),
        Some(r) => rows::opt_ts(&r, "last_success_at"),
    }
}

pub async fn record_success(store: &Store, namespace: &SyncNamespace) -> Result<()> {
    sqlx::query(
        "INSERT INTO sync_cursor (namespace, last_success_at) VALUES (?1, ?2)
         ON CONFLICT(namespace) DO UPDATE SET last_success_at = ?2",
    )
    .bind(namespace.key())
    .bind(rows::now_text())
    .execute(store.pool())
    .await?;
    Ok(())
}

/// What the server last said it could hold, for this namespace's peer.
///
/// A `project:*` and a `team:*` namespace against the same server can disagree
/// here in principle (a capability probe races differently per namespace), so
/// this is stored per namespace rather than assumed shared — the same reason
/// backoff is (below).
pub async fn server_capability(store: &Store, namespace: &SyncNamespace) -> Result<Option<String>> {
    let row = sqlx::query("SELECT server_capability FROM sync_cursor WHERE namespace = ?1")
        .bind(namespace.key())
        .fetch_optional(store.pool())
        .await?;
    Ok(row.and_then(|r| {
        r.try_get::<Option<String>, _>("server_capability")
            .ok()
            .flatten()
    }))
}

pub async fn set_server_capability(
    store: &Store,
    namespace: &SyncNamespace,
    capability: &str,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO sync_cursor (namespace, server_capability) VALUES (?1, ?2)
         ON CONFLICT(namespace) DO UPDATE SET server_capability = ?2",
    )
    .bind(namespace.key())
    .bind(capability)
    .execute(store.pool())
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn project_ns() -> SyncNamespace {
        SyncNamespace::Project(Uuid::now_v7())
    }

    fn personal_ns() -> SyncNamespace {
        SyncNamespace::Personal(Uuid::now_v7(), Uuid::now_v7())
    }

    fn team_ns() -> SyncNamespace {
        SyncNamespace::Team(Uuid::now_v7())
    }

    #[tokio::test]
    async fn a_fresh_namespace_has_no_cursor() {
        let store = Store::open_memory().await.unwrap();
        assert_eq!(pull_cursor(&store, &project_ns()).await.unwrap(), None);
    }

    /// Invariant 1 (FR-487): advancing one namespace's cursor writes exactly
    /// its own row and never another's.
    #[tokio::test]
    async fn advancing_one_namespace_cursor_never_touches_another() {
        let store = Store::open_memory().await.unwrap();
        let project = project_ns();
        let personal = personal_ns();
        let team = team_ns();

        set_pull_cursor(&store, &project, "2026-01-01T00:00:00Z")
            .await
            .unwrap();
        set_pull_cursor(&store, &personal, "2026-02-02T00:00:00Z")
            .await
            .unwrap();

        assert_eq!(
            pull_cursor(&store, &project).await.unwrap().as_deref(),
            Some("2026-01-01T00:00:00Z")
        );
        assert_eq!(
            pull_cursor(&store, &personal).await.unwrap().as_deref(),
            Some("2026-02-02T00:00:00Z")
        );
        // Never written at all: still absent, not merely unequal to the others.
        assert_eq!(pull_cursor(&store, &team).await.unwrap(), None);

        // Re-advancing the project cursor still leaves personal's alone.
        set_pull_cursor(&store, &project, "2026-03-03T00:00:00Z")
            .await
            .unwrap();
        assert_eq!(
            pull_cursor(&store, &personal).await.unwrap().as_deref(),
            Some("2026-02-02T00:00:00Z"),
            "advancing project's cursor moved personal's"
        );
    }

    /// A rename into an occupied key keeps the **destination**.
    ///
    /// `UPDATE OR REPLACE` resolved the collision the other way: it deleted the
    /// destination and let the provisional row take its place, replacing an
    /// authoritative pull cursor, backoff and capability state with stale
    /// provisional values — and the doc comment promised the opposite. The visible
    /// cost is a full-history replay from a cursor that had already advanced.
    ///
    /// Reachable whenever an endpoint stops reporting its instance id for a while
    /// and later recovers: the provisional lane opens beside the real one.
    ///
    /// Falsified by restoring `UPDATE OR REPLACE`.
    #[tokio::test]
    async fn renaming_into_an_occupied_key_keeps_the_destinations_state() {
        let store = Store::open_memory().await.unwrap();
        let user = Uuid::now_v7();
        let provisional = SyncNamespace::Personal(Uuid::now_v7(), user);
        let real = SyncNamespace::Personal(Uuid::now_v7(), user);

        set_pull_cursor(&store, &provisional, "1970-01-01T00:00:00Z")
            .await
            .unwrap();
        set_pull_cursor(&store, &real, "2026-08-01T00:00:00Z")
            .await
            .unwrap();
        set_server_capability(&store, &real, "schema=3;capabilities=team_knowledge")
            .await
            .unwrap();

        rename(&store, &provisional, &real).await.unwrap();

        assert_eq!(
            pull_cursor(&store, &real).await.unwrap().as_deref(),
            Some("2026-08-01T00:00:00Z"),
            "the provisional cursor replaced the authoritative one, forcing a \
             full-history replay"
        );
        assert_eq!(
            server_capability(&store, &real).await.unwrap().as_deref(),
            Some("schema=3;capabilities=team_knowledge"),
            "the destination's capability state was lost"
        );
        assert_eq!(
            pull_cursor(&store, &provisional).await.unwrap(),
            None,
            "the provisional row was left behind"
        );
    }

    /// A rename into a free key still moves everything the lane holds.
    #[tokio::test]
    async fn renaming_into_a_free_key_carries_the_lanes_state_across() {
        let store = Store::open_memory().await.unwrap();
        let user = Uuid::now_v7();
        let provisional = SyncNamespace::Personal(Uuid::now_v7(), user);
        let real = SyncNamespace::Personal(Uuid::now_v7(), user);

        set_pull_cursor(&store, &provisional, "2026-07-01T00:00:00Z")
            .await
            .unwrap();
        set_server_capability(&store, &provisional, "schema=2;capabilities=")
            .await
            .unwrap();

        rename(&store, &provisional, &real).await.unwrap();

        assert_eq!(
            pull_cursor(&store, &real).await.unwrap().as_deref(),
            Some("2026-07-01T00:00:00Z"),
            "the lane's cursor did not move with it"
        );
        assert_eq!(
            server_capability(&store, &real).await.unwrap().as_deref(),
            Some("schema=2;capabilities=")
        );
        assert_eq!(pull_cursor(&store, &provisional).await.unwrap(), None);
    }

    #[tokio::test]
    async fn capability_and_success_round_trip_per_namespace() {
        let store = Store::open_memory().await.unwrap();
        let personal = personal_ns();

        assert_eq!(last_success_at(&store, &personal).await.unwrap(), None);
        record_success(&store, &personal).await.unwrap();
        assert!(last_success_at(&store, &personal).await.unwrap().is_some());

        assert_eq!(server_capability(&store, &personal).await.unwrap(), None);
        set_server_capability(&store, &personal, "schema=3;capabilities=team_knowledge")
            .await
            .unwrap();
        assert_eq!(
            server_capability(&store, &personal)
                .await
                .unwrap()
                .as_deref(),
            Some("schema=3;capabilities=team_knowledge")
        );
    }

    /// `Personal` keys on both the server instance and the account (D438,
    /// FR-568): two identities that share a user id but differ in server
    /// instance must not collide on one cursor row.
    #[tokio::test]
    async fn personal_namespaces_partition_by_instance_and_account() {
        let store = Store::open_memory().await.unwrap();
        let user = Uuid::now_v7();
        let server_a = SyncNamespace::Personal(Uuid::now_v7(), user);
        let server_b = SyncNamespace::Personal(Uuid::now_v7(), user);

        set_pull_cursor(&store, &server_a, "a").await.unwrap();
        set_pull_cursor(&store, &server_b, "b").await.unwrap();

        assert_eq!(
            pull_cursor(&store, &server_a).await.unwrap().as_deref(),
            Some("a")
        );
        assert_eq!(
            pull_cursor(&store, &server_b).await.unwrap().as_deref(),
            Some("b")
        );
    }
}
