//! `cairn bench` — measure the token savings Cairn delivers on a real codebase.
//!
//! Three of the engine's mechanisms turned into hard numbers: AST signature outlines (reading code
//! as structure), the re-read killer (cached unchanged reads), and shell-output compression. Every
//! one is lossless — the full original is always one `expand` away.

use anyhow::Result;
use cairn_api::AppState;
use cairn_context::{ContextEngine, ReadMode, ReadStatus};
use std::path::{Path, PathBuf};

/// Extensions Cairn can outline (matches the `cairn-context` language set).
const CODE_EXTS: &[&str] = &[
    "rs", "py", "pyi", "js", "mjs", "cjs", "jsx", "ts", "mts", "cts", "tsx", "go", "c", "cpp",
    "cc", "cxx", "c++", "h", "hpp", "hh", "hxx", "java", "cs", "rb", "sh", "bash",
];
/// Directories we never descend into.
const SKIP_DIRS: &[&str] = &[
    "target",
    "node_modules",
    "dist",
    "build",
    "out",
    ".next",
    "vendor",
];

/// Result of the AST-outline benchmark.
#[derive(Debug, Default, Clone, Copy)]
pub struct OutlineBench {
    pub files: usize,
    pub full_tokens: usize,
    pub lean_tokens: usize,
}

impl OutlineBench {
    pub fn saved_pct(&self) -> f64 {
        if self.full_tokens == 0 {
            0.0
        } else {
            100.0 * (1.0 - self.lean_tokens as f64 / self.full_tokens as f64)
        }
    }
}

pub fn run(state: &AppState, root: &Path) -> Result<()> {
    println!("Cairn bench — measured on {}\n", root.display());

    let files = collect_code_files(root, 800);

    // 1) AST outlines — read each file full vs. signatures-only.
    let ctx = ContextEngine::new(state.store.clone());
    let o = bench_outline(&ctx, &files)?;
    println!("AST outlines (lean code reading)");
    if o.files == 0 {
        println!("  no supported code files found here\n");
    } else {
        println!("  {} code files", o.files);
        println!("  full : ~{} tokens", thousands(o.full_tokens));
        println!("  lean : ~{} tokens", thousands(o.lean_tokens));
        println!("  saved: {:.1}%\n", o.saved_pct());
    }

    // 2) Re-read killer — read the biggest file twice through a fresh cache.
    if let Some(big) = files
        .iter()
        .max_by_key(|f| std::fs::metadata(f).map(|m| m.len()).unwrap_or(0))
    {
        let fresh = ContextEngine::new(state.store.clone());
        let first = fresh.read(big, ReadMode::Full)?;
        let again = fresh.read(big, ReadMode::Auto)?;
        let saved = pct_saved(first.est_tokens, again.est_tokens);
        let name = big.file_name().and_then(|n| n.to_str()).unwrap_or("");
        println!("Re-reading an unchanged file ({name})");
        println!(
            "  first read ~{} tokens → cached re-read ~{} tokens ({saved:.1}% saved)\n",
            thousands(first.est_tokens),
            thousands(again.est_tokens),
        );
    }

    // 3) Shell-output compression — on a representative verbose log.
    let sample = sample_shell_output();
    let c = state.shell.compress("cargo test", &sample)?;
    println!("Shell-output compression (sample verbose log)");
    println!(
        "  {} lines → {} lines ({:.1}% saved)\n",
        c.original_lines,
        c.compressed_lines,
        c.saved_ratio * 100.0,
    );

    println!(
        "Every compression here is lossless — the full original is retained and one `expand` away."
    );
    Ok(())
}

/// Read each file full and as a signature outline, summing token costs for files we can outline.
pub fn bench_outline(ctx: &ContextEngine, files: &[PathBuf]) -> Result<OutlineBench> {
    let mut b = OutlineBench::default();
    for f in files {
        let full = ctx.read(f, ReadMode::Full)?;
        let lean = ctx.read(f, ReadMode::Signatures)?;
        // Only count files the outliner actually applied to (others would net zero).
        if lean.status == ReadStatus::Outline {
            b.files += 1;
            b.full_tokens += full.est_tokens;
            b.lean_tokens += lean.est_tokens;
        }
    }
    Ok(b)
}

/// Recursively collect code files under `root`, skipping build/vendor and hidden directories.
pub fn collect_code_files(root: &Path, cap: usize) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if out.len() >= cap {
            break;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
                    if !name.starts_with('.') && !SKIP_DIRS.contains(&name) {
                        stack.push(p);
                    }
                }
            } else if p
                .extension()
                .and_then(|x| x.to_str())
                .is_some_and(|x| CODE_EXTS.contains(&x))
            {
                out.push(p);
                if out.len() >= cap {
                    break;
                }
            }
        }
    }
    out
}

fn pct_saved(before: usize, after: usize) -> f64 {
    if before == 0 {
        0.0
    } else {
        100.0 * (1.0 - after as f64 / before as f64)
    }
}

/// Group a number with thousands separators (e.g. `124300` → `124,300`).
fn thousands(n: usize) -> String {
    let s = n.to_string();
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 && (bytes.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(*b as char);
    }
    out
}

/// A representative verbose test log — mostly noise a human never needs to see.
fn sample_shell_output() -> String {
    let mut s = String::from("   Compiling cairn-core v0.1.0\n   Compiling cairn-store v0.1.0\n");
    for i in 0..150 {
        s.push_str(&format!("test suite::case_{i} ... ok\n"));
    }
    s.push_str("test result: ok. 150 passed; 0 failed; 0 ignored\n");
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_store::Store;
    use std::sync::Arc;

    #[test]
    fn outline_bench_reports_real_savings_and_skips_build_dirs() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("a.rs"),
            "pub fn foo(x: u32) -> u32 {\n    let y = x + 1;\n    y * 2\n}\n\
             pub struct S {\n    a: u32,\n    b: u32,\n}\n\
             impl S {\n    pub fn sum(&self) -> u32 {\n        self.a + self.b\n    }\n}\n",
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join("target")).unwrap();
        std::fs::write(dir.path().join("target/ignored.rs"), "fn skip() {}\n").unwrap();

        // Collect before opening the store so the data dir can't interfere.
        let files = collect_code_files(dir.path(), 100);
        assert_eq!(files.len(), 1, "target/ must be skipped");

        let Some(store) = Store::open_for_test() else {
            return;
        };
        let ctx = ContextEngine::new(Arc::new(store));
        let b = bench_outline(&ctx, &files).unwrap();
        assert_eq!(b.files, 1);
        assert!(b.lean_tokens < b.full_tokens, "outline must be cheaper");
        assert!(b.saved_pct() > 0.0);
    }

    #[test]
    fn thousands_groups_digits() {
        assert_eq!(thousands(124300), "124,300");
        assert_eq!(thousands(42), "42");
        assert_eq!(thousands(1000), "1,000");
    }
}
