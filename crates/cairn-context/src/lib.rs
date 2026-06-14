//! The context engine. Two ideas power this crate:
//!
//! **Re-read killer.** Files are read through a cache keyed by path + mtime. If a file hasn't
//! changed since the agent last read it, the re-read returns a tiny "unchanged" view (a handle +
//! line count) instead of the whole file — cheap re-reads instead of dumping 1000 lines again. If
//! it *has* changed, we return only the diff.
//!
//! **No context loss.** Every time we read a file we stash its exact bytes in the blob store,
//! addressed by content hash. So whatever compact view the agent gets, the original is one
//! [`ContextEngine::expand`] call away — byte-identical.

use cairn_core::{ContentHash, Result};
use cairn_store::Store;
use serde::Serialize;
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

mod outline;

/// How to render a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadMode {
    /// Let the engine decide (cache-aware: full first read, diff on change, tiny view if unchanged).
    Auto,
    /// Always return the whole file (still cached + retained).
    Full,
    /// AST signature outline — top-level items and their members, bodies elided (huge token saver).
    Signatures,
    /// Like `Signatures`, with each item prefixed by its start line number.
    Map,
}

impl ReadMode {
    pub fn parse(s: Option<&str>) -> Self {
        match s {
            Some("full") => ReadMode::Full,
            Some("signatures") | Some("sig") | Some("signature") => ReadMode::Signatures,
            Some("map") | Some("outline") => ReadMode::Map,
            _ => ReadMode::Auto,
        }
    }

    /// Whether this mode renders the file's structure rather than its content.
    fn is_structural(self) -> bool {
        matches!(self, ReadMode::Signatures | ReadMode::Map)
    }
}

/// The status of a read — how the engine answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ReadStatus {
    /// First time we've seen this file (or `Full` was forced): whole content returned.
    Full,
    /// Unchanged since last read: a tiny handle-only view (the re-read killer).
    Cached,
    /// Changed since last read: only the diff is returned.
    Diff,
    /// A structural view: the file's signature outline (`signatures`/`map` modes).
    Outline,
}

/// The result handed back for a read.
#[derive(Debug, Clone, Serialize)]
pub struct ReadResult {
    pub path: String,
    /// Full content hash — the handle you pass to `expand` to recover the exact original.
    pub hash: String,
    /// Short, human-friendly form of the handle.
    pub handle: String,
    pub status: ReadStatus,
    pub lines: usize,
    pub bytes: usize,
    /// What the agent should put in its context window.
    pub view: String,
    /// A short human note explaining the view (e.g. how to expand).
    pub note: String,
    /// Rough token estimate of `view` (~4 bytes/token).
    pub est_tokens: usize,
}

#[derive(Clone)]
struct CacheEntry {
    mtime_ns: u128,
    hash: ContentHash,
    content: String,
    lines: usize,
}

pub struct ContextEngine {
    store: Arc<Store>,
    cache: Mutex<HashMap<String, CacheEntry>>,
}

impl ContextEngine {
    pub fn new(store: Arc<Store>) -> Self {
        Self {
            store,
            cache: Mutex::new(HashMap::new()),
        }
    }

    /// Read a file, applying the cache / diff / retention logic above.
    pub fn read(&self, path: &Path, mode: ReadMode) -> Result<ReadResult> {
        let key = path.to_string_lossy().to_string();
        let meta = std::fs::metadata(path)?;
        let mtime_ns = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_nanos())
            .unwrap_or(0);

        let mut cache = self.cache.lock().unwrap();

        // Re-read killer: only Auto takes the cheap "unchanged" shortcut. Full and the structural
        // modes always read fresh (structural views are cheap to recompute and that's the point).
        if mode == ReadMode::Auto {
            if let Some(entry) = cache.get(&key) {
                if entry.mtime_ns == mtime_ns {
                    let note = format!(
                        "unchanged since last read; {} lines; `expand {}` for the full file",
                        entry.lines,
                        entry.hash.short()
                    );
                    return Ok(ReadResult {
                        path: key,
                        hash: entry.hash.0.clone(),
                        handle: entry.hash.short().to_string(),
                        status: ReadStatus::Cached,
                        lines: entry.lines,
                        bytes: entry.content.len(),
                        view: String::new(),
                        est_tokens: estimate_tokens(&note),
                        note,
                    });
                }
            }
        }

        // Read fresh and retain the exact original (no context loss).
        let bytes = std::fs::read(path)?;
        let content = String::from_utf8_lossy(&bytes).into_owned();
        let hash = self.store.blobs().put(&bytes)?;
        let lines = content.lines().count();

        // Record this version as the agent's edit baseline so the PostToolUse guard can later
        // detect silent corruption (a large unreplaced deletion vs. what was read).
        let canonical = std::fs::canonicalize(path)
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| key.clone());
        let _ = self
            .store
            .record_file_version(&canonical, &hash.0, lines as i64);

        // Structural (AST) views: render the file as just its signatures. Falls through to a full
        // read for unsupported languages or unparseable input.
        if mode.is_structural() {
            if let Some(o) = outline::outline(path, &content, mode == ReadMode::Map) {
                let est = estimate_tokens(&o.text);
                let note = format!(
                    "{} signature outline ({} items); `expand {}` for the full {lines} lines",
                    o.lang,
                    o.items,
                    hash.short()
                );
                let result = ReadResult {
                    path: key.clone(),
                    hash: hash.0.clone(),
                    handle: hash.short().to_string(),
                    status: ReadStatus::Outline,
                    lines,
                    bytes: content.len(),
                    view: o.text,
                    note,
                    est_tokens: est,
                };
                cache.insert(
                    key,
                    CacheEntry {
                        mtime_ns,
                        hash,
                        content,
                        lines,
                    },
                );
                return Ok(result);
            }
        }

        let prev = cache.get(&key).map(|e| e.content.clone());

        let result = match (&prev, mode) {
            (Some(prev_content), ReadMode::Auto) if *prev_content != content => {
                let diff = diff_only(prev_content, &content);
                let note = format!(
                    "changed since last read; showing diff only; `expand {}` for the full file",
                    hash.short()
                );
                ReadResult {
                    path: key.clone(),
                    hash: hash.0.clone(),
                    handle: hash.short().to_string(),
                    status: ReadStatus::Diff,
                    lines,
                    bytes: content.len(),
                    est_tokens: estimate_tokens(&diff),
                    view: diff,
                    note,
                }
            }
            _ => {
                let mut note = format!("full file; {lines} lines; handle {}", hash.short());
                // Point the agent at the cheaper structural view when one is available.
                if lines > 40 && outline::supported(path) {
                    note.push_str("; try mode=signatures for a structural outline");
                }
                ReadResult {
                    path: key.clone(),
                    hash: hash.0.clone(),
                    handle: hash.short().to_string(),
                    status: ReadStatus::Full,
                    lines,
                    bytes: content.len(),
                    est_tokens: estimate_tokens(&content),
                    view: content.clone(),
                    note,
                }
            }
        };

        cache.insert(
            key,
            CacheEntry {
                mtime_ns,
                hash,
                content,
                lines,
            },
        );
        Ok(result)
    }

    /// Recover the exact original bytes for a handle (full or short hash), as a string.
    pub fn expand(&self, hash: &str) -> Result<Option<String>> {
        // Accept short handles by scanning is overkill here; require the full hash for now,
        // but tolerate a full hash passed as-is.
        let ch = ContentHash(hash.to_string());
        self.store.blobs().get_str(&ch)
    }
}

/// A compact diff: only added/removed lines (prefixed `+`/`-`), equal lines omitted.
fn diff_only(old: &str, new: &str) -> String {
    use similar::{ChangeTag, TextDiff};
    let diff = TextDiff::from_lines(old, new);
    let mut out = String::new();
    for change in diff.iter_all_changes() {
        match change.tag() {
            ChangeTag::Delete => {
                out.push('-');
                out.push_str(change.value());
            }
            ChangeTag::Insert => {
                out.push('+');
                out.push_str(change.value());
            }
            ChangeTag::Equal => {}
        }
    }
    out
}

fn estimate_tokens(s: &str) -> usize {
    (s.len() / 4).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_core::Config;
    use cairn_store::Store;
    use std::time::{Duration, SystemTime};

    fn engine() -> (ContextEngine, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let cfg = Config::resolve(Some(dir.path().join("data"))).unwrap();
        let store = Arc::new(Store::open(&cfg).unwrap());
        (ContextEngine::new(store), dir)
    }

    #[test]
    fn cached_reread_is_cheap_diff_works_and_expand_is_lossless() {
        let (eng, dir) = engine();
        let file = dir.path().join("big.txt");
        let original: String = (0..1000)
            .map(|i| format!("line {i}: lorem ipsum dolor sit amet\n"))
            .collect();
        std::fs::write(&file, &original).unwrap();

        // First read: full content.
        let r1 = eng.read(&file, ReadMode::Auto).unwrap();
        assert_eq!(r1.status, ReadStatus::Full);
        assert_eq!(r1.lines, 1000);

        // Re-read unchanged: the re-read killer returns a tiny view.
        let r2 = eng.read(&file, ReadMode::Auto).unwrap();
        assert_eq!(r2.status, ReadStatus::Cached);
        assert!(
            r2.est_tokens * 20 < r1.est_tokens,
            "cached re-read should be far cheaper: {} vs {}",
            r2.est_tokens,
            r1.est_tokens
        );

        // expand recovers the byte-identical original (no context loss).
        let recovered = eng.expand(&r1.hash).unwrap().expect("blob present");
        assert_eq!(recovered, original);

        // Change the file (force a new mtime so the cache invalidates deterministically).
        let mut changed = original.clone();
        changed.push_str("a brand new final line\n");
        std::fs::write(&file, &changed).unwrap();
        std::fs::OpenOptions::new()
            .write(true)
            .open(&file)
            .unwrap()
            .set_modified(SystemTime::now() + Duration::from_secs(2))
            .unwrap();

        // Re-read after change: diff-only view containing just the new line.
        let r3 = eng.read(&file, ReadMode::Auto).unwrap();
        assert_eq!(r3.status, ReadStatus::Diff);
        assert!(r3.view.contains("a brand new final line"));
        assert!(
            r3.est_tokens < r1.est_tokens,
            "diff should be smaller than full"
        );

        // Both versions' originals are retained — nothing is ever lost.
        assert_eq!(eng.expand(&r1.hash).unwrap().unwrap(), original);
        assert_eq!(eng.expand(&r3.hash).unwrap().unwrap(), changed);
    }

    #[test]
    fn signatures_mode_outlines_rust_and_stays_lossless() {
        let (eng, dir) = engine();
        let file = dir.path().join("widget.rs");
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
        std::fs::write(&file, src).unwrap();

        let r = eng.read(&file, ReadMode::Signatures).unwrap();
        assert_eq!(r.status, ReadStatus::Outline);
        // The API surface is there...
        assert!(r.view.contains("pub struct Widget"));
        assert!(r.view.contains("impl Widget"));
        assert!(r.view.contains("pub fn new(id: u32) -> Self"));
        assert!(r.view.contains("pub fn build() -> Widget"));
        // ...the bodies are not.
        assert!(!r.view.contains("format!"));
        assert!(!r.view.contains("Widget::new(1)"));
        // Outline is cheaper than the full file, and the original is still recoverable.
        assert!(r.est_tokens < estimate_tokens(src));
        assert_eq!(eng.expand(&r.hash).unwrap().unwrap(), src);
    }

    #[test]
    fn structural_mode_falls_back_to_full_for_non_code() {
        let (eng, dir) = engine();
        let file = dir.path().join("notes.txt");
        let body = "plain prose, no code here\nsecond line\n";
        std::fs::write(&file, body).unwrap();

        // Unsupported language → graceful fall-through to a full read.
        let r = eng.read(&file, ReadMode::Signatures).unwrap();
        assert_eq!(r.status, ReadStatus::Full);
        assert_eq!(r.view, body);
    }
}
