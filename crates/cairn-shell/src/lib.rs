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
}
