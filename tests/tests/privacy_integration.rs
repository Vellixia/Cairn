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

// ---------------------------------------------------------------------------
// T133, T134 — the boundary as a property, and deletion as a report
// ---------------------------------------------------------------------------

/// Record kinds that are local to the machine that produced them (FR-503).
const LOCAL_ONLY_KINDS: &[&str] = &[
    "evidence_fact",
    "verification_run",
    "continuity_checkpoint",
    "reusable_pattern",
    "pattern_application",
    "task_change",
    "criterion_evidence",
    "observation",
];

/// Nothing local can be queued, because there is no name to queue it under
/// (FR-501, FR-502, FR-503, FR-508, SC-316).
///
/// Asserted **structurally**: `OutboxEntityType` has no variant for any of
/// these, so an outbox row for one is not something the code declines to write
/// — it is something that cannot be spelled. A rule enforced by a check can be
/// forgotten at a new call site; a rule enforced by the type system cannot.
#[test]
fn nothing_local_escapes() {
    use cairn_core::domain::OutboxEntityType;
    use std::str::FromStr;

    for kind in LOCAL_ONLY_KINDS {
        assert!(
            OutboxEntityType::from_str(kind).is_err(),
            "`{kind}` can be named as an outbox entity type, so a row for it can be written"
        );
    }

    // And the four that *are* syncable are still exactly the four.
    let mut syncable: Vec<&str> = OutboxEntityType::ALL.iter().map(|t| t.as_str()).collect();
    syncable.sort();
    assert_eq!(
        syncable,
        vec![
            "handoff",
            "memory",
            "memory_relation",
            "project",
            "session",
            "task",
            "task_blocker",
            "task_criterion",
        ],
        "the syncable set changed; every addition to it is a privacy decision"
    );
}

/// A `local_only` memory produces no outbox row, and neither does anything
/// derived from it — including a pinned one (FR-457, FR-504).
#[test]
fn a_local_only_memory_produces_nothing_to_send() {
    let s = Sandbox::new();
    s.must(&["init"]);

    // Linked, so anything that *could* be queued would be.
    let local = s.json(&[
        "memory",
        "add",
        "A note that never leaves this machine",
        "--type",
        "decision",
        "--scope",
        "project",
        "--local-only",
        "--topic-key",
        "infra.local_note",
        "--value-key",
        "private",
    ]);
    let id = local["memory"]["id"].as_str().expect("id").to_string();

    let queued = s.query_column(&format!(
        "SELECT CAST(COUNT(*) AS TEXT) FROM outbox WHERE entity_id = '{id}'"
    ));
    assert_eq!(
        queued,
        vec!["0".to_string()],
        "a local-only memory was queued for delivery"
    );

    // `pin_reason` is free text about local context and never travels, even
    // though the `pinned` flag itself may (`contracts/privacy-sync.md`).
    let payloads = s.query_column("SELECT COALESCE(GROUP_CONCAT(payload, ' '), '') FROM outbox");
    let all = payloads.join(" ");
    for forbidden in ["pin_reason", "content_norm_digest", "local_revision"] {
        assert!(
            !all.contains(forbidden),
            "`{forbidden}` appears in a queued payload: {all}"
        );
    }
}

/// Every row of the deletion table: nothing dangles, and nothing is restored
/// (FR-505, `contracts/privacy-sync.md` §Deletion).
#[test]
fn deleted_origin_reports_deletion() {
    let s = Sandbox::new();
    s.must(&["init"]);
    s.write_file("config/app.yml", "server:\n  port: 8080\n");

    let memory = s.json(&[
        "memory",
        "add",
        "The API listens on port 8080",
        "--type",
        "fact",
        "--scope",
        "project",
    ]);
    let memory_id = memory["memory"]["id"].as_str().expect("id").to_string();

    let evidence = s.json(&[
        "evidence",
        "add",
        "--type",
        "configuration",
        "--subject",
        "API port",
        "--value",
        "8080",
        "--locator",
        "config/app.yml#server.port",
        "--collector",
        "cairn",
        "--memory",
        &memory_id,
    ]);
    let evidence_id = evidence["evidence"]["id"]
        .as_str()
        .unwrap_or_else(|| panic!("an evidence id: {evidence}"))
        .to_string();

    // --- Evidence fact: tombstoned, not erased. Identity and provenance
    //     survive; the value, locator, digest and fingerprint do not.
    // Driven through the store: forgetting an evidence fact has no CLI verb
    // today, and what FR-505 constrains is the deletion itself rather than the
    // command that reaches it.
    forget_evidence(&s, &evidence_id);
    let row = s.query_column(&format!(
        "SELECT COALESCE(observed_value, '<cleared>') || '|' ||
                COALESCE(source_locator, '<cleared>') || '|' ||
                COALESCE(value_digest, '<cleared>') || '|' ||
                COALESCE(fingerprint, '<cleared>') || '|' ||
                CASE WHEN deleted_at IS NULL THEN 'live' ELSE 'tombstoned' END
           FROM evidence_facts WHERE id = '{evidence_id}'"
    ));
    assert_eq!(
        row,
        vec!["<cleared>|<cleared>|<cleared>|<cleared>|tombstoned".to_string()],
        "a deleted evidence fact must keep its identity and lose its content"
    );

    // The link survives and resolves to "evidence deleted" rather than to
    // nothing — a dangling reference is what this rule exists to prevent.
    let links = s.query_column(&format!(
        "SELECT CAST(COUNT(*) AS TEXT) FROM memory_evidence_facts
          WHERE evidence_id = '{evidence_id}'"
    ));
    assert_eq!(
        links,
        vec!["1".to_string()],
        "the link from the memory was removed instead of resolving to deleted"
    );
    let listed = s.json(&["evidence", "list", "--memory", &memory_id]);
    let text = listed.to_string();
    assert!(
        !text.contains("8080") || !text.contains("config/app.yml"),
        "a deleted fact's content came back: {text}"
    );

    // --- Memory: tombstoned, and a relation naming it survives.
    let other = s.json(&[
        "memory",
        "add",
        "The API listens on port 9000",
        "--type",
        "fact",
        "--scope",
        "project",
    ]);
    let other_id = other["memory"]["id"].as_str().expect("id").to_string();
    s.json(&[
        "memory",
        "reconcile",
        "--from",
        &other_id,
        "--to",
        &memory_id,
        "--relation",
        "supersedes",
        "--basis",
        "explicit_user",
    ]);
    s.must(&["delete", "memory", &memory_id]);

    let relations = s.query_column(&format!(
        "SELECT CAST(COUNT(*) AS TEXT) FROM memory_relations
          WHERE to_memory_id = '{memory_id}' AND deleted_at IS NULL"
    ));
    assert_eq!(
        relations,
        vec!["1".to_string()],
        "the decision naming a deleted memory was removed rather than kept"
    );
    let content = s.query_column(&format!(
        "SELECT content FROM memories WHERE id = '{memory_id}'"
    ));
    assert_eq!(
        content,
        vec![String::new()],
        "a deleted memory's content survived"
    );

    // --- Session: tombstoned, and the relations it decided survive.
    //
    // The decision above was made by a session; deleting that session must not
    // take the decision with it. What the session recorded is a fact about the
    // project, not a fact about the session.
    let sessions = s.query_column("SELECT id FROM sessions ORDER BY started_at LIMIT 1");
    if let Some(session_id) = sessions.first() {
        s.must(&["delete", "session", session_id]);
        let surviving = s.query_column(&format!(
            "SELECT CAST(COUNT(*) AS TEXT) FROM memory_relations
              WHERE decided_by_session = '{session_id}' AND deleted_at IS NULL"
        ));
        assert_eq!(
            surviving,
            vec!["1".to_string()],
            "deleting the deciding session removed the decision it made"
        );
    }

    // Nothing anywhere is a reference to a row that is not there.
    for (table, column, target) in [
        ("memory_relations", "from_memory_id", "memories"),
        ("memory_relations", "to_memory_id", "memories"),
        ("memory_evidence_facts", "evidence_id", "evidence_facts"),
        ("task_criteria", "task_id", "tasks"),
        ("task_blockers", "task_id", "tasks"),
    ] {
        let dangling = s.query_column(&format!(
            "SELECT CAST(COUNT(*) AS TEXT) FROM {table} t
              WHERE NOT EXISTS (SELECT 1 FROM {target} x WHERE x.id = t.{column})"
        ));
        assert_eq!(
            dangling,
            vec!["0".to_string()],
            "{table}.{column} points at a {target} row that does not exist"
        );
    }
}

/// Tombstone an evidence fact in a sandbox's store.
fn forget_evidence(s: &Sandbox, id: &str) {
    let id = uuid::Uuid::parse_str(id).expect("an evidence id");
    let path = s.db_path();
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime")
        .block_on(async move {
            let store = cairn_store::Store::open(&path).await.expect("store");
            cairn_store::evidence::forget(&store, id)
                .await
                .expect("forget");
        });
}
