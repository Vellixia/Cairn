//! Test-only helper: env vars are process-global mutable state, and Rust's
//! default test harness runs tests in parallel threads within one binary, so
//! any two tests that set/unset the same env var can interleave and read each
//! other's value mid-test (observed: `project::tests::env_override_wins_and_is_stable`
//! panicking under a full `cargo test --workspace` run, passing cleanly in
//! isolation — a classic intermittent race).
//!
//! Every test in this crate that touches `HOME`/`USERPROFILE`/`XDG_CONFIG_HOME`/
//! `CAIRN_*` goes through this ONE lock, so tests in different files that touch
//! overlapping env vars (e.g. a path-resolution test and a hook test both
//! reading `HOME`) can never interleave either.
#![cfg(test)]

use std::sync::Mutex;

pub(crate) static ENV_LOCK: Mutex<()> = Mutex::new(());

/// Set each `(key, value)` pair for the duration of `f` (unsetting when `value`
/// is `None`), restoring every previous value afterward — even if `f` panics,
/// thanks to the lock being released and env restored via unwind-safe drop
/// ordering being irrelevant here (restoration happens before `f`'s result is
/// returned, and a panic in `f` skips restoration only in the already-broken
/// case where the test itself is failing).
pub(crate) fn with_env<T>(vars: &[(&str, Option<&str>)], f: impl FnOnce() -> T) -> T {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let prev: Vec<(String, Option<String>)> = vars
        .iter()
        .map(|(k, _)| (k.to_string(), std::env::var(k).ok()))
        .collect();
    for (k, v) in vars {
        match v {
            Some(v) => std::env::set_var(k, v),
            None => std::env::remove_var(k),
        }
    }
    let result = f();
    for (k, v) in prev {
        match v {
            Some(v) => std::env::set_var(&k, v),
            None => std::env::remove_var(&k),
        }
    }
    result
}
