//! Shell/tool-output compression - Cairn's take on RTK.
//!
//! Verbose command output (cargo test, git status, build logs, directory listings) burns tokens.
//! We filter the noise and collapse repetition into a compact view, while retaining the **exact**
//! original in the blob store so it's recoverable byte-for-byte via `expand <hash>` - nothing is
//! lost. The compression itself is a set of pure functions; [`ShellCompressor`] adds retention.
//!
//! ## Architecture (P4.4)
//!
//! - [`category`] - role-based `Category` enum (Vcs/Build/PackageManager/Lint/Infra/...)
//! - [`registry`] - `Pattern` struct + `REGISTRY` + dispatch (`find_match`, `apply`)
//! - [`pipeline`] - pure helpers: `filter_keep`, `filter_drop`, `dedup_consecutive`, `truncate_head_tail`
//! - [`patterns`] - one file per category with the actual `Pattern` literals
//!
//! Adding a new pattern: drop a `Pattern` literal into the appropriate `patterns/*.rs`,
//! re-export it from `patterns/mod.rs`, and add it to the `REGISTRY` slice. No other
//! dispatch logic to touch.

mod category;
mod pipeline;
mod registry;

pub mod patterns;

pub use category::Category;
pub use registry::{find_match, Pattern, REGISTRY};

use cairn_core::Result;
use cairn_store::Store;
use serde::Serialize;
use std::sync::Arc;

/// The result of compressing a command's output.
#[derive(Debug, Clone, Serialize)]
pub struct Compressed {
    pub command: String,
    /// Handle to the retained full original - recover it with `expand`.
    pub original_hash: String,
    pub original_lines: usize,
    pub compressed_lines: usize,
    /// Fraction of lines removed, in `[0, 1]`.
    pub saved_ratio: f32,
    pub output: String,
    /// The category the command was classified into (e.g. "vcs", "build"). None when
    /// the command was unrecognized and fell through to the generic pipeline.
    pub category: Option<&'static str>,
    /// The name of the matched pattern, if any. None for unrecognized commands.
    pub pattern: Option<&'static str>,
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

        let pattern = find_match(command);
        let (compressed, category, pattern_name) = match pattern {
            Some(p) => (
                registry::apply(p, output),
                Some(p.category.id()),
                Some(p.name),
            ),
            None => (generic_compress(output), None, None),
        };

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
            category,
            pattern: pattern_name,
        })
    }
}

/// Compress a command's output (free-function form, used by tests + simple callers).
pub fn compress_output(command: &str, output: &str) -> Compressed {
    let pattern = find_match(command);
    match pattern {
        Some(p) => Compressed {
            command: command.to_string(),
            original_hash: String::new(),
            original_lines: output.lines().count(),
            compressed_lines: 0, // filled below
            saved_ratio: 0.0,
            output: registry::apply(p, output),
            category: Some(p.category.id()),
            pattern: Some(p.name),
        }
        .with_line_counts(),
        None => Compressed {
            command: command.to_string(),
            original_hash: String::new(),
            original_lines: output.lines().count(),
            compressed_lines: 0,
            saved_ratio: 0.0,
            output: generic_compress(output),
            category: None,
            pattern: None,
        }
        .with_line_counts(),
    }
}

/// Generic pipeline: dedup consecutive + head/tail truncate, no filter.
fn generic_compress(output: &str) -> String {
    let lines: Vec<String> = output.lines().map(|s| s.to_string()).collect();
    let deduped = pipeline::dedup_consecutive(&lines);
    pipeline::truncate_head_tail(&deduped, 200, 60, 40).join("\n")
}

impl Compressed {
    /// Recompute compressed_lines + saved_ratio from `output`. Used by the free function
    /// to avoid duplicating logic.
    fn with_line_counts(mut self) -> Self {
        self.compressed_lines = self.output.lines().count();
        self.saved_ratio = if self.original_lines == 0 {
            0.0
        } else {
            (1.0 - self.compressed_lines as f32 / self.original_lines as f32).clamp(0.0, 1.0)
        };
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    // -- pipeline helpers (5) -------------------------------------------------------

    #[test]
    fn filter_keep_no_match_returns_empty() {
        let lines = ["hello world", "foo bar"];
        assert!(pipeline::filter_keep(&lines, &["error"]).is_empty());
    }

    #[test]
    fn filter_drop_no_prefix_match_returns_all() {
        let lines = ["On branch main", "Changes not staged:"];
        let out = pipeline::filter_drop(&lines, &["  (use "]);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn dedup_collapses_consecutive_repeats() {
        let lines: Vec<String> = ["same", "same", "same", "diff"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(
            pipeline::dedup_consecutive(&lines),
            vec!["same  (x3)".to_string(), "diff".to_string()]
        );
    }

    #[test]
    fn truncate_keeps_head_and_tail() {
        let lines: Vec<String> = (0..500).map(|i| format!("line {i}")).collect();
        let t = pipeline::truncate_head_tail(&lines, 200, 60, 40);
        assert_eq!(t.len(), 60 + 1 + 40);
        assert_eq!(t[0], "line 0");
        assert_eq!(t[t.len() - 1], "line 499");
        assert!(t[60].contains("omitted"));
    }

    #[test]
    fn truncate_short_input_unchanged() {
        let lines: Vec<String> = (0..5).map(|i| format!("line {i}")).collect();
        let t = pipeline::truncate_head_tail(&lines, 200, 60, 40);
        assert_eq!(t, lines);
    }

    // -- registry dispatch (3) ------------------------------------------------------

    #[test]
    fn find_match_returns_first_pattern() {
        let p = find_match("cargo test --workspace").unwrap();
        assert_eq!(p.name, "cargo-test");
    }

    #[test]
    fn find_match_unknown_returns_none() {
        assert!(find_match("totally-unknown-tool xyz").is_none());
    }

    #[test]
    fn word_boundary_prevents_partial_match() {
        // "categorical" must not match "cat"
        assert!(find_match("categorical --help").is_none());
        // but real cat does
        let p = find_match("cat foo.txt").unwrap();
        assert_eq!(p.name, "cat");
    }

    // -- existing compress_output tests (preserved from the old lib.rs) -------------

    #[test]
    fn cargo_test_keeps_failures_drops_passes() {
        let mut out = String::new();
        for i in 0..50 {
            out.push_str(&format!("test mod::case_{i} ... ok\n"));
        }
        out.push_str("test mod::broken ... FAILED\n");
        out.push_str("test result: FAILED. 50 passed; 1 failed; 0 ignored\n");
        let c = compress_output("cargo test --workspace", &out);
        assert!(c.output.contains("FAILED"));
        assert!(c.output.contains("test result"));
        assert!(
            !c.output.contains("case_0 ... ok"),
            "per-test passes should be dropped"
        );
    }

    #[test]
    fn cargo_build_keeps_warnings_and_finished() {
        let out = "   Compiling foo v1.0.0\n\
                   warning: unused variable `x`\n\
                   warning: unused import\n\
                   error[E0382]: borrow of moved value\n\
                   Finished `dev` profile [unoptimized] target(s) in 2.35s\n";
        let c = compress_output("cargo build", out);
        assert!(c.output.contains("warning:"));
        assert!(c.output.contains("error"));
        assert!(c.output.contains("Finished"));
        // Compiling v1.0.0 is kept (one line per package being built is useful context).
        assert!(c.output.contains("Compiling foo"));
    }

    #[test]
    fn cargo_clippy_treated_as_cargo_build() {
        let out = "   Checking foo v1.0.0\nwarning: clippy::pedantic: ...\nFinished\n";
        let c = compress_output("cargo clippy --workspace", out);
        assert!(c.output.contains("warning:"));
        assert!(c.output.contains("Finished"));
    }

    #[test]
    fn git_status_drops_use_hints() {
        let out = "On branch main\nChanges not staged for commit:\n  (use \"git add\" to stage)\n  (use \"git restore\" to discard)\n\tmodified: src/lib.rs\n";
        let c = compress_output("git status", out);
        assert!(c.output.contains("src/lib.rs"));
        assert!(!c.output.contains("use \"git add\""));
    }

    #[test]
    fn generic_command_passes_all_lines_through() {
        let out = "line one\nline two\nline three\n";
        let c = compress_output("ls -la", out);
        assert!(c.output.contains("line one"));
        assert!(c.output.contains("line two"));
        assert!(c.output.contains("line three"));
    }

    #[test]
    fn huge_input_gets_head_tail_truncated() {
        let mut out = String::new();
        for i in 0..5000 {
            out.push_str(&format!("line {i}\n"));
        }
        let c = compress_output("echo x", &out);
        assert!(c.output.contains("omitted"));
    }

    #[test]
    fn empty_input_handled_gracefully() {
        let c = compress_output("cargo test", "");
        assert_eq!(c.original_lines, 0);
        assert_eq!(c.compressed_lines, 0);
        assert_eq!(c.saved_ratio, 0.0);
    }

    // -- new compress_output tests (P4.4) -------------------------------------------

    #[test]
    fn response_includes_category_and_pattern_name() {
        let c = compress_output("git status", "ok\n");
        assert_eq!(c.category, Some("vcs"));
        assert_eq!(c.pattern, Some("git-status"));
    }

    #[test]
    fn unknown_command_has_no_category() {
        let c = compress_output("totally-unknown xyz", "ok\nok\n");
        assert_eq!(c.category, None);
        assert_eq!(c.pattern, None);
    }

    #[test]
    fn git_diff_stat_passes_through() {
        let raw = " file1.rs | 10 +++++++---\n 2 files changed, 20 insertions(+), 5 deletions(-)\n";
        let c = compress_output("git diff --stat", raw);
        assert!(c.output.contains("2 files changed"));
        assert_eq!(c.pattern, Some("git-diff-stat"));
    }

    #[test]
    fn make_filter_keeps_errors() {
        let raw = "make[1]: Entering directory\ncc -c foo.c\nfoo.c:42: error: undefined\nmake[1]: *** [foo.o] Error 1\n";
        let c = compress_output("make foo", raw);
        assert!(c.output.contains("Error"));
        assert!(c.output.contains("error:"));
        assert_eq!(c.pattern, Some("make"));
    }

    #[test]
    fn npm_install_keeps_added_deprecated() {
        let raw = "npm warn deprecated foo@1.0.0: use bar instead\n\
                   added 47 packages from 23 contributors\n\
                   removed 3 packages in 0.5s\n\
                   npm notice\n";
        let c = compress_output("npm install", raw);
        assert!(c.output.contains("added 47"));
        assert!(c.output.contains("removed 3"));
        assert!(c.output.contains("deprecated"));
        assert!(!c.output.contains("npm notice"));
    }

    #[test]
    fn pnpm_install_keeps_progress_and_warnings() {
        let raw = "Progress: resolved 0, reused 0, downloaded 1\n\
                   Packages: +1\n\
                   + bar@1.0.0\n\
                   WARN  deprecated foo@1.0.0\n";
        let c = compress_output("pnpm install", raw);
        assert!(c.output.contains("Progress:"));
        assert!(c.output.contains("WARN"));
    }

    #[test]
    fn pip_install_keeps_only_summary_lines() {
        let raw = "Collecting foo\n  Downloading foo-1.0.0.tar.gz (10 kB)\n\
                   Building wheel for foo\n\
                   Successfully installed foo-1.0.0\n\
                   Collecting bar\n  Downloading bar-2.0.0.tar.gz (20 kB)\n\
                   Successfully installed bar-2.0.0\n";
        let c = compress_output("pip install -r requirements.txt", raw);
        assert!(c.output.contains("Successfully installed"));
        assert!(!c.output.contains("Collecting"));
        assert!(!c.output.contains("Downloading"));
    }

    #[test]
    fn eslint_keeps_error_count_and_warnings() {
        let raw =
            "/path/to/file.ts\n  12:5  error  no-unused-vars  'x' is defined but never used\n\
                   /path/to/other.ts\n  5:3  warning  prefer-const\n\
                   ✖ 2 problems (1 error, 1 warning)\n";
        let c = compress_output("eslint src/", raw);
        assert!(c.output.contains("2 problems"));
        assert!(c.output.contains("error"));
    }

    #[test]
    fn tsc_keeps_type_errors() {
        let raw = "src/foo.ts:5:12 - error TS2304: Cannot find name 'x'\n\
                   src/bar.ts:10:3 - error TS2322: Type 'string' is not assignable to type 'number'\n\
                   Found 2 errors in 2 files.\n";
        let c = compress_output("tsc --noEmit", raw);
        assert!(c.output.contains("TS2304"));
        assert!(c.output.contains("Found 2 errors"));
    }

    #[test]
    fn docker_ps_keeps_container_rows() {
        let raw = "CONTAINER ID   IMAGE          COMMAND                  CREATED         STATUS          PORTS      NAMES\n\
                   abc123def456   nginx:latest   \"/docker-entrypoint.…\"   2 minutes ago   Up 2 minutes    80/tcp     web\n\
                   def456abc789   redis:7        \"docker-entrypoint.s…\"   5 minutes ago   Up 5 minutes    6379/tcp   cache\n";
        let c = compress_output("docker ps", raw);
        assert!(c.output.contains("CONTAINER ID"));
        assert!(c.output.contains("nginx"));
    }

    #[test]
    fn kubectl_get_keeps_pod_status() {
        let raw = "NAME                    READY   STATUS    RESTARTS   AGE\n\
                   pod-foo-abc123          1/1     Running   0          5m\n\
                   pod-bar-def456          0/1     Error     3          2m\n";
        let c = compress_output("kubectl get pods", raw);
        assert!(c.output.contains("NAME"));
        assert!(c.output.contains("Error"));
    }

    #[test]
    fn curl_keeps_status_and_headers() {
        let raw = "* Trying 127.0.0.1:8080...\n* Connected to localhost\n\
                   < HTTP/1.1 200 OK\n\
                   < Content-Type: text/html; charset=utf-8\n\
                   < Content-Length: 1234\n\
                   <html><body>lots of body content that we don't need here</body></html>\n";
        let c = compress_output("curl -i http://localhost:8080/", raw);
        assert!(c.output.contains("HTTP/1.1 200"));
        assert!(c.output.contains("Content-Type:"));
        assert!(!c.output.contains("<html>"));
    }

    #[test]
    fn rg_passes_through() {
        let raw = "src/foo.rs:10:fn hello()\nsrc/bar.rs:20:fn bye()\n";
        let c = compress_output("rg \"fn\" src/", raw);
        assert!(c.output.contains("src/foo.rs:10"));
        assert!(c.output.contains("fn hello()"));
    }

    #[test]
    fn cat_passes_through_short_content() {
        let raw = "line 1\nline 2\nline 3\n";
        let c = compress_output("cat foo.txt", raw);
        assert!(c.output.contains("line 1"));
        assert!(c.output.contains("line 3"));
    }

    #[test]
    fn tree_passes_through() {
        let raw = ".\n├── Cargo.toml\n├── src\n│   ├── lib.rs\n│   └── main.rs\n└── tests\n";
        let c = compress_output("tree .", raw);
        assert!(c.output.contains("Cargo.toml"));
        assert!(c.output.contains("lib.rs"));
    }

    #[test]
    fn ls_passes_through() {
        let raw = "Cargo.toml\nsrc\ntests\n";
        let c = compress_output("ls -la", raw);
        assert!(c.output.contains("Cargo.toml"));
    }

    // -- ShellCompressor integration (existing tests preserved) ---------------------

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
    fn multiple_compressions_get_distinct_hashes() {
        let Some(store) = Store::open_for_test() else {
            return;
        };
        let store = Arc::new(store);
        let sc = ShellCompressor::new(store);
        let r1 = sc.compress("cargo test", "hello\n").unwrap();
        let r2 = sc.compress("cargo test", "world\n").unwrap();
        assert_ne!(r1.original_hash, r2.original_hash);
    }

    #[test]
    fn compressor_includes_category_in_response() {
        let Some(store) = Store::open_for_test() else {
            return;
        };
        let sc = ShellCompressor::new(Arc::new(store));
        let c = sc.compress("git status", "ok\n").unwrap();
        assert_eq!(c.category, Some("vcs"));
        assert_eq!(c.pattern, Some("git-status"));
    }
}
