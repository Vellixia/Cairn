//! A domain is not a scope, and nothing lets the two leak into each other
//! (FR-431, FR-517, FR-521, SC-459).
//!
//! This is the feature's central constraint and it survived three review passes
//! with a task and no success criterion behind it, which is why the assertions
//! here are structural rather than behavioural: they read the schema and the
//! type, so that *adding* the thing the requirement forbids fails a test rather
//! than passing review.

use std::path::{Path, PathBuf};

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .to_path_buf()
}

fn local_migrations() -> Vec<(String, String)> {
    let dir = workspace_root().join("crates/cairn-store/migrations");
    let mut out: Vec<(String, String)> = std::fs::read_dir(&dir)
        .expect("migrations directory")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "sql"))
        .map(|p| {
            (
                p.file_name().unwrap().to_string_lossy().to_string(),
                std::fs::read_to_string(&p).expect("read migration"),
            )
        })
        .collect();
    out.sort();
    out
}

// ---------------------------------------------------------------------------
// T004 / FR-521 / SC-459 — `MemoryScope` is unchanged and `memories` was not
// rebuilt
// ---------------------------------------------------------------------------

/// The scope enum has exactly four variants, spelled exactly as before.
///
/// Asserted against the literal list rather than a count alone: a fifth variant
/// would fail the count, but *renaming* one would pass a count and break every
/// stored row.
#[test]
fn memory_scope_has_exactly_the_four_original_variants() {
    use cairn_core::domain::MemoryScope;
    let names: Vec<&str> = MemoryScope::ALL.iter().map(|s| s.as_str()).collect();
    assert_eq!(
        names,
        vec!["project", "branch", "task", "session"],
        "MemoryScope changed; FR-521 forbids it and every stored scope value depends on it"
    );
}

/// A domain is not a scope value.
///
/// The whole design turns on this: `KnowledgeDomain` answers "whose knowledge",
/// `MemoryScope` answers "how narrow inside a project", and the reason
/// `memories` needs no rebuild is that the second gained nothing. A `global`
/// scope would be the failure this test exists to catch.
#[test]
fn no_knowledge_domain_name_is_a_memory_scope_name() {
    use cairn_core::domain::{KnowledgeDomain, MemoryScope};
    let scopes: Vec<&str> = MemoryScope::ALL.iter().map(|s| s.as_str()).collect();
    for domain in KnowledgeDomain::ALL {
        if domain.as_str() == "project" {
            // The one shared word, and it is a coincidence of English rather
            // than a shared concept: `MemoryScope::Project` narrows inside one
            // project, `KnowledgeDomain::Project` says the record belongs to a
            // project at all.
            continue;
        }
        assert!(
            !scopes.contains(&domain.as_str()),
            "{} is both a domain and a scope",
            domain.as_str()
        );
    }
    assert!(
        "global".parse::<MemoryScope>().is_err(),
        "MemoryScope::Global exists; the feature's central constraint is gone"
    );
}

/// `memories` is not rebuilt, and its scope `CHECK` still lists exactly the
/// four values.
///
/// Read from the migration files rather than from a live database, because the
/// claim is about what the schema *says* — a rebuild that produced an
/// equivalent table would still have rewritten it, and this is what
/// distinguishes "unchanged" from "currently equivalent".
#[test]
fn the_memories_table_is_never_rebuilt_and_its_scope_check_is_unchanged() {
    let migrations = local_migrations();
    assert!(
        migrations.len() >= 7,
        "expected migration 0007 to exist; found {:?}",
        migrations.iter().map(|(n, _)| n).collect::<Vec<_>>()
    );

    // The original CHECK, verbatim from 0001.
    let (_, init) = migrations
        .iter()
        .find(|(name, _)| name.starts_with("0001"))
        .expect("0001_init.sql");
    assert!(
        init.contains("CHECK (scope IN ('project', 'branch', 'task', 'session'))"),
        "0001's scope CHECK is not where this test expects it; the assertion below would pass vacuously"
    );

    // No later migration may rebuild, drop or rename `memories`, and none may
    // restate its scope CHECK — restating it is how a fifth value gets in.
    for (name, sql) in migrations.iter().filter(|(n, _)| !n.starts_with("0001")) {
        let upper = sql.to_uppercase();
        for forbidden in [
            "DROP TABLE MEMORIES",
            "DROP TABLE IF EXISTS MEMORIES",
            "ALTER TABLE MEMORIES RENAME",
            "CREATE TABLE MEMORIES",
            "CREATE TABLE IF NOT EXISTS MEMORIES",
        ] {
            assert!(
                !upper.contains(forbidden),
                "{name} contains {forbidden:?}; `memories` must not be rebuilt (FR-521)"
            );
        }
        assert!(
            !upper.contains("CHECK (SCOPE IN"),
            "{name} restates the scope CHECK; only 0001 may declare it (FR-521)"
        );
    }
}

// ---------------------------------------------------------------------------
// T005 / FR-517 — no relation table can link two domains
// ---------------------------------------------------------------------------

/// A relation never spans two domains, and the guarantee is structural.
///
/// The mechanism is **one relations table per domain, with endpoint columns
/// that name only that domain's records**. There is no table with one endpoint
/// in `memories` and the other in `personal_knowledge`, so a cross-domain
/// relation is not something a query gets wrong — it is something no query can
/// express.
///
/// This test deliberately does **not** assert a foreign key from an endpoint to
/// its domain's table, because Feature 003's `memory_relations` never had one:
/// `from_memory_id` and `to_memory_id` are plain `TEXT` with no `REFERENCES`
/// (`0005_project_intelligence.sql`). Asserting an FK would have failed against
/// the shipped schema, and "make it pass" would have meant adding a constraint
/// to Feature 003's table that no Feature 004 requirement asks for. What is
/// asserted instead is what the three-table design actually provides, plus the
/// weaker-but-real FK rule where an FK does exist.
#[test]
fn every_relation_table_names_exactly_one_domains_records() {
    let migrations = local_migrations();
    let all: String = migrations.iter().map(|(_, sql)| sql.as_str()).collect();

    // Each domain's relations table, and the column-name fragment that would
    // betray an endpoint belonging to a *different* domain.
    let domains: &[(&str, &str, &[&str])] = &[
        ("memory_relations", "memories", &["personal", "team"]),
        (
            "personal_knowledge_relations",
            "personal_knowledge",
            &["memory", "team"],
        ),
        (
            "team_knowledge_relations",
            "team_knowledge",
            &["memory", "personal"],
        ),
    ];

    for (table, own_domain, foreign_markers) in domains {
        let declaration = table_declaration(&all, table)
            .unwrap_or_else(|| panic!("no CREATE TABLE for {table}; FR-517 needs one per domain"));
        let lowered = declaration.to_lowercase();

        // No endpoint column may name another domain. A table carrying both
        // `from_memory_id` and `from_personal_id` would be exactly the
        // cross-domain relation this forbids.
        for marker in *foreign_markers {
            let cross = lowered
                .lines()
                .filter(|line| {
                    line.contains("_id") || line.contains("from_") || line.contains("to_")
                })
                .any(|line| line.contains(marker));
            assert!(
                !cross,
                "{table} has an endpoint column naming {marker:?}, outside its domain \
                 ({own_domain}) — a relation must not link two domains (FR-517)"
            );
        }

        // Where a foreign key *is* declared, it stays inside the domain. Rows
        // that merely record who decided a relation and why are not knowledge
        // records and cannot carry one across.
        for target in referenced_tables(&declaration) {
            let permitted = target == *own_domain
                || target == "sessions"
                || target == "users"
                || target == "projects";
            assert!(
                permitted,
                "{table} references {target}, outside its domain ({own_domain}) (FR-517)"
            );
        }
    }
}

/// The three relations tables are three distinct tables.
///
/// Stated separately because the test above would pass on a schema with one
/// shared relations table and a `domain` discriminator column — which is the
/// design `plan.md`'s Complexity Tracking rejected, on the grounds that a
/// forgotten `WHERE domain = ?` in any one query becomes a privacy breach. A
/// single table makes that mistake *writable*; three make it a query error.
#[test]
fn each_domain_has_its_own_relations_table_rather_than_a_shared_one() {
    let migrations = local_migrations();
    let all: String = migrations.iter().map(|(_, sql)| sql.as_str()).collect();
    for table in [
        "memory_relations",
        "personal_knowledge_relations",
        "team_knowledge_relations",
    ] {
        assert!(
            table_declaration(&all, table).is_some(),
            "{table} does not exist; the per-domain isolation rests on it"
        );
    }
    // And no relations table carries a domain discriminator, which is how a
    // shared table would announce itself.
    for table in [
        "memory_relations",
        "personal_knowledge_relations",
        "team_knowledge_relations",
    ] {
        let declaration = table_declaration(&all, table).unwrap();
        assert!(
            !declaration.to_lowercase().contains("domain"),
            "{table} carries a domain column; the three tables must not be one"
        );
    }
}

/// The body of one `CREATE TABLE`, from its name to the matching close paren.
fn table_declaration(sql: &str, table: &str) -> Option<String> {
    let upper = sql.to_uppercase();
    let needle = format!("CREATE TABLE IF NOT EXISTS {}", table.to_uppercase());
    let bare = format!("CREATE TABLE {}", table.to_uppercase());
    let start = upper.find(&needle).or_else(|| upper.find(&bare))?;
    let open = sql[start..].find('(')? + start;
    let mut depth = 0usize;
    for (offset, ch) in sql[open..].char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(sql[open..open + offset + 1].to_string());
                }
            }
            _ => {}
        }
    }
    None
}

/// Every table named by a `REFERENCES` clause in one declaration.
fn referenced_tables(declaration: &str) -> Vec<String> {
    let mut out = Vec::new();
    let lowered = declaration.to_lowercase();
    let mut rest = lowered.as_str();
    while let Some(at) = rest.find("references") {
        rest = &rest[at + "references".len()..];
        let name: String = rest
            .trim_start()
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        if !name.is_empty() && !out.contains(&name) {
            out.push(name);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Phase 5 — personal knowledge crosses projects and never crosses users
// (T083, T084, T089, T090, T091; FR-431, FR-432, FR-437, FR-446, FR-517,
// SC-409, SC-410, SC-424, SC-429)
// ---------------------------------------------------------------------------

use cairn_core::domain::{ApplicabilityFact, ApplicabilityKind, MemoryType, ProjectTrait};
use cairn_core::validate::ProjectIdentity;
use cairn_store::global::{create_personal, recall_personal, NewPersonalKnowledge};
use cairn_store::Store;
use uuid::Uuid;

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime")
}

async fn store() -> Store {
    Store::open_memory().await.expect("in-memory store")
}

fn fact(kind: ApplicabilityKind, value: &str) -> ApplicabilityFact {
    ApplicabilityFact {
        kind,
        value: value.to_string(),
    }
}

fn trait_of(kind: ApplicabilityKind, value: &str) -> ProjectTrait {
    ProjectTrait {
        kind,
        value: value.to_string(),
    }
}

/// T084 / SC-409 — privacy holds across **both** axes.
///
/// A personal note recorded while working in project A is retrievable in project
/// B for the same user, and invisible to a different user in either. Two axes,
/// two assertions: a record that crossed projects but leaked to another user
/// would pass a project-only test, and one private to its owner but stuck in its
/// origin project would pass a user-only test.
#[test]
fn a_personal_note_crosses_projects_and_never_crosses_users() {
    runtime().block_on(async {
        let store = store().await;
        let alice = Uuid::now_v7();
        let bob = Uuid::now_v7();
        let no_identity: [ProjectIdentity; 0] = [];

        create_personal(
            &store,
            NewPersonalKnowledge::direct(
                alice,
                MemoryType::Convention,
                "prefer thiserror over hand-rolled Display impls",
                Some("errors.display"),
                Some("thiserror"),
                vec![],
            ),
            &no_identity,
        )
        .await
        .expect("alice records a note");

        // Project A, project B — the record names neither, so "retrievable in
        // another project" is not a query it has to satisfy: there is no project
        // in the row at all. Recall is by owner.
        let hers = recall_personal(&store, alice, None, None, &[], 50)
            .await
            .expect("alice recalls");
        assert_eq!(hers.len(), 1, "alice cannot see her own note");
        assert!(hers[0].content.contains("thiserror"));

        let his = recall_personal(&store, bob, None, None, &[], 50)
            .await
            .expect("bob recalls");
        assert!(
            his.is_empty(),
            "another user saw alice's personal knowledge: {his:?}"
        );
    });
}

/// T083 / SC-424 — the record has no field for the things that must not travel.
///
/// Inspected on the **serialized** form as well as the struct, because a field
/// that exists locally and is skipped on the wire is a different guarantee from
/// one that does not exist at all — and FR-517 asks for the second.
#[test]
fn a_personal_record_has_no_field_for_a_project_or_a_verification() {
    runtime().block_on(async {
        let store = store().await;
        let owner = Uuid::now_v7();
        let no_identity: [ProjectIdentity; 0] = [];
        let outcome = create_personal(
            &store,
            NewPersonalKnowledge::direct(
                owner,
                MemoryType::Convention,
                "wrap at 95 columns",
                Some("style.width"),
                Some("ninety_five"),
                vec![],
            ),
            &no_identity,
        )
        .await
        .expect("create");

        let serialized = serde_json::to_string(&outcome.record).expect("serialize");
        for forbidden in [
            "project_id",
            "evidence",
            "observation",
            "verification",
            "verified",
            "attested",
            "authority",
            "origin_digest",
        ] {
            assert!(
                !serialized.contains(forbidden),
                "a personal record serialized a `{forbidden}` field: {serialized}"
            );
        }
        // And the row itself, in the schema — the stronger half.
        let names: Vec<String> =
            sqlx::query_scalar("SELECT name FROM pragma_table_info('personal_knowledge')")
                .fetch_all(store.pool())
                .await
                .expect("columns");
        for forbidden in [
            "project_id",
            "verification",
            "evidence_count",
            "observation_ids",
        ] {
            assert!(
                !names.iter().any(|n| n == forbidden),
                "personal_knowledge has a `{forbidden}` column: {names:?}"
            );
        }
    });
}

/// T089 / SC-410 — applicability filtering is deterministic.
///
/// The same record and the same traits give the same verdict every time. Asserted
/// by repetition rather than by inspection: a filter that consulted a clock, a
/// hash seed or row order would diverge across runs, and this is what would
/// catch it.
#[test]
fn applicability_filtering_is_deterministic_across_repeated_queries() {
    runtime().block_on(async {
        let store = store().await;
        let owner = Uuid::now_v7();
        let no_identity: [ProjectIdentity; 0] = [];
        create_personal(
            &store,
            NewPersonalKnowledge::direct(
                owner,
                MemoryType::Convention,
                "use the workspace lockfile",
                Some("build.lockfile"),
                Some("workspace"),
                vec![fact(ApplicabilityKind::Language, "rust")],
            ),
            &no_identity,
        )
        .await
        .expect("create");

        let rust = [trait_of(ApplicabilityKind::Language, "rust")];
        let python = [trait_of(ApplicabilityKind::Language, "python")];

        for _ in 0..5 {
            let matched = recall_personal(&store, owner, None, None, &rust, 50)
                .await
                .expect("recall");
            assert_eq!(matched.len(), 1, "a matching project stopped matching");
            let unmatched = recall_personal(&store, owner, None, None, &python, 50)
                .await
                .expect("recall");
            assert!(
                unmatched.is_empty(),
                "a non-matching project matched: {unmatched:?}"
            );
        }
    });
}

/// T091 / SC-429 — a project with no derivable traits admits only universal
/// records.
///
/// The interesting direction is the exclusion. A test that only checked the
/// universal record appears would pass on an implementation that ignored
/// applicability entirely.
#[test]
fn a_project_with_no_traits_admits_only_universal_records() {
    runtime().block_on(async {
        let store = store().await;
        let owner = Uuid::now_v7();
        let no_identity: [ProjectIdentity; 0] = [];

        create_personal(
            &store,
            NewPersonalKnowledge::direct(
                owner,
                MemoryType::Convention,
                "universal guidance with no restriction",
                Some("general.always"),
                Some("everywhere"),
                vec![],
            ),
            &no_identity,
        )
        .await
        .expect("universal");
        create_personal(
            &store,
            NewPersonalKnowledge::direct(
                owner,
                MemoryType::Convention,
                "guidance restricted to one language",
                Some("general.restricted"),
                Some("rust_only"),
                vec![fact(ApplicabilityKind::Language, "rust")],
            ),
            &no_identity,
        )
        .await
        .expect("restricted");

        // No traits at all: the bare-directory case.
        let admitted = recall_personal(&store, owner, None, None, &[], 50)
            .await
            .expect("recall");
        assert_eq!(
            admitted.len(),
            1,
            "a project with no traits admitted a kind-restricted record: {admitted:?}"
        );
        assert!(admitted[0].content.contains("universal"));
    });
}

/// T090 / FR-446 — an applicability value outside the closed vocabulary is
/// refused on **direct creation**, not only on promotion.
///
/// Refused, never stored with the value silently dropped. Asserting only that
/// valid values are accepted would pass on an implementation that discarded the
/// invalid ones, which is the exact failure FR-446 names.
#[test]
fn an_unrepresentable_applicability_value_is_refused_on_direct_creation() {
    runtime().block_on(async {
        let store = store().await;
        let owner = Uuid::now_v7();
        let no_identity: [ProjectIdentity; 0] = [];

        for bad in ["not a value", "has-dash", "path/like", "UPPER!"] {
            let refused = create_personal(
                &store,
                NewPersonalKnowledge::direct(
                    owner,
                    MemoryType::Convention,
                    "ordinary guidance",
                    Some("style.width"),
                    Some("ninety_five"),
                    vec![fact(ApplicabilityKind::Tool, bad)],
                ),
                &no_identity,
            )
            .await;
            assert!(
                refused.is_err(),
                "applicability value {bad:?} was accepted on direct creation"
            );
        }
        // Nothing was stored by any of the refusals (FR-548).
        let all = recall_personal(&store, owner, None, None, &[], 50)
            .await
            .expect("recall");
        assert!(
            all.is_empty(),
            "a refused creation left a record behind: {all:?}"
        );
    });
}

// ===========================================================================
// The team lifecycle (T134–T144, T172)
// ===========================================================================
//
// Team knowledge is the one thing in Cairn that becomes visible to people who
// did not write it and did not ask for it, which is why its state machine is the
// most guarded thing in the feature: an agent may propose, and only a human
// administrator may make a proposal authoritative.
//
// These tests are split by level on purpose. The lifecycle itself — compare-and-
// swap, visibility, role filtering, applicability — is `cairn-store`'s and is
// driven directly, because that is where the statements are and a test one layer
// up would be asserting the CLI's error mapping as much as the invariant. The
// two that are about the *surface* rather than the mechanism — that no MCP action
// can author or ratify team knowledge — read the tool schema, because the claim
// is about what the surface exposes and nothing else can check that.

use cairn_store::global::{
    list_team, propose_team, ratify_team, recall_team, retire_team, team_subject, NewTeamKnowledge,
    TeamViewer, STATE_CONFLICT,
};
use cairn_store::StoreError;

fn no_identities() -> Vec<ProjectIdentity> {
    Vec::new()
}

fn user() -> Uuid {
    Uuid::now_v7()
}

/// One proposed entry, ready to be ratified.
async fn proposed(store: &Store, proposer: Uuid, content: &str) -> Uuid {
    proposed_about(store, proposer, content, None, None, Vec::new()).await
}

async fn proposed_about(
    store: &Store,
    proposer: Uuid,
    content: &str,
    topic_key: Option<&str>,
    value_key: Option<&str>,
    applicability: Vec<ApplicabilityFact>,
) -> Uuid {
    propose_team(
        store,
        NewTeamKnowledge::direct(
            proposer,
            MemoryType::Convention,
            content,
            topic_key,
            value_key,
            applicability,
        ),
        &no_identities(),
    )
    .await
    .expect("propose")
    .record
    .id
}

// ---------------------------------------------------------------------------
// T134 / SC-460 — no tool action authors or ratifies team knowledge
// ---------------------------------------------------------------------------

/// Every action the six tools expose, enumerated **from the schema**, and none
/// of them can produce an `authoritative` team entry.
///
/// Enumerated rather than listed, because a test against a hardcoded list of
/// actions passes unchanged on the day a seventh action is added — which is
/// precisely when it needed to fail. The one place the action vocabulary is
/// declared is `cairn_remember`'s input schema, so that is what this reads.
#[test]
fn no_mcp_action_can_author_or_ratify_team_knowledge() {
    let source = std::fs::read_to_string(workspace_root().join("crates/cairn/src/mcp.rs"))
        .expect("read mcp.rs");

    // The `action` enum, read out of the schema literal.
    let actions = enum_after(&source, "\"action\": { \"type\": \"string\", \"enum\": [")
        .expect("the `action` enum moved; this test can no longer enumerate the surface");
    assert!(
        actions.len() >= 10,
        "found only {} actions, so the enumeration is not reading the real list: {actions:?}",
        actions.len()
    );

    // Nothing in the vocabulary is shaped like ratification. This is the
    // constitutional line (FR-455): the surface has no action of this shape at
    // all, so an agent holding an admin's token still cannot cross it.
    for forbidden in ["ratify", "retire", "authorize", "approve", "publish"] {
        assert!(
            !actions.iter().any(|a| a == forbidden),
            "`cairn_remember` exposes `{forbidden}`, which moves team guidance to \
             authoritative from the agent surface (FR-455, SC-460)"
        );
    }

    // And `create` cannot name the team domain: the advertised enum omits it,
    // and the daemon refuses it besides.
    let domains = enum_after(&source, "\"domain\": { \"type\": \"string\", \"enum\": [")
        .expect("the `domain` enum is not advertised; `create` cannot be checked");
    assert_eq!(
        domains,
        vec!["project".to_string(), "personal".to_string()],
        "`domain` advertises something other than exactly project and personal"
    );

    // The promotion target *may* name team — that is the one path in, and it
    // lands `proposed`. Asserted so this test does not silently start passing
    // because promotion lost the capability altogether.
    let targets = enum_after(&source, "\"target\": { \"type\": \"string\", \"enum\": [")
        .expect("the promotion `target` enum is not advertised");
    assert!(
        targets.iter().any(|t| t == "team"),
        "promotion can no longer reach the team domain at all: {targets:?}"
    );

    let handlers = std::fs::read_to_string(workspace_root().join("crates/cairnd/src/handlers.rs"))
        .expect("read handlers.rs");
    assert!(
        handlers.contains("cannot be created through cairn_remember"),
        "the daemon no longer refuses `domain: \"team\"` on create"
    );
}

/// The string list inside a JSON-schema `"enum": [ ... ]` that follows `needle`.
fn enum_after(source: &str, needle: &str) -> Option<Vec<String>> {
    let start = source.find(needle)? + needle.len();
    let end = start + source[start..].find(']')?;
    Some(
        source[start..end]
            .split(',')
            .filter_map(|part| {
                let part = part.trim().trim_matches('"');
                (!part.is_empty()).then(|| part.to_string())
            })
            .collect(),
    )
}

// ---------------------------------------------------------------------------
// T135 / SC-413 — a proposal is invisible to recall, including its proposer's
// ---------------------------------------------------------------------------

/// A `proposed` entry is absent from recall for **every** user, its proposer
/// included.
///
/// Not merely filtered from a default: absent from every path that reads
/// `team_knowledge` for display. The proposer sees their own pending proposals
/// through `cairn team list` and nowhere else, because `list` is a review
/// surface and recall is a briefing surface — guidance nobody has approved must
/// not reach an agent's context, however it got there.
///
/// Falsified by relaxing any recall query's `state = 'authoritative'` predicate.
#[test]
fn a_proposal_is_invisible_to_recall_for_everyone_including_its_proposer() {
    runtime().block_on(async {
        let store = Store::open_memory().await.unwrap();
        let proposer = user();
        let id = proposed(
            &store,
            proposer,
            "commit messages follow Conventional Commits",
        )
        .await;

        let recalled = recall_team(&store, None, None, &[], 50).await.unwrap();
        assert!(
            recalled.iter().all(|r| r.id != id),
            "a proposed entry reached recall"
        );

        // The subject read is the other display path, and it must agree.
        let subject = proposed_about(
            &store,
            proposer,
            "trailing whitespace is stripped on save",
            Some("style.whitespace"),
            Some("stripped"),
            Vec::new(),
        )
        .await;
        let view = team_subject(&store, "style.whitespace").await.unwrap();
        assert!(
            view.members.iter().all(|m| m.id != subject),
            "a proposed entry reached the subject read"
        );

        // But its proposer can see it on the review surface.
        let listed = list_team(&store, &TeamViewer::Member(proposer), 50)
            .await
            .unwrap();
        assert!(
            listed.iter().any(|r| r.id == id),
            "a proposer cannot see their own pending proposal"
        );
    });
}

// ---------------------------------------------------------------------------
// T137 / SC-415 — ratification is compare-and-swap, and names the actual state
// ---------------------------------------------------------------------------

/// Two ratifications of the same proposal: the second is refused, and the
/// refusal names the state the entry is actually in.
///
/// Naming the state is the part worth testing. A bare "conflict" tells an
/// administrator nothing about whether someone else approved it a second ago or
/// whether it was retired last week, and those call for opposite next actions.
///
/// Falsified by replacing the conditional `UPDATE` with a read followed by a
/// write, which would let the second ratification apply on top of the first.
#[test]
fn a_second_ratification_is_refused_and_names_the_entrys_actual_state() {
    runtime().block_on(async {
        let store = Store::open_memory().await.unwrap();
        let admin = user();
        let id = proposed(&store, user(), "CI runs on GitHub Actions").await;

        ratify_team(&store, id, admin, None).await.expect("first");

        match ratify_team(&store, id, admin, None).await {
            Err(StoreError::Refused { code, message }) => {
                assert_eq!(code, STATE_CONFLICT);
                assert!(
                    message.contains("authoritative"),
                    "the refusal does not name the actual state: {message}"
                );
            }
            other => panic!("expected a state_conflict refusal, got {other:?}"),
        }
    });
}

// ---------------------------------------------------------------------------
// T138 / FR-462 / SC-466 — disagreement stays visible, in either order
// ---------------------------------------------------------------------------

/// Two contradicting authoritative entries both stay visible, and which one was
/// ratified first changes nothing.
///
/// Ratification is not a resolution step. The temptation is to treat the newer
/// approval as the answer, which would quietly delete a disagreement two
/// administrators actually have — and the disagreement is the information.
///
/// Both orders are exercised, because a test that ran one order would pass on an
/// implementation that always preferred whichever came second.
#[test]
fn two_disagreeing_authoritative_entries_both_stay_visible_in_either_order() {
    runtime().block_on(async {
        for order in [false, true] {
            let store = Store::open_memory().await.unwrap();
            let admin = user();

            let first = proposed_about(
                &store,
                user(),
                "the retry budget is four attempts",
                Some("retry.budget"),
                Some("four"),
                Vec::new(),
            )
            .await;
            let second = proposed_about(
                &store,
                user(),
                "the retry budget is two attempts",
                Some("retry.budget"),
                Some("two"),
                Vec::new(),
            )
            .await;

            let (a, b) = if order {
                (second, first)
            } else {
                (first, second)
            };
            ratify_team(&store, a, admin, None).await.expect("ratify a");
            ratify_team(&store, b, admin, None).await.expect("ratify b");

            let view = team_subject(&store, "retry.budget").await.unwrap();
            assert_eq!(
                view.members.len(),
                2,
                "ratification order {order} lost one of two disagreeing entries"
            );
            assert!(
                format!("{:?}", view.view).contains("Conflicted"),
                "two differing value keys did not surface as a conflict: {:?}",
                view.view
            );
        }
    });
}

// ---------------------------------------------------------------------------
// T140 / SC-416 — server-wide visibility, and none once retired
// ---------------------------------------------------------------------------

/// An authoritative entry is visible regardless of project membership; a retired
/// one is visible to no recall path.
///
/// This is the one place in the feature where authorization is not mediated by
/// project membership, and it is deliberate: team guidance is a server-wide
/// default. Falsified by adding a membership predicate to a team read, or by
/// leaving a retired entry in recall.
#[test]
fn an_authoritative_entry_reaches_every_account_and_a_retired_one_reaches_none() {
    runtime().block_on(async {
        let store = Store::open_memory().await.unwrap();
        let admin = user();
        let id = proposed(&store, user(), "release tags are annotated").await;
        ratify_team(&store, id, admin, None).await.expect("ratify");

        // No caller identity is even an argument to `recall_team` — membership
        // cannot gate what it cannot see.
        let recalled = recall_team(&store, None, None, &[], 50).await.unwrap();
        assert!(
            recalled.iter().any(|r| r.id == id),
            "an authoritative entry did not reach recall"
        );

        retire_team(&store, id, admin).await.expect("retire");
        let after = recall_team(&store, None, None, &[], 50).await.unwrap();
        assert!(
            after.iter().all(|r| r.id != id),
            "a retired entry is still in recall"
        );
    });
}

// ---------------------------------------------------------------------------
// T141 / FR-465 — retirement is not reversible
// ---------------------------------------------------------------------------

/// A retired entry refuses re-ratification, and restoring its guidance means a
/// new proposal.
///
/// So every piece of currently authoritative guidance has exactly one
/// ratification event in its own history, never a resurrection — which is what
/// makes the ratification record answerable to "who approved this, and when".
///
/// Falsified by any transition out of `retired`.
#[test]
fn a_retired_entry_refuses_re_ratification_and_needs_a_new_proposal() {
    runtime().block_on(async {
        let store = Store::open_memory().await.unwrap();
        let admin = user();
        let id = proposed(&store, user(), "dependencies are pinned by lockfile").await;
        ratify_team(&store, id, admin, None).await.expect("ratify");
        retire_team(&store, id, admin).await.expect("retire");

        match ratify_team(&store, id, admin, None).await {
            Err(StoreError::Refused { code, message }) => {
                assert_eq!(code, STATE_CONFLICT);
                assert!(
                    message.contains("retired"),
                    "the refusal does not name `retired`: {message}"
                );
            }
            other => panic!("expected a state_conflict refusal, got {other:?}"),
        }

        // Retiring it again is likewise refused rather than idempotently
        // reapplied: a second `retired_at` would overwrite the record of when
        // the guidance actually stopped applying.
        assert!(retire_team(&store, id, admin).await.is_err());

        // The restoration path is a new proposal, which is itself only proposed.
        let restored = proposed(&store, user(), "dependencies are pinned by lockfile").await;
        assert_ne!(restored, id);
        let recalled = recall_team(&store, None, None, &[], 50).await.unwrap();
        assert!(
            recalled.iter().all(|r| r.id != restored),
            "a restoration proposal was authoritative on creation"
        );
    });
}

// ---------------------------------------------------------------------------
// T142 / FR-464 — role-filtered listing
// ---------------------------------------------------------------------------

/// A member sees authoritative entries and only their **own** proposals; an
/// admin sees every state.
///
/// The member half is the one with teeth: a proposal is somebody's draft
/// judgement about how the whole server should work, and showing everyone's
/// drafts to everyone turns a review queue into a broadcast. Falsified by
/// dropping the `proposed_by_user_id` predicate from the member query.
#[test]
fn listing_is_filtered_by_role_and_a_member_sees_only_their_own_proposals() {
    runtime().block_on(async {
        let store = Store::open_memory().await.unwrap();
        let admin = user();
        let me = user();
        let someone_else = user();

        let authoritative = proposed(&store, someone_else, "changelogs are hand-written").await;
        ratify_team(&store, authoritative, admin, None)
            .await
            .expect("ratify");
        let mine = proposed(&store, me, "review comments are resolved by the author").await;
        let theirs = proposed(&store, someone_else, "squash before merge").await;
        let gone = proposed(&store, someone_else, "an idea that did not last").await;
        ratify_team(&store, gone, admin, None)
            .await
            .expect("ratify");
        retire_team(&store, gone, admin).await.expect("retire");

        let as_member = list_team(&store, &TeamViewer::Member(me), 50)
            .await
            .unwrap();
        let seen: Vec<Uuid> = as_member.iter().map(|r| r.id).collect();
        assert!(seen.contains(&authoritative), "a member sees authoritative");
        assert!(seen.contains(&mine), "a member sees their own proposal");
        assert!(
            !seen.contains(&theirs),
            "a member sees another member's pending proposal"
        );

        let as_admin = list_team(&store, &TeamViewer::Admin, 50).await.unwrap();
        let all: Vec<Uuid> = as_admin.iter().map(|r| r.id).collect();
        for id in [authoritative, mine, theirs, gone] {
            assert!(
                all.contains(&id),
                "an admin cannot see every state; {id} is missing"
            );
        }
    });
}

// ---------------------------------------------------------------------------
// T143 / FR-460 — team applicability
// ---------------------------------------------------------------------------

/// An entry restricted to one language does not apply to a project lacking that
/// trait, and does apply where the project carries it.
///
/// Both halves, because a predicate that refused everything would pass the first
/// assertion alone. Falsified by dropping the applicability filter from
/// `recall_team`, or by making an empty trait set match every restriction.
#[test]
fn team_applicability_narrows_by_project_trait_in_both_directions() {
    runtime().block_on(async {
        let store = Store::open_memory().await.unwrap();
        let admin = user();

        let rust_only = proposed_about(
            &store,
            user(),
            "clippy runs with -D warnings",
            None,
            None,
            vec![fact(ApplicabilityKind::Language, "rust")],
        )
        .await;
        let universal = proposed(&store, user(), "secrets never enter the repository").await;
        for id in [rust_only, universal] {
            ratify_team(&store, id, admin, None).await.expect("ratify");
        }

        let in_rust = recall_team(
            &store,
            None,
            None,
            &[trait_of(ApplicabilityKind::Language, "rust")],
            50,
        )
        .await
        .unwrap();
        let ids: Vec<Uuid> = in_rust.iter().map(|r| r.id).collect();
        assert!(ids.contains(&rust_only), "the restriction did not match");
        assert!(ids.contains(&universal), "a universal entry was excluded");

        let in_go = recall_team(
            &store,
            None,
            None,
            &[trait_of(ApplicabilityKind::Language, "go")],
            50,
        )
        .await
        .unwrap();
        let ids: Vec<Uuid> = in_go.iter().map(|r| r.id).collect();
        assert!(
            !ids.contains(&rust_only),
            "a rust-only entry applied to a go project"
        );
        assert!(
            ids.contains(&universal),
            "a universal entry stopped being universal"
        );
    });
}

// ---------------------------------------------------------------------------
// T144 / FR-517 / SC-424 — a team record carries nothing about where it came
// from
// ---------------------------------------------------------------------------

/// No column and no serialized field for a project, an evidence reference, an
/// observation identifier, a file path, a command, or verification of any kind —
/// not an authority, not a state, not a timestamp.
///
/// Structural, because behavioural would be weaker: a column nobody writes to
/// yet is still a column the next contributor will find and fill in.
#[test]
fn a_team_record_has_no_field_for_a_project_a_command_or_a_verification() {
    runtime().block_on(async {
        let store = Store::open_memory().await.unwrap();
        let columns: Vec<String> =
            sqlx::query_scalar("SELECT name FROM pragma_table_info('team_knowledge')")
                .fetch_all(store.pool())
                .await
                .unwrap();
        assert!(
            !columns.is_empty(),
            "team_knowledge does not exist; this test would pass vacuously"
        );

        const FORBIDDEN: &[&str] = &[
            "project",
            "evidence",
            "observation",
            "path",
            "file",
            "command",
            "verification",
            "verified",
            "authority",
            "worktree",
            "remote",
        ];
        for column in &columns {
            let lowered = column.to_ascii_lowercase();
            for forbidden in FORBIDDEN {
                assert!(
                    !lowered.contains(forbidden),
                    "team_knowledge carries `{column}`, which holds `{forbidden}` (FR-517)"
                );
            }
        }

        // The serialized form too: the wire is where a field escapes, and a
        // record type can gain one without a column behind it.
        let id = proposed(&store, user(), "a claim").await;
        let record = list_team(&store, &TeamViewer::Admin, 50)
            .await
            .unwrap()
            .into_iter()
            .find(|r| r.id == id)
            .expect("the record");
        let json = serde_json::to_value(&record).unwrap();
        let rendered = json.to_string().to_ascii_lowercase();
        for forbidden in ["project_id", "evidence", "observation", "verification"] {
            assert!(
                !rendered.contains(forbidden),
                "the serialized team record carries `{forbidden}`: {json}"
            );
        }
    });
}

// ---------------------------------------------------------------------------
// T172 / FR-469 / FR-470 / SC-417 — three arrays, and `total` counts one of them
// ---------------------------------------------------------------------------

/// `cairn_search` returns project, personal and team results in three distinct
/// arrays, and the project count is identical whether or not the other two hold
/// anything.
///
/// Merging a personal note into `results` would hand a caller knowledge from
/// outside the project among its own memories with nothing in the shape to say
/// which was which — and would silently change `total`, which callers page on.
///
/// Falsified by merging any array into `results`, or by making `total` count
/// what it does not describe.
#[test]
fn search_returns_three_distinct_arrays_and_total_counts_project_results_alone() {
    let s = cairn_e2e::Sandbox::new();
    let mut mcp = cairn_e2e::Mcp::start(&s);
    let cwd = s.repo_path().to_string_lossy().to_string();

    s.must(&[
        "memory",
        "add",
        "--type",
        "convention",
        "--scope",
        "project",
        "the retry backoff is exponential",
    ]);

    let baseline = mcp.tool_result(
        "cairn_search",
        serde_json::json!({ "query": "retry" }),
        &cwd,
    );
    let baseline = &baseline["content"][0]["text"];
    let project_count = baseline["total"].as_i64().expect("total");
    let project_len = baseline["results"].as_array().expect("results").len();
    assert_eq!(project_count as usize, project_len);
    assert!(project_len >= 1, "the project result did not match at all");
    assert!(
        baseline["personal"].is_array() && baseline["team"].is_array(),
        "the sibling arrays are absent even when empty: {baseline}"
    );
    assert!(
        baseline["personal"].as_array().unwrap().is_empty(),
        "personal results appeared before any personal record existed"
    );

    // Now give the personal domain something that matches the same query.
    let created = mcp.tool_result(
        "cairn_remember",
        serde_json::json!({
            "action": "create",
            "domain": "personal",
            "type": "fact",
            "content": "retry budgets are worth writing down",
        }),
        &cwd,
    );
    assert_eq!(
        created["isError"], false,
        "personal create failed: {created}"
    );

    let after = mcp.tool_result(
        "cairn_search",
        serde_json::json!({ "query": "retry" }),
        &cwd,
    );
    let after = &after["content"][0]["text"];
    assert_eq!(
        after["total"].as_i64(),
        Some(project_count),
        "a personal record changed the project result count: {after}"
    );
    assert_eq!(
        after["results"].as_array().map(|a| a.len()),
        Some(project_len),
        "a personal record entered `results`: {after}"
    );
    assert_eq!(
        after["personal"].as_array().map(|a| a.len()),
        Some(1),
        "the personal record did not reach its own array: {after}"
    );

    // And `domains` narrows without merging.
    let project_only = mcp.tool_result(
        "cairn_search",
        serde_json::json!({ "query": "retry", "domains": ["project"] }),
        &cwd,
    );
    let project_only = &project_only["content"][0]["text"];
    assert_eq!(
        project_only["total"].as_i64(),
        Some(project_count),
        "narrowing to `project` changed the project count: {project_only}"
    );
    assert!(
        project_only["personal"].as_array().unwrap().is_empty(),
        "`domains: [\"project\"]` still returned personal results: {project_only}"
    );
}
