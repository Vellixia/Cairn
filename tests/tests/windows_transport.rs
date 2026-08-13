//! Windows named-pipe transport — connect, concurrent clients, daemon
//! auto-start, and the exclusive `first_pipe_instance` ownership check.
//!
//! On Unix the transport tests live in `us1_sessions.rs` and
//! `foundation.rs`, where they use Unix-domain sockets: a file on disk
//! that the daemon renames atomically to claim. Windows has no equivalent
//! of the rename; instead the daemon opens a *named pipe* with
//! `first_pipe_instance(true)`, which fails with `ERROR_ACCESS_DENIED` if
//! another daemon already holds the name. This file exercises that path on
//! Windows specifically, so the windows-support branch has tests for the
//! transport it introduced.
//!
//! Skipped on non-Windows platforms via `#[cfg(windows)]`.

#![cfg(windows)]

use cairn_e2e::{daemon_listening, sandbox_socket, Sandbox};
use std::process::Command;
use std::time::{Duration, Instant};

/// A second `cairnd` started against a pipe another daemon already holds
/// must exit immediately with no error — `first_pipe_instance` is the
/// exclusive ownership check (FR-046, the windows equivalent of the Unix
/// rename race).
#[test]
fn a_second_daemon_against_a_held_pipe_exits_cleanly() {
    let s = Sandbox::new();
    assert!(daemon_listening(&s.socket), "sandbox daemon should be up");

    // Start a second cairnd against the *same* pipe name. It should
    // return Ok without ever serving, since the first daemon owns the
    // pipe.
    let out = Command::new(cairn_e2e::binary("cairnd"))
        .args(["--socket", &s.socket.display().to_string()])
        .env("CAIRN_HOME", s.home.path())
        .output()
        .expect("cairnd runs");

    assert!(
        out.status.success(),
        "a losing daemon should exit cleanly, not error: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // The original daemon should still be the one serving.
    assert!(
        daemon_listening(&s.socket),
        "the original daemon should still own the pipe"
    );
}

/// The daemon auto-starts when a CLI command arrives and nothing is
/// listening (FR-046). On Windows the first `cairn status` after a
/// clean state should bring the daemon up over the named pipe.
#[test]
fn the_daemon_auto_starts_over_a_named_pipe() {
    let home = tempfile::TempDir::new().expect("home");
    let socket = sandbox_socket();
    let repo = tempfile::TempDir::new().expect("repo");

    // Initialize a git repo so `cairn init` has something to register.
    Command::new("git")
        .args(["init", "--initial-branch=main"])
        .current_dir(repo.path())
        .output()
        .expect("git init");
    Command::new("git")
        .args(["config", "user.email", "test@example.com"])
        .current_dir(repo.path())
        .output()
        .expect("git config");
    Command::new("git")
        .args(["config", "user.name", "Cairn Test"])
        .current_dir(repo.path())
        .output()
        .expect("git config");
    std::fs::write(repo.path().join("README.md"), "# pipe\n").expect("write");
    Command::new("git")
        .args(["add", "."])
        .current_dir(repo.path())
        .output()
        .expect("git add");
    Command::new("git")
        .args(["commit", "-m", "init", "--no-gpg-sign"])
        .current_dir(repo.path())
        .output()
        .expect("git commit");

    // Nothing is listening yet.
    assert!(
        !daemon_listening(&socket),
        "no daemon should be listening before the first command"
    );

    // The first `cairn init` starts one.
    let out = Command::new(cairn_e2e::binary("cairn"))
        .args(["init"])
        .current_dir(repo.path())
        .env("CAIRN_HOME", home.path())
        .env("CAIRN_SOCKET", &socket)
        .env("CAIRND_BIN", cairn_e2e::binary("cairnd"))
        .output()
        .expect("cairn runs");
    assert!(
        out.status.success(),
        "cairn init failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // The daemon should have bound the pipe by now.
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline && !daemon_listening(&socket) {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(
        daemon_listening(&socket),
        "cairn should have auto-started a daemon on the named pipe"
    );

    // Clean up: stop the daemon.
    let _ = Command::new(cairn_e2e::binary("cairn"))
        .args(["daemon", "stop"])
        .current_dir(repo.path())
        .env("CAIRN_HOME", home.path())
        .env("CAIRN_SOCKET", &socket)
        .output();
}

/// Several CLI clients connecting concurrently over the named pipe must
/// each get a reply, not be refused. Windows named pipes can have only one
/// instance by default; `cairnd` opens extra instances as needed (see
/// `main.rs`'s Windows `run` loop), so this exercises that path.
#[test]
fn concurrent_clients_over_one_named_pipe_each_get_a_reply() {
    let s = Sandbox::new();
    assert!(daemon_listening(&s.socket), "sandbox daemon should be up");

    let n = 8;
    let mut handles = Vec::new();
    for i in 0..n {
        let repo = s.repo_path().to_path_buf();
        let home = s.home.path().to_path_buf();
        let socket = s.socket.clone();
        handles.push(std::thread::spawn(move || {
            let out = Command::new(cairn_e2e::binary("cairn"))
                .args(["--json", "status"])
                .current_dir(&repo)
                .env("CAIRN_HOME", &home)
                .env("CAIRN_SOCKET", &socket)
                .env("CAIRND_BIN", cairn_e2e::binary("cairnd"))
                .output()
                .expect("cairn runs");
            (i, out)
        }));
    }
    for h in handles {
        let (i, out) = h.join().expect("thread");
        assert!(
            out.status.success(),
            "client {i} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let envelope: serde_json::Value = serde_json::from_slice(&out.stdout).expect("envelope");
        assert_eq!(
            envelope["ok"],
            serde_json::Value::Bool(true),
            "client {i} did not get an ok envelope"
        );
    }
}

/// A daemon stopped via `cairn daemon stop` should release the named pipe
/// so a fresh daemon can take it. Windows has no leftover socket file to
/// remove, but the pipe name must be free.
#[test]
fn daemon_stop_releases_the_named_pipe_for_a_new_daemon() {
    let s = Sandbox::new();
    assert!(daemon_listening(&s.socket), "sandbox daemon should be up");

    s.cairn(&["daemon", "stop"]);
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline && daemon_listening(&s.socket) {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        !daemon_listening(&s.socket),
        "the pipe should be free after `daemon stop`"
    );

    // A fresh command should be able to start a new daemon on the same pipe.
    let out = s.cairn(&["status"]);
    assert!(
        out.ok(),
        "a new daemon should start on the freed pipe: {}",
        out.stderr
    );
    assert!(
        daemon_listening(&s.socket),
        "the new daemon should be listening on the same pipe name"
    );
}

/// A unique pipe name derived from `CAIRN_HOME` should never collide
/// between two sandboxes (FR-004, the windows-specific half). Two
/// `Sandbox` instances must each reach their own daemon.
#[test]
fn two_sandboxes_never_collide_on_one_pipe() {
    let a = Sandbox::new();
    let b = Sandbox::new();
    assert_ne!(
        a.socket, b.socket,
        "each sandbox must get a distinct pipe name"
    );

    // Both daemons should be reachable at the same time.
    assert!(daemon_listening(&a.socket), "sandbox A daemon should be up");
    assert!(daemon_listening(&b.socket), "sandbox B daemon should be up");

    // A status call against A must not affect B's daemon and vice versa.
    let a_status = a.json(&["status"]);
    let b_status = b.json(&["status"]);
    let a_project = a_status["project"]["id"].as_str().unwrap().to_string();
    let b_project = b_status["project"]["id"].as_str().unwrap().to_string();
    assert_ne!(
        a_project, b_project,
        "two sandboxes must not share a project"
    );
}
