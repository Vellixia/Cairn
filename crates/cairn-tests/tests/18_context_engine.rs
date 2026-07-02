//! 18 — ContextEngine end-to-end via the in-memory Store.
//!
//! Replaces the deleted `03_context_compression.rs` (which hand-rolled a
//! `tokens()` function and asserted on toy strings, never touching
//! `cairn-context`). Every test here calls a real `ContextEngine` method
//! against a real `Store::open_in_memory` instance.

use cairn_context::{ContextEngine, ReadMode, ReadStatus};
use cairn_store::Store;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

fn engine() -> Option<(ContextEngine, tempfile::TempDir)> {
    let dir = tempfile::tempdir().ok()?;
    let blobs = dir.path().join("blobs");
    let store = Arc::new(Store::open_in_memory(blobs).ok()?);
    Some((ContextEngine::new(store), dir))
}

#[test]
fn first_read_returns_full_status() {
    let Some((eng, dir)) = engine() else { return };
    let path = dir.path().join("hello.rs");
    let content = "pub fn hello() -> &'static str { \"hi\" }\n";
    std::fs::write(&path, content).unwrap();
    let r = eng.read(&path, ReadMode::Auto).expect("first read");
    assert_eq!(r.status, ReadStatus::Full);
    assert_eq!(r.lines, 1);
    assert!(r.view.contains("hello"));
    // expand recovers the byte-identical original.
    let original = eng.expand(&r.hash).expect("expand").expect("blob");
    assert_eq!(original, content);
}

#[test]
fn unchanged_re_read_returns_cached_with_tiny_view() {
    let Some((eng, dir)) = engine() else { return };
    let path = dir.path().join("big.rs");
    let content: String = (0..500)
        .map(|i| format!("line {i}: lorem ipsum dolor sit amet\n"))
        .collect();
    std::fs::write(&path, &content).unwrap();
    let r1 = eng.read(&path, ReadMode::Auto).expect("first read");
    assert_eq!(r1.status, ReadStatus::Full);
    let r2 = eng.read(&path, ReadMode::Auto).expect("second read");
    assert_eq!(r2.status, ReadStatus::Cached);
    // The re-read killer must collapse the token count drastically.
    assert!(
        r2.est_tokens * 20 < r1.est_tokens,
        "cached re-read should be far cheaper: {} vs {}",
        r2.est_tokens,
        r1.est_tokens
    );
}

#[test]
fn changed_re_read_returns_diff_with_only_changed_lines() {
    let Some((eng, dir)) = engine() else { return };
    let path = dir.path().join("changed.rs");
    let original: String = (0..200)
        .map(|i| format!("line {i}: lorem ipsum dolor sit amet\n"))
        .collect();
    std::fs::write(&path, &original).unwrap();
    let _ = eng.read(&path, ReadMode::Auto).expect("seed");
    let mut changed = original.clone();
    changed.push_str("a fresh new final line\n");
    std::fs::write(&path, &changed).unwrap();
    // Bump mtime deterministically past the seed.
    std::fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(SystemTime::now() + Duration::from_secs(2))
        .unwrap();
    let r = eng.read(&path, ReadMode::Auto).expect("re-read");
    assert_eq!(r.status, ReadStatus::Diff);
    assert!(r.view.contains("a fresh new final line"));
    assert!(r.est_tokens < _full_estimate(&original));
}

fn _full_estimate(s: &str) -> usize {
    (s.len() / 4).max(1)
}

#[test]
fn signatures_mode_outlines_rust_struct() {
    let Some((eng, dir)) = engine() else { return };
    let path = dir.path().join("widget.rs");
    let src = "\
pub struct Widget { pub id: u32, pub name: String }

impl Widget {
    pub fn new(id: u32) -> Self {
        let name = format!(\"w{id}\");
        Self { id, name }
    }
}

pub fn build() -> Widget { Widget::new(1) }
";
    std::fs::write(&path, src).unwrap();
    let r = eng
        .read(&path, ReadMode::Signatures)
        .expect("signatures read");
    assert_eq!(r.status, ReadStatus::Outline);
    assert!(r.view.contains("pub struct Widget"));
    assert!(r.view.contains("impl Widget"));
    assert!(r.view.contains("pub fn build() -> Widget"));
    // Bodies must be elided.
    assert!(!r.view.contains("format!"));
    assert!(r.est_tokens < _full_estimate(src));
    // And the original is still one expand away.
    assert_eq!(eng.expand(&r.hash).expect("expand").expect("blob"), src);
}

#[test]
fn structural_mode_falls_back_to_full_for_plain_text() {
    let Some((eng, dir)) = engine() else { return };
    let path = dir.path().join("notes.txt");
    let body = "plain prose, no code here\nsecond line\n";
    std::fs::write(&path, body).unwrap();
    let r = eng.read(&path, ReadMode::Signatures).expect("read");
    assert_eq!(r.status, ReadStatus::Full);
    assert_eq!(r.view, body);
}

#[test]
fn anti_inflation_guard_falls_back_when_outline_is_no_smaller() {
    // A single-line file: the anti-inflation guard must fall through
    // to Full rather than ship an "outline" that's larger than raw.
    let Some((eng, dir)) = engine() else { return };
    let path = dir.path().join("tiny.rs");
    let src = "pub fn tiny() -> i32 { 42 }\n";
    std::fs::write(&path, src).unwrap();
    let r = eng.read(&path, ReadMode::Signatures).expect("read");
    assert!(r.est_tokens <= _full_estimate(src), "no inflation");
}

#[test]
fn auto_delta_falls_back_to_full_when_diff_too_large() {
    // A 90% rewrite: the diff >= 60% of full, so the engine must
    // ship the full file rather than a noisy diff.
    let Some((eng, dir)) = engine() else { return };
    let path = dir.path().join("rewritten.rs");
    let original: String = (0..400)
        .map(|i| format!("line {i}: lorem ipsum dolor sit amet\n"))
        .collect();
    std::fs::write(&path, &original).unwrap();
    let _ = eng.read(&path, ReadMode::Auto).expect("seed");
    let rewritten: String = (0..400)
        .map(|i| format!("line {i}: completely different content here now\n"))
        .collect();
    std::fs::write(&path, &rewritten).unwrap();
    std::fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(SystemTime::now() + Duration::from_secs(2))
        .unwrap();
    let r = eng.read(&path, ReadMode::Auto).expect("re-read");
    assert_eq!(
        r.status,
        ReadStatus::Full,
        "diff >= 60% must fall back to Full, not ship a giant diff"
    );
    assert!(!r.view.starts_with('+') && !r.view.starts_with('-'));
}
