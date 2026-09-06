//! Every duplicated schema, key and reference definition says the same thing
//! (T162, ADVERSARIAL GATE; SC-766, `contracts/verification-summary.md` §4,
//! `data-model.md` §5/§6/§6.1).
//!
//! **This file needs no database.** It compares four *texts* — the PostgreSQL
//! migration, the SQLite migration, `data-model.md`, and two Rust
//! implementations — against each other. Nothing a running server could
//! report would settle a disagreement between the source of truth documents;
//! only reading them can. There is accordingly no `pg!()` skip macro here: it
//! always runs.
//!
//! # Why parsing and not eyeballing
//!
//! `data-model.md` §5 and §6 are meant to describe the two migrations
//! byte-for-byte in every column that matters, and a human proofreading pass
//! is exactly the kind of check that quietly stops being exercised the moment
//! it starts passing. This file instead parses every `CREATE TABLE` out of
//! both migrations and both documented schema sections into (table → column →
//! declared type) maps and a normalized-text fingerprint of each table's
//! `ref_kind`/`domain` discriminator machinery, and diffs those structures —
//! so a real edit to one side and not the other fails loudly instead of
//! waiting for someone to notice.
//!
//! # Why column *order* is deliberately not part of the comparison
//! `crates/cairn-store/migrations/0008_safe_events.sql`'s own comment on
//! `retained_local` says it outright: SQLite's grammar ends a table's column
//! list at the first table-level constraint, so `dedupe_key` has to be
//! declared *before* that table's `CHECK`, while `data-model.md` §5 puts the
//! `CHECK` first for readability. "Column order is not semantics" is the
//! document's own claim, and a test that treated it as semantics would fail
//! on a change nothing here actually depends on. So every comparison below is
//! by column *name*, not position.
//!
//! # What "the same UUID as four kinds gives four distinct keys" tests
//!
//! `Reference::reference_key()` (`cairn-core::domain`) and
//! `RetainedRef::dedupe_key()` (`cairn-store::migrate`) are two independent
//! Rust spellings of the identical rule the SQL `GENERATED ALWAYS AS` columns
//! encode a third time. Three spellings of one rule is three chances for one
//! to quietly drift from the other two — which is exactly what SC-766 exists
//! to catch — so this file asserts they agree by computing all three and
//! comparing, rather than reading each in isolation and trusting they match.

use cairn_core::domain::{
    KnowledgeDomain, KnowledgeRef, PatternRef, Reference, RelationKind, RelationRef,
};
use cairn_store::migrate::RetainedRef;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use uuid::Uuid;

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("tests/ has a parent")
        .to_path_buf()
}

fn read(root: &Path, rel: &str) -> String {
    let path = root.join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()))
}

// ---------------------------------------------------------------------------
// A narrow SQL DDL reader — just enough to answer "what tables, what columns,
// what discriminator CHECKs", for exactly the shapes this migration pair
// uses. Not a general SQL parser.
// ---------------------------------------------------------------------------

/// Strip `-- ...` line comments. Every comment in these files is a line
/// comment; none of the string literals this schema uses contains `--`
/// (checked by eye across every table this file reads, since a rule this
/// file relies on deserves to be stated, not just assumed).
fn strip_line_comments(sql: &str) -> String {
    sql.lines()
        .map(|line| match line.find("--") {
            Some(i) => &line[..i],
            None => line,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The concatenated contents of every ```sql fenced block in a Markdown file.
fn sql_fences(markdown: &str) -> String {
    let mut out = String::new();
    let mut in_fence = false;
    for line in markdown.lines() {
        let trimmed = line.trim();
        if !in_fence && trimmed == "```sql" {
            in_fence = true;
            continue;
        }
        if in_fence && trimmed == "```" {
            in_fence = false;
            out.push('\n');
            continue;
        }
        if in_fence {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

/// The contents of only the *first* ```sql fenced block in a Markdown text.
///
/// `data-model.md` §5b documents the v10 migration's DDL in one fence and then
/// shows an illustrative (non-executable) `WHERE` snippet in a second one —
/// `sql_fences` would concatenate both, which is right for a section that
/// documents one table's DDL in one fence but wrong here, where only the first
/// fence is the migration text being held to word-for-word agreement.
fn first_sql_fence(markdown: &str) -> String {
    let mut out = String::new();
    let mut in_fence = false;
    for line in markdown.lines() {
        let trimmed = line.trim();
        if !in_fence && trimmed == "```sql" {
            in_fence = true;
            continue;
        }
        if in_fence && trimmed == "```" {
            break;
        }
        if in_fence {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

/// The text of `data-model.md` between one `##`-level heading and the next,
/// exclusive of both boundaries' following content — i.e. exactly one
/// section. Hard-coded heading text is deliberate: the task names §5 as the
/// local schema and §6 as the server schema, and finding those two specific
/// sections is exactly the join key this file needs, not a general
/// Markdown-section splitter.
fn section<'a>(doc: &'a str, start_heading: &str, end_heading: &str) -> &'a str {
    let start = doc.find(start_heading).unwrap_or_else(|| {
        panic!("data-model.md has no heading {start_heading:?} — this file's section markers have moved")
    });
    let after = &doc[start..];
    let end_rel = after.find(end_heading).unwrap_or_else(|| {
        panic!("data-model.md has no heading {end_heading:?} after {start_heading:?} — this file's section markers have moved")
    });
    &after[..end_rel]
}

/// Every `CREATE TABLE <name> ( ... )` in `sql` (comments already stripped),
/// keyed by lower-cased table name, value is the raw text strictly between
/// the outermost matching parentheses. Walks by `char` rather than by byte so
/// it never has to reason about UTF-8 boundaries, since this corpus is ASCII
/// throughout once comments are stripped.
fn create_tables(sql: &str) -> BTreeMap<String, String> {
    let chars: Vec<char> = sql.chars().collect();
    let upper: Vec<char> = sql.to_uppercase().chars().collect();
    assert_eq!(
        chars.len(),
        upper.len(),
        "the SQL this file reads was expected to be pure ASCII once comments were stripped; \
         upper-casing changed its length, which means some byte here is not what this parser \
         assumes"
    );
    let marker: Vec<char> = "CREATE TABLE".chars().collect();

    let mut out = BTreeMap::new();
    let mut i = 0usize;
    while i + marker.len() <= upper.len() {
        if upper[i..i + marker.len()] != marker[..] {
            i += 1;
            continue;
        }
        let mut j = i + marker.len();
        while j < chars.len() && chars[j].is_whitespace() {
            j += 1;
        }
        let name_start = j;
        while j < chars.len() && chars[j] != '(' && !chars[j].is_whitespace() {
            j += 1;
        }
        let name: String = chars[name_start..j].iter().collect();
        while j < chars.len() && chars[j] != '(' {
            j += 1;
        }
        if j >= chars.len() {
            break;
        }
        j += 1; // past the opening '('
        let body_start = j;
        let mut depth = 1i32;
        while j < chars.len() && depth > 0 {
            match chars[j] {
                '(' => depth += 1,
                ')' => depth -= 1,
                _ => {}
            }
            j += 1;
        }
        let body_end = j - 1;
        let body: String = chars[body_start..body_end].iter().collect();
        out.insert(name.to_lowercase(), body);
        i = j;
    }
    out
}

/// Split a table body on top-level commas — commas inside a nested `(...)`
/// (a `CHECK`, a `UNIQUE (...)`, a `GENERATED ALWAYS AS (...)`) do not count.
fn split_top_level(body: &str) -> Vec<String> {
    let mut items = Vec::new();
    let mut depth = 0i32;
    let mut current = String::new();
    for c in body.chars() {
        match c {
            '(' => {
                depth += 1;
                current.push(c);
            }
            ')' => {
                depth -= 1;
                current.push(c);
            }
            ',' if depth == 0 => {
                items.push(current.trim().to_string());
                current.clear();
            }
            _ => current.push(c),
        }
    }
    if !current.trim().is_empty() {
        items.push(current.trim().to_string());
    }
    items
}

fn normalize_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

const CONSTRAINT_KEYWORDS: [&str; 5] = ["PRIMARY", "UNIQUE", "CHECK", "FOREIGN", "CONSTRAINT"];

struct ParsedTable {
    /// column name -> declared type (the token right after the name)
    columns: BTreeMap<String, String>,
    /// column name -> the column's full normalized definition text, used for
    /// the discriminator fingerprint below
    column_defs: BTreeMap<String, String>,
    /// normalized text of every standalone table-level `CHECK (...)` item
    checks: Vec<String>,
}

fn parse_table(body: &str) -> ParsedTable {
    let mut columns = BTreeMap::new();
    let mut column_defs = BTreeMap::new();
    let mut checks = Vec::new();
    for item in split_top_level(body) {
        let normalized = normalize_ws(&item);
        if normalized.is_empty() {
            continue;
        }
        let mut words = normalized.split(' ');
        let first = words.next().unwrap_or("");
        let first_upper = first.to_uppercase();
        if CONSTRAINT_KEYWORDS.contains(&first_upper.as_str()) {
            if first_upper == "CHECK" {
                checks.push(normalized);
            }
            continue;
        }
        let name = first.to_lowercase();
        let type_word = words.next().unwrap_or("").to_uppercase();
        columns.insert(name.clone(), type_word);
        column_defs.insert(name, normalized);
    }
    ParsedTable {
        columns,
        column_defs,
        checks,
    }
}

/// The `ref_kind`/`domain` null-slot rule this table encodes, as one
/// normalized string: the `ref_kind` column's own definition (which carries
/// its `CHECK (ref_kind IN (...))` vocabulary), the `domain` column's own
/// definition (same), and any standalone table-level `CHECK` that mentions
/// `ref_kind` — concatenated in a fixed order chosen by this function, not by
/// document order, so a reordering of unrelated constraints cannot change the
/// fingerprint.
fn discriminator_fingerprint(t: &ParsedTable) -> String {
    let mut parts = Vec::new();
    if let Some(rk) = t.column_defs.get("ref_kind") {
        parts.push(format!("ref_kind: {rk}"));
    }
    if let Some(d) = t.column_defs.get("domain") {
        parts.push(format!("domain: {d}"));
    }
    let mut relevant: Vec<&String> = t.checks.iter().filter(|c| c.contains("ref_kind")).collect();
    relevant.sort();
    for c in relevant {
        parts.push(format!("check: {c}"));
    }
    parts.join(" | ")
}

// ---------------------------------------------------------------------------
// The five polymorphic-reference tables this file holds to the same rule
// pairwise (migration vs. doc). `knowledge_verification`'s `domain` CHECK
// deliberately excludes `'project'` — project knowledge already carries its
// verification columns on `memories` (migration 0002) — so this file compares
// each table's fingerprint against *its own* documented counterpart, never
// against a different table's fingerprint.
// ---------------------------------------------------------------------------

const SERVER_DISCRIMINATOR_TABLES: [&str; 4] = [
    "retrieval_trace_items",
    "verification_reports",
    "knowledge_verification",
    "delivered_context",
];
const LOCAL_DISCRIMINATOR_TABLES: [&str; 1] = ["retained_local"];

fn server_migration_tables(root: &Path) -> BTreeMap<String, String> {
    let sql = strip_line_comments(&read(
        root,
        "crates/cairn-server/migrations/0004_autonomous_memory.sql",
    ));
    create_tables(&sql)
}

fn local_migration_tables(root: &Path) -> BTreeMap<String, String> {
    let sql = strip_line_comments(&read(
        root,
        "crates/cairn-store/migrations/0008_safe_events.sql",
    ));
    create_tables(&sql)
}

/// Local schema v9 — `crates/cairn-store/migrations/0009_pattern_cache.sql`,
/// the owner's pulled-pattern cache (`cached_patterns`).
fn local_migration_tables_v9(root: &Path) -> BTreeMap<String, String> {
    let sql = strip_line_comments(&read(
        root,
        "crates/cairn-store/migrations/0009_pattern_cache.sql",
    ));
    create_tables(&sql)
}

/// Feature 002's local pattern table, `reusable_patterns`
/// (`crates/cairn-store/migrations/0005_project_intelligence.sql`), read here
/// only as the *other* table `cached_patterns` (v9) must never be confused
/// with. This migration declares it `CREATE TABLE IF NOT EXISTS`, which
/// `create_tables` does not parse (it would read the table name as `IF`) —
/// rather than teach the parser a general `IF NOT EXISTS` grammar rule it
/// needs nowhere else in this corpus, the one migration that writes it is
/// normalized to plain `CREATE TABLE` before parsing.
fn legacy_reusable_patterns_table(root: &Path) -> BTreeMap<String, String> {
    let sql = strip_line_comments(&read(
        root,
        "crates/cairn-store/migrations/0005_project_intelligence.sql",
    ));
    let sql = sql.replace("CREATE TABLE IF NOT EXISTS", "CREATE TABLE");
    create_tables(&sql)
}

fn data_model(root: &Path) -> String {
    read(
        root,
        "specs/005-server-authoritative-autonomous-memory/data-model.md",
    )
}

fn doc_server_tables(doc: &str) -> BTreeMap<String, String> {
    let section = section(doc, "## 6. Server schema v4", "## 6.1");
    create_tables(&strip_line_comments(&sql_fences(section)))
}

fn doc_local_tables(doc: &str) -> BTreeMap<String, String> {
    let section = section(doc, "## 5. Local schema v8", "## 5a.");
    create_tables(&strip_line_comments(&sql_fences(section)))
}

/// `data-model.md` §5a — the v9 `cached_patterns` cache table.
fn doc_local_v9_tables(doc: &str) -> BTreeMap<String, String> {
    let section = section(doc, "## 5a. Local schema v9", "## 5b.");
    create_tables(&strip_line_comments(&sql_fences(section)))
}

/// `data-model.md` §5b's own text, unmodified — used both for the DDL
/// comparison (its first fence) and for the prose phrase comparison (the
/// section's own words), so callers slice it however either check needs.
fn doc_local_v10_section(doc: &str) -> &str {
    section(doc, "## 5b. Local schema v10", "## 6. Server schema v4")
}

/// Every `CREATE TABLE` the server migration defines has the same column set
/// (name and declared type) as the same-named table in `data-model.md` §6,
/// and vice versa — no table exists on only one side.
#[test]
fn every_server_table_matches_data_model_section_6_column_for_column() {
    let root = workspace_root();
    let migration = server_migration_tables(&root);
    let doc = doc_server_tables(&data_model(&root));

    assert!(
        migration.len() >= 15,
        "expected at least 15 CREATE TABLE statements in 0004_autonomous_memory.sql, parsed {}; \
         a parser regression here would make every comparison below vacuous",
        migration.len()
    );

    for (name, body) in &migration {
        let doc_body = doc.get(name).unwrap_or_else(|| {
            panic!(
                "`{name}` is created by crates/cairn-server/migrations/0004_autonomous_memory.sql \
                 but data-model.md §6 documents no table of that name"
            )
        });
        let m = parse_table(body);
        let d = parse_table(doc_body);
        assert_eq!(
            m.columns, d.columns,
            "`{name}`'s column set (name -> declared type) disagrees between \
             0004_autonomous_memory.sql and data-model.md §6"
        );
    }
    for name in doc.keys() {
        assert!(
            migration.contains_key(name),
            "data-model.md §6 documents `{name}` but 0004_autonomous_memory.sql does not create it"
        );
    }
}

/// The same comparison for the local (SQLite) schema: every table
/// `0008_safe_events.sql` creates has the same columns as `data-model.md` §5's
/// same-named table, and vice versa.
#[test]
fn every_local_table_matches_data_model_section_5_column_for_column() {
    let root = workspace_root();
    let migration = local_migration_tables(&root);
    let doc = doc_local_tables(&data_model(&root));

    assert!(
        migration.len() >= 8,
        "expected at least 8 CREATE TABLE statements in 0008_safe_events.sql, parsed {}",
        migration.len()
    );

    for (name, body) in &migration {
        let doc_body = doc.get(name).unwrap_or_else(|| {
            panic!(
                "`{name}` is created by crates/cairn-store/migrations/0008_safe_events.sql but \
                 data-model.md §5 documents no table of that name"
            )
        });
        let m = parse_table(body);
        let d = parse_table(doc_body);
        assert_eq!(
            m.columns, d.columns,
            "`{name}`'s column set (name -> declared type) disagrees between \
             0008_safe_events.sql and data-model.md §5"
        );
    }
    for name in doc.keys() {
        assert!(
            migration.contains_key(name),
            "data-model.md §5 documents `{name}` but 0008_safe_events.sql does not create it"
        );
    }
}

/// The same comparison, extended to local schema v9:
/// `0009_pattern_cache.sql`'s `cached_patterns` has the same columns *and* the
/// same per-column definitions — which is what carries the embedded
/// `CHECK (trust = 'sanitized')`, since that CHECK is written inline on the
/// `trust` column rather than as a standalone table-level constraint, so it
/// never appears in `ParsedTable::checks` and has to be caught in
/// `column_defs` instead — as `data-model.md` §5a's copy of the same table.
///
/// **Falsified by** a column added, removed, retyped, or a `NOT NULL` /
/// `DEFAULT` / `CHECK` changed on one side and not the other.
#[test]
fn cached_patterns_matches_data_model_section_5a_columns_and_checks() {
    let root = workspace_root();
    let migration = local_migration_tables_v9(&root);
    let doc = doc_local_v9_tables(&data_model(&root));

    let migration_body = migration
        .get("cached_patterns")
        .expect("0009_pattern_cache.sql creates `cached_patterns`");
    let doc_body = doc
        .get("cached_patterns")
        .unwrap_or_else(|| panic!("data-model.md §5a documents no `cached_patterns` table"));

    let m = parse_table(migration_body);
    let d = parse_table(doc_body);

    assert_eq!(
        m.columns, d.columns,
        "`cached_patterns`'s column set (name -> declared type) disagrees between \
         0009_pattern_cache.sql and data-model.md §5a"
    );
    assert_eq!(
        m.column_defs, d.column_defs,
        "`cached_patterns`'s full column definitions disagree between \
         0009_pattern_cache.sql and data-model.md §5a — this is what would miss a changed \
         NOT NULL, DEFAULT, or the embedded `CHECK (trust = 'sanitized')` on `trust`, none of \
         which a bare column-type comparison would notice"
    );
}

/// `cached_patterns` (v9) is a genuinely separate table from `reusable_patterns`
/// (Feature 002, v5) — not a second name for the same rows.
///
/// `data-model.md` §5a ("Why this is not `reusable_patterns`") states the
/// design reason: `reusable_patterns` declares `signals`, `signal_digest`,
/// `origin_ref` and `sanitization_report` NOT NULL, and those (with
/// `source_memory_id` and `origin_deleted`) are exactly the six field names
/// the privacy boundary refuses at the sync boundary (FR-708b,
/// `contracts/knowledge-commands.md` §3.3) — so a server-pulled row can never
/// be stored as a `reusable_patterns` row without fabricating content for a
/// NOT NULL column the server never sent. This test checks that claim against
/// the two migrations directly: the column sets differ, `cached_patterns`
/// carries the server's canonical identity columns
/// (`owner_user_id` + `content_key` + `pattern_id`), and none of the four
/// refused local-only names leak into it.
///
/// **Falsified by** `cached_patterns` gaining any of `signals`,
/// `signal_digest`, `origin_ref` or `sanitization_report`, or by the two
/// tables' column sets becoming equal.
#[test]
fn cached_patterns_is_a_separate_table_from_reusable_patterns() {
    let root = workspace_root();
    let cache_migration = local_migration_tables_v9(&root);
    let promoted_migration = legacy_reusable_patterns_table(&root);

    let cached = parse_table(
        cache_migration
            .get("cached_patterns")
            .expect("0009_pattern_cache.sql creates `cached_patterns`"),
    );
    let promoted = parse_table(
        promoted_migration
            .get("reusable_patterns")
            .expect("0005_project_intelligence.sql creates `reusable_patterns`"),
    );

    let cached_names: BTreeSet<&String> = cached.columns.keys().collect();
    let promoted_names: BTreeSet<&String> = promoted.columns.keys().collect();
    assert_ne!(
        cached_names, promoted_names,
        "`cached_patterns` and `reusable_patterns` have identical column sets — they are \
         supposed to be two different kinds of record, a local promoted row and a pulled \
         canonical row, and this equality would mean the split collapsed"
    );

    for col in ["owner_user_id", "content_key", "pattern_id"] {
        assert!(
            cached.columns.contains_key(col),
            "`cached_patterns` is missing `{col}`, the server's canonical identity column"
        );
    }

    for col in [
        "signals",
        "signal_digest",
        "origin_ref",
        "sanitization_report",
    ] {
        assert!(
            promoted.columns.contains_key(col),
            "`reusable_patterns` is missing `{col}` — this test's premise (it carries the \
             local-only names the privacy boundary refuses) no longer holds against \
             0005_project_intelligence.sql"
        );
        assert!(
            !cached.columns.contains_key(col),
            "`cached_patterns` carries `{col}`, one of the field names the privacy boundary \
             refuses (FR-708b) — a server-pulled row must never carry this local-only, \
             pre-sanitization evidence"
        );
    }
}

/// Local schema v10: `event_spool` and `command_spool` gain
/// `server_instance_id`, and the claim index is rebuilt to match both
/// identities. The migration's DDL and `data-model.md` §5b's first fenced
/// block are compared as normalized text rather than table-by-table, because
/// the change here is two `ALTER TABLE ADD COLUMN`s and two rebuilt indexes on
/// existing tables, not new `CREATE TABLE`s — `create_tables` has nothing to
/// find in either.
///
/// The second half checks that the *documented isolation semantics* — not
/// just the column and index text — are the same claim in both places, by
/// requiring the migration's own explanatory phrases to also appear in
/// `data-model.md` §5b's prose. A migration and a doc can agree on DDL while
/// disagreeing on what the DDL means; this is what would catch that.
///
/// **Falsified by** the migration and the doc's DDL diverging in any column,
/// index definition, or claim predicate column order, or by either side
/// dropping the stated exact-deployment-isolation rule.
#[test]
fn v10_server_instance_columns_and_isolation_semantics_match_data_model_section_5b() {
    let root = workspace_root();
    let migration_raw = read(
        &root,
        "crates/cairn-store/migrations/0010_spool_server_instance.sql",
    );
    let migration_ddl = strip_line_comments(&migration_raw);

    let doc_text = data_model(&root);
    let section_5b = doc_local_v10_section(&doc_text);
    let doc_ddl = strip_line_comments(&first_sql_fence(section_5b));

    assert_eq!(
        normalize_ws(&migration_ddl),
        normalize_ws(&doc_ddl),
        "0010_spool_server_instance.sql's DDL (the two `server_instance_id` columns and the \
         two rebuilt claim indexes) disagrees, word for word, with data-model.md §5b's first \
         fenced SQL block"
    );

    for phrase in [
        "An endpoint is not an identity",
        "safe first-binding rule",
        "provisional id",
    ] {
        assert!(
            migration_raw.contains(phrase),
            "0010_spool_server_instance.sql no longer states {phrase:?} — this test's list of \
             shared terms is stale"
        );
        assert!(
            section_5b.contains(phrase),
            "data-model.md §5b does not state {phrase:?} in those words, though \
             0010_spool_server_instance.sql's own comments do — the documented \
             exact-deployment-isolation semantics have drifted from the migration's stated \
             reasoning"
        );
    }
}

/// The structural discriminator CHECKs — the `ref_kind`/`domain` null-slot
/// rule — agree between each migration and its documented counterpart, table
/// by table.
#[test]
fn discriminator_checks_agree_between_migrations_and_data_model() {
    let root = workspace_root();
    let doc_text = data_model(&root);

    let server_migration = server_migration_tables(&root);
    let server_doc = doc_server_tables(&doc_text);
    for name in SERVER_DISCRIMINATOR_TABLES {
        let m = parse_table(
            server_migration
                .get(name)
                .unwrap_or_else(|| panic!("0004_autonomous_memory.sql has no `{name}` table")),
        );
        let d = parse_table(
            server_doc
                .get(name)
                .unwrap_or_else(|| panic!("data-model.md §6 has no `{name}` table")),
        );
        assert_eq!(
            discriminator_fingerprint(&m),
            discriminator_fingerprint(&d),
            "`{name}`'s ref_kind/domain discriminator rule disagrees between \
             0004_autonomous_memory.sql and data-model.md §6"
        );
    }

    let local_migration = local_migration_tables(&root);
    let local_doc = doc_local_tables(&doc_text);
    for name in LOCAL_DISCRIMINATOR_TABLES {
        let m = parse_table(
            local_migration
                .get(name)
                .unwrap_or_else(|| panic!("0008_safe_events.sql has no `{name}` table")),
        );
        let d = parse_table(
            local_doc
                .get(name)
                .unwrap_or_else(|| panic!("data-model.md §5 has no `{name}` table")),
        );
        assert_eq!(
            discriminator_fingerprint(&m),
            discriminator_fingerprint(&d),
            "`{name}`'s ref_kind/domain discriminator rule disagrees between \
             0008_safe_events.sql and data-model.md §5"
        );
    }
}

/// The canonical `reference_key` `GENERATED ALWAYS AS` expression is the
/// *identical* text everywhere it is spelled out in SQL — all four server
/// tables that carry one, and both times `data-model.md` §6 quotes one — not
/// four independent formulas that happen to agree today.
#[test]
fn the_generated_reference_key_expression_is_one_definition_in_sql() {
    let root = workspace_root();
    let doc_text = data_model(&root);
    let server_migration = server_migration_tables(&root);
    let server_doc = doc_server_tables(&doc_text);

    let mut expressions: BTreeSet<String> = BTreeSet::new();
    let mut found_in: Vec<String> = Vec::new();

    for (label, tables) in [
        ("0004_autonomous_memory.sql", &server_migration),
        ("data-model.md §6", &server_doc),
    ] {
        for (name, body) in tables.iter() {
            let parsed = parse_table(body);
            if let Some(def) = parsed.column_defs.get("reference_key") {
                expressions.insert(def.clone());
                found_in.push(format!("{label}::{name}"));
            }
        }
    }

    assert!(
        found_in.len() >= 6,
        "expected `reference_key` GENERATED columns in at least 3 server tables, found in {} \
         locations: {found_in:?} — a parser regression would make this comparison vacuous",
        found_in.len()
    );
    assert_eq!(
        expressions.len(),
        1,
        "the `reference_key` generated-column expression is not one definition — found {} \
         distinct texts across {found_in:?}:\n{expressions:#?}",
        expressions.len()
    );

    let canonical = expressions.iter().next().unwrap();
    assert!(
        canonical.contains("'knowledge:' || domain || ':' || knowledge_id")
            && canonical.contains("'pattern:' || knowledge_id"),
        "the canonical reference_key expression does not have the expected shape \
         (`knowledge:<domain>:<id>` / `pattern:<id>`): {canonical}"
    );
}

/// The same rule, expressed a third and fourth way in Rust —
/// `Reference::reference_key()` and `RetainedRef::dedupe_key()` — agrees with
/// itself and with the SQL formula's shape, and one UUID interpreted as
/// project, personal, team and pattern produces four distinct keys (SC-766).
#[test]
fn one_uuid_as_project_personal_team_and_pattern_gives_four_distinct_reference_keys() {
    let id = Uuid::parse_str("11111111-2222-3333-4444-555555555555").unwrap();

    let project = Reference::Knowledge(KnowledgeRef::project(id));
    let personal = Reference::Knowledge(KnowledgeRef::personal(id));
    let team = Reference::Knowledge(KnowledgeRef::team(id));
    let pattern = Reference::Pattern(PatternRef(id));

    let keys = [
        project.reference_key(),
        personal.reference_key(),
        team.reference_key(),
        pattern.reference_key(),
    ];

    assert_eq!(
        keys[0],
        format!("knowledge:project:{id}"),
        "Reference::reference_key() for a project KnowledgeRef must match the SQL formula's \
         `knowledge:<domain>:<id>` shape exactly"
    );
    assert_eq!(keys[1], format!("knowledge:personal:{id}"));
    assert_eq!(keys[2], format!("knowledge:team:{id}"));
    assert_eq!(
        keys[3],
        format!("pattern:{id}"),
        "Reference::reference_key() for a PatternRef must match the SQL formula's \
         `pattern:<id>` shape exactly"
    );

    let unique: BTreeSet<&String> = keys.iter().collect();
    assert_eq!(
        unique.len(),
        4,
        "one UUID as project/personal/team/pattern must produce four distinct reference keys \
         (SC-766); got {keys:?}"
    );

    // `RetainedRef::dedupe_key()` (crates/cairn-store/src/migrate.rs) is the
    // local side's independent spelling of the identical rule
    // (`retained_local.dedupe_key`, data-model.md §5). Two derivations of one
    // rule that could silently diverge is exactly the drift SC-766 exists to
    // catch, so they are compared directly rather than trusted to agree.
    let retained_keys = [
        RetainedRef::Knowledge {
            domain: KnowledgeDomain::Project,
            id,
        }
        .dedupe_key(),
        RetainedRef::Knowledge {
            domain: KnowledgeDomain::Personal,
            id,
        }
        .dedupe_key(),
        RetainedRef::Knowledge {
            domain: KnowledgeDomain::Team,
            id,
        }
        .dedupe_key(),
        RetainedRef::Pattern(id).dedupe_key(),
    ];
    assert_eq!(
        retained_keys, keys,
        "RetainedRef::dedupe_key() and Reference::reference_key() must produce the identical \
         string for the identical reference — a second spelling of the same rule is the drift \
         SC-766 forbids"
    );
}

/// The **relation** dedupe key has no `Reference` counterpart, so it is checked
/// against the document instead.
///
/// # The gap this closes
///
/// The test above compares `RetainedRef::dedupe_key()` with
/// `Reference::reference_key()`, which covers knowledge and patterns because
/// both derivations exist for those. A relation has no `Reference` — it is a
/// `(from, to, kind)` triple and deliberately has no id of its own — so nothing
/// was comparing its prefix to anything at all. Renaming `relation:` to `rel:`
/// in `cairn-store` passed the whole gate.
///
/// `data-model.md` §5 states the three spellings on `retained_local.dedupe_key`
/// itself, so that comment is the counterpart, and this reads it rather than
/// restating it — a hardcoded expectation here would be a fourth definition of
/// the thing the file exists to keep at one.
///
/// **Falsified by** any of the three prefixes changing on either side.
#[test]
fn every_retained_dedupe_key_uses_the_prefix_the_document_states() {
    let doc = data_model(&workspace_root());
    let comment = doc
        .lines()
        .find(|l| l.contains("dedupe_key") && l.contains("--"))
        .unwrap_or_else(|| {
            panic!("data-model.md no longer states the dedupe key's shapes on the column")
        })
        .to_string();

    let id = Uuid::now_v7();
    let relation = RetainedRef::Relation(RelationRef {
        from_memory_id: id,
        to_memory_id: Uuid::now_v7(),
        kind: RelationKind::Supersedes,
    });
    for (what, key) in [
        (
            "knowledge",
            RetainedRef::Knowledge {
                domain: KnowledgeDomain::Team,
                id,
            }
            .dedupe_key(),
        ),
        ("pattern", RetainedRef::Pattern(id).dedupe_key()),
        ("relation", relation.dedupe_key()),
    ] {
        let prefix = format!("{what}:");
        assert!(
            key.starts_with(&prefix),
            "a {what} dedupe key is `{key}`, which does not begin `{prefix}`"
        );
        assert!(
            comment.contains(&format!("'{prefix}")),
            "`{prefix}` is not one of the shapes data-model.md §5 states for \
             `retained_local.dedupe_key`, so the code and the document disagree \
             about how a {what} is named: {comment}"
        );
    }

    // And the relation's own body is the natural key, spelled the one way the
    // rest of the system spells it.
    let RetainedRef::Relation(r) = relation else {
        unreachable!("just constructed")
    };
    assert_eq!(
        relation.dedupe_key(),
        format!("relation:{}", r.relation_key()),
        "the retained-local spelling of a relation has drifted from \
         `RelationRef::relation_key()`, which is the one definition of that key"
    );
}
