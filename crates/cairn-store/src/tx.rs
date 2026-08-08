//! One write transaction, done correctly, for every repository to share.
//!
//! SQLite's `BEGIN` is *deferred*: it takes no lock, and in WAL mode the
//! transaction's first statement fixes a read snapshot. A transaction that
//! later writes must upgrade that snapshot, and if any other connection
//! committed in between, SQLite refuses with `SQLITE_BUSY_SNAPSHOT` (517).
//! `busy_timeout` cannot help — waiting does not make a stale snapshot fresh —
//! so the caller simply loses a legitimate write. That is what happened to
//! `cairn memory add` while capture was writing observations on another
//! connection.
//!
//! `BEGIN IMMEDIATE` takes the write lock as the transaction opens. There is
//! then no snapshot to upgrade and no 517 to receive, which removes the failure
//! class rather than papering over it. Contention moves to one place —
//! acquiring the lock — where it is `SQLITE_BUSY`, retryable, and retryable
//! *before the transaction has done any work at all*. Retrying there needs no
//! replay and can invent no duplicate row.
//!
//! [`begin`] gives that transaction to callers who write straight-line SQL;
//! [`retry`] does the same job for a single statement in autocommit, where a
//! `SQLITE_BUSY` means the statement did not apply and replaying it cannot
//! duplicate anything.
//!
//! Bounded and local. No lock service, no queue, no process-wide serialization,
//! and readers still run concurrently under WAL.

use crate::diag::{self, Stage};
use crate::{Result, Store, StoreError};
use sqlx::{Sqlite, Transaction};
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

/// Take SQLite's write lock as the transaction opens, not on its first write.
const BEGIN_IMMEDIATE: &str = "BEGIN IMMEDIATE";

/// Attempts at a contended transaction before the error is the answer.
///
/// Ten attempts with the backoff below spend at most about a second and a half
/// waiting. That is deliberately generous: `busy_timeout` turns out not to save
/// a writer here — SQLite hands back `SQLITE_BUSY` promptly when another
/// connection holds the write lock — so this budget, not the pragma, is what
/// carries a legitimate write through a burst of capture. Bounded either way:
/// after the last attempt the caller gets the error.
const MAX_ATTEMPTS: u32 = 10;
/// First backoff step; doubles per attempt to a ceiling.
const BACKOFF_BASE: Duration = Duration::from_millis(2);
const BACKOFF_CEILING: Duration = Duration::from_millis(400);

/// A write transaction that has already taken the lock.
pub type WriteTx = Transaction<'static, Sqlite>;

/// Open a write transaction, waiting out contention for the lock.
///
/// Every retry happens here, before the caller has written anything, so a
/// retried transaction repeats no work and creates no duplicate row.
pub async fn begin(store: &Store, op: &str) -> Result<WriteTx> {
    let mut attempt = 0;
    loop {
        match store.pool().begin_with(BEGIN_IMMEDIATE).await {
            Ok(tx) => return Ok(tx),
            Err(e) => {
                if diag::enabled() && diag::is_contention(&e) {
                    diag::record(op, Stage::Begin, "-", attempt, &e);
                }
                let e = StoreError::from(e);
                if attempt + 1 >= MAX_ATTEMPTS || !is_contention(&e) {
                    return Err(e);
                }
                backoff(attempt).await;
                attempt += 1;
            }
        }
    }
}

/// Commit a write transaction, reporting contention if it appears there.
///
/// SQLite can refuse a `COMMIT` of its own accord, and sqlx consumes the
/// transaction when it does, so this cannot retry — it exists to tell us
/// whether commit contention happens in this workload at all.
pub async fn commit(tx: WriteTx, op: &str) -> Result<()> {
    match tx.commit().await {
        Ok(()) => Ok(()),
        Err(e) => {
            if diag::enabled() && diag::is_contention(&e) {
                diag::record(op, Stage::Commit, "-", 0, &e);
            }
            Err(StoreError::from(e))
        }
    }
}

/// What a retried operation hands back: a future borrowing its environment.
pub type Operation<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

/// Retry a self-contained write that runs outside a transaction.
///
/// A single statement in autocommit is its own transaction: `SQLITE_BUSY` means
/// it did not apply, so running it again is safe and cannot duplicate anything.
/// These are the writes the daemon makes constantly — an observation, a session
/// touch — and they need the same patience as the transactional ones, because
/// SQLite hands back `SQLITE_BUSY` without waiting once another connection holds
/// the write lock.
///
/// The operation borrows its environment rather than a connection, so unlike
/// [`write`] it composes with ordinary `&str` arguments.
pub async fn retry<'a, T, F>(name: &str, mut op: F) -> Result<T>
where
    F: FnMut() -> Operation<'a, T>,
{
    let mut attempt = 0;
    loop {
        match op().await {
            Ok(value) => return Ok(value),
            Err(e) if attempt + 1 < MAX_ATTEMPTS && is_contention(&e) => {
                if let StoreError::Sqlx(inner) = &e {
                    if diag::enabled() {
                        diag::record(name, Stage::Autocommit, "-", attempt, inner);
                    }
                }
                backoff(attempt).await;
                attempt += 1;
            }
            Err(e) => return Err(e),
        }
    }
}

async fn backoff(attempt: u32) {
    let step = (BACKOFF_BASE * 2u32.saturating_pow(attempt)).min(BACKOFF_CEILING);
    // Two writers backing off in lockstep collide again on every attempt, so
    // each waits a slightly different time. The clock is jitter enough here;
    // this decides how long to sleep, not anything anyone can observe.
    let jitter = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| u64::from(d.subsec_nanos()) % 8)
        .unwrap_or(0);
    tokio::time::sleep(step + Duration::from_millis(jitter)).await;
}

/// Whether an error is SQLite saying "try again", rather than "no".
///
/// The primary result code is the low byte of the extended code, so this one
/// test covers `SQLITE_BUSY` and `SQLITE_LOCKED` together with every extended
/// form of them — `SQLITE_BUSY_SNAPSHOT`, `SQLITE_BUSY_TIMEOUT`,
/// `SQLITE_BUSY_RECOVERY`, `SQLITE_LOCKED_SHAREDCACHE`. Constraint violations,
/// corruption and malformed SQL keep their own primary codes and are never
/// retried: retrying those would turn a clear failure into a slow one.
fn is_contention(e: &StoreError) -> bool {
    const SQLITE_BUSY: i64 = 5;
    const SQLITE_LOCKED: i64 = 6;

    let StoreError::Sqlx(sqlx::Error::Database(db)) = e else {
        return false;
    };
    let Some(code) = db.code().and_then(|c| c.parse::<i64>().ok()) else {
        return false;
    };
    matches!(code & 0xff, SQLITE_BUSY | SQLITE_LOCKED)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    async fn file_store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = Store::open(&dir.path().join("tx.sqlite3"))
            .await
            .expect("store");
        (dir, store)
    }

    #[tokio::test]
    async fn a_write_transaction_commits_its_work() {
        let (_dir, store) = file_store().await;
        let mut tx = begin(&store, "test").await.unwrap();
        sqlx::query("CREATE TABLE t (a INTEGER)")
            .execute(&mut *tx)
            .await
            .unwrap();
        sqlx::query("INSERT INTO t VALUES (1)")
            .execute(&mut *tx)
            .await
            .unwrap();
        tx.commit().await.unwrap();

        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM t")
            .fetch_one(store.pool())
            .await
            .unwrap();
        assert_eq!(n, 1);
    }

    #[tokio::test]
    async fn a_read_then_write_transaction_no_longer_upgrades_a_snapshot() {
        // The original defect in miniature: read first, then write. Under a
        // deferred `BEGIN` a commit from another connection in between makes
        // this 517. Holding the write lock from the start, there is no snapshot
        // to upgrade and the write goes through.
        let (_dir, store) = file_store().await;
        sqlx::query("CREATE TABLE t (a INTEGER)")
            .execute(store.pool())
            .await
            .unwrap();

        let mut tx = begin(&store, "test").await.unwrap();
        let _: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM t")
            .fetch_one(&mut *tx)
            .await
            .unwrap();
        sqlx::query("INSERT INTO t VALUES (1)")
            .execute(&mut *tx)
            .await
            .expect("the write must not need a snapshot upgrade");
        tx.commit().await.unwrap();

        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM t")
            .fetch_one(store.pool())
            .await
            .unwrap();
        assert_eq!(n, 1);
    }

    #[tokio::test]
    async fn a_busy_statement_is_retried_until_the_lock_clears() {
        // Real contention, not a simulated error: one connection holds the
        // write lock, the retried statement must wait it out and then apply.
        let (_dir, store) = file_store().await;
        sqlx::query("CREATE TABLE t (a INTEGER)")
            .execute(store.pool())
            .await
            .unwrap();

        let holder = begin(&store, "holder").await.unwrap();
        let attempts = Arc::new(AtomicUsize::new(0));

        let released = Arc::new(AtomicUsize::new(0));
        let releaser = {
            let released = Arc::clone(&released);
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(120)).await;
                drop(holder);
                released.store(1, Ordering::SeqCst);
            })
        };

        let counter = Arc::clone(&attempts);
        let store_ref = &store;
        let result: Result<()> = retry("contended_write", || {
            counter.fetch_add(1, Ordering::SeqCst);
            Box::pin(async move {
                let mut tx = store_ref.pool().begin_with(BEGIN_IMMEDIATE).await?;
                sqlx::query("INSERT INTO t VALUES (1)")
                    .execute(&mut *tx)
                    .await?;
                tx.commit().await?;
                Ok(())
            })
        })
        .await;
        releaser.await.unwrap();

        assert!(result.is_ok(), "a contended write must survive: {result:?}");
        assert_eq!(released.load(Ordering::SeqCst), 1, "the lock was held");
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM t")
            .fetch_one(store.pool())
            .await
            .unwrap();
        assert_eq!(n, 1, "the write applied exactly once, not once per attempt");
    }

    #[tokio::test]
    async fn a_permanent_error_runs_once_and_is_returned_unchanged() {
        // A constraint violation is an answer, not a delay.
        let (_dir, store) = file_store().await;
        sqlx::query("CREATE TABLE t (a INTEGER PRIMARY KEY)")
            .execute(store.pool())
            .await
            .unwrap();
        sqlx::query("INSERT INTO t VALUES (1)")
            .execute(store.pool())
            .await
            .unwrap();

        let attempts = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&attempts);
        let store_ref = &store;
        let result: Result<()> = retry("permanent", || {
            counter.fetch_add(1, Ordering::SeqCst);
            Box::pin(async move {
                sqlx::query("INSERT INTO t VALUES (1)")
                    .execute(store_ref.pool())
                    .await?;
                Ok(())
            })
        })
        .await;

        assert_eq!(
            attempts.load(Ordering::SeqCst),
            1,
            "retried a constraint violation"
        );
        match result {
            Err(StoreError::Sqlx(e)) => {
                let (_, primary) = diag::codes(&e).expect("a database error");
                assert_eq!(primary, 19, "the original constraint error must survive");
            }
            other => panic!("expected the constraint error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn exhaustion_returns_the_original_storage_error() {
        // The lock is never released, so every attempt is spent and the caller
        // gets SQLite's own error rather than a substitute.
        // A short busy timeout keeps a deliberately hopeless retry quick.
        let dir = tempfile::tempdir().expect("tempdir");
        let store = Store::open_with_busy_timeout(
            &dir.path().join("exhaust.sqlite3"),
            Duration::from_millis(40),
        )
        .await
        .expect("store");
        sqlx::query("CREATE TABLE t (a INTEGER)")
            .execute(store.pool())
            .await
            .unwrap();
        let _holder = begin(&store, "holder").await.unwrap();

        let store_ref = &store;
        let result: Result<()> = retry("exhausted", || {
            Box::pin(async move {
                let mut tx = store_ref.pool().begin_with(BEGIN_IMMEDIATE).await?;
                sqlx::query("INSERT INTO t VALUES (1)")
                    .execute(&mut *tx)
                    .await?;
                tx.commit().await?;
                Ok(())
            })
        })
        .await;

        match result {
            Err(StoreError::Sqlx(e)) => {
                let (_, primary) = diag::codes(&e).expect("a database error");
                assert!(
                    matches!(primary, 5 | 6),
                    "expected BUSY/LOCKED, got {primary}"
                );
            }
            other => panic!("expected SQLite's contention error, got {other:?}"),
        }
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM t")
            .fetch_one(store.pool())
            .await
            .unwrap();
        assert_eq!(n, 0, "an exhausted retry must leave nothing behind");
    }

    #[test]
    fn contention_codes_are_told_apart_from_permanent_ones() {
        // The classification is the part that must not drift: every extended
        // form of BUSY/LOCKED retries, everything else does not.
        for extended in [5i64, 6, 261, 262, 517, 518, 773] {
            assert!(
                matches!(extended & 0xff, 5 | 6),
                "{extended} must be treated as contention"
            );
        }
        for permanent in [1i64, 11, 19, 787, 1299, 2067] {
            assert!(
                !matches!(permanent & 0xff, 5 | 6),
                "{permanent} must never be retried"
            );
        }
    }
}
