//! T184 — the documentation-lint that keeps T183's corrections from silently
//! regressing (FR-550, FR-570, FR-543, SC-467).
//!
//! `promotion-privacy.md` and `data-model.md` were both corrected (D456/F11)
//! to stop describing a free-text field — `content`, a topic key, a value
//! key, or an applicability value — as structurally incapable of carrying a
//! path or a command, when in fact those fields hold no such guarantee by
//! *absence*: they are validated free text (Layer B), not an absent column
//! (Layer A). Nothing stops a future edit from reintroducing that overstated
//! claim, or from re-conflating an applicability fact with a record's own
//! `topic_key` (FR-570), except a check that runs every time.
//!
//! Exactly like `global_content_validation.rs` does for SC-453, this is
//! written as a pure function over (path, text) pairs and exercised twice:
//! once over the real documents, where it must find nothing, and once over
//! the real documents **plus a phrase deliberately reinserted**, where it
//! must find it. An audit that only ever inspects today's text passes today
//! and passes again the day someone undoes T183 by hand — which is the exact
//! failure mode `scope_audit.rs` (T174) demonstrated for an unrelated claim.

use std::path::{Path, PathBuf};

fn workspace_root() -> PathBuf {
    // `CARGO_MANIFEST_DIR` is `tests/`; the workspace is its parent.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .to_path_buf()
}

/// The documents T183 corrected, plus the two others FR-550/FR-570 bind
/// alongside them (`tasks.md` T184).
const TARGET_DOCS: &[&str] = &[
    "specs/004-collaborative-global-memory/contracts/promotion-privacy.md",
    "specs/004-collaborative-global-memory/data-model.md",
    "specs/004-collaborative-global-memory/compatibility.md",
    "specs/004-collaborative-global-memory/contracts/global-memory.md",
];

fn read_docs(paths: &[&str]) -> Vec<(String, String)> {
    let root = workspace_root();
    paths
        .iter()
        .map(|p| {
            let text = std::fs::read_to_string(root.join(p))
                .unwrap_or_else(|e| panic!("reading {p}: {e}"));
            (p.to_string(), text)
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Check 1 — a free-text field is never called structurally incapable
// (FR-550, SC-467)
// ---------------------------------------------------------------------------

/// The literal phrasing FR-550 forbids applying to a Layer B value. Matched
/// literally, not paraphrased — exactly as `data-model.md` itself describes
/// the audit it expects to exist: "an audit ... fails on the forbidden
/// phrasing itself".
const LAYER_A_PHRASES: &[&str] = &[
    "structurally incapable",
    "impossible by construction",
    "no column exists",
];

/// The Layer B values FR-550 protects: a record's own free-text fields, never
/// describable as structurally incapable of carrying a path or a command.
const FREE_TEXT_FIELD_MARKERS: &[&str] = &[
    "`content`",
    "content,",
    "content`",
    "topic key",
    "topic_key",
    "value key",
    "value_key",
    "applicability value",
    "applicability fact",
];

/// Clauses in the real documents that state the *rule* FR-550 imposes,
/// rather than violating it — they necessarily use this exact forbidden
/// phrasing, in the same breath as the field names it must never describe,
/// because that is what a document correcting and guarding against the
/// mistake has to say. Narrowly tied to today's exact wording, the same
/// discipline `ALLOWED_DUPLICATES` uses in `global_content_validation.rs`:
/// growing this list is a decision a reader can disagree with, not a
/// convenience, and it must never grow just to silence a real regression.
const LAYER_CLAIM_EXEMPTIONS: &[&str] = &[
    // "...MUST NOT describe a Layer B value — `content`, ... — as
    // structurally incapable..." (promotion-privacy.md §2b)
    "MUST NOT",
    // "...no Layer B value — `content`, ... — is ever described as
    // structurally incapable..." (promotion-privacy.md, Invariant 18)
    "is ever described as",
    // "...an audit over this document fails on the forbidden phrasing
    // itself — "structurally incapable", ... — appearing anywhere `content`,
    // ..." (data-model.md §4a)
    "the forbidden phrasing itself",
];

/// Split into clauses roughly at sentence and table-cell boundaries.
///
/// A markdown table row has no sentence-ending punctuation between cells, so
/// a naive `.split('.')` merges an unrelated cell's field name into the same
/// "sentence" as a phrase two cells over — treating `|` as a boundary too is
/// what keeps a table row from reading as one long, falsely-associated
/// clause. `.**` (a bolded clause ending) is normalized the same way a
/// period is, since Markdown emphasis is what several of these documents use
/// in place of a plain sentence break.
fn clauses(text: &str) -> Vec<String> {
    let flat: String = text.split_whitespace().collect::<Vec<&str>>().join(" ");
    let normalized = flat.replace(".**", ". ").replace('|', ". ");
    normalized
        .split(". ")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Every clause in `text` that applies forbidden Layer A phrasing to a
/// Layer B (free-text) value, without stating the rule rather than
/// violating it.
fn layer_claim_violations(path: &str, text: &str) -> Vec<String> {
    let mut findings = Vec::new();
    for clause in clauses(text) {
        let has_phrase = LAYER_A_PHRASES.iter().any(|p| clause.contains(p));
        if !has_phrase {
            continue;
        }
        let has_field = FREE_TEXT_FIELD_MARKERS.iter().any(|f| clause.contains(f));
        if !has_field {
            continue;
        }
        let is_exempt = LAYER_CLAIM_EXEMPTIONS.iter().any(|e| clause.contains(e));
        if is_exempt {
            continue;
        }
        findings.push(format!("{path}: {clause}"));
    }
    findings
}

/// SC-467, part one: the shipped documents make no such claim today.
#[test]
fn no_free_text_field_is_described_as_structurally_incapable() {
    let docs = read_docs(TARGET_DOCS);
    let mut findings = Vec::new();
    for (path, text) in &docs {
        findings.extend(layer_claim_violations(path, text));
    }
    assert!(
        findings.is_empty(),
        "a free-text field is described as structurally incapable: {findings:#?}"
    );
}

/// SC-467, part two — the half that makes part one worth anything. Proved by
/// seeding the exact regression T183 fixed, not by re-inspecting today's
/// (already correct) text, which would pass either way.
#[test]
fn the_layer_claim_lint_fails_when_the_phrasing_is_reinserted() {
    let (path, text) = &read_docs(TARGET_DOCS)[0];
    let seeded =
        format!("{text}\n\n`content` is structurally incapable of carrying a path or a command.\n");
    let findings = layer_claim_violations(path, &seeded);
    assert!(
        !findings.is_empty(),
        "the lint missed a plainly reinserted violation"
    );
    assert!(
        findings
            .iter()
            .any(|f| f.contains("structurally incapable")),
        "the lint found something, but not the seeded phrase: {findings:#?}"
    );

    // And every clause that already exists in the real document must still
    // read as exempt — the seed must not have been caught only because the
    // exemption list stopped matching the genuine, correct sentences.
    let real_findings = layer_claim_violations(path, text);
    assert!(
        real_findings.is_empty(),
        "the unmodified document now fails too, which means the seed corrupted \
         a real sentence rather than adding a new one: {real_findings:#?}"
    );
}

// ---------------------------------------------------------------------------
// Check 2 — an applicability fact is never called "a topic," or conflated
// with a record's own `topic_key` (FR-570)
// ---------------------------------------------------------------------------

/// Phrase shapes that positively attribute a `topic_key` to an applicability
/// fact/kind/value, or call an applicability fact "a topic" outright — the
/// specific conflation FR-570 forbids. The real documents distinguish the
/// two instead ("is unrelated to", "a different question", "must not be
/// conflated"), which is why these positive-attribution shapes do not appear
/// in them, and why matching them does not also catch the correct,
/// distinguishing prose that necessarily mentions both terms together.
const TOPIC_CONFLATION_PATTERNS: &[&str] = &[
    "applicability's topic_key",
    "applicability fact's topic_key",
    "applicability facts' topic_key",
    "applicability value's topic_key",
    "applicability kind's topic_key",
    "topic_key of the applicability",
    "topic_key of an applicability",
    "an applicability topic",
    "the applicability topic",
    "applicability fact is a topic",
    "applicability facts are topics",
    "applicability value is a topic",
    "applicability kind is a topic",
];

fn topic_conflation_violations(path: &str, text: &str) -> Vec<String> {
    let flat: String = text.split_whitespace().collect::<Vec<&str>>().join(" ");
    TOPIC_CONFLATION_PATTERNS
        .iter()
        .filter(|p| flat.contains(**p))
        .map(|p| format!("{path}: {p}"))
        .collect()
}

#[test]
fn an_applicability_fact_is_never_called_a_topic_or_conflated_with_topic_key() {
    let docs = read_docs(TARGET_DOCS);
    let mut findings = Vec::new();
    for (path, text) in &docs {
        findings.extend(topic_conflation_violations(path, text));
    }
    assert!(
        findings.is_empty(),
        "an applicability fact is conflated with a record's topic_key: {findings:#?}"
    );
}

#[test]
fn the_topic_conflation_lint_fails_when_reinserted() {
    let (path, text) = &read_docs(TARGET_DOCS)[1];
    let seeded = format!(
        "{text}\n\nAn applicability fact's topic_key determines which projects it applies to.\n"
    );
    let findings = topic_conflation_violations(path, &seeded);
    assert!(
        !findings.is_empty(),
        "the lint missed a plainly reinserted conflation between an \
         applicability fact and a record's topic_key"
    );
}

// ---------------------------------------------------------------------------
// Check 3 — README.md and SECURITY.md state the environment-account trust
// statement (FR-543)
// ---------------------------------------------------------------------------

/// The trust statement `identity-administration.md` requires be written down
/// rather than left implicit: whoever can set the admin environment
/// variables and restart the server can always obtain an administrator
/// account, regardless of any role or status change made through the API.
/// Matched on its most load-bearing clause plus the environment variable it
/// names, rather than the whole sentence verbatim, so a harmless rewording
/// of the surrounding prose does not make this lint flap.
fn has_trust_statement(text: &str) -> bool {
    let flat: String = text.split_whitespace().collect::<Vec<&str>>().join(" ");
    flat.contains("can always obtain administrator access") && flat.contains("CAIRN_ADMIN_EMAIL")
}

#[test]
fn readme_and_security_state_the_environment_account_trust_statement() {
    let root = workspace_root();
    for name in ["README.md", "SECURITY.md"] {
        let text = std::fs::read_to_string(root.join(name))
            .unwrap_or_else(|e| panic!("reading {name}: {e}"));
        assert!(
            has_trust_statement(&text),
            "{name} does not state the environment-account trust statement \
             (FR-543): whoever can set CAIRN_ADMIN_EMAIL/CAIRN_ADMIN_PASSWORD \
             and restart the server can always obtain administrator access"
        );
    }
}

#[test]
fn the_trust_statement_lint_fails_when_the_statement_is_absent() {
    assert!(
        !has_trust_statement("This document says nothing about administrator recovery."),
        "the lint reported the trust statement present in text that plainly lacks it"
    );
    assert!(
        !has_trust_statement(
            "CAIRN_ADMIN_EMAIL selects the bootstrap account, whose access is unspecified here."
        ),
        "the lint passed on the environment variable alone, with no trust claim attached"
    );
}

// ---------------------------------------------------------------------------
// The web client's handoff shape matches the Rust one (FR-532)
// ---------------------------------------------------------------------------

/// `web/lib/api.ts` must not name a field the wire denylist forbids, and must
/// name the one the Rust type actually carries.
///
/// This exists because of a regression `cargo test` structurally could not see.
/// Feature 004 renamed `TestRunRecord.command` to `runner` — the recursive wire
/// check screens field *names*, so a `command` key anywhere inside a handoff
/// payload is refused on sight, which would make every handoff carrying a
/// completed test run undeliverable. The Rust side was updated and its tests
/// passed. The web client still declared `command` and rendered `t.command`, and
/// the e2e seed still *posted* `command` — so the server refused the seeded
/// handoff, the session page had nothing to render, and the whole thing was
/// invisible to the Rust workspace because the web app is not in it.
///
/// A file-text lint rather than a generated type: the two definitions live in
/// different languages and different build graphs, and the cheapest honest link
/// between them is an assertion that reads both.
#[test]
fn the_web_client_names_the_same_handoff_fields_the_rust_type_does() {
    let root = workspace_root();
    let api = std::fs::read_to_string(root.join("web/lib/api.ts")).expect("web/lib/api.ts");

    // The Rust type is the authority for the name.
    let domain =
        std::fs::read_to_string(root.join("crates/cairn-core/src/domain.rs")).expect("domain.rs");
    let struct_body = domain
        .split("pub struct TestRunRecord")
        .nth(1)
        .and_then(|rest| rest.split('}').next())
        .expect("TestRunRecord is declared");
    assert!(
        struct_body.contains("pub runner:"),
        "TestRunRecord no longer has a `runner` field, so this lint is checking \
         against a name that moved: {struct_body}"
    );
    assert!(
        !struct_body.contains("pub command:"),
        "TestRunRecord carries a `command` field again; the recursive wire check \
         refuses that name and every handoff with a test run becomes undeliverable"
    );

    // The `tests_executed` declaration in the web client, and the render and
    // seed sites that consume it.
    let declared = api
        .split("tests_executed:")
        .nth(1)
        .and_then(|rest| rest.split(';').next())
        .expect("web/lib/api.ts declares tests_executed");
    assert!(
        declared.contains("runner"),
        "the web client's `tests_executed` does not name `runner`: {declared}"
    );
    assert!(
        !declared.contains("command"),
        "the web client's `tests_executed` still names `command`, which the Rust \
         type no longer carries: {declared}"
    );

    for path in [
        "web/app/(app)/projects/[id]/sessions/[sessionId]/page.tsx",
        "web/e2e/seed.ts",
    ] {
        let body = std::fs::read_to_string(root.join(path)).unwrap_or_else(|e| {
            panic!("{path} is not readable, so this lint cannot check it: {e}")
        });
        assert!(
            !body.contains("t.command") && !body.contains("command: \"cargo"),
            "{path} still reads or writes a test run's `command`; the field is \
             `runner` (FR-532)"
        );
    }
}
