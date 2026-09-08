//! Which `cairn-server` the end-to-end suite actually spawns (T-CI).
//!
//! The suite resolves its binaries next to the test executable, which under
//! `cargo test` is `target/debug/` — so every server it started was an
//! unoptimized build, and a sign-in is an argon2 verify. The workflow already
//! records that cost (`~0.7s` unoptimized against `~0.03s` released) and
//! already builds a release server for the web end-to-end job for exactly that
//! reason.
//!
//! `CAIRN_SERVER_BIN` is how CI now points the Rust suite at the same release
//! build. This file is the proof that the override is real: not "a release
//! binary was built somewhere", which is what a `cargo build --release` step
//! alone would show, but "the path CI sets is the path that gets spawned".
//!
//! Deliberately not a server test. It needs no database and no port, so it
//! runs everywhere the suite runs, including the macOS and Windows jobs that
//! have no PostgreSQL service and skip the server suites entirely.

use cairn_e2e::server_binary;

/// The environment variable wins over the sibling-of-the-test-executable path.
///
/// **Falsified by** `server_binary()` ignoring `CAIRN_SERVER_BIN`, which is the
/// failure mode that would leave CI building a release server and testing the
/// debug one — green, and proving nothing.
#[test]
fn the_environment_names_the_server_that_gets_spawned() {
    // A real file, because the resolver refuses a path that does not exist
    // rather than handing a spawn something that cannot run.
    let named = std::env::temp_dir().join(format!("cairn-server-probe-{}", std::process::id()));
    std::fs::write(&named, b"#!/bin/sh\nexit 0\n").expect("write the probe");

    // Set and restored around the assertion: the variable is process-global,
    // and a sibling test resolving a server mid-flight must not see this one.
    let previous = std::env::var_os("CAIRN_SERVER_BIN");
    std::env::set_var("CAIRN_SERVER_BIN", &named);
    let resolved = server_binary();
    match previous {
        Some(v) => std::env::set_var("CAIRN_SERVER_BIN", v),
        None => std::env::remove_var("CAIRN_SERVER_BIN"),
    }
    let _ = std::fs::remove_file(&named);

    assert_eq!(
        resolved, named,
        "CAIRN_SERVER_BIN did not decide which server the suite spawns"
    );
}

/// With nothing set, the previous behaviour is unchanged: the server beside the
/// test executable.
///
/// A developer running `cargo test` locally gets the debug server they always
/// got, and the override is CI's choice rather than a new requirement.
#[test]
fn without_the_variable_the_binary_beside_the_tests_is_used() {
    let previous = std::env::var_os("CAIRN_SERVER_BIN");
    std::env::remove_var("CAIRN_SERVER_BIN");
    let resolved = std::panic::catch_unwind(server_binary);
    if let Some(v) = previous {
        std::env::set_var("CAIRN_SERVER_BIN", v);
    }

    let mut expected = std::env::current_exe().expect("test exe");
    expected.pop();
    if expected.ends_with("deps") {
        expected.pop();
    }
    let expected = expected.join(if cfg!(windows) {
        "cairn-server.exe"
    } else {
        "cairn-server"
    });

    match resolved {
        Ok(path) => assert_eq!(path, expected, "the fallback moved"),
        // `cargo test` without a prior `cargo build --workspace` has no server
        // to find, and the resolver says so rather than returning a path that
        // cannot be spawned. That is the documented behaviour, not a failure
        // of this claim.
        Err(_) => assert!(
            !expected.exists(),
            "the server exists at {} but the fallback refused it",
            expected.display()
        ),
    }
}

/// Ten processes migrating one database concurrently all succeed (D436's
/// idiom, applied to migration).
///
/// Every server applies migrations on start, which is what lets a fresh
/// deployment need no separate step — and it means a rolling restart, a scaled
/// deployment, and this suite (a server per test) all issue the same
/// `CREATE TABLE` at once. PostgreSQL's catalog refuses the loser with
/// `duplicate key value violates unique constraint "pg_type_typname_nsp_index"`,
/// and `IF NOT EXISTS` does not help: it is checked before the catalog insert
/// rather than atomically with it.
///
/// **Driven through `users add`, not through starting servers.** `Server::spawn`
/// retries a failed start four times, and ten racers retrying stagger enough
/// that they all eventually come up — a test written that way passed with the
/// fix reverted, which is worse than no test. `users add` goes through the same
/// `db::connect`, and once, so a lost race is visible.
///
/// Measured on this shape with the lock reverted: nine of ten servers exited.
///
/// **Falsified by** removing the advisory lock from `db::migrate`.
#[test]
fn concurrent_migrations_of_one_database_all_succeed() {
    let Some(url) = cairn_e2e::Server::fresh_database() else {
        eprintln!("skipped: CAIRN_TEST_DATABASE_URL is not set");
        return;
    };
    let racers: Vec<_> = (0..10)
        .map(|i| {
            let url = url.clone();
            std::thread::spawn(move || {
                std::process::Command::new(cairn_e2e::server_binary())
                    .args([
                        "--database-url",
                        &url,
                        "users",
                        "add",
                        "--email",
                        &format!("racer-{i}@example.test"),
                        "--display-name",
                        "racer",
                        "--password",
                        "hunter2hunter2",
                    ])
                    .output()
                    .expect("cairn-server runs")
            })
        })
        .collect();

    let mut lost = Vec::new();
    for handle in racers {
        let out = handle.join().expect("a racer thread");
        if !out.status.success() {
            lost.push(String::from_utf8_lossy(&out.stderr).trim().to_string());
        }
    }
    assert!(
        lost.is_empty(),
        "{} of ten concurrent migrations of one database failed; \
         concurrent `CREATE TABLE` loses to PostgreSQL's catalog unless \
         `db::migrate` serializes on its advisory lock. First: {}",
        lost.len(),
        lost.first().map(String::as_str).unwrap_or("")
    );
}
