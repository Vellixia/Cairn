//! A pre-004 client against a 004 server, and the two routes that are gone
//! (T192, T193; FR-586, FR-587, FR-588, SC-457, SC-458).
//!
//! This feature is additive on the wire, so a client built before it should not
//! notice: it never asks for anything schema 3 added. But the security
//! prerequisite it builds on is *not* additive — it removes `POST
//! /api/auth/register` and `POST /api/projects/{id}/join` — and removal is a
//! compatibility event for every client built before the removal, whether or not
//! that client has ever heard of personal or team knowledge.
//!
//! So there are two claims here. Project synchronization is unaffected: push,
//! pull and cursor advance exactly as before, with no namespace degraded. And a
//! removed route answers in a way an operator can act on, with the release
//! documenting the change in terms an operator can act on too.

use cairn_e2e::{post_json_status_bearer, post_status_anon, Server};
use serde_json::json;
use std::path::{Path, PathBuf};

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .to_path_buf()
}

fn server() -> Option<Server> {
    match Server::start() {
        Some(s) => Some(s),
        None => {
            eprintln!("SKIPPED: set CAIRN_TEST_DATABASE_URL to run the compatibility suite");
            None
        }
    }
}

// ---------------------------------------------------------------------------
// T193 / FR-587 / FR-588 / SC-458 — the removed routes, and the documentation
// ---------------------------------------------------------------------------

/// Each removed route answers `410 Gone` with a body naming its replacement.
///
/// `404` was the shape an earlier draft shipped, and it is the one thing this
/// response must not be: a not-found is indistinguishable from a typo'd URL or a
/// route that never existed, so an operator debugging a suddenly-failing client
/// learns nothing from it. `410 Gone` means "this existed and was deliberately
/// retired", which is exactly the fact they need.
///
/// Falsified by restoring either route, or by letting either fall through to the
/// router's default not-found.
#[test]
fn each_removed_route_answers_gone_and_names_its_replacement() {
    let Some(server) = server() else { return };

    // Registration, unauthenticated — which is how a pre-004 client reached it.
    let status = post_status_anon(
        &server.base,
        "/api/auth/register",
        &json!({ "email": "someone@example.test", "password": "hunter2hunter2" }),
    );
    assert_eq!(status, 410, "self-registration answered {status}");

    let token = server.new_user_token("compat");
    let (body, status) = post_json_status_bearer(
        &server.base,
        "/api/auth/register",
        &json!({ "email": "someone@example.test", "password": "hunter2hunter2" }),
        &token,
    );
    assert_eq!(status, 410, "self-registration answered {status}: {body}");
    assert_eq!(body["error"]["code"].as_str(), Some("route_removed"));
    let message = body["error"]["message"].as_str().unwrap_or_default();
    assert!(
        message.contains("/api/admin/users") && message.contains("cairn user create"),
        "the refusal names neither the replacement route nor the CLI verb: {message}"
    );

    // Self-join, for an arbitrary project id — the shape that used to be enough
    // to become a member of any project whose UUID you could name.
    let (body, status) = post_json_status_bearer(
        &server.base,
        &format!("/api/projects/{}/join", uuid::Uuid::now_v7()),
        &json!({}),
        &token,
    );
    assert_eq!(status, 410, "the join route answered {status}: {body}");
    assert_eq!(body["error"]["code"].as_str(), Some("route_removed"));
    let message = body["error"]["message"].as_str().unwrap_or_default();
    assert!(
        message.contains("/api/projects/{id}/members")
            && message.contains("cairn project member add"),
        "the refusal names neither the replacement route nor the CLI verb: {message}"
    );
}

/// The shipped documentation states all three facts, plus the operator's remedy.
///
/// Not "a paragraph exists somewhere": each of the three is a separate thing an
/// operator has to learn, and a release note that mentioned only the first would
/// leave a team unable to onboard anyone. SC-458 verifies the response *and* the
/// documentation because either alone leaves the other unaccounted for.
///
/// Falsified by deleting any of the three statements or the remedy.
#[test]
fn the_release_documentation_states_the_removal_and_the_operators_remedy() {
    let readme = std::fs::read_to_string(workspace_root().join("README.md")).expect("README.md");
    let lowered = readme.to_ascii_lowercase();

    assert!(
        lowered.contains("no\nself-registration")
            || lowered.contains("no self-registration")
            || lowered.contains("self-registration is gone")
            || lowered.contains("there is no sign-up form"),
        "the README does not state that self-registration is gone"
    );
    assert!(
        lowered.contains("self-join"),
        "the README does not state that self-join is gone"
    );
    assert!(
        lowered.contains("created by an administrator")
            || lowered.contains("administrator-created"),
        "the README does not state that accounts are now administrator-created"
    );

    // The remedy: what the operator runs instead, for each of the two things a
    // user used to do themselves.
    assert!(
        readme.contains("cairn user create"),
        "the README does not name the replacement for self-registration"
    );
    assert!(
        readme.contains("cairn project member add"),
        "the README does not name the replacement for self-join"
    );
}

// ---------------------------------------------------------------------------
// T192 / FR-586 / SC-457 — a real pre-004 client, through a full sync cycle
// ---------------------------------------------------------------------------

/// The commit this feature was implemented on top of — the pre-004 client.
///
/// Named rather than derived, because "the last commit before feature 004" is not
/// a thing git can be asked for: the feature's own commits are not on this branch
/// in any particular shape, and a heuristic that guessed would silently start
/// testing the wrong binary the day the shape changed.
const PRE_004_BASE: &str = "214154f";

/// Build the pre-004 `cairn`/`cairnd` pair from this repository's own history.
///
/// Built rather than simulated, because a hand-rolled request proves less than it
/// looks like it does: it omits whatever fields its author knew had changed, which
/// is exactly the set a real shipped binary still sends. SC-457 says so directly.
///
/// Built rather than read from an environment variable, because this suite is
/// hermetic by rule — `ci_hermeticity` allows exactly one variable, and it is not
/// this one. Everything needed is already in the repository, so needing a variable
/// would have meant needing something CI cannot provide.
///
/// The build is cached under `target/pre004`, so the cost is paid once per
/// checkout rather than once per run. A checkout without that commit — a shallow
/// clone, or a fork whose history was rewritten — skips with a named reason rather
/// than falling back to a simulation that would pass while proving less.
fn pre004_binaries() -> Option<PathBuf> {
    let root = workspace_root();
    let out = root.join("target/pre004");
    let cairn = out.join("debug/cairn");
    let cairnd = out.join("debug/cairnd");
    if cairn.exists() && cairnd.exists() {
        return Some(out.join("debug"));
    }

    // The base commit's tree, extracted with `git archive` piped into `tar`.
    //
    // **Not `git worktree add`.** That was the first shape this took, and it
    // writes into the repository's *shared* `.git/worktrees/` — metadata outside
    // the worktree under test, in a checkout a reviewer was told to leave alone.
    // It is also not self-cleaning: `cargo clean` deletes `target/pre004-src`
    // and leaves the registration behind, so the repository accumulates an
    // orphan entry per clean. `git archive` reads history and writes nothing but
    // the files asked for.
    let checkout = root.join("target/pre004-src");
    if !checkout.join("Cargo.toml").exists() {
        let _ = std::fs::remove_dir_all(&checkout);
        if std::fs::create_dir_all(&checkout).is_err() {
            eprintln!("SKIPPED: could not create {}", checkout.display());
            return None;
        }
        let extracted = std::process::Command::new("bash")
            .arg("-c")
            .arg(format!(
                "git archive {PRE_004_BASE} | tar -x -C {}",
                checkout.display()
            ))
            .current_dir(&root)
            .output();
        let ok = matches!(&extracted, Ok(o) if o.status.success())
            && checkout.join("Cargo.toml").exists();
        if !ok {
            eprintln!(
                "SKIPPED: could not extract the pre-004 base commit {PRE_004_BASE}, so there \
                 is no real pre-004 client to test against. A simulated client is \
                 deliberately not substituted: SC-457 requires a real binary. ({extracted:?})"
            );
            return None;
        }
    }

    let built = std::process::Command::new(env!("CARGO"))
        .args(["build", "-p", "cairn", "-p", "cairnd", "--target-dir"])
        .arg(&out)
        .current_dir(&checkout)
        .output();
    match built {
        Ok(o) if o.status.success() && cairn.exists() && cairnd.exists() => Some(out.join("debug")),
        other => {
            eprintln!(
                "SKIPPED: the pre-004 client would not build, so there is no real pre-004 \
                 client to test against. ({other:?})"
            );
            None
        }
    }
}

/// A pre-004 client runs a full project synchronization cycle against a 004
/// server, with no namespace blocked and no throughput loss.
///
/// Push, pull, cursor advance — the whole cycle, because a push that succeeded
/// while the pull silently returned nothing would look like success from the
/// pushing side and be a total failure from the receiving one.
#[test]
fn a_pre_004_client_completes_a_full_sync_cycle_against_a_004_server() {
    let Some(server) = server() else { return };
    // `pre004_binaries` reports its own reason when it declines.
    let Some(bin_dir) = pre004_binaries() else {
        return;
    };

    // The old client gets a home of its own, and therefore a store of its own at
    // the schema its build supports. Sharing the current suite's store would not
    // be a compatibility test at all: the current daemon migrates that store to
    // schema 7, and the old client correctly refuses to open it — which is the
    // *local* upgrade guard doing its job, not the wire compatibility this test
    // is about.
    let repo = tempfile::TempDir::new().expect("repo");
    let home = tempfile::TempDir::new().expect("home");
    let fake_home = tempfile::TempDir::new().expect("fake home");
    let socket = home.path().join("cairnd.sock");
    let git = |args: &[&str]| {
        let out = std::process::Command::new("git")
            .args(args)
            .current_dir(repo.path())
            .output()
            .expect("git runs");
        assert!(out.status.success(), "git {args:?} failed");
    };
    git(&["init", "--initial-branch=main"]);
    git(&["config", "user.email", "test@example.com"]);
    git(&["config", "user.name", "Cairn Test"]);
    git(&["config", "commit.gpgsign", "false"]);
    std::fs::write(repo.path().join("README.md"), "# fixture\n").expect("write");
    git(&["add", "."]);
    git(&["commit", "-m", "init", "--no-gpg-sign"]);
    let remote = "git@localhost:cairnfixture/pre004.git";
    git(&["remote", "add", "origin", remote]);

    let token = server.new_user_token("pre004");
    let (created, status) = post_json_status_bearer(
        &server.base,
        "/api/projects",
        &json!({ "name": "pre004", "repository_remote": remote }),
        &token,
    );
    assert_eq!(status, 200, "create project: {created}");
    let project = created["id"].as_str().expect("id").to_string();

    // `CAIRND_BIN` points at the old daemon, so the pair under test is genuinely
    // the old pair rather than an old CLI driving a current daemon.
    let old = |args: &[&str]| -> (i32, String, String) {
        let out = std::process::Command::new(bin_dir.join("cairn"))
            .args(args)
            .current_dir(repo.path())
            .env("CAIRN_HOME", home.path())
            .env("CAIRN_SOCKET", &socket)
            .env("CAIRND_BIN", bin_dir.join("cairnd"))
            .env("HOME", fake_home.path())
            .env("XDG_CONFIG_HOME", fake_home.path().join(".config"))
            .output()
            .expect("the pre-004 cairn runs");
        (
            out.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&out.stdout).to_string(),
            String::from_utf8_lossy(&out.stderr).to_string(),
        )
    };

    // The extraction must not have registered a git worktree. `git worktree add`
    // writes into the repository's shared `.git/worktrees/`, which is state
    // outside this worktree entirely — and it is not self-cleaning, so a
    // `cargo clean` would orphan the registration.
    let registered = std::process::Command::new("git")
        .args(["worktree", "list", "--porcelain"])
        .current_dir(workspace_root())
        .output()
        .expect("git worktree list runs");
    let listing = String::from_utf8_lossy(&registered.stdout);
    assert!(
        !listing.contains("pre004-src"),
        "extracting the pre-004 source registered a git worktree, mutating shared \
         repository metadata:\n{listing}"
    );

    for args in [
        vec!["init"],
        vec!["auth", "token", "set", &token, "--server", &server.base],
        vec!["link", "--project", &project],
        vec![
            "memory",
            "add",
            "--type",
            "convention",
            "--scope",
            "project",
            "written by a pre-004 client",
        ],
        vec!["sync", "now"],
    ] {
        let (code, out, err) = old(&args);
        assert_eq!(code, 0, "the pre-004 client failed at {args:?}: {err}{out}");
    }

    // The push landed on a 004 server.
    // Scoped to this run's project. The suite shares one database, so an
    // unscoped count would be counting every previous run's row as well.
    assert_eq!(
        server.count(&format!(
            "SELECT COUNT(*) FROM memories \
              WHERE project_id = '{project}' AND content = 'written by a pre-004 client'"
        )),
        1,
        "a pre-004 client's push did not reach a 004 server"
    );

    // The pull works too: a record written by a *current* client arrives on the
    // old one's next cycle. A push that succeeded while the pull silently
    // returned nothing would look like success from one side and total failure
    // from the other.
    let current = cairn_e2e::Sandbox::new();
    current.git(&["remote", "add", "origin", remote]);
    current.must(&["init"]);
    cairn_e2e::attach_server(&current, &server, &token);
    current.must(&["link", "--project", &project]);
    current.must(&[
        "memory",
        "add",
        "--type",
        "convention",
        "--scope",
        "project",
        "written by a current client",
    ]);
    current.must(&["sync", "now"]);

    let (code, _, err) = old(&["sync", "now"]);
    assert_eq!(code, 0, "the pre-004 client's pull failed: {err}");

    let (code, out, err) = old(&["--json", "memory", "search", "current client"]);
    assert_eq!(code, 0, "the pre-004 client's search failed: {err}");
    let payload: serde_json::Value = serde_json::from_str(&out).expect("a JSON envelope");
    let rendered = payload.to_string();
    assert!(
        rendered.contains("written by a current client"),
        "a pre-004 client did not receive a record the 004 server held: {rendered}"
    );

    // No namespace degraded, nothing failed, and the cursor advanced.
    let (code, out, err) = old(&["--json", "sync", "status"]);
    assert_eq!(code, 0, "sync status failed: {err}");
    let payload: serde_json::Value = serde_json::from_str(&out).expect("a JSON envelope");
    let body = &payload["data"];
    assert!(
        body["degradation"].is_null(),
        "a pre-004 client reports a degradation against a 004 server: {body}"
    );
    assert_eq!(
        body["failed"].as_i64().unwrap_or(-1),
        0,
        "a pre-004 client has failed outbox rows: {body}"
    );
    assert!(
        body["last_success_at"].is_string(),
        "the pre-004 client never recorded a successful cycle: {body}"
    );
}
