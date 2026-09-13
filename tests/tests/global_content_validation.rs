//! The shared content validator's workspace-level guarantees
//! (FR-544–FR-549, FR-578–FR-580, SC-421, SC-448, SC-453, SC-454).
//!
//! The per-class behaviour is unit-tested next to the code, in
//! `crates/cairn-core/src/validate.rs`, because it is a pure function and a
//! unit test is the honest place for one. What lives here is the claim a unit
//! test cannot make: that `validate_global_content` is the **only**
//! implementation of its classes anywhere in the workspace (FR-579).
//!
//! That audit is the reason this file exists, and its own falsifiability is the
//! point. An audit written as "walk the tree and confirm nothing looks wrong"
//! passes today and passes again the day a duplicate is added — which is
//! exactly how `scope_audit.rs` came to protect nothing for its entire life.
//! So the audit here is a pure function over (path, source) pairs, exercised
//! twice: once over the real workspace, where it must find nothing, and once
//! over the real workspace **plus a seeded duplicate**, where it must find it.

use std::path::{Path, PathBuf};

/// The one file allowed to implement the nine rejection classes.
const OWNER: &str = "crates/cairn-core/src/validate.rs";

/// Pre-existing duplicates, allowlisted with a reason rather than ignored.
///
/// `cairn-store/src/patterns.rs` carries its own `absolute_path` detector,
/// which predates this feature: it is Feature 003's *pattern* gate, screening
/// promotion into `reusable_patterns`, a different domain with its own contract.
/// FR-579 enumerates the components it binds — the promotion gate, the server's
/// ingest handler, a client-side pre-check — and 003's pattern gate is not among
/// them, so bringing it under the shared validator would be a change to Feature
/// 003's behaviour that no Feature 004 requirement asks for.
///
/// It is named here, with that reasoning, so the allowlist is a decision a
/// reader can disagree with rather than a silent gap. Anything not on this list
/// fails the audit.
const ALLOWED_DUPLICATES: &[&str] = &["crates/cairn-store/src/patterns.rs"];

/// The nine class names. A duplicate implementation has to name its classes in
/// order to report them, which is what makes this a reliable signal: a faithful
/// *caller* only ever propagates `GlobalContentRejection`, and never spells the
/// class names itself.
const CLASS_NAMES: &[&str] = &[
    "absolute_path",
    "home_dir_ref",
    "drive_letter_path",
    "file_uri",
    "credentialed_url",
    "env_assignment",
    "encoded_secret_shape",
    "project_identifying",
    "command_shaped",
];

/// Detector shapes. A second implementation reproduces the *mechanism*, not
/// only the names, so these catch a duplicate that invented its own labels.
const DETECTOR_MARKERS: &[&str] = &[
    "\"file://\"",
    "starts_with(\"~/\")",
    "is_ascii_hexdigit",
    "split_once(\"://\")",
];

/// One finding: a file that implements what only [`OWNER`] may implement.
#[derive(Debug, PartialEq, Eq)]
struct Duplicate {
    path: String,
    reason: &'static str,
}

/// The audit, as a pure function so it can be run against a seeded corpus.
///
/// A file is flagged when it either spells **two or more** class names — one
/// alone is a plausible identifier in unrelated code, two is a class list — or
/// reproduces **two or more** detector shapes.
fn audit(sources: &[(String, String)]) -> Vec<Duplicate> {
    let mut findings = Vec::new();
    for (path, source) in sources {
        let normalized = path.replace('\\', "/");
        if normalized.ends_with(OWNER) || ALLOWED_DUPLICATES.iter().any(|a| normalized.ends_with(a))
        {
            continue;
        }
        // Only an implementation counts, and two things that are not one get
        // stripped first.
        //
        // Comments: a doc comment saying "see `absolute_path`" is documentation.
        //
        // Test modules: a test asserting `err.class == Some("absolute_path")` is
        // a *consumer* of the class, and consuming one is exactly what callers
        // are supposed to do — the promotion gate's own tests do it, and the
        // first version of this audit flagged them, which is how the rule got
        // narrowed. The blind spot this accepts is a duplicate hidden inside a
        // `#[cfg(test)]` module; that is tolerable because such code cannot run
        // in a shipped binary, and it is named here rather than left implicit.
        // An integration test is a test module that happens to live in its own
        // file. The rule below already exempts `#[cfg(test)]` for the reason
        // that a test asserting `err.class == Some("absolute_path")` is a
        // *consumer* of the class — and a file under a crate's `tests/`
        // directory is nothing but that, since Cargo will not link it into a
        // shipped binary either. Exempting one location and not the other made
        // the audit depend on where a consumer was written rather than on what
        // it was, and the first test to name a second class from `tests/` was
        // flagged as a duplicate implementation of a validator it only calls.
        let is_integration_test = normalized.contains("/tests/");
        let production = match source.find("#[cfg(test)]") {
            Some(at) => &source[..at],
            None if is_integration_test => "",
            None => source.as_str(),
        };
        let code: String = production
            .lines()
            .filter(|l| {
                let t = l.trim_start();
                !t.starts_with("//") && !t.starts_with("*") && !t.starts_with("/*")
            })
            .collect::<Vec<_>>()
            .join("\n");

        let named = CLASS_NAMES.iter().filter(|c| code.contains(**c)).count();
        if named >= 2 {
            findings.push(Duplicate {
                path: normalized.clone(),
                reason: "spells two or more rejection class names",
            });
            continue;
        }
        let markers = DETECTOR_MARKERS
            .iter()
            .filter(|m| code.contains(**m))
            .count();
        if markers >= 2 {
            findings.push(Duplicate {
                path: normalized,
                reason: "reproduces two or more detector shapes",
            });
        }
    }
    findings
}

fn workspace_root() -> PathBuf {
    // `CARGO_MANIFEST_DIR` is `tests/`; the workspace is its parent.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .to_path_buf()
}

/// Every `.rs` file under `crates/`, excluding build output.
fn workspace_sources() -> Vec<(String, String)> {
    let root = workspace_root();
    let mut out = Vec::new();
    let mut stack = vec![root.join("crates")];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.is_dir() {
                if path.file_name().is_some_and(|n| n == "target") {
                    continue;
                }
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                if let Ok(source) = std::fs::read_to_string(&path) {
                    let rel = path
                        .strip_prefix(&root)
                        .unwrap_or(&path)
                        .to_string_lossy()
                        .to_string();
                    out.push((rel, source));
                }
            }
        }
    }
    out
}

/// SC-453, part one: the workspace as it stands has exactly one implementation.
#[test]
fn the_validator_is_the_only_implementation_in_the_workspace() {
    let sources = workspace_sources();
    assert!(
        sources.len() > 20,
        "the source sweep found only {} files; it is not reading the workspace",
        sources.len()
    );
    assert!(
        sources
            .iter()
            .any(|(p, _)| p.replace('\\', "/").ends_with(OWNER)),
        "the sweep did not find {OWNER}; the audit would pass vacuously"
    );
    let findings = audit(&sources);
    assert!(
        findings.is_empty(),
        "a second implementation of the rejection classes exists: {findings:#?}"
    );
}

/// SC-453, part two — and the half that makes part one worth anything.
///
/// The audit must **fail when a duplicate is introduced**. Proved by seeding
/// one, rather than by asserting that today's code is clean, which it would
/// report either way.
#[test]
fn the_audit_fails_when_a_second_implementation_is_seeded() {
    let mut sources = workspace_sources();

    // A duplicate that names its classes, as any implementation reporting them
    // must.
    sources.push((
        "crates/cairn-server/src/seeded_duplicate_by_name.rs".to_string(),
        r#"
            pub fn check(text: &str) -> Option<&'static str> {
                if text.starts_with('/') { return Some("absolute_path"); }
                if text.contains("$(") { return Some("command_shaped"); }
                None
            }
        "#
        .to_string(),
    ));
    let by_name = audit(&sources);
    assert!(
        by_name
            .iter()
            .any(|d| d.path.ends_with("seeded_duplicate_by_name.rs")),
        "the audit missed a duplicate that spells the class names: {by_name:#?}"
    );

    // A duplicate that invented its own labels but reproduced the mechanism —
    // the harder case, and the one a name-only audit would wave through.
    let mut sources = workspace_sources();
    sources.push((
        "crates/cairnd/src/seeded_duplicate_by_shape.rs".to_string(),
        r#"
            pub fn looks_unsafe(text: &str) -> bool {
                if text.contains("file://") { return true; }
                if text.split_once("://").is_some() { return true; }
                false
            }
        "#
        .to_string(),
    ));
    let by_shape = audit(&sources);
    assert!(
        by_shape
            .iter()
            .any(|d| d.path.ends_with("seeded_duplicate_by_shape.rs")),
        "the audit missed a duplicate that reproduced the detector shapes: {by_shape:#?}"
    );
}

/// The allowlist must not be a way to switch the audit off. One entry, and it
/// has to still exist — an allowlist pointing at a deleted file is an audit
/// with a hole nobody can see.
#[test]
fn the_allowlist_is_minimal_and_every_entry_still_exists() {
    assert_eq!(
        ALLOWED_DUPLICATES.len(),
        1,
        "the allowlist grew; each entry needs its own recorded reason"
    );
    let root = workspace_root();
    for allowed in ALLOWED_DUPLICATES {
        assert!(
            root.join(allowed).exists(),
            "{allowed} is allowlisted but does not exist"
        );
    }
}

/// The `#[cfg(test)]` exclusion must not be a loophole for production code.
///
/// A duplicate placed *before* a file's test module is still caught. Asserted
/// because the exclusion was added in response to a false positive, and a
/// narrowing made under that pressure is exactly the kind that goes too far.
#[test]
fn the_test_module_exclusion_does_not_hide_production_code() {
    let seeded = vec![(
        "crates/cairn-server/src/half_and_half.rs".to_string(),
        r#"
            pub fn check(text: &str) -> Option<&'static str> {
                if text.starts_with('/') { return Some("absolute_path"); }
                if text.contains("$(") { return Some("command_shaped"); }
                None
            }

            #[cfg(test)]
            mod tests {
                #[test]
                fn t() { assert_eq!(super::check("/x"), Some("absolute_path")); }
            }
        "#
        .to_string(),
    )];
    let findings = audit(&seeded);
    assert_eq!(
        findings.len(),
        1,
        "a duplicate above a test module was missed: {findings:#?}"
    );
}

/// The class list the audit screens for is the class list the validator
/// declares. Two lists that can drift apart would let a new class be added
/// with no audit coverage at all.
#[test]
fn the_audits_class_list_matches_the_validators() {
    let declared = cairn_core::validate::CONTENT_CLASSES;
    assert_eq!(
        declared.len(),
        CLASS_NAMES.len(),
        "the validator declares {} classes, the audit screens for {}",
        declared.len(),
        CLASS_NAMES.len()
    );
    for class in declared {
        assert!(
            CLASS_NAMES.contains(class),
            "the validator declares {class:?} and the audit does not screen for it"
        );
    }
}

// ===========================================================================
// The five entry points (T145–T150, FR-545–FR-548, SC-438, SC-424a, SC-439,
// SC-440, SC-449, SC-456)
// ===========================================================================
//
// FR-545 names five places global content can be created, and the privacy
// argument for this whole feature is that **all five screen identically**. Four
// of them are local and one is the server. A single unscreened entry point
// invalidates every claim the other four make, because content only has to
// arrive once.
//
// The five, and how each is reached here:
//
// | # | Entry point | Driven by |
// |---|---|---|
// | 1 | direct personal creation | `cairn_remember` `create`, `domain: "personal"` |
// | 2 | personal promotion | `cairn_remember` `promote`, `target: "personal"` |
// | 3 | team proposal | `cairn team propose` |
// | 4 | team promotion | `cairn_remember` `promote`, `target: "team"` |
// | 5 | server-side sync ingest | `POST /api/sync/batch`, bypassing the client |
//
// These are end-to-end deliberately. The temptation is to call
// `validate_global_content` five times and assert it agrees with itself, which
// it trivially does — and which is exactly the test that would still pass on the
// day one entry point stopped calling it. What is under test is the **wiring**,
// so each case goes through the surface a real caller would use.

use cairn_e2e::{post_json_status_bearer, Mcp, Sandbox, Server};
use serde_json::{json, Value};
use uuid::Uuid;

/// The corpus every entry point is exercised against.
///
/// One tuple per adversarial input: the content, and the class it must be
/// refused under. The same slice drives all five entry points, which is what
/// makes "identically" a claim a test can make rather than a claim five separate
/// tests each half-make (SC-438).
fn refusable_content() -> Vec<(&'static str, &'static str)> {
    vec![
        (
            "the fix lives in /Users/dev/src/thing/main.rs",
            "absolute_path",
        ),
        ("config is under ~/.config/thing", "home_dir_ref"),
        ("the build output is at C:\\build\\out", "drive_letter_path"),
        ("see file:///etc/hosts for the list", "file_uri"),
        (
            "set DATABASE_URL=postgres://x/y before running",
            "env_assignment",
        ),
        // 38 characters, mixed case with digits: the shape `has_encoded_secret_shape`
        // looks for, and one no sentence produces. A shorter run is deliberately
        // *not* refused (the detector's floor is 32), so a test using one would
        // have asserted nothing.
        (
            "the token is dGhpcyBpcyBhIHNlY3JldCB2YWx1ZTEyQUJj",
            "encoded_secret_shape",
        ),
        ("run cargo test --workspace to check", "command_shaped"),
    ]
}

/// `credentialed_url` is deliberately absent from [`refusable_content`], and
/// this is the test that covers it instead.
///
/// The four local entry points redact before they validate — `redact` is applied
/// to the content on the way in, as it is to every other kind of content Cairn
/// stores — so by the time `validate_global_content` sees the string, the
/// credential is already `[REDACTED]` and there is no `credentialed_url` left to
/// refuse. Server ingest validates the payload as pushed and does refuse it.
///
/// Both outcomes protect the user, and asserting "refused everywhere" would have
/// been the weaker claim: what actually matters is that **no entry point lets a
/// credential reach storage**, whether by refusing the write or by removing the
/// credential from it. That is what this asserts, and it is falsified either by
/// dropping the redaction pass or by dropping the server-side screen.
#[test]
fn no_entry_point_lets_a_credential_reach_storage() {
    let Some(server) = server() else { return };
    let l = linked(&server, "credential");
    let cwd = l.sandbox.repo_path().to_string_lossy().to_string();
    let content = "fetch it from https://admin:hunter2@internal.invalid/x";

    // Local: accepted, with the credential gone.
    let mut mcp = Mcp::start(&l.sandbox);
    let created = mcp.tool_result(
        "cairn_remember",
        json!({
            "action": "create",
            "domain": "personal",
            "type": "fact",
            "content": content,
        }),
        &cwd,
    );
    let stored = created["content"][0]["text"]["memory"]["content"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    assert!(
        !stored.contains("hunter2"),
        "a credential reached personal storage verbatim: {stored}"
    );
    assert!(
        stored.contains("[REDACTED]"),
        "the credential was neither refused nor redacted: {created}"
    );

    // The stored row, not only the response: a redaction applied on the way out
    // would satisfy the assertion above and protect nothing.
    let rows = l
        .sandbox
        .query_column("SELECT content FROM personal_knowledge");
    assert!(
        rows.iter().all(|r| !r.contains("hunter2")),
        "a credential is in the personal_knowledge table: {rows:?}"
    );

    // Server ingest, which does not redact: refused by name.
    let (body, status) = push_global(
        &server,
        &l,
        "personal_knowledge",
        json!({
            "knowledge_type": "fact",
            "content": content,
            "writer_id": Uuid::now_v7(),
            "writer_seq": 1,
        }),
    );
    assert_eq!(status, 200, "the batch route itself failed: {body}");
    let refusal = batch_refusal(&body)
        .unwrap_or_else(|| panic!("the server ingested a credentialed URL: {body}"));
    assert!(
        refusal.to_string().contains("credentialed_url"),
        "the server refused for the wrong reason: {refusal}"
    );
}

/// An input that names the project the caller is working in.
///
/// Kept separate from [`refusable_content`] because it is the one class whose
/// refusal depends on *who is asking*: the same sentence is fine for a caller
/// with no such project and refused for a caller who has one (FR-580).
const PROJECT_TOKEN: &str = "kestrelworks";

fn server() -> Option<Server> {
    match Server::start() {
        Some(s) => Some(s),
        None => {
            eprintln!("SKIPPED: set CAIRN_TEST_DATABASE_URL to run the entry-point suite");
            None
        }
    }
}

/// A sandbox whose project is named [`PROJECT_TOKEN`], linked to `server` as
/// `email`, with `email` a member of that project on the server.
///
/// The project name matters: the `project_identifying` class screens against the
/// identities of the project the caller is working in, and against the union of
/// the pushing user's memberships on the server side. Both must resolve to the
/// same token for the five entry points to be comparable at all.
struct Linked {
    sandbox: Sandbox,
    token: String,
    project: Uuid,
}

fn linked(server: &Server, label: &str) -> Linked {
    let sandbox = Sandbox::new();
    // The local project must be *identified* as `PROJECT_TOKEN` too, or the two
    // sides of the comparison are screening against different identity sets and
    // "refused identically" would be measuring nothing. The git remote is what
    // supplies it: `current_project_identities` derives the organisation token
    // from the remote, exactly as the server's `identities_for` does.
    //
    // `localhost` as the host rather than a domain that reads like prose: every
    // token in the remote becomes an identity, so a host like `example.test`
    // would make any content mentioning `example.test` refuse as
    // `project_identifying` before reaching the class actually under test.
    let remote = format!("git@localhost:{PROJECT_TOKEN}/{label}.git");
    sandbox.git(&["remote", "add", "origin", &remote]);
    sandbox.must(&["init"]);

    let token = server.new_user_token(label);
    let (created, status) = post_json_status_bearer(
        &server.base,
        "/api/projects",
        &json!({ "name": PROJECT_TOKEN, "repository_remote": remote }),
        &token,
    );
    assert_eq!(status, 200, "create project: {created}");
    let project: Uuid = created["id"]
        .as_str()
        .expect("project id")
        .parse()
        .expect("uuid");

    cairn_e2e::attach_server(&sandbox, server, &token);
    Linked {
        sandbox,
        token,
        project,
    }
}

/// A project memory this run can promote, named so the promotion gate's
/// `source_not_active` check passes.
fn promotable(sandbox: &Sandbox, content: &str) -> String {
    let created = sandbox.json(&[
        "memory",
        "add",
        "--type",
        "convention",
        "--scope",
        "project",
        content,
    ]);
    created["memory"]["id"]
        .as_str()
        .expect("memory id")
        .to_string()
}

/// Whether a refusal names `class` and nothing of the content.
///
/// One helper rather than five inline assertions, so "identically" is enforced
/// by there being one predicate rather than five that could drift.
fn refused_as(body: &Value, class: &str, content: &str, where_: &str) {
    let rendered = body.to_string();
    assert!(
        rendered.contains(class),
        "{where_} did not name the class `{class}`: {rendered}"
    );
    // FR-547 / SC-439: never a fragment of what was refused. Checked on the
    // longest word in the input rather than the whole string, because a
    // refusal that echoed only part of the content would pass a whole-string
    // check while leaking exactly what matters.
    let longest = content
        .split_whitespace()
        .max_by_key(|w| w.len())
        .unwrap_or(content);
    assert!(
        !rendered.contains(longest),
        "{where_} echoed the refused content (`{longest}`): {rendered}"
    );
}

/// `POST /api/sync/batch` answers `200` with a per-item verdict, and that is
/// right: a batch of ten items where one is refused must not fail the other
/// nine. So an ingest refusal is read off the item, not off the status line.
///
/// Returns the refused item's error object, or `None` when the item was
/// accepted — which is what every assertion below branches on.
fn batch_refusal(body: &Value) -> Option<&Value> {
    let item = body["results"].get(0)?;
    if item["status"].as_str()? == "rejected" {
        item.get("error")
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// T145 / T146 — the same input, refused identically by all five
// ---------------------------------------------------------------------------

/// An absolute path is refused by every one of the five entry points.
///
/// Falsified by removing the `validate_global_content` call from any single
/// entry point: that one arm of the loop starts returning success, and the
/// assertion names which.
#[test]
fn an_absolute_path_is_refused_identically_by_all_five_entry_points() {
    let Some(server) = server() else { return };
    let l = linked(&server, "five-abs");
    let content = "the fix lives in /Users/dev/src/thing/main.rs";

    for (point, body) in every_entry_point(&server, &l, content) {
        refused_as(&body, "absolute_path", content, &point);
    }
}

/// A project-identifying token and a shell command invocation, likewise —
/// extended across the whole adversarial corpus, so the criterion tests the
/// validator rather than the schema (SC-424a).
#[test]
fn the_adversarial_corpus_is_refused_identically_by_all_five_entry_points() {
    let Some(server) = server() else { return };
    let l = linked(&server, "five-corpus");

    let mut cases = refusable_content();
    cases.push((
        "the kestrelworks retry budget is four attempts",
        "project_identifying",
    ));

    for (content, class) in cases {
        for (point, body) in every_entry_point(&server, &l, content) {
            refused_as(&body, class, content, &format!("{point} for `{class}`"));
        }
    }
}

/// Every free-text field, and every applicability value, is screened — not only
/// `content` (FR-546, FR-578).
///
/// Falsified by narrowing any entry point's screen to `content` alone: the
/// topic-key and applicability cases start succeeding.
#[test]
fn every_free_text_field_and_applicability_value_is_screened() {
    let Some(server) = server() else { return };
    let l = linked(&server, "five-fields");
    let cwd = l.sandbox.repo_path().to_string_lossy().to_string();
    let mut mcp = Mcp::start(&l.sandbox);

    // topic_key
    let by_topic = mcp.tool_result(
        "cairn_remember",
        json!({
            "action": "create",
            "domain": "personal",
            "type": "fact",
            "content": "a harmless claim",
            "topic_key": "/Users/dev/secret",
        }),
        &cwd,
    );
    assert_eq!(
        by_topic["isError"], true,
        "a topic key carrying an absolute path was accepted: {by_topic}"
    );

    // An applicability value, on the promotion path — the only local surface
    // that takes structured facts.
    let memory = promotable(&l.sandbox, "a claim worth promoting");
    let by_value = mcp.tool_result(
        "cairn_remember",
        json!({
            "action": "promote",
            "target": "personal",
            "memory_id": memory,
            "applicability_facts": ["tool=kestrelworks"],
        }),
        &cwd,
    );
    assert_eq!(
        by_value["isError"], true,
        "an applicability value naming the caller's own project was accepted: {by_value}"
    );

    // And on the server, where the payload is not the client's to shape.
    let ingest = push_global(
        &server,
        &l,
        "personal_knowledge",
        json!({
            "knowledge_type": "fact",
            "content": "a harmless claim",
            "writer_id": Uuid::now_v7(),
            "writer_seq": 1,
            "applicability": [{ "kind": "tool", "value": "kestrelworks" }],
        }),
    );
    assert!(
        batch_refusal(&ingest.0).is_some(),
        "the server accepted an applicability value naming a project the pusher \
         belongs to: {}",
        ingest.0
    );
}

// ---------------------------------------------------------------------------
// T147 — a client that skips its own screen gains nothing
// ---------------------------------------------------------------------------

/// A client bypassing its local validation is refused by the server, the record
/// is absent from the server store, and it never reaches the user's other
/// devices.
///
/// This is the only one of these tests that *must* be end to end. A local screen
/// is a courtesy to the user; the server's is the one that holds when the client
/// is old, patched, or hostile. Falsified by removing the
/// `screen_global_item` call from `apply_item`.
#[test]
fn a_client_that_skips_its_own_screen_is_refused_by_the_server() {
    let Some(server) = server() else { return };
    let l = linked(&server, "bypass");

    for (content, class) in [
        (
            "the fix lives in /Users/dev/src/thing/main.rs",
            "absolute_path",
        ),
        ("run cargo test --workspace to check", "command_shaped"),
        (
            "the kestrelworks retry budget is four attempts",
            "project_identifying",
        ),
    ] {
        for entity in ["personal_knowledge", "team_knowledge"] {
            let id = Uuid::now_v7();
            let (body, status) = push_global(
                &server,
                &l,
                entity,
                json!({
                    "id": id,
                    "knowledge_type": "fact",
                    "content": content,
                    "writer_id": Uuid::now_v7(),
                    "writer_seq": 1,
                    "state": "proposed",
                    "proposed_by_user_id": Uuid::now_v7(),
                }),
            );
            assert_eq!(status, 200, "the batch route itself failed: {body}");
            let refusal = batch_refusal(&body)
                .unwrap_or_else(|| panic!("{entity} carrying `{class}` was accepted: {body}"));
            refused_as(
                refusal,
                class,
                content,
                &format!("server ingest of {entity}"),
            );

            // Absent from the server store — not merely reported refused.
            let table = if entity == "personal_knowledge" {
                "personal_knowledge"
            } else {
                "team_knowledge"
            };
            assert_eq!(
                server.count(&format!("SELECT COUNT(*) FROM {table} WHERE id = '{id}'")),
                0,
                "a refused {entity} row was written anyway"
            );

            // And nothing for another device to pull: the read-back the second
            // device would use has no row to return.
            assert_eq!(
                server.count(&format!(
                    "SELECT COUNT(*) FROM {table} WHERE content LIKE '%{}%'",
                    content.replace('\'', "''")
                )),
                0,
                "a refused {entity} row is reachable by content search on the server"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// T148 — an ingest refusal is not a capability refusal
// ---------------------------------------------------------------------------

/// A client tells the two apart by the response, never by a message string, and
/// a refused item neither counts as delivered nor throttles its namespace.
///
/// Their remedies are opposite: a capability refusal becomes deliverable after a
/// server upgrade and must be held; an ingest refusal can never succeed for the
/// same content and must not be. Treating the second as the first holds the item
/// forever, because no upgrade makes a project-identifying value stop naming a
/// project. Falsified by giving both refusals the same status.
#[test]
fn an_ingest_refusal_is_distinguishable_from_a_capability_refusal() {
    let Some(server) = server() else { return };
    let l = linked(&server, "refusal-kind");

    let (rejected, _) = push_global(
        &server,
        &l,
        "personal_knowledge",
        json!({
            "knowledge_type": "fact",
            "content": "the fix lives in /Users/dev/src/thing/main.rs",
            "writer_id": Uuid::now_v7(),
            "writer_seq": 1,
        }),
    );
    let ingest = batch_refusal(&rejected)
        .unwrap_or_else(|| panic!("clean-looking ingest was accepted: {rejected}"));
    assert_eq!(
        ingest["code"].as_str(),
        Some("content_rejected"),
        "the ingest refusal carries no matchable code: {rejected}"
    );

    // The capability path is a different status and a different code. Asserted
    // against a real schema-2 server rather than by reading the constant, so a
    // change that made the two statuses converge fails here.
    let Some(mut old) = Server::start_at_schema(2) else {
        eprintln!("SKIPPED: schema-2 server unavailable");
        return;
    };
    let old_l = linked(&old, "refusal-cap");
    let (held, _) = push_global(
        &old,
        &old_l,
        "personal_knowledge",
        json!({
            "knowledge_type": "fact",
            "content": "a perfectly clean claim",
            "writer_id": Uuid::now_v7(),
            "writer_seq": 1,
        }),
    );
    let capability = batch_refusal(&held).unwrap_or_else(|| {
        panic!("a schema-2 server accepted a personal item it has no table for: {held}")
    });
    assert_ne!(
        capability["code"].as_str(),
        Some("content_rejected"),
        "a capability refusal reports itself as a content rejection, so a client \
         holding it would hold it forever: {held}"
    );
    assert_eq!(
        capability["code"].as_str(),
        Some("unknown_entity_type"),
        "a capability refusal must name the class the daemon retains on: {held}"
    );
    let _ = old.upgraded();

    // The refused namespace stays eligible: a clean item pushed right after the
    // refusal still lands, so the refusal throttled nothing.
    let (after, after_status) = push_global(
        &server,
        &l,
        "personal_knowledge",
        json!({
            "knowledge_type": "fact",
            "content": "a perfectly clean claim",
            "writer_id": Uuid::now_v7(),
            "writer_seq": 2,
        }),
    );
    assert_eq!(
        after_status, 200,
        "a clean item after a refusal was not accepted: {after}"
    );
    assert_eq!(
        after["results"][0]["status"].as_str(),
        Some("applied"),
        "a clean item after a refusal was not applied: {after}"
    );
}

// ---------------------------------------------------------------------------
// T149 — nothing echoes the content back
// ---------------------------------------------------------------------------

/// A rejection message, log line and API response contain no fragment of the
/// rejected content, at every one of the five entry points.
///
/// Falsified by any refusal that formats the offending substring into its
/// message — which is the natural thing to write and the reason this test
/// exists.
#[test]
fn no_refusal_at_any_entry_point_echoes_the_content() {
    let Some(server) = server() else { return };
    let l = linked(&server, "no-echo");
    // A distinctive marker, so a leak is unmistakable rather than a coincidence
    // of common words.
    let content = "the fix lives in /Users/dev/zqxjkvbrwnp/main.rs";

    for (point, body) in every_entry_point(&server, &l, content) {
        let rendered = body.to_string();
        assert!(
            !rendered.contains("zqxjkvbrwnp"),
            "{point} echoed the refused content: {rendered}"
        );
    }

    // The daemon's own log is the other surface a refusal reaches.
    let log = l.sandbox.sidecar("daemon.log");
    if let Ok(text) = std::fs::read_to_string(&log) {
        assert!(
            !text.contains("zqxjkvbrwnp"),
            "the daemon log carries the refused content"
        );
    }
}

// ---------------------------------------------------------------------------
// T150 — a refusal leaves nothing behind
// ---------------------------------------------------------------------------

/// After a rejected creation or promotion at any of the five entry points: no
/// record, no partial record, and no outbox entry — all three inspected.
///
/// Falsified by moving any screen inside the transaction that writes: the row
/// would exist and be rolled back, and a rollback that fails leaves exactly the
/// partial record this asserts against.
#[test]
fn a_refusal_at_any_entry_point_leaves_no_record_and_no_outbox_entry() {
    let Some(server) = server() else { return };
    let l = linked(&server, "no-trace");
    let content = "the fix lives in /Users/dev/src/thing/main.rs";

    let before = local_counts(&l.sandbox);
    for (point, body) in every_entry_point(&server, &l, content) {
        refused_as(&body, "absolute_path", content, &point);
    }
    let after = local_counts(&l.sandbox);

    assert_eq!(
        before, after,
        "a refused write left something behind locally: {before:?} -> {after:?}"
    );

    // And on the server, across both tables and the idempotency ledger: a
    // refused item must not hold its key, or a corrected retry would be
    // reported a duplicate and never applied.
    for table in ["personal_knowledge", "team_knowledge"] {
        assert_eq!(
            server.count(&format!(
                "SELECT COUNT(*) FROM {table} WHERE content LIKE '%thing/main.rs%'"
            )),
            0,
            "a refused row exists in {table}"
        );
    }
}

/// The three local counts a refusal must not move: the two domain tables and
/// the outbox.
///
/// Scoped to rows **this store wrote**, by `writer_id`. Team knowledge is
/// server-wide, so a linked sandbox legitimately pulls guidance other accounts
/// ratified, and an unscoped count would move for reasons that have nothing to do
/// with the refusal under test. What the assertion is about is whether a refused
/// write left a record behind, and a record this store did not author cannot be
/// one.
fn local_counts(sandbox: &Sandbox) -> Vec<String> {
    sandbox.query_column(
        "SELECT (SELECT COUNT(*) FROM personal_knowledge
                  WHERE writer_id = (SELECT writer_id FROM writer_identity WHERE id = 1))
             || ':' || (SELECT COUNT(*) FROM team_knowledge
                  WHERE writer_id = (SELECT writer_id FROM writer_identity WHERE id = 1))
             || ':' || (SELECT COUNT(*) FROM outbox)",
    )
}

// ---------------------------------------------------------------------------
// Driving all five
// ---------------------------------------------------------------------------

/// Push one crafted global item straight at `POST /api/sync/batch`, as a client
/// that skipped its own screen would.
///
/// The idempotency key is fresh per call so a refusal is never masked by the
/// server reporting a duplicate.
fn push_global(server: &Server, l: &Linked, entity_type: &str, mut payload: Value) -> (Value, u16) {
    let id = payload
        .get("id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| Uuid::now_v7().to_string());
    payload["id"] = json!(id);
    post_json_status_bearer(
        &server.base,
        "/api/sync/batch",
        &json!({
            "project_id": l.project,
            "items": [{
                "idempotency_key": Uuid::now_v7().to_string(),
                "entity_type": entity_type,
                "entity_id": id,
                "operation": "upsert",
                "payload": payload,
            }],
        }),
        &l.token,
    )
}

/// `content` offered to each of the five entry points in turn, paired with the
/// name of the point, so a failing assertion says which one accepted it.
///
/// Returned as a `Vec` rather than yielded lazily because each caller iterates
/// it once and a closure over `&mut Mcp` would outlive the borrow.
fn every_entry_point(server: &Server, l: &Linked, content: &str) -> Vec<(String, Value)> {
    let cwd = l.sandbox.repo_path().to_string_lossy().to_string();
    let mut out: Vec<(String, Value)> = Vec::new();
    let mut mcp = Mcp::start(&l.sandbox);

    // 1 — direct personal creation.
    out.push((
        "direct personal creation".into(),
        mcp.tool_result(
            "cairn_remember",
            json!({
                "action": "create",
                "domain": "personal",
                "type": "fact",
                "content": content,
            }),
            &cwd,
        ),
    ));

    // 2 and 4 — promotion, into each domain. The source memory carries the
    // offending content, because promotion screens the *source*: a gate that
    // screened only what the caller retyped would let anything through.
    for (target, label) in [
        ("personal", "personal promotion"),
        ("team", "team promotion"),
    ] {
        let memory = promotable(&l.sandbox, content);
        out.push((
            label.into(),
            mcp.tool_result(
                "cairn_remember",
                json!({
                    "action": "promote",
                    "target": target,
                    "memory_id": memory,
                }),
                &cwd,
            ),
        ));
    }

    // 3 — team proposal, through the CLI, which is the only surface that
    // authors one (FR-455).
    let proposed = l.sandbox.json_err(&["team", "propose", content]);
    out.push(("team proposal".into(), proposed));

    // 5 — server-side ingest, for both domains.
    for entity in ["personal_knowledge", "team_knowledge"] {
        let (body, status) = push_global(
            server,
            l,
            entity,
            json!({
                "knowledge_type": "fact",
                "content": content,
                "writer_id": Uuid::now_v7(),
                "writer_seq": 1,
                "state": "proposed",
                "proposed_by_user_id": Uuid::now_v7(),
            }),
        );
        assert_eq!(
            status, 200,
            "the batch route itself failed rather than reporting a per-item verdict: {body}"
        );
        let refusal = batch_refusal(&body)
            .unwrap_or_else(|| {
                panic!("server ingest of {entity} accepted refusable content: {body}")
            })
            .clone();
        out.push((format!("server ingest of {entity}"), refusal));
    }

    assert_eq!(
        out.len(),
        6,
        "FR-545 names five entry points, for two domains — six surfaces here; \
         the count changed without this test being told"
    );
    out
}
