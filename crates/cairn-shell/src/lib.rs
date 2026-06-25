//! Shell/tool-output compression — Cairn's take on RTK.
//!
//! Verbose command output (cargo test, git status, build logs, directory listings) burns tokens.
//! We filter the noise and collapse repetition into a compact view, while retaining the **exact**
//! original in the blob store so it's recoverable byte-for-byte via `expand <hash>` — nothing is
//! lost. The compression itself is a set of pure functions; [`ShellCompressor`] adds retention.

use cairn_core::Result;
use cairn_store::Store;
use serde::Serialize;
use std::sync::Arc;

/// The result of compressing a command's output.
#[derive(Debug, Clone, Serialize)]
pub struct Compressed {
    pub command: String,
    /// Handle to the retained full original — recover it with `expand`.
    pub original_hash: String,
    pub original_lines: usize,
    pub compressed_lines: usize,
    /// Fraction of lines removed, in `[0, 1]`.
    pub saved_ratio: f32,
    pub output: String,
}

pub struct ShellCompressor {
    store: Arc<Store>,
}

impl ShellCompressor {
    pub fn new(store: Arc<Store>) -> Self {
        Self { store }
    }

    /// Compress `output` for `command`, retaining the exact original for lossless recovery.
    pub fn compress(&self, command: &str, output: &str) -> Result<Compressed> {
        let original_hash = self.store.blobs().put_str(output)?.0;
        let original_lines = output.lines().count();
        let compressed = compress_output(command, output);
        let compressed_lines = compressed.lines().count();
        let saved_ratio = if original_lines == 0 {
            0.0
        } else {
            (1.0 - compressed_lines as f32 / original_lines as f32).clamp(0.0, 1.0)
        };
        Ok(Compressed {
            command: command.to_string(),
            original_hash,
            original_lines,
            compressed_lines,
            saved_ratio,
            output: compressed,
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Category {
    CargoTest,
    CargoBuild,
    GitStatus,
    Generic,
}

fn categorize(command: &str) -> Category {
    let c = command.to_lowercase();
    let has = |w: &str| c.split(|ch: char| !ch.is_alphanumeric()).any(|t| t == w);
    if has("cargo") && has("test") {
        Category::CargoTest
    } else if has("cargo") && (has("build") || has("clippy") || has("check")) {
        Category::CargoBuild
    } else if has("git") && has("status") {
        Category::GitStatus
    } else {
        Category::Generic
    }
}

/// The compression pipeline: category-specific filter → collapse consecutive repeats →
/// head/tail truncate if still huge.
fn compress_output(command: &str, output: &str) -> String {
    let lines: Vec<&str> = output.lines().collect();
    let filtered: Vec<String> = match categorize(command) {
        Category::CargoTest => filter_keep(
            &lines,
            &[
                "test result",
                "failed",
                "failures:",
                "panicked",
                "error",
                "warning:",
            ],
        ),
        Category::CargoBuild => filter_keep(&lines, &["warning:", "error", "finished"]),
        Category::GitStatus => filter_drop(&lines, &["  (use "]),
        Category::Generic => lines.iter().map(|s| s.to_string()).collect(),
    };
    let deduped = dedup_consecutive(&filtered);
    truncate_head_tail(&deduped, 200, 60, 40).join("\n")
}

/// Keep only lines containing one of `needles` (case-insensitive).
fn filter_keep(lines: &[&str], needles: &[&str]) -> Vec<String> {
    lines
        .iter()
        .filter(|l| {
            let low = l.to_lowercase();
            needles.iter().any(|n| low.contains(n))
        })
        .map(|s| s.to_string())
        .collect()
}

/// Drop lines that start with any of `prefixes`.
fn filter_drop(lines: &[&str], prefixes: &[&str]) -> Vec<String> {
    lines
        .iter()
        .filter(|l| !prefixes.iter().any(|p| l.starts_with(p)))
        .map(|s| s.to_string())
        .collect()
}

/// Collapse runs of identical consecutive lines into `line  (×N)`.
fn dedup_consecutive(lines: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    let mut i = 0;
    while i < lines.len() {
        let mut j = i + 1;
        while j < lines.len() && lines[j] == lines[i] {
            j += 1;
        }
        let count = j - i;
        if count > 1 {
            out.push(format!("{}  (×{count})", lines[i]));
        } else {
            out.push(lines[i].clone());
        }
        i = j;
    }
    out
}

/// If `lines` exceeds `limit`, keep `head` + `tail` with an omission marker between.
fn truncate_head_tail(lines: &[String], limit: usize, head: usize, tail: usize) -> Vec<String> {
    if lines.len() <= limit || head + tail >= lines.len() {
        return lines.to_vec();
    }
    let omitted = lines.len() - head - tail;
    let mut out = Vec::with_capacity(head + tail + 1);
    out.extend(lines[..head].iter().cloned());
    out.push(format!(
        "… {omitted} lines omitted (recover the full output with `expand`) …"
    ));
    out.extend(lines[lines.len() - tail..].iter().cloned());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- filter_keep ---

    #[test]
    fn filter_keep_empty_input_returns_empty() {
        assert!(filter_keep(&[], &["error"]).is_empty());
    }

    #[test]
    fn filter_keep_no_match_returns_empty() {
        let lines = ["hello world", "foo bar"];
        assert!(filter_keep(&lines, &["error"]).is_empty());
    }

    #[test]
    fn filter_keep_all_match_returns_all() {
        let lines = ["error here", "another error", "error again"];
        let out = filter_keep(&lines, &["error"]);
        assert_eq!(out.len(), 3);
    }

    #[test]
    fn filter_keep_case_insensitive() {
        let lines = ["WARNING: something", "clean line", "Error occurred"];
        let out = filter_keep(&lines, &["warning:", "error"]);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn filter_keep_partial_needle_matches() {
        let lines = ["test result: ok"];
        let out = filter_keep(&lines, &["test result"]);
        assert_eq!(out.len(), 1);
    }

    // --- filter_drop ---

    #[test]
    fn filter_drop_empty_input_returns_empty() {
        assert!(filter_drop(&[], &["  ("]).is_empty());
    }

    #[test]
    fn filter_drop_no_prefix_match_returns_all() {
        let lines = ["On branch main", "Changes not staged:"];
        let out = filter_drop(&lines, &["  (use "]);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn filter_drop_all_match_returns_empty() {
        let lines = ["  (use git add)", "  (use git restore)"];
        assert!(filter_drop(&lines, &["  ("]).is_empty());
    }

    #[test]
    fn filter_drop_mixed() {
        let lines = ["keep this", "  (use git add)", "also keep"];
        let out = filter_drop(&lines, &["  ("]);
        assert_eq!(out, vec!["keep this", "also keep"]);
    }

    // --- dedup_consecutive ---

    #[test]
    fn dedup_empty() {
        assert!(dedup_consecutive(&[]).is_empty());
    }

    #[test]
    fn dedup_single_line() {
        let lines = vec!["only".to_string()];
        assert_eq!(dedup_consecutive(&lines), lines);
    }

    #[test]
    fn dedup_no_repeats_unchanged() {
        let lines: Vec<String> = ["a", "b", "c"].iter().map(|s| s.to_string()).collect();
        assert_eq!(dedup_consecutive(&lines), lines);
    }

    #[test]
    fn dedup_all_same_collapses() {
        let lines: Vec<String> = ["x"; 5].iter().map(|s| s.to_string()).collect();
        let out = dedup_consecutive(&lines);
        assert_eq!(out.len(), 1);
        assert!(out[0].contains("×5"));
    }

    #[test]
    fn dedup_two_identical_collapsed() {
        let lines = vec!["same".to_string(), "same".to_string()];
        let out = dedup_consecutive(&lines);
        assert_eq!(out.len(), 1);
        assert!(out[0].contains("×2"));
    }

    #[test]
    fn dedup_non_adjacent_duplicates_not_collapsed() {
        let lines: Vec<String> = ["a", "b", "a"].iter().map(|s| s.to_string()).collect();
        // "a" appears twice but not consecutively
        assert_eq!(dedup_consecutive(&lines).len(), 3);
    }

    // --- truncate_head_tail ---

    #[test]
    fn truncate_below_limit_unchanged() {
        let lines: Vec<String> = (0..10).map(|i| format!("line {i}")).collect();
        let out = truncate_head_tail(&lines, 200, 60, 40);
        assert_eq!(out, lines, "below limit: unchanged");
    }

    #[test]
    fn truncate_exactly_at_limit_unchanged() {
        let lines: Vec<String> = (0..200).map(|i| format!("line {i}")).collect();
        let out = truncate_head_tail(&lines, 200, 60, 40);
        assert_eq!(out.len(), 200, "at limit: unchanged");
    }

    #[test]
    fn truncate_when_head_plus_tail_covers_all_unchanged() {
        // head(60) + tail(40) = 100; if lines <= 100, no truncation
        let lines: Vec<String> = (0..100).map(|i| format!("line {i}")).collect();
        let out = truncate_head_tail(&lines, 50, 60, 40); // limit=50 but head+tail=100 >= len=100
        assert_eq!(out.len(), 100, "head+tail covers all: unchanged");
    }

    #[test]
    fn truncate_large_input_correct_shape() {
        let lines: Vec<String> = (0..500).map(|i| format!("line {i}")).collect();
        let out = truncate_head_tail(&lines, 200, 60, 40);
        assert_eq!(out.len(), 60 + 1 + 40, "head + omission + tail");
        assert!(
            out[60].contains("omitted"),
            "middle line is omission marker"
        );
        assert_eq!(out[0], "line 0");
        assert_eq!(out[out.len() - 1], "line 499");
    }

    #[test]
    fn truncate_omission_count_is_correct() {
        let lines: Vec<String> = (0..300).map(|i| format!("line {i}")).collect();
        let out = truncate_head_tail(&lines, 200, 60, 40);
        let omitted = 300 - 60 - 40;
        assert!(
            out[60].contains(&omitted.to_string()),
            "omission count shown"
        );
    }

    // --- compress_output ---

    #[test]
    fn cargo_test_keeps_failures_drops_passes() {
        let mut out = String::new();
        for i in 0..50 {
            out.push_str(&format!("test mod::case_{i} ... ok\n"));
        }
        out.push_str("test mod::broken ... FAILED\n");
        out.push_str("test result: FAILED. 50 passed; 1 failed; 0 ignored\n");
        let c = compress_output("cargo test --workspace", &out);
        assert!(c.contains("FAILED"));
        assert!(c.contains("test result"));
        assert!(
            !c.contains("case_0 ... ok"),
            "per-test passes should be dropped"
        );
    }

    #[test]
    fn dedup_collapses_consecutive_repeats() {
        let lines: Vec<String> = ["same", "same", "same", "diff"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(
            dedup_consecutive(&lines),
            vec!["same  (×3)".to_string(), "diff".to_string()]
        );
    }

    #[test]
    fn truncate_keeps_head_and_tail() {
        let lines: Vec<String> = (0..500).map(|i| format!("line {i}")).collect();
        let t = truncate_head_tail(&lines, 200, 60, 40);
        assert_eq!(t.len(), 60 + 1 + 40);
        assert_eq!(t[0], "line 0");
        assert_eq!(t[t.len() - 1], "line 499");
        assert!(t[60].contains("omitted"));
    }

    #[test]
    fn compress_retains_original_for_recovery() {
        let Some(store) = Store::open_for_test() else {
            return;
        };
        let store = Arc::new(store);
        let sc = ShellCompressor::new(store.clone());

        let mut out = String::new();
        for i in 0..300 {
            out.push_str(&format!("noise line {i}\n"));
        }
        let c = sc.compress("ls -R /huge", &out).unwrap();
        assert!(c.compressed_lines < c.original_lines);
        assert!(c.saved_ratio > 0.0);

        // The exact original is recoverable from the blob store via the handle.
        let recovered = store
            .blobs()
            .get_str(&cairn_core::ContentHash(c.original_hash.clone()))
            .unwrap()
            .unwrap();
        assert_eq!(recovered, out);
    }

    #[test]
    fn cargo_build_keeps_warnings_and_finished() {
        let out = "   Compiling foo v1.0.0\n\
                   warning: unused variable `x`\n\
                   warning: unused import\n\
                   error[E0382]: borrow of moved value\n\
                   Finished `dev` profile [unoptimized] target(s) in 2.35s\n";
        let c = compress_output("cargo build", out);
        assert!(c.contains("warning:"), "warnings kept");
        assert!(c.contains("error"), "errors kept");
        assert!(c.contains("Finished"), "finished kept");
        assert!(!c.contains("Compiling foo"), "compile lines dropped");
    }

    #[test]
    fn cargo_clippy_treated_as_cargo_build() {
        let out = "   Checking foo v1.0.0\nwarning: clippy::pedantic: ...\nFinished\n";
        let c = compress_output("cargo clippy --workspace", out);
        assert!(c.contains("warning:"));
        assert!(c.contains("Finished"));
    }

    #[test]
    fn git_status_drops_use_hints() {
        // Use explicit \n so the leading two spaces are preserved (backslash line-continuation
        // strips leading whitespace, breaking the "  (use " prefix the filter expects).
        let out = "On branch main\nChanges not staged for commit:\n  (use \"git add\" to stage)\n  (use \"git restore\" to discard)\n\tmodified: src/lib.rs\n";
        let c = compress_output("git status", out);
        assert!(c.contains("src/lib.rs"), "modified files kept");
        assert!(!c.contains("use \"git add\""), "use-hints dropped");
    }

    #[test]
    fn generic_command_passes_all_lines_through() {
        let out = "line one\nline two\nline three\n";
        let c = compress_output("ls -la", out);
        assert!(c.contains("line one"));
        assert!(c.contains("line two"));
        assert!(c.contains("line three"));
    }

    #[test]
    fn compress_empty_output_saved_ratio_is_zero() {
        let Some(store) = Store::open_for_test() else {
            return;
        };
        let sc = ShellCompressor::new(Arc::new(store));
        let c = sc.compress("ls", "").unwrap();
        assert_eq!(c.saved_ratio, 0.0, "empty output → 0 ratio");
        assert_eq!(c.original_lines, 0);
    }

    #[test]
    fn compress_single_line_output() {
        let Some(store) = Store::open_for_test() else {
            return;
        };
        let sc = ShellCompressor::new(Arc::new(store));
        let c = sc.compress("echo hello", "hello\n").unwrap();
        assert!(c.saved_ratio >= 0.0 && c.saved_ratio <= 1.0);
    }

    #[test]
    fn compress_ratio_clamped_between_zero_and_one() {
        let Some(store) = Store::open_for_test() else {
            return;
        };
        let sc = ShellCompressor::new(Arc::new(store));
        let mut out = String::new();
        for i in 0..50 {
            out.push_str(&format!("test case_{i} ... ok\n"));
        }
        out.push_str("test result: ok. 50 passed\n");
        let c = sc.compress("cargo test", &out).unwrap();
        assert!(
            c.saved_ratio >= 0.0 && c.saved_ratio <= 1.0,
            "ratio must be clamped to [0,1], got {}",
            c.saved_ratio
        );
    }
}
