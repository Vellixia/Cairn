//! Tier 4 — the environment itself is broken (`docs/testing.md`).
//!
//! Not a repository, a corrupt database, a home directory that cannot be
//! written, no `git` on `PATH`. In every case Cairn must refuse in a way that
//! names the problem, and must leave nothing behind: a failed first use that
//! half-registers a project is worse than one that refuses.
//!
//! Split out of `foundation.rs` because each of these spawns a `cairn` that
//! cannot reach a daemon and waits out its start timeout. Together they cost
//! seconds, and holding them alongside the fast journey tests made every e2e
//! run pay for them.

use cairn_e2e::{binary, Sandbox};
use std::process::Command;

#[test]
fn a_non_repository_fails_cleanly_and_creates_no_state() {
    let home = tempfile::TempDir::new().unwrap();
    let plain = tempfile::TempDir::new().unwrap();
    // Guarded, so the daemon `cairn` spawns below is stopped even when the
    // request it was spawned for fails.
    let socket = cairn_e2e::DaemonSocket::new();

    let out = Command::new(binary("cairn"))
        .args(["--json", "status"])
        .current_dir(plain.path())
        .env("CAIRN_HOME", home.path())
        .env("CAIRN_SOCKET", &socket)
        .env("CAIRND_BIN", binary("cairnd"))
        .output()
        .expect("cairn runs");

    let envelope: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("envelope");
    assert_eq!(envelope["ok"], false);
    assert_eq!(envelope["error"]["code"], "not_a_repository");
    assert_eq!(out.status.code(), Some(1), "a user error exits 1");

    // No partial state: no project row was written (FR-005).
    let db = home.path().join("cairn.sqlite3");
    if db.exists() {
        let bytes = std::fs::read(&db).unwrap_or_default();
        let text = String::from_utf8_lossy(&bytes);
        assert!(
            !text.contains(&plain.path().display().to_string()),
            "a project was registered for a non-repository"
        );
    }

    let _ = Command::new(binary("cairn"))
        .args(["daemon", "stop"])
        .env("CAIRN_HOME", home.path())
        .env("CAIRN_SOCKET", &socket)
        .output();
    #[cfg(unix)]
    {
        let _ = std::fs::remove_file(&socket);
    }
}

/// What this test writes over the database, named once so the corruption and
/// the assertion that it took cannot drift apart.
const GARBAGE: &[u8] = b"not a valid sqlite database";

/// The store's on-disk state, for the assertions that have to say which file
/// the daemon actually read.
fn snapshot(s: &Sandbox, label: &str) -> String {
    let mut out = format!("--- {label} ---\n");
    for (name, path) in [
        ("db", s.db_path()),
        ("-wal", s.sidecar("-wal")),
        ("-shm", s.sidecar("-shm")),
    ] {
        match std::fs::read(&path) {
            Ok(bytes) => {
                let head: String = bytes
                    .iter()
                    .take(16)
                    .map(|b| format!("{b:02x}"))
                    .collect::<Vec<_>>()
                    .join("");
                out.push_str(&format!(
                    "  {name} {} len={} head={head}\n",
                    path.display(),
                    bytes.len()
                ));
            }
            Err(e) => out.push_str(&format!(
                "  {name} {} absent/unreadable: {:?}\n",
                path.display(),
                e.kind()
            )),
        }
    }
    out
}

#[test]
fn a_corrupt_database_is_detected_and_reported() {
    let s = Sandbox::new();
    let mut diag = String::new();
    diag.push_str(&snapshot(&s, "before anything is stopped"));

    // Wait for the daemon *process* to be gone, not merely for its socket to
    // stop answering. SQLite checkpoints the write-ahead log into the main
    // database when the connection closes, so a daemon that has stopped
    // listening but is still shutting down will write a valid database back
    // over the garbage below — and `status` then succeeds, which is what made
    // this test fail about a third of the time.
    //
    // **Taken before anything stops it**, which is the half the first repair
    // missed. There was a `daemon stop` above this line, so by the time the
    // processes were identified the daemon had usually stopped answering — and
    // the two platforms then disagree about what that means. On Unix
    // `daemons_for_socket` scans processes by name and environment, so it still
    // finds a daemon that is shutting down and the wait below does its job. On
    // Windows it asks the named pipe who is serving it, and a pipe that has
    // already gone answers nothing: the list came back empty, the loop below
    // iterated over nothing, and the garbage was written straight into a
    // checkpoint still in flight. The guard was a no-op on exactly the platform
    // that needed it, which is why this surfaced there and only there.
    let victims = cairn_sys::daemons_for_socket(&s.socket);
    diag.push_str(&format!("victims={victims:?}\n"));
    s.stop_daemon();
    for pid in &victims {
        let exited = cairn_sys::wait_for_exit(*pid, std::time::Duration::from_secs(5));
        diag.push_str(&format!("wait_for_exit({pid})={exited}\n"));
        assert!(
            exited,
            "daemon {pid} should exit after `daemon stop`\n{diag}"
        );
    }
    diag.push_str(&snapshot(&s, "after the daemon exited"));

    // Overwrite the database with garbage — *and* take the write-ahead log out
    // beside it. Truncating only the main file does not corrupt the store:
    // SQLite in WAL mode reads page 1 through the log when the log holds it, so
    // the daemon opens a perfectly valid database and `status` succeeds.
    //
    // **Removal is not enough, and where it is not enough is Windows.** A file
    // with a live handle cannot be unlinked there — SQLite opens without
    // `FILE_SHARE_DELETE` — so `remove_file` returns `PermissionDenied` and a
    // `let _ =` swallowed it. The log survived, carried the whole database, and
    // the test reported "a corrupt database must not report success": a message
    // about the product, for a fixture that had not corrupted anything. Unix
    // never showed it, because `unlink` succeeds on an open file.
    //
    // Truncation needs no delete and works through a shared handle, so it is
    // the fallback. A zero-length log is not a log SQLite recovers from.
    for suffix in ["-wal", "-shm"] {
        let path = s.sidecar(suffix);
        let outcome = match std::fs::remove_file(&path) {
            Ok(()) => "removed".to_string(),
            Err(e) => match std::fs::File::create(&path) {
                Ok(_) => format!("truncated after {:?}", e.kind()),
                Err(t) => format!(
                    "left behind: remove {:?}, truncate {:?}",
                    e.kind(),
                    t.kind()
                ),
            },
        };
        diag.push_str(&format!("{suffix}: {outcome}\n"));
    }
    std::fs::write(s.db_path(), GARBAGE).expect("write");
    diag.push_str(&snapshot(&s, "after writing garbage"));

    // **The premise, asserted rather than assumed.**
    //
    // Everything below is about how Cairn reports a damaged store, and none of
    // it means anything if the store is not damaged. Checking it here is what
    // separates "Cairn mishandled a corrupt database" from "the fixture failed
    // to corrupt one" — two findings the old message could not tell apart, and
    // conflating them sent three rounds of investigation at the product for a
    // fault in the test.
    assert_eq!(
        std::fs::read(s.db_path()).unwrap_or_default(),
        GARBAGE,
        "the database is not the garbage this test wrote, so nothing below is \
         about a corrupt store\n{diag}"
    );
    for suffix in ["-wal", "-shm"] {
        let len = std::fs::metadata(s.sidecar(suffix))
            .map(|m| m.len())
            .unwrap_or(0);
        assert_eq!(
            len, 0,
            "a {suffix} survived beside the garbage, and SQLite reads the \
             database through it — the store is intact and this test would be \
             asserting against a daemon that is behaving correctly\n{diag}"
        );
    }

    let out = s.cairn(&["--json", "status"]);
    diag.push_str(&snapshot(&s, "after `status`"));
    diag.push_str(&format!(
        "status code={} stdout={} stderr={}\n",
        out.code, out.stdout, out.stderr
    ));
    assert!(
        !out.ok(),
        "a corrupt database must not report success\n{diag}"
    );
    // And it must fail as a reported storage problem, not a panic. Asserting
    // only `!ok` would pass on a crash, which is the failure mode this is
    // meant to rule out.
    let envelope: serde_json::Value =
        serde_json::from_str(&out.stdout).unwrap_or(serde_json::Value::Null);
    assert_eq!(
        envelope["ok"],
        serde_json::Value::Bool(false),
        "a corrupt database must be reported, not crashed on: {} / {}",
        out.stdout,
        out.stderr
    );
    // This used to be pinned to `daemon_unavailable: cairnd did not start` —
    // true, and useless. The daemon does start; it cannot open the store and
    // exits, and starting another one is not the remedy. The reason now travels
    // out of the daemon's log and is reported as what it is.
    assert_eq!(
        envelope["error"]["code"], "storage_unavailable",
        "a damaged store must not be reported as a missing daemon: {}",
        out.stdout
    );
    let message = envelope["error"]["message"].as_str().unwrap_or_default();
    assert!(
        message.contains("not a database"),
        "the message must name the real cause: {message}"
    );

    // And a silenced log must not silence the diagnosis. `CAIRN_LOG` filters the
    // daemon's tracing events, so a diagnosis carried by an ordinary one would
    // vanish for exactly the user who is least likely to work out why nothing
    // starts.
    let quiet = s.cairn_with_env(&["--json", "status"], &[("CAIRN_LOG", "off")]);
    let envelope: serde_json::Value =
        serde_json::from_str(&quiet.stdout).expect("envelope with the log off");
    assert_eq!(
        envelope["error"]["code"], "storage_unavailable",
        "the reason must reach the CLI whatever CAIRN_LOG says: {}",
        quiet.stdout
    );
}

#[test]
fn an_unwritable_cairn_home_is_reported_cleanly() {
    let repo = tempfile::TempDir::new().expect("repo");
    // Guarded, so the daemon `cairn` spawns below is stopped even when the
    // request it was spawned for fails.
    let socket = cairn_e2e::DaemonSocket::new();

    // Initialize a git repo.
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
    std::fs::write(repo.path().join("README.md"), "# fixture\n").expect("write");
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

    // Use a file as CAIRN_HOME — writing inside it must fail on every platform.
    let home_file = tempfile::NamedTempFile::new().expect("file");
    let invalid_home = home_file.path().to_path_buf();

    let out = Command::new(binary("cairn"))
        .args(["--json", "init"])
        .current_dir(repo.path())
        .env("CAIRN_HOME", &invalid_home)
        .env("CAIRN_SOCKET", &socket)
        .env("CAIRND_BIN", binary("cairnd"))
        .output()
        .expect("cairn runs");

    // The command should fail, not panic or hang.
    // A reported failure, not merely a non-zero exit: `!success || ok == false`
    // was satisfied by a panic or a signal death too, so it could not tell a
    // clean refusal from a crash — which is the whole point of the test.
    let envelope: serde_json::Value = serde_json::from_str(&String::from_utf8_lossy(&out.stdout))
        .unwrap_or(serde_json::Value::Null);
    assert_eq!(
        envelope["ok"],
        serde_json::Value::Bool(false),
        "an unwritable home must fail, as a reported error: stdout={} stderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        envelope["error"]["code"].is_string(),
        "the refusal should carry a code: {}",
        String::from_utf8_lossy(&out.stdout)
    );
}

#[test]
fn git_not_on_path_is_reported_cleanly() {
    let home = tempfile::TempDir::new().expect("home");
    let repo = tempfile::TempDir::new().expect("repo");
    // Guarded, so the daemon `cairn` spawns below is stopped even when the
    // request it was spawned for fails.
    let socket = cairn_e2e::DaemonSocket::new();

    // Initialize a git repo so the directory is valid.
    Command::new("git")
        .args(["init", "--initial-branch=main"])
        .current_dir(repo.path())
        .output()
        .expect("git init");

    // Run cairn with a PATH that does not include git.
    let out = Command::new(binary("cairn"))
        .args(["--json", "init"])
        .current_dir(repo.path())
        .env("CAIRN_HOME", home.path())
        .env("CAIRN_SOCKET", &socket)
        .env("CAIRND_BIN", binary("cairnd"))
        .env("PATH", "") // Empty PATH — git won't be found.
        .output()
        .expect("cairn runs");

    // A reported failure, not merely a non-zero exit: `!success || ok == false`
    // was satisfied by a panic or a signal death too, so it could not tell a
    // clean refusal from a crash — which is the whole point of the test.
    let envelope: serde_json::Value = serde_json::from_str(&String::from_utf8_lossy(&out.stdout))
        .unwrap_or(serde_json::Value::Null);
    assert_eq!(
        envelope["ok"],
        serde_json::Value::Bool(false),
        "missing git must fail, as a reported error: stdout={} stderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        envelope["error"]["code"].is_string(),
        "the refusal should carry a code: {}",
        String::from_utf8_lossy(&out.stdout)
    );
}
