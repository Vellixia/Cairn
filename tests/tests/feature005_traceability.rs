//! Every FR and SC identifier this feature named is accounted for exactly
//! once, and the mechanical inventory at the end of `tasks.md` matches both
//! the traceability table it summarizes and `spec.md`'s own vocabulary (T163).
//!
//! **This file needs no database.** It reads three Markdown files
//! (`tasks.md` twice over — its table and its trailing HTML comments — and
//! `spec.md` once) and checks that the sets of identifiers they name agree.
//! There is no `pg!()` skip macro here: it always runs.
//!
//! # What "appears in the traceability table" means, operationally
//!
//! The Traceability Index does not spell out 272 FR identifiers one at a
//! time — it states ranges, `FR-701–FR-712a`, one row at a time, exactly
//! because spelling out every one would be the unmaintainable thing this test
//! exists to check instead. So "does this identifier appear in the table" is
//! answered by: parse every range token out of every row, expand a range to
//! every baseline identifier whose numeric part falls between the range's
//! numeric endpoints (a range's own end may carry a letter suffix —
//! `FR-712a` — and that suffix is not part of the *numeric* comparison, only
//! the identifier's own spelling is), and take the union. An identifier not
//! covered by any row's range is not traced, and that is the defect this file
//! looks for.
//!
//! # Why the expected counts are asserted, not adjusted
//!
//! 272 and 63 are what this file's own count of the two HTML comments
//! produces today, recorded as the expected values a caller can see failed
//! rather than silently drift. If a future edit to `tasks.md` changes the
//! count, this test fails and says what the actual number was and which
//! identifiers are new or missing — it does not quietly accept a new count as
//! the correct one.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

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

fn tasks_md(root: &Path) -> String {
    read(
        root,
        "specs/005-server-authoritative-autonomous-memory/tasks.md",
    )
}

fn spec_md(root: &Path) -> String {
    read(
        root,
        "specs/005-server-authoritative-autonomous-memory/spec.md",
    )
}

/// Every maximal `<prefix><digits><lowercase/digit suffix>` token in `text`,
/// e.g. `extract_identifiers(text, "FR-")` finds `FR-708`, `FR-708a`,
/// `FR-777a1`. Walks by `char_indices` so a UTF-8 multi-byte character
/// elsewhere in the document (an em dash in prose, say) never lands the scan
/// mid-character.
fn extract_identifiers(text: &str, prefix: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for (i, _) in text.char_indices() {
        if !text[i..].starts_with(prefix) {
            continue;
        }
        let rest = &text[i + prefix.len()..];
        let digit_len = rest.bytes().take_while(u8::is_ascii_digit).count();
        if digit_len == 0 {
            continue;
        }
        let bytes = rest.as_bytes();
        let mut end = digit_len;
        while end < bytes.len() && (bytes[end].is_ascii_lowercase() || bytes[end].is_ascii_digit())
        {
            end += 1;
        }
        out.insert(format!("{prefix}{}", &rest[..end]));
    }
    out
}

/// The numeric part of an identifier like `FR-708a` (`708`), used only to
/// test range membership — never to reconstruct or compare identities.
fn numeric_part(id: &str) -> u32 {
    let after_dash = id.split_once('-').map(|(_, rest)| rest).unwrap_or(id);
    let digits: String = after_dash
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    digits
        .parse()
        .unwrap_or_else(|_| panic!("`{id}` has no leading digits after its dash"))
}

/// The exhaustive identifier list inside `<!-- <marker>: ... -->` at the end
/// of `tasks.md` — the mechanical coverage inventory the file's own comment
/// says exists so this test can compare sets.
fn html_comment_list(tasks: &str, marker: &str) -> Vec<String> {
    let needle = format!("<!-- {marker}:");
    let start = tasks
        .find(&needle)
        .unwrap_or_else(|| panic!("tasks.md has no `{needle}` comment"));
    let after = &tasks[start + needle.len()..];
    let end = after
        .find("-->")
        .unwrap_or_else(|| panic!("the `{needle}` comment in tasks.md is never closed"));
    after[..end]
        .split_whitespace()
        .map(str::to_string)
        .collect()
}

/// Every range token (`FR-701–FR-712a`) or standalone identifier token in one
/// Traceability Index cell, as `(start, end)` pairs (a standalone identifier
/// is a range of one). Only tokens whose prefix is `FR-` or `SC-` are
/// considered; prose words in the same cell (`"and"`, `"storage"`, ...) are
/// silently not ranges and are skipped.
fn ranges_in_cell(cell: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for raw_tok in cell.split_whitespace() {
        let tok =
            raw_tok.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '-' && c != '–');
        if tok.is_empty() {
            continue;
        }
        let is_ident = |s: &str| {
            (s.starts_with("FR-") || s.starts_with("SC-"))
                && s.split_once('-')
                    .map(|(_, rest)| rest.chars().next().is_some_and(|c| c.is_ascii_digit()))
                    .unwrap_or(false)
        };
        if let Some((lo, hi)) = tok.split_once('\u{2013}') {
            // an en-dash range, e.g. "FR-701–FR-712a"
            if is_ident(lo) && is_ident(hi) {
                out.push((lo.to_string(), hi.to_string()));
            }
        } else if is_ident(tok) {
            out.push((tok.to_string(), tok.to_string()));
        }
    }
    out
}

/// The rows of the `## Traceability Index` table, as (left cell, right cell)
/// pairs, skipping the header and separator rows.
fn traceability_rows(tasks: &str) -> Vec<(String, String)> {
    let start = tasks
        .find("## Traceability Index")
        .expect("tasks.md has a `## Traceability Index` heading");
    let after = &tasks[start..];
    let end = after[1..]
        .find("\n## ")
        .map(|i| i + 1)
        .unwrap_or(after.len());
    let section = &after[..end];

    let mut rows = Vec::new();
    for line in section.lines() {
        let line = line.trim();
        if !line.starts_with('|') {
            continue;
        }
        let cells: Vec<&str> = line.trim_matches('|').split('|').map(str::trim).collect();
        if cells.len() < 2 {
            continue;
        }
        if cells[0] == "Requirement block" || cells[0].chars().all(|c| c == '-') {
            continue;
        }
        rows.push((cells[0].to_string(), cells[1].to_string()));
    }
    assert!(
        rows.len() >= 15,
        "expected at least 15 data rows in the Traceability Index, parsed {} — the table \
         parser may have mis-located the section",
        rows.len()
    );
    rows
}

/// Expand every range in every traceability row into the set of `master`
/// identifiers (of the matching FR/SC prefix) whose numeric part falls
/// within that range, inclusive.
fn covered_by_table(rows: &[(String, String)], master: &BTreeSet<String>) -> BTreeSet<String> {
    let mut covered = BTreeSet::new();
    for (left, _) in rows {
        for (lo, hi) in ranges_in_cell(left) {
            let prefix = if lo.starts_with("FR-") { "FR-" } else { "SC-" };
            let lo_n = numeric_part(&lo);
            let hi_n = numeric_part(&hi);
            for id in master {
                if id.starts_with(prefix) {
                    let n = numeric_part(id);
                    if n >= lo_n && n <= hi_n {
                        covered.insert(id.clone());
                    }
                }
            }
        }
    }
    covered
}

/// Identifiers are unique within each list, and the counts are the ones this
/// file's own doc comment records: 272 FR, 63 SC. If tasks.md's inventory has
/// genuinely changed, this fails and names the actual count — the expected
/// constants below are not adjusted to match a drifted file.
#[test]
fn the_fr_and_sc_inventories_are_unique_and_the_recorded_counts() {
    let root = workspace_root();
    let tasks = tasks_md(&root);

    let fr_list = html_comment_list(&tasks, "FR");
    let sc_list = html_comment_list(&tasks, "SC");

    let fr_set: BTreeSet<&str> = fr_list.iter().map(String::as_str).collect();
    let sc_set: BTreeSet<&str> = sc_list.iter().map(String::as_str).collect();

    assert_eq!(
        fr_list.len(),
        fr_set.len(),
        "the FR inventory comment lists {} entries but only {} distinct identifiers — a \
         duplicate is present",
        fr_list.len(),
        fr_set.len()
    );
    assert_eq!(
        sc_list.len(),
        sc_set.len(),
        "the SC inventory comment lists {} entries but only {} distinct identifiers — a \
         duplicate is present",
        sc_list.len(),
        sc_set.len()
    );

    // **Moved deliberately, and here is what moved.** FR-792's freshness
    // semantics were a specification gap: reviewed material said the reason
    // delivery is not progressing must be visible, and said nothing about
    // where status gets reachability or peer identity from. CI reported no
    // mismatch while a replacement deployment was answering, because both
    // came from process-local state that a daemon replacement discards.
    //
    // Added: FR-792a (status takes a fresh, bounded, read-only peer-identity
    // sample), FR-792b (that probe may touch nothing durable — no binding, no
    // lane, no cursor, no claim, no attempt), FR-792c (a cached observation is
    // telemetry and decides no reported reason), FR-792d (the blocked-reason
    // precedence is stated rather than incidental), and SC-718a (the report is
    // correct across a daemon replacement, in both directions and when the
    // endpoint is unreachable). Four FRs, one SC: 272 → 276 and 63 → 64.
    // FR-791a/FR-791b added with the team row revision: `changed_at` was being
    // used as a row version, and PostgreSQL `now()` is transaction-start time —
    // so `GREATEST(...)` can hold one value across two different states, and the
    // feed keyed on it can drop a change entirely. Two FRs, no new SC: SC-718a
    // already grades the report, and the divergence is graded by the server
    // schema target rather than by a criterion of its own. 276 -> 278.
    // FR-791a1 added with the commit-order allocator: FR-791a asked for a
    // strictly increasing version, which a sequence satisfies while still
    // letting a client advance its cursor past a change that commits later.
    // 278 -> 279.
    // FR-749c1/FR-749c2 added with the deadline-drop record. FR-749c required a
    // `capture_deadline_exceeded` disposition surfaced in capture health, and
    // the value existed in every vocabulary, both schemas' CHECK constraints and
    // the health funnel's own column while **no code path produced one** — the
    // hook that detects the drop holds no store and the daemon never saw the
    // delivery that failed, and the requirement said nothing about the gap
    // between them. FR-749c1 states it (journal, collect, idempotent, never
    // fails the hook); FR-749c2 states that a decline caused by Cairn's own
    // deadline is not a decline about the content. 279 -> 281.
    const EXPECTED_FR: usize = 281;
    const EXPECTED_SC: usize = 64;
    assert_eq!(
        fr_list.len(),
        EXPECTED_FR,
        "expected {EXPECTED_FR} FR identifiers in tasks.md's inventory comment, found {} — \
         report the actual count and which identifiers changed rather than adjusting this \
         constant",
        fr_list.len()
    );
    assert_eq!(
        sc_list.len(),
        EXPECTED_SC,
        "expected {EXPECTED_SC} SC identifiers in tasks.md's inventory comment, found {} — \
         report the actual count and which identifiers changed rather than adjusting this \
         constant",
        sc_list.len()
    );
}

/// Every identifier in the two HTML-comment inventories is covered by some
/// row of the Traceability Index above them — "covered" meaning its numeric
/// part falls inside a range that row states.
#[test]
fn every_inventoried_identifier_is_covered_by_the_traceability_table() {
    let root = workspace_root();
    let tasks = tasks_md(&root);

    let fr_master: BTreeSet<String> = html_comment_list(&tasks, "FR").into_iter().collect();
    let sc_master: BTreeSet<String> = html_comment_list(&tasks, "SC").into_iter().collect();
    let rows = traceability_rows(&tasks);

    let ranges_found = rows
        .iter()
        .map(|(l, _)| ranges_in_cell(l).len())
        .sum::<usize>();
    assert!(
        ranges_found >= 15,
        "parsed only {ranges_found} FR/SC ranges out of the Traceability Index's {} rows — the \
         range parser may have regressed",
        rows.len()
    );

    let covered_fr = covered_by_table(&rows, &fr_master);
    let missing_fr: Vec<&String> = fr_master.difference(&covered_fr).collect();
    assert!(
        missing_fr.is_empty(),
        "these FR identifiers are listed in tasks.md's inventory but are not covered by any \
         range in the Traceability Index: {missing_fr:?}"
    );

    let covered_sc = covered_by_table(&rows, &sc_master);
    let missing_sc: Vec<&String> = sc_master.difference(&covered_sc).collect();
    assert!(
        missing_sc.is_empty(),
        "these SC identifiers are listed in tasks.md's inventory but are not covered by any \
         range in the Traceability Index: {missing_sc:?}"
    );
}

/// Every FR/SC identifier `spec.md` actually uses appears in tasks.md's
/// inventory — a requirement or success criterion the spec names but the
/// task plan never inventoried would be untraceable work.
#[test]
fn every_identifier_spec_md_uses_appears_in_the_tasks_md_inventory() {
    let root = workspace_root();
    let tasks = tasks_md(&root);
    let spec = spec_md(&root);

    let fr_master: BTreeSet<String> = html_comment_list(&tasks, "FR").into_iter().collect();
    let sc_master: BTreeSet<String> = html_comment_list(&tasks, "SC").into_iter().collect();

    // Feature 005's own numbering floor, derived from the inventory itself
    // rather than hard-coded: spec.md states outright (its own §"why FR-701,
    // not FR-401") that 003 and 004 already occupy the FR-401–FR-519 band and
    // that 005 was numbered to start above it. So any identifier spec.md
    // mentions *below* the lowest number this feature's own inventory
    // actually uses is — by the document's own stated numbering rule — a
    // citation of an earlier feature's identifier for contrast, never one of
    // this spec's own requirements. A genuinely-owned FR/SC omitted from the
    // inventory by mistake would still have to number at or above this floor,
    // so the filter below cannot hide that kind of gap.
    let fr_floor = fr_master.iter().map(|id| numeric_part(id)).min().unwrap();
    let sc_floor = sc_master.iter().map(|id| numeric_part(id)).min().unwrap();

    let fr_used: BTreeSet<String> = extract_identifiers(&spec, "FR-")
        .into_iter()
        .filter(|id| numeric_part(id) >= fr_floor)
        .collect();
    let sc_used: BTreeSet<String> = extract_identifiers(&spec, "SC-")
        .into_iter()
        .filter(|id| numeric_part(id) >= sc_floor)
        .collect();

    assert!(
        fr_used.len() > 100,
        "expected spec.md to use well over 100 distinct FR identifiers at or above this \
         feature's own numbering floor ({fr_floor}), found {} — the extractor may have \
         regressed",
        fr_used.len()
    );
    assert!(
        sc_used.len() > 30,
        "expected spec.md to use well over 30 distinct SC identifiers at or above this \
         feature's own numbering floor ({sc_floor}), found {} — the extractor may have \
         regressed",
        sc_used.len()
    );

    let missing_fr: Vec<&String> = fr_used.difference(&fr_master).collect();
    assert!(
        missing_fr.is_empty(),
        "spec.md uses these FR identifiers, but tasks.md's inventory comment does not list \
         them: {missing_fr:?}"
    );

    let missing_sc: Vec<&String> = sc_used.difference(&sc_master).collect();
    assert!(
        missing_sc.is_empty(),
        "spec.md uses these SC identifiers, but tasks.md's inventory comment does not list \
         them: {missing_sc:?}"
    );
}

/// A record of what this file actually measured, printed unconditionally so a
/// passing run still states the numbers rather than only a failing one.
#[test]
fn report_measured_counts() {
    let root = workspace_root();
    let tasks = tasks_md(&root);
    let spec = spec_md(&root);

    let fr_master_len = html_comment_list(&tasks, "FR").len();
    let sc_master_len = html_comment_list(&tasks, "SC").len();
    let fr_used_len = extract_identifiers(&spec, "FR-").len();
    let sc_used_len = extract_identifiers(&spec, "SC-").len();

    eprintln!(
        "feature005_traceability: tasks.md inventory FR={fr_master_len} SC={sc_master_len}; \
         spec.md uses FR={fr_used_len} SC={sc_used_len}"
    );
}
