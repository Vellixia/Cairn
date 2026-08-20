//! T139 — the metric table, with numbers (`contracts/evaluation.md` §Metrics
//! and gates).
//!
//! Every row of that table names a metric, a target and the test that proves
//! it. This walks the table, counts what the corpus actually contains, checks
//! that each named test exists, and emits the whole thing with real numbers —
//! so a release note carries measurements rather than assertions.
//!
//! What it deliberately does **not** do is re-run the assertions. Each named
//! test already runs in the suite; duplicating it here would double the cost
//! and, worse, create a second place where the answer could be wrong. This
//! reports coverage and size, and fails when a required row has no test to
//! stand behind it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("a workspace root")
        .to_path_buf()
}

fn corpus_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("knowledge")
}

/// One row of the table, as parsed.
struct Row {
    number: String,
    metric: String,
    target: String,
    tests: Vec<String>,
    sc: String,
}

fn table() -> Vec<Row> {
    let text = std::fs::read_to_string(
        root().join("specs/003-project-intelligence/contracts/evaluation.md"),
    )
    .expect("the evaluation contract");

    let mut rows = Vec::new();
    for line in text.lines() {
        if !line.starts_with("| ") {
            continue;
        }
        let cells: Vec<&str> = line.trim_matches('|').split('|').map(str::trim).collect();
        if cells.len() < 5 {
            continue;
        }
        let number = cells[0].to_string();
        // Row numbers are `7`, `12a`, `34d`; the header and the rules are not.
        if number.is_empty()
            || !number.chars().next().is_some_and(|c| c.is_ascii_digit())
            || !number.chars().all(|c| c.is_ascii_alphanumeric())
        {
            continue;
        }
        let tests = cells[3]
            .split('`')
            .skip(1)
            .step_by(2)
            .map(|s| s.to_string())
            .collect();
        rows.push(Row {
            number,
            metric: cells[1].to_string(),
            target: cells[2].to_string(),
            tests,
            sc: cells[4].to_string(),
        });
    }
    rows
}

/// Every test name the suite defines, `binary::function` and bare function.
fn test_names() -> (Vec<String>, Vec<String>) {
    // `--list` is not available from inside a test, so the names come from the
    // sources. A test is `fn name()` under a `#[test]`, which is unambiguous in
    // this workspace.
    let mut functions = Vec::new();
    let mut binaries = Vec::new();

    fn walk(dir: &Path, functions: &mut Vec<String>, binaries: &mut Vec<String>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name != "target" && name != ".git" {
                    walk(&path, functions, binaries);
                }
            } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                let Ok(text) = std::fs::read_to_string(&path) else {
                    continue;
                };
                if text.contains("#[test]") || text.contains("#[tokio::test]") {
                    if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                        binaries.push(stem.to_string());
                    }
                }
                let mut previous_was_test = false;
                for line in text.lines() {
                    let trimmed = line.trim();
                    if previous_was_test {
                        if let Some(rest) = trimmed.strip_prefix("fn ") {
                            if let Some(name) = rest.split(['(', '<']).next() {
                                functions.push(name.to_string());
                            }
                        }
                    }
                    previous_was_test = trimmed == "#[test]" || trimmed == "#[tokio::test]";
                }
            }
        }
    }

    walk(&root().join("tests/tests"), &mut functions, &mut binaries);
    walk(&root().join("crates"), &mut functions, &mut binaries);
    functions.sort();
    functions.dedup();
    binaries.sort();
    binaries.dedup();
    (functions, binaries)
}

/// How many cases each corpus group holds.
fn corpus_sizes() -> BTreeMap<String, usize> {
    fn walk(dir: &Path, prefix: &str, out: &mut BTreeMap<String, usize>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        let mut here = 0;
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if path.is_dir() {
                let child = if prefix.is_empty() {
                    name.clone()
                } else {
                    format!("{prefix}/{name}")
                };
                walk(&path, &child, out);
            } else if path.extension().and_then(|e| e.to_str()) == Some("json") {
                here += 1;
            }
        }
        if !prefix.is_empty() {
            out.insert(prefix.to_string(), here);
        }
    }
    let mut out = BTreeMap::new();
    walk(&corpus_root(), "", &mut out);
    out
}

/// The sizes the corpus contract asks for by name.
const REQUIRED_SIZES: &[(&str, usize)] = &[
    ("reconciliation/equivalent", 20),
    ("reconciliation/distinct", 20),
    ("reconciliation/coarse_value_key", 15),
    ("conflict/real", 15),
    ("conflict/scope_exception", 10),
    ("conflict/disjoint", 10),
    ("privacy", 30),
    ("patterns/refuse", 10),
];

/// Emit the metric table with actual numbers, and fail if a required row has
/// no test behind it.
#[test]
fn the_metric_table_has_numbers() {
    let rows = table();
    assert!(
        rows.len() >= 36,
        "the metric table has {} rows; the contract states 36",
        rows.len()
    );

    let (functions, binaries) = test_names();
    let sizes = corpus_sizes();
    let mut missing: Vec<String> = Vec::new();
    let mut report = String::new();

    report.push_str("\nFeature 003 — metrics and gates\n");
    report.push_str(&"=".repeat(78));
    report.push('\n');
    report.push_str(&format!(
        "{:<5} {:<8} {:<44} {}\n",
        "#", "SC", "metric", "evidence"
    ));
    report.push_str(&"-".repeat(78));
    report.push('\n');

    for row in &rows {
        let mut evidence = Vec::new();
        for name in &row.tests {
            let (found, label) = match name.split_once("::") {
                Some((_, function)) => (functions.iter().any(|f| f == function), name.clone()),
                None => (
                    binaries.iter().any(|b| b == name) || functions.iter().any(|f| f == name),
                    name.clone(),
                ),
            };
            if found {
                evidence.push(label);
            } else {
                missing.push(format!(
                    "row {} names `{name}`, which does not exist",
                    row.number
                ));
                evidence.push(format!("{label} (MISSING)"));
            }
        }
        if row.tests.is_empty() {
            // Row 31 names "existing suites" rather than one test: the whole
            // Feature 001 and 002 suite is its evidence, and it runs.
            evidence.push("the Feature 001 and 002 suites".into());
        }

        report.push_str(&format!(
            "{:<5} {:<8} {:<44} {}\n",
            row.number,
            row.sc,
            truncate(&row.metric, 44),
            evidence.join(", ")
        ));
        report.push_str(&format!("{:<5} {:<8} target: {}\n", "", "", row.target));
    }

    report.push('\n');
    report.push_str("Corpus\n");
    report.push_str(&"-".repeat(78));
    report.push('\n');
    let total: usize = sizes.values().sum();
    for (group, count) in &sizes {
        let required = REQUIRED_SIZES
            .iter()
            .find(|(name, _)| name == group)
            .map(|(_, n)| *n);
        match required {
            Some(n) => report.push_str(&format!("  {group:<38} {count:>4}   (contract: ≥{n})\n")),
            None => report.push_str(&format!("  {group:<38} {count:>4}\n")),
        }
    }
    report.push_str(&format!("  {:<38} {total:>4}\n", "total cases"));

    // Printed rather than written to a file: the run's own output is the
    // record, and a file would be a second thing to keep in step.
    println!("{report}");

    let mut short: Vec<String> = Vec::new();
    for (group, minimum) in REQUIRED_SIZES {
        let count = sizes.get(*group).copied().unwrap_or(0);
        if count < *minimum {
            short.push(format!(
                "{group} holds {count} cases, contract asks for ≥{minimum}"
            ));
        }
    }

    assert!(
        missing.is_empty() && short.is_empty(),
        "the metric table is not backed by evidence:\n  {}\n  {}",
        missing.join("\n  "),
        short.join("\n  ")
    );
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        return s.to_string();
    }
    let mut out: String = s.chars().take(n.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// Every corpus directory the contract's tree names exists and holds cases.
///
/// The tree in `contracts/evaluation.md` §The corpus is the list; a group that
/// is named there and empty here is a metric with nothing behind it.
#[test]
fn every_named_corpus_group_holds_cases() {
    let sizes = corpus_sizes();
    for group in [
        "reconciliation/equivalent",
        "reconciliation/distinct",
        "reconciliation/coarse_value_key",
        "reconciliation/duplicate_content",
        "reconciliation/free_form",
        "conflict/real",
        "conflict/scope_exception",
        "conflict/disjoint",
        "supersession",
        "merge",
        "merge/symmetric_relation",
        "merge/task_divergence",
        "merge/blocked_recovery",
        "verification/authority",
        "drift",
        "budget/oversized_task",
        "continuity",
        "staleness/external_edit",
        "patterns/promote",
        "patterns/refuse",
        "patterns/independence",
        "patterns/counterexample",
        "privacy",
        "tasks",
    ] {
        let count = sizes.get(group).copied().unwrap_or(0);
        assert!(count > 0, "the corpus group `{group}` holds no cases");
    }
}
