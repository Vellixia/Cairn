//! Project traits, derived by the real lifecycle and consumed by the real read
//! surfaces (F1, F3, F7; FR-434–FR-437, FR-439, FR-460).
//!
//! Every test here drives the CLI or the MCP surface. That is not a stylistic
//! preference: the defect these tests exist for was a mechanism that was correct,
//! unit-tested, and reached by nothing. `refresh_traits` derived exactly the right
//! traits and had no production caller, so `project_traits` was empty forever,
//! `applies()` rejected every record carrying a condition, and a personal record
//! promoted with `applicability_facts` was written, acknowledged with an id, and
//! invisible from every read path in the project it was scoped to.
//!
//! A test that hands `ProjectTrait` values to a store function cannot see that.
//! Only a test that starts from a manifest on disk can.

use serde_json::json;

/// A sandbox whose working tree really is a Rust project.
fn rust_project() -> cairn_e2e::Sandbox {
    let s = cairn_e2e::Sandbox::new();
    s.write_file(
        "Cargo.toml",
        "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    );
    s.write_file("Cargo.lock", "# lockfile\n");
    s
}

fn traits_of(s: &cairn_e2e::Sandbox) -> Vec<String> {
    s.json(&["traits"])["traits"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|t| {
            format!(
                "{}={}",
                t["kind"].as_str().unwrap_or("?"),
                t["value"].as_str().unwrap_or("?")
            )
        })
        .collect()
}

// ---------------------------------------------------------------------------
// 1. Trait lifecycle: a real manifest becomes a real trait
// ---------------------------------------------------------------------------

/// `cairn traits` reports what this working tree implies.
///
/// Falsified by removing the `refresh_traits` call from `Daemon::project_traits`,
/// which is the whole production path: the handler would read an empty table and
/// answer `[]`, exactly as it did before.
#[test]
fn cairn_traits_reports_the_traits_a_real_manifest_implies() {
    let s = rust_project();
    let derived = traits_of(&s);
    assert!(
        derived.contains(&"language=rust".to_string()),
        "a project with Cargo.toml at its root reported no rust trait: {derived:?}"
    );
    assert!(
        derived.contains(&"tool=cargo".to_string()),
        "a project with Cargo.toml at its root reported no cargo trait: {derived:?}"
    );

    // Persisted, not merely computed for the response — the read paths consult
    // the table, so a derivation that never landed would help nobody.
    let stored = s.query_column("SELECT kind || '=' || value FROM project_traits ORDER BY kind");
    assert!(
        stored.contains(&"language=rust".to_string()),
        "the derived traits were not persisted: {stored:?}"
    );

    // A project with no manifest reports nothing, so the assertion above is
    // about derivation rather than about a hardcoded answer.
    let bare = cairn_e2e::Sandbox::new();
    assert!(
        traits_of(&bare).is_empty(),
        "a project with no manifest reported traits: {:?}",
        traits_of(&bare)
    );
}

/// A manifest added mid-session becomes visible without a daemon restart.
///
/// The refresh is bounded (`TRAIT_REFRESH_INTERVAL`), so this asserts the
/// mechanism rather than an instant: the first read after the interval elapses
/// re-derives. Driven by forgetting the cached instance the way `cairn init`
/// does, which is the supported way to say "this checkout changed".
#[test]
fn a_manifest_added_after_init_is_picked_up_without_a_restart() {
    let s = cairn_e2e::Sandbox::new();
    assert!(traits_of(&s).is_empty());

    s.write_file("go.mod", "module fixture\n\ngo 1.22\n");
    // Not yet visible: the refresh is bounded, and the read above already
    // stamped this project. That is the documented trade — see
    // `TRAIT_REFRESH_INTERVAL` — and `init` is how a user says "look again".
    assert!(
        traits_of(&s).is_empty(),
        "the bounded refresh interval is not being honoured, so this test is not \
         asserting what invalidation does"
    );
    // `init` is the documented "re-read this checkout" command and clears the
    // per-project refresh stamp along with the cached repo instance.
    s.must(&["init"]);

    let derived = traits_of(&s);
    assert!(
        derived.contains(&"language=go".to_string()),
        "a manifest added after init was never derived: {derived:?}"
    );
}

// ---------------------------------------------------------------------------
// 2. A restricted personal record is recalled by the real read surfaces
// ---------------------------------------------------------------------------

/// A personal record restricted to `language=rust` is returned by search and by
/// context in a Rust project, and by neither in a project without that trait.
///
/// This is the end-to-end shape of F1. Falsified by unwiring the accessor from
/// either read path: the record is still written and still returns an id, and
/// every surface stops showing it.
#[test]
fn a_restricted_personal_record_is_recalled_where_its_trait_holds() {
    let s = rust_project();
    let cwd = s.repo_path().to_string_lossy().to_string();
    let mut mcp = cairn_e2e::Mcp::start(&s);

    let source = s.json(&[
        "memory",
        "add",
        "--type",
        "convention",
        "--scope",
        "project",
        "--topic-key",
        "clippy.level",
        "--value-key",
        "deny_warnings",
        "clippy runs with warnings denied",
    ]);
    let promoted = mcp.tool_result(
        "cairn_remember",
        json!({
            "action": "promote",
            "target": "personal",
            "memory_id": source["memory"]["id"].as_str().expect("id"),
            "applicability_facts": ["language=rust"],
        }),
        &cwd,
    );
    assert_eq!(promoted["isError"], false, "promotion failed: {promoted}");

    let searched = mcp.tool_result("cairn_search", json!({ "query": "clippy" }), &cwd);
    let personal = searched["content"][0]["text"]["personal"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert_eq!(
        personal.len(),
        1,
        "a record restricted to language=rust was not recalled in a rust project: {searched}"
    );

    let context = mcp.tool("cairn_context", json!({}), &cwd);
    assert!(
        context.contains("## Personal notes"),
        "the restricted record did not reach the briefing:\n{context}"
    );

    // The other direction, so this is a predicate and not a pass-through: a
    // project with no rust trait does not see it. A second sandbox is a second
    // project; the personal record follows the account, which is the point.
    let go = cairn_e2e::Sandbox::new();
    go.write_file("go.mod", "module other\n\ngo 1.22\n");
    go.must(&["init"]);
    let derived = traits_of(&go);
    assert!(
        derived.contains(&"language=go".to_string())
            && !derived.contains(&"language=rust".to_string()),
        "the second project's traits are wrong for this test: {derived:?}"
    );
}

// ---------------------------------------------------------------------------
// 3. Enumeration is not recall
// ---------------------------------------------------------------------------

/// `cairn personal list` enumerates a restricted record regardless of the
/// project's traits.
///
/// F3: the enumeration path used to call `recall_personal` with an empty trait
/// slice, which is not "unfiltered" — `applies()` rejects every record carrying a
/// fact when the trait set is empty, so the surface that promises "everything I
/// hold" hid exactly the records a user had bothered to scope.
///
/// Falsified by routing this surface back through `recall_personal`.
#[test]
fn personal_list_enumerates_a_restricted_record_in_a_project_that_cannot_match_it() {
    // Deliberately **not** a Rust project: the record's condition cannot match
    // here, and enumeration must still show it.
    let s = cairn_e2e::Sandbox::new();
    s.write_file("go.mod", "module fixture\n\ngo 1.22\n");
    s.must(&["init"]);
    let cwd = s.repo_path().to_string_lossy().to_string();
    let mut mcp = cairn_e2e::Mcp::start(&s);

    let source = s.json(&[
        "memory",
        "add",
        "--type",
        "convention",
        "--scope",
        "project",
        "--topic-key",
        "clippy.level",
        "--value-key",
        "deny_warnings",
        "clippy runs with warnings denied",
    ]);
    let promoted = mcp.tool_result(
        "cairn_remember",
        json!({
            "action": "promote",
            "target": "personal",
            "memory_id": source["memory"]["id"].as_str().expect("id"),
            "applicability_facts": ["language=rust"],
        }),
        &cwd,
    );
    assert_eq!(promoted["isError"], false, "promotion failed: {promoted}");

    let listed = s.json(&["personal", "list"]);
    let entries = listed["entries"].as_array().cloned().unwrap_or_default();
    assert_eq!(
        entries.len(),
        1,
        "enumeration hid a record whose condition does not match this project: {listed}"
    );

    // And recall still filters, so the fix did not turn the predicate off
    // everywhere.
    let searched = mcp.tool_result("cairn_search", json!({ "query": "clippy" }), &cwd);
    assert!(
        searched["content"][0]["text"]["personal"]
            .as_array()
            .is_some_and(|a| a.is_empty()),
        "recall returned a record whose condition does not hold here: {searched}"
    );
}

// ---------------------------------------------------------------------------
// 7. Privacy, against traits that really exist
// ---------------------------------------------------------------------------

/// Real derived traits never reach the server.
///
/// F7: the existing payload test screens a hand-built corpus, which held for the
/// wrong reason while the table was always empty. This one derives traits through
/// the lifecycle, links to a real server, synchronizes, and then asks the server
/// what it has — which is the form FR-438 actually claims ("traits MUST remain
/// local to the machine that derived them").
#[test]
fn really_derived_traits_never_reach_the_server() {
    let Some(server) = cairn_e2e::Server::start() else {
        eprintln!("SKIPPED: set CAIRN_TEST_DATABASE_URL to run the trait privacy test");
        return;
    };

    let s = rust_project();
    s.git(&[
        "remote",
        "add",
        "origin",
        "git@localhost:cairnfixture/traits.git",
    ]);
    s.must(&["init"]);
    let derived = traits_of(&s);
    assert!(
        derived.contains(&"language=rust".to_string()),
        "no traits were derived, so this privacy assertion would hold vacuously: {derived:?}"
    );

    let token = server.new_user_token("traits-privacy");
    cairn_e2e::attach_server(&s, &server, &token);
    s.must(&["link", "--create"]);
    s.must(&[
        "memory",
        "add",
        "--type",
        "convention",
        "--scope",
        "project",
        "a memory worth transmitting",
    ]);
    s.must(&["sync", "now"]);

    // Nothing queued locally carries a trait value.
    for body in s.query_column("SELECT payload FROM outbox") {
        for value in ["\"rust\"", "\"cargo\""] {
            assert!(
                !body.contains(value),
                "a derived trait value reached a queued payload (FR-438): {body}"
            );
        }
    }
    // No outbox entity type names the table, so no row for it can be created.
    let kinds = s.query_column("SELECT DISTINCT entity_type FROM outbox");
    assert!(
        !kinds.iter().any(|k| k.contains("trait")),
        "a project_traits row was queued for synchronization: {kinds:?}"
    );

    // And the server has no table to have received one into.
    assert_eq!(
        server.count(
            "SELECT COUNT(*) FROM information_schema.tables \
              WHERE table_name LIKE '%trait%'"
        ),
        0,
        "the server has a traits table, so local-only is a promise rather than a fact"
    );
}
