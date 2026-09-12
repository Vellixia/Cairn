//! T127 — the audit, as tests rather than as a promise.
//!
//! Every item here is something Feature 002 committed to *not* building, and
//! every one of them is a thing that would have been easy to build by
//! accident. A seventh MCP tool is one convenient helper away. A committed
//! manifest is one "wouldn't it be nice if cloning set it up" away. An
//! agent-keyed memory scope is one line in a filter away.
//!
//! Prose in a specification does not prevent any of that. A failing test does.

use cairn_e2e::Sandbox;
use serde_json::json;

#[test]
fn the_mcp_surface_is_the_same_six_tools_feature_001_shipped() {
    // FR-128, SC-106, and §Out of Scope: "expanding the MCP tool surface
    // beyond the six Feature 001 tools".
    let s = Sandbox::new();
    let mut mcp = cairn_e2e::Mcp::start(&s);
    mcp.call("initialize", json!({}));
    let names: Vec<String> = mcp.call("tools/list", json!({}))["tools"]
        .as_array()
        .expect("tools")
        .iter()
        .filter_map(|t| t["name"].as_str().map(str::to_string))
        .collect();

    assert_eq!(
        names,
        vec![
            "cairn_context",
            "cairn_search",
            "cairn_remember",
            "cairn_session",
            "cairn_task",
            "cairn_handoff",
        ],
        "the MCP surface changed"
    );
    // Feature 002's own operations are developer commands, not agent tools:
    // an agent that could connect and disconnect itself is an agent that can
    // edit the developer's configuration unprompted.
    for forbidden in [
        "cairn_connect",
        "cairn_doctor",
        "cairn_repair",
        "cairn_disconnect",
        "cairn_agents",
        "cairn_integration",
    ] {
        assert!(
            !names.iter().any(|n| n == forbidden),
            "{forbidden} is a tool"
        );
    }
}

#[test]
fn no_feature_002_record_has_an_outbox_entity_type_or_a_server_column() {
    // FR-183, FR-184: integration state is local, and the server schema knows
    // nothing about it. Asserted against the schema rather than against
    // behavior, because a CHECK constraint cannot be forgotten.
    let s = Sandbox::new();
    s.must(&["init"]);

    let outbox = s
        .query_column("SELECT sql FROM sqlite_master WHERE type='table' AND name='outbox'")
        .first()
        .cloned()
        .unwrap_or_default();
    assert!(
        outbox.contains("'project', 'task', 'session', 'memory', 'handoff'"),
        "{outbox}"
    );

    // Every Feature 002 table exists locally and none of them is syncable.
    for table in [
        "agent_integrations",
        "manager_integrations",
        "installed_resources",
        "resource_bindings",
        "capability_evidence",
        "migration_states",
        "recovery_artifacts",
    ] {
        let sql = s
            .query_column(&format!(
                "SELECT sql FROM sqlite_master WHERE type='table' AND name='{table}'"
            ))
            .first()
            .cloned()
            .unwrap_or_default();
        assert!(!sql.is_empty(), "{table} does not exist");
        assert!(
            !outbox.contains(table),
            "{table} appears in the outbox schema"
        );
        // No sync bookkeeping crept onto an integration row.
        for column in ["synced_at", "server_id", "outbox", "remote_"] {
            assert!(
                !sql.contains(column),
                "{table} carries `{column}`, which is sync state"
            );
        }
    }
}

#[test]
fn no_memory_scope_partition_or_filter_is_keyed_to_an_agent() {
    // §Out of Scope: "any new memory scope, partition, or ownership domain
    // based on agent identity". The scope vocabulary is Feature 001's four,
    // and agent identity is provenance only (FR-189, FR-190).
    let s = Sandbox::new();
    s.install_agent("claude-code");
    s.install_agent("codex");
    s.must(&["init"]);

    s.hook(
        "SessionStart",
        json!({ "session_id": "audit-1", "source": "startup" }),
    );
    s.settle_session_count(1);
    s.must(&[
        "memory",
        "add",
        "A fact about pagination",
        "--type",
        "fact",
        "--scope",
        "project",
    ]);

    let scopes = s.query_column("SELECT DISTINCT scope FROM memories");
    for scope in &scopes {
        assert!(
            ["project", "branch", "task", "session"].contains(&scope.as_str()),
            "an agent-shaped memory scope exists: {scope}"
        );
    }

    // And the scope column of the schema admits nothing else.
    let sql = s
        .query_column("SELECT sql FROM sqlite_master WHERE type='table' AND name='memories'")
        .first()
        .cloned()
        .unwrap_or_default();
    assert!(!sql.to_lowercase().contains("agent"), "{sql}");
}

#[test]
fn cloning_a_repository_installs_and_activates_nothing() {
    // FR-227 and §Out of Scope: no committed manifest, no drift handling, no
    // merge semantics, and no automatic application of intent on clone. The
    // committed half of a `--shared` installation is an *offer*.
    let s = Sandbox::new();
    s.install_agent("claude-code");
    s.must(&["init"]);
    s.must(&["connect", "claude-code", "--shared", "--yes"]);

    // Everything a collaborator would receive by cloning.
    let committed: Vec<std::path::PathBuf> = [".mcp.json", "CLAUDE.md", ".claude/settings.json"]
        .iter()
        .map(|p| s.repo_dir().join(p))
        .filter(|p| p.exists())
        .collect();
    assert!(
        committed.len() >= 2,
        "a shared connect committed almost nothing, so this proves little: {committed:?}"
    );

    // Commit them, the way the developer who ran `--shared` would.
    s.git(&["add", "-A"]);
    s.git(&[
        "-c",
        "user.email=t@example.test",
        "-c",
        "user.name=t",
        "commit",
        "-qm",
        "share",
    ]);

    // The clone: the same committed files, a machine that has never run Cairn.
    let clone = s.cairn_home().join("clone");
    let out = std::process::Command::new("git")
        .args(["clone", "--quiet"])
        .arg(s.repo_dir())
        .arg(&clone)
        .output()
        .expect("git clone");
    assert!(
        out.status.success(),
        "clone failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // Nothing about Cairn is active there: no project, no session, no capture.
    let projects_before = s.query_column("SELECT CAST(COUNT(*) AS TEXT) FROM projects");
    let sessions_before = s.query_column("SELECT CAST(COUNT(*) AS TEXT) FROM sessions");

    // Reading the committed configuration is all a clone does. Cairn is not
    // invoked, so nothing is applied — and if something were watching for a
    // manifest, this is where it would fire.
    assert!(clone.join("CLAUDE.md").exists(), "the offer did not travel");
    assert_eq!(
        s.query_column("SELECT CAST(COUNT(*) AS TEXT) FROM projects"),
        projects_before,
        "cloning registered a project"
    );
    assert_eq!(
        s.query_column("SELECT CAST(COUNT(*) AS TEXT) FROM sessions"),
        sessions_before,
        "cloning started a session"
    );

    // And no manifest of Cairn's own was committed for it to apply.
    for name in [
        "cairn.toml",
        "cairn.json",
        ".cairn.toml",
        ".cairn.json",
        ".cairn/integration.json",
        "cairn.integration.json",
    ] {
        assert!(
            !clone.join(name).exists(),
            "a committed integration manifest exists: {name}"
        );
    }
}

#[test]
fn a_hand_edited_resource_is_reported_and_never_adopted() {
    // §Out of Scope: "adopting a developer's hand-edited version of a
    // Cairn-managed resource as the new Cairn-managed content".
    let s = Sandbox::new();
    s.install_agent("claude-code");
    s.must(&["init"]);
    s.must(&["connect", "claude-code", "--yes"]);

    let path = s.repo_dir().join("CLAUDE.md");
    let original = std::fs::read_to_string(&path).expect("the instruction file");
    let edited = original.replace(
        "Search Cairn memory",
        "Search Cairn memory (edited by the developer)",
    );
    assert_ne!(edited, original, "the fixture did not edit anything");
    std::fs::write(&path, &edited).expect("write");

    // Doctor reports it rather than accepting it.
    let doctor = s.cairn(&["--json", "doctor"]).stdout;
    assert!(
        doctor.contains("modified"),
        "a hand-edited resource was not reported: {doctor}"
    );

    // And a default repair refuses rather than silently taking either side.
    let repair = s.cairn(&["repair", "claude-code"]);
    assert_ne!(repair.code, 0, "a default repair overwrote a hand edit");
    assert_eq!(
        std::fs::read_to_string(&path).expect("still there"),
        edited,
        "a refused repair changed the file anyway"
    );
}

#[test]
fn cairn_introduces_no_second_service_and_no_second_datastore() {
    // §Out of Scope: "a second Cairn service, broker, or datastore introduced
    // to support adapters". Feature 002 added a crate, not a process.
    let s = Sandbox::new();
    s.install_agent("claude-code");
    s.must(&["init"]);
    s.must(&["connect", "claude-code", "--yes"]);

    // One database file, and it is Feature 001's.
    let mut databases: Vec<String> = Vec::new();
    let mut sockets: Vec<String> = Vec::new();
    let mut stack = vec![s.cairn_home()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path == s.fake_home() {
                continue;
            }
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            if name.ends_with(".sqlite3") {
                databases.push(name);
            } else if name.ends_with(".sock") {
                sockets.push(name);
            }
        }
    }
    assert_eq!(
        databases,
        vec!["cairn.sqlite3"],
        "a second datastore exists"
    );
    assert!(
        sockets.len() <= 1,
        "a second service is listening: {sockets:?}"
    );
}

// ---------------------------------------------------------------------------
// T135 — what Feature 003 guarantees by absence (FR-314, FR-345, FR-381,
// SC-327)
// ---------------------------------------------------------------------------

/// Every `.rs`, `.sql` and asset file this workspace ships.
fn source_files() -> Vec<(std::path::PathBuf, String)> {
    fn walk(dir: &std::path::Path, out: &mut Vec<(std::path::PathBuf, String)>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if name == "target" || name == ".git" || name == "node_modules" {
                continue;
            }
            if path.is_dir() {
                walk(&path, out);
            } else if matches!(
                path.extension().and_then(|e| e.to_str()),
                Some("rs") | Some("sql") | Some("md") | Some("json")
            ) {
                if let Ok(text) = std::fs::read_to_string(&path) {
                    out.push((path, text));
                }
            }
        }
    }
    // `CARGO_MANIFEST_DIR` is `tests/`; the workspace root is its parent.
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("a workspace root")
        .to_path_buf();
    let mut out = Vec::new();
    for sub in ["crates", "tests/src", "skills"] {
        walk(&root.join(sub), &mut out);
    }
    out
}

/// No scope, partition, ownership domain or retrieval filter was added beyond
/// Feature 001's four (FR-381).
///
/// Feature 003 adds subject identity, verification, importance and pins, and
/// **none** of them is a scope. The distinction is the whole reason the
/// mechanism is safe to add: a subject narrows *within* a scope and never
/// crosses one.
#[test]
fn feature_003_added_no_scope_and_no_precedence() {
    use cairn_core::domain::MemoryScope;

    let mut scopes: Vec<&str> = MemoryScope::ALL.iter().map(|s| s.as_str()).collect();
    scopes.sort();
    assert_eq!(
        scopes,
        vec!["branch", "project", "session", "task"],
        "the scope vocabulary changed; every addition to it is a retrieval decision"
    );

    // And nothing Feature 003 added can change which scope wins. Scope
    // precedence is a function of the scope alone: an `importance: high`
    // branch memory must never outrank a task memory, or "narrowest correct
    // scope" stops meaning anything.
    let ranking = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("root")
            .join("crates/cairn-store/src/search.rs"),
    )
    .expect("search.rs");

    // `scope_bucket` is a SQL column alias produced by a `CASE m.scope WHEN
    // ... AS scope_bucket` expression, not a Rust function — there is no
    // `fn scope_bucket` anywhere in this codebase. The original test looked
    // for one anyway: `.split("fn scope_bucket").nth(1)` never matched, so
    // `.unwrap_or_default()` silently produced `""`, and every assertion
    // below passed against an empty string for the test's entire life
    // (T174). `.expect()` in place of `.unwrap_or_default()` turns "the
    // thing I'm inspecting doesn't exist" into a hard failure instead of a
    // vacuous pass — which is the whole fix: this test must break, loudly,
    // the moment scope precedence moves or is renamed out from under it.
    let case_start = ranking.find("CASE m.scope").expect(
        "the scope_bucket CASE expression is missing from search.rs — \
         has scope precedence moved, been renamed, or been removed?",
    );
    let alias_offset = ranking[case_start..].find("AS scope_bucket").expect(
        "`AS scope_bucket` is missing from search.rs — has the precedence \
         column been renamed?",
    );
    let bucket = &ranking[case_start..case_start + alias_offset];

    for forbidden in [
        "importance",
        "pinned",
        "verification",
        "verification_authority",
    ] {
        assert!(
            !bucket.contains(forbidden),
            "`{forbidden}` takes part in scope precedence: {bucket}"
        );
    }

    // Excluding those signals from the bucket expression proves nothing on
    // its own unless the bucket actually governs ordering ahead of
    // relevance — a differently-shaped `ORDER BY` could still let a
    // relevance or importance signal outrank scope even with a "clean"
    // bucket expression sitting unused elsewhere in the file.
    let order_by_start = ranking.find("ORDER BY scope_bucket").expect(
        "the query no longer orders by scope_bucket — has scope precedence \
         been dropped from the ORDER BY clause?",
    );
    let order_by = &ranking[order_by_start..];
    let bucket_pos = order_by
        .find("scope_bucket")
        .expect("unreachable: matched above");
    let relevance_pos = order_by
        .find("relevance")
        .expect("`relevance` is missing from the ORDER BY clause");
    assert!(
        bucket_pos < relevance_pos,
        "scope_bucket must be ordered before relevance, or scope no longer \
         dominates: {order_by}"
    );
}

/// No vocabulary, taxonomy or registry of topic keys exists anywhere (FR-314).
///
/// A topic key is a *convention*, agreed by agents writing them, and not a
/// controlled list. A registry would need someone to maintain it, would be
/// wrong the moment a project did something new, and would turn an
/// unrepresentable key from "stored free-form and reported" into a rejection.
#[test]
fn no_topic_key_vocabulary_exists() {
    let suspicious = [
        "TOPIC_KEYS",
        "KNOWN_TOPICS",
        "TOPIC_VOCABULARY",
        "TOPIC_TAXONOMY",
        "TOPIC_REGISTRY",
        "ALLOWED_TOPIC",
        "topic_keys.json",
        "topics.toml",
    ];
    for (path, text) in source_files() {
        for marker in suspicious {
            assert!(
                !text.contains(marker),
                "{} names `{marker}`, which would make topic keys a controlled list",
                path.display()
            );
        }
    }

    // The one place a list could hide is a CHECK constraint on the column.
    let s = Sandbox::new();
    s.must(&["init"]);
    let sql = s
        .query_column("SELECT sql FROM sqlite_master WHERE type='table' AND name='memories'")
        .first()
        .cloned()
        .unwrap_or_default();
    let topic_line = sql
        .lines()
        .find(|l| l.contains("topic_key"))
        .unwrap_or_default();
    assert!(
        !topic_line.to_uppercase().contains("CHECK"),
        "`topic_key` is constrained to a fixed set: {topic_line}"
    );
}

/// No valid-time table, retroactive correction or branching history exists
/// (FR-345).
///
/// Cairn records when a proposal was *effective* and when it was superseded.
/// It does not model when a fact was true in the world, and it cannot rewrite
/// what an earlier session was told — a history that can be edited is not a
/// history.
#[test]
fn no_valid_time_or_branching_history_exists() {
    let s = Sandbox::new();
    s.must(&["init"]);
    let tables = s.query_column("SELECT name FROM sqlite_master WHERE type='table'");
    for table in &tables {
        let lower = table.to_lowercase();
        for forbidden in ["valid_time", "valid_from", "bitemporal", "history_branch"] {
            assert!(
                !lower.contains(forbidden),
                "a {forbidden} table exists: {table}"
            );
        }
    }

    // And no code path rewrites an interval after the fact.
    //
    // **Comments are stripped first**, the same way
    // `global_content_validation.rs` strips them before its own source sweep
    // and for the same reason: naming a thing in prose is not implementing it.
    // The word that tripped this was a comment in the ingest path explaining
    // that a schema change must *not* destroy decisions retroactively — an
    // audit that reads a promise not to do something as evidence of doing it
    // teaches people to stop writing the promise down.
    //
    // A function called `retroactive_amend` is still caught, which is the
    // thing FR-345 is actually about.
    for (path, text) in source_files() {
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let code: String = text
            .lines()
            .filter(|l| {
                let t = l.trim_start();
                !t.starts_with("//") && !t.starts_with("*") && !t.starts_with("/*")
            })
            .collect::<Vec<_>>()
            .join("\n");
        for forbidden in ["retroactive", "rewrite_history", "amend_interval"] {
            assert!(
                !code.contains(forbidden),
                "{} names `{forbidden}` in code",
                path.display()
            );
        }
    }
}
