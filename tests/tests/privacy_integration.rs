//! T034, T115 — integration state is local, and it stays local (FR-183,
//! FR-197, FR-200, SC-119, SC-120, SC-133).
//!
//! Feature 002 reads the developer's agent configuration. That is where their
//! API keys live. Two rules follow, and both are absolute: nothing Cairn
//! records about an integration is ever queued for the shared server, and no
//! credential Cairn read on the way past is ever written down — not in a
//! recovery artifact, not in the local database, not in a log, not in a
//! diagnostic.
//!
//! The first rule is enforced structurally rather than by filtering: there is
//! no outbox entity type an integration record could use, so the enqueue path
//! cannot be reached by mistake. This suite asserts the structure and the
//! behavior, because a filter that is currently correct is one refactor away
//! from not being.

use cairn_e2e::{attach_server, Sandbox, Server};
use serde_json::json;

/// Credentials seeded into the developer's real agent configuration. Every one
/// of these is a shape Cairn will actually walk past while editing.
const API_KEY: &str = "sk-ant-api03-SEEDEDCREDENTIALDONOTLEAK";
const OAUTH_TOKEN: &str = "ya29.SEEDEDOAUTHTOKENDONOTLEAK";
const PASSWORD: &str = "correct-horse-battery-staple-SEEDED";
const BEARER: &str = "Bearer SEEDEDBEARERDONOTLEAK";

const SECRETS: [&str; 4] = [API_KEY, OAUTH_TOKEN, PASSWORD, BEARER];

fn server() -> Option<Server> {
    match Server::start() {
        Some(s) => Some(s),
        None => {
            eprintln!(
                "SKIPPED: set CAIRN_TEST_DATABASE_URL (e.g. `docker compose up -d postgres`) \
                 to run the server half of the privacy suite"
            );
            None
        }
    }
}

/// Seed each agent's own configuration with credentials, the way a developer's
/// really looks.
fn seed_credentials(s: &Sandbox) {
    let home = s.fake_home();
    std::fs::create_dir_all(home.join(".claude")).unwrap();
    std::fs::create_dir_all(home.join(".codex")).unwrap();
    std::fs::create_dir_all(home.join(".config").join("opencode")).unwrap();

    std::fs::write(
        home.join(".claude.json"),
        json!({
            "version": "2.1.220",
            "primaryApiKey": API_KEY,
            "oauthAccount": { "accessToken": OAUTH_TOKEN },
            "mcpServers": {
                "internal": {
                    "command": "internal-mcp",
                    "env": { "AUTHORIZATION": BEARER, "DB_PASSWORD": PASSWORD }
                }
            }
        })
        .to_string(),
    )
    .unwrap();

    std::fs::write(
        home.join(".codex").join("config.toml"),
        format!(
            "model = \"gpt-5-codex\"\n\
             [mcp_servers.internal]\n\
             command = \"internal-mcp\"\n\
             env = {{ AUTHORIZATION = \"{BEARER}\", DB_PASSWORD = \"{PASSWORD}\" }}\n"
        ),
    )
    .unwrap();

    std::fs::write(
        home.join(".config").join("opencode").join("opencode.json"),
        json!({
            "version": "1.4.2",
            "provider": { "anthropic": { "options": { "apiKey": API_KEY } } }
        })
        .to_string(),
    )
    .unwrap();
}

/// Every place Cairn writes on this machine, as one searchable blob.
fn everything_cairn_wrote(s: &Sandbox) -> String {
    let mut out = String::from_utf8_lossy(&s.db_bytes()).to_string();
    // Recovery artifacts, the daemon log, and anything else under CAIRN_HOME.
    collect(s, &s.cairn_home(), &mut out);
    out
}

/// Walk everything Cairn owns, and nothing the developer owns.
///
/// The sandbox puts the agents' home inside `CAIRN_HOME` so a test can never
/// reach the developer's real `~/.claude`. That makes the agents' own
/// configuration a subtree of what is being searched, and finding a
/// credential in the file the developer wrote it in proves nothing.
fn collect(s: &Sandbox, root: &std::path::Path, out: &mut String) {
    let agent_home = s.fake_home();
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if path == agent_home {
            continue;
        }
        if path.is_dir() {
            collect(s, &path, out);
        } else if let Ok(bytes) = std::fs::read(&path) {
            out.push_str(&path.display().to_string());
            out.push('\n');
            out.push_str(&String::from_utf8_lossy(&bytes));
            out.push('\n');
        }
    }
}

fn assert_no_secret(where_: &str, text: &str) {
    for secret in SECRETS {
        assert!(
            !text.contains(secret),
            "a seeded credential reached {where_}"
        );
    }
}

// ------------------------------------------------------------------ T034 ---

mod privacy {
    use super::*;

    /// FR-183, SC-120: no integration record is ever queued for the shared
    /// server, and the schema gives none of them a way to be.
    #[test]
    fn no_integration_outbox() {
        let s = Sandbox::new();
        for a in ["claude-code", "codex", "opencode"] {
            s.install_agent(a);
        }
        s.must(&["init"]);

        let before = s.query_column("SELECT CAST(COUNT(*) AS TEXT) FROM outbox");

        // Write every Feature 002 record type there is: the agent row, the
        // installed resources and their bindings, capability evidence of both
        // kinds, a migration, and a recovery artifact.
        s.must(&["connect", "claude-code", "--yes"]);
        s.must(&["connect", "codex", "--yes"]);
        s.must(&["connect", "opencode", "--yes"]);
        s.hook(
            "SessionStart",
            json!({ "session_id": "outbox-1", "source": "startup" }),
        );
        s.settle_session_count(1);
        s.hook(
            "PostToolUse",
            json!({
                "session_id": "outbox-1",
                "tool_name": "Read",
                "tool_input": { "file_path": "README.md" }
            }),
        );
        s.cairn(&["doctor"]);
        s.cairn(&["repair"]);
        s.must(&["disconnect", "opencode"]);

        // Every integration table has rows, so this is a real test.
        for table in [
            "agent_integrations",
            "installed_resources",
            "resource_bindings",
            "capability_evidence",
        ] {
            let count = s.query_column(&format!("SELECT CAST(COUNT(*) AS TEXT) FROM {table}"));
            assert_ne!(
                count.first().map(String::as_str),
                Some("0"),
                "{table} is empty, so this assertion proves nothing"
            );
        }

        let after = s.query_column("SELECT CAST(COUNT(*) AS TEXT) FROM outbox");
        assert_eq!(
            before, after,
            "an integration record was queued for the shared server"
        );

        // And structurally: the outbox's entity types are a closed set that
        // contains nothing from Feature 002. A filter can be forgotten; a
        // CHECK constraint cannot.
        let schema = s
            .query_column("SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'outbox'");
        let sql = schema.first().cloned().unwrap_or_default();
        for entity in [
            "agent_integration",
            "installed_resource",
            "resource_binding",
            "capability_evidence",
            "integration",
        ] {
            assert!(
                !sql.contains(entity),
                "the outbox admits `{entity}` as an entity type: {sql}"
            );
        }
        for allowed in ["project", "task", "session", "memory", "handoff"] {
            assert!(sql.contains(allowed), "{allowed} left the outbox: {sql}");
        }
    }
}

// ------------------------------------------------------------------ T115 ---

#[test]
fn no_credential_survives_any_integration_command() {
    // SC-119: connect, repair, migrate and disconnect all walk past the
    // developer's credentials while editing the files those credentials live
    // in. None of them may write one down anywhere.
    let s = Sandbox::new();
    seed_credentials(&s);
    s.must(&["init"]);

    s.must(&["connect", "claude-code", "--yes"]);
    s.must(&["connect", "codex", "--yes"]);
    s.must(&["connect", "opencode", "--yes"]);
    s.cairn(&["doctor"]);
    s.cairn(&["repair"]);
    s.cairn(&["integration", "migrate"]);
    s.must(&["disconnect", "codex"]);

    assert_no_secret("Cairn's own state", &everything_cairn_wrote(&s));

    // Diagnostics are the easiest place to leak: they exist to be pasted into
    // an issue.
    for command in [
        vec!["doctor"],
        vec!["--json", "doctor"],
        vec!["agents"],
        vec!["--json", "agents"],
        vec!["status"],
    ] {
        let out = s.cairn(&command);
        assert_no_secret(
            &format!("the output of `cairn {}`", command.join(" ")),
            &format!("{}{}", out.stdout, out.stderr),
        );
    }

    // The credentials are still where the developer put them: Cairn read past
    // them without disturbing them.
    let claude = std::fs::read_to_string(s.fake_home().join(".claude.json")).unwrap();
    assert!(claude.contains(API_KEY), "Cairn disturbed a credential");
    assert!(claude.contains(OAUTH_TOKEN));
    let codex = std::fs::read_to_string(s.fake_home().join(".codex").join("config.toml")).unwrap();
    assert!(codex.contains(BEARER));
}

#[test]
fn no_whole_file_copy_of_a_developers_configuration_is_ever_created() {
    // FR-197: a recovery artifact is what Cairn needs to undo its own change,
    // and a copy of the whole file would be a copy of everything in it —
    // including the credentials above, sitting in a second place forever.
    let s = Sandbox::new();
    seed_credentials(&s);
    s.must(&["init"]);
    s.must(&["connect", "claude-code", "--yes"]);
    s.must(&["connect", "codex", "--yes"]);
    s.cairn(&["repair"]);

    let originals: Vec<String> = ["claude.json", "config.toml"]
        .iter()
        .map(|name| {
            let path = match *name {
                "claude.json" => s.fake_home().join(".claude.json"),
                _ => s.fake_home().join(".codex").join("config.toml"),
            };
            std::fs::read_to_string(path).unwrap_or_default()
        })
        .collect();

    let mut written = String::new();
    collect(&s, &s.cairn_home(), &mut written);
    for original in originals {
        // The distinctive part of the developer's file: their own MCP server,
        // which Cairn neither wrote nor owns.
        assert!(
            !written.contains("internal-mcp"),
            "a copy of the developer's configuration was written under CAIRN_HOME"
        );
        assert!(original.contains("internal-mcp"), "the fixture is real");
    }
}

#[test]
fn integration_state_never_reaches_the_shared_server() {
    // SC-120, SC-133: asserted twice, against the outbound payload the daemon
    // would send and against the server's own database afterwards. A rule
    // enforced only at the API boundary is one that a future endpoint
    // silently repeals.
    let Some(server) = server() else { return };
    let s = Sandbox::new();
    seed_credentials(&s);
    let token = server.new_user_token("integration-privacy");
    attach_server(&s, &server, &token);
    s.must(&["init"]);
    s.must(&["connect", "claude-code", "--yes"]);
    s.must(&["connect", "codex", "--yes"]);

    // Real, shareable work, so the sync has something legitimate to carry.
    s.hook(
        "SessionStart",
        json!({ "session_id": "shared-1", "source": "startup" }),
    );
    s.settle_session_count(1);
    s.must(&[
        "memory",
        "add",
        "The pool is single-writer on purpose",
        "--type",
        "decision",
        "--scope",
        "project",
    ]);
    s.hook(
        "SessionEnd",
        json!({ "session_id": "shared-1", "reason": "clear" }),
    );
    s.settle("the closed session's handoff", |s| {
        s.cairn(&["--json", "status"])
            .stdout
            .contains("\"sessions_awaiting_handoff\": 0")
    });
    s.json(&["link", "--create"]);
    s.json(&["sync", "now"]);
    assert_eq!(s.json(&["sync", "status"])["pending"], 0);

    let dump = server.dump();
    // The legitimate half arrived, so an empty database is not what is being
    // asserted here.
    assert!(
        dump.contains("single-writer on purpose"),
        "nothing reached the server, so this assertion proves nothing"
    );

    assert_no_secret("the shared server's database", &dump);

    // No agent configuration content.
    for forbidden in ["internal-mcp", "mcp_servers", "mcpServers", "cairn:managed"] {
        assert!(
            !dump.contains(forbidden),
            "agent configuration content reached the server: {forbidden}"
        );
    }
    // No integration health detail.
    for forbidden in [
        "capability_evidence",
        "installed_resource",
        "completion_guarantee",
        "mcp_plus",
        "lifecycle_session_open",
        "compatible_unverified",
    ] {
        assert!(
            !dump.contains(forbidden),
            "integration health detail reached the server: {forbidden}"
        );
    }
    // No absolute path from this machine.
    let home = s.fake_home().display().to_string();
    assert!(
        !dump.contains(&home),
        "an absolute path from this machine reached the server"
    );
    assert!(
        !dump.contains(&s.repo_dir().display().to_string()),
        "the worktree path reached the server"
    );
}
