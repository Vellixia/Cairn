//! The context engine. Two ideas power this crate:
//!
//! **Re-read killer.** Files are read through a cache keyed by path + mtime. If a file hasn't
//! changed since the agent last read it, the re-read returns a tiny "unchanged" view (a handle +
//! line count) instead of the whole file - cheap re-reads instead of dumping 1000 lines again. If
//! it *has* changed, we return only the diff.
//!
//! **No context loss.** Every time we read a file we stash its exact bytes in the blob store,
//! addressed by content hash. So whatever compact view the agent gets, the original is one
//! [`ContextEngine::expand`] call away - byte-identical.

use cairn_core::{ContentHash, Error, Result};
use cairn_store::Store;
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};

pub mod bounce_tracker;
pub mod context_ledger;
mod outline;
pub use bounce_tracker::{BounceStats, BounceTracker};
pub use context_ledger::{
    ContextLedger, ContextPressure, LedgerEntry, PressureAction, DEFAULT_WINDOW_SIZE,
};

/// How to render a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadMode {
    /// Let the engine decide (cache-aware: full first read, diff on change, tiny view if unchanged).
    Auto,
    /// Always return the whole file (still cached + retained).
    Full,
    /// AST signature outline - top-level items and their members, bodies elided (huge token saver).
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

/// The status of a read - how the engine answered.
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
    /// Full content hash - the handle you pass to `expand` to recover the exact original.
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
    root: Option<PathBuf>,
    cache: Mutex<HashMap<String, CacheEntry>>,
    bounce_tracker: Mutex<BounceTracker>,
}

impl ContextEngine {
    pub fn new(store: Arc<Store>) -> Self {
        Self::new_with_root(store, None)
    }

    pub fn new_with_root(store: Arc<Store>, root: Option<PathBuf>) -> Self {
        Self {
            store,
            root,
            cache: Mutex::new(HashMap::new()),
            bounce_tracker: Mutex::new(BounceTracker::new()),
        }
    }

    /// Access the bounce tracker (for inspection by the API layer / metrics endpoint).
    pub fn bounce_tracker(&self) -> &Mutex<BounceTracker> {
        &self.bounce_tracker
    }

    /// Canonicalize `path` and ensure it stays inside the configured workspace root.
    /// Symlinks are resolved; attempts to escape with `..`, absolute paths outside the root, or
    /// symlinks pointing outside the root return [`cairn_core::Error::WorkspaceEscape`].
    pub fn resolve_path(&self, path: &Path) -> Result<PathBuf> {
        resolve_within_root(self.root.as_deref(), path)
    }

    /// Write `content` to `path` after confining it to the workspace root. The new content is
    /// retained in the blob store and recorded as the current file version.
    pub fn write(&self, path: &Path, content: &str) -> Result<()> {
        let path = self.resolve_path(path)?;
        let bytes = content.as_bytes();
        let hash = self.store.blobs().put(bytes)?;
        std::fs::write(&path, bytes)?;
        let lines = content.lines().count();
        let key = path.to_string_lossy().into_owned();
        let _ = self.store.record_file_version(&key, &hash.0, lines as i64);
        // Update the read cache so later reads see the new state without a fresh disk hit.
        let mtime_ns = std::fs::metadata(&path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        self.cache.lock().unwrap().insert(
            key,
            CacheEntry {
                mtime_ns,
                hash,
                content: content.to_string(),
                lines,
            },
        );
        Ok(())
    }

    /// Read a file, applying the cache / diff / retention logic above.
    pub fn read(&self, path: &Path, mode: ReadMode) -> Result<ReadResult> {
        let path = self.resolve_path(path)?;
        let key = path.to_string_lossy().to_string();
        let meta = std::fs::metadata(&path)?;
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
        let bytes = std::fs::read(&path)?;
        let content = String::from_utf8_lossy(&bytes).into_owned();
        let hash = self.store.blobs().put(&bytes)?;
        let lines = content.lines().count();
        let original_tokens = estimate_tokens(&content);

        // Record this version as the agent's edit baseline so the PostToolUse guard can later
        // detect silent corruption (a large unreplaced deletion vs. what was read).
        let canonical = std::fs::canonicalize(&path)
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| key.clone());
        let _ = self
            .store
            .record_file_version(&canonical, &hash.0, lines as i64);

        // Structural (AST) views: render the file as just its signatures. Falls through to a full
        // read for unsupported languages or unparseable input.
        //
        // P1.1 anti-inflation guard: a structural view is only worth shipping when it's
        // cheaper than the raw file. If `outline()` produces something bigger than the
        // original (e.g. heavily-macroed Rust, a single mega-line file), we fall through
        // to the Full branch below rather than waste tokens on a "compression" that isn't.
        if mode.is_structural() {
            if let Some(o) = outline::outline(&path, &content, mode == ReadMode::Map) {
                let est = estimate_tokens(&o.text);
                if est < original_tokens {
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
                // Outline is no cheaper than raw - fall through to Full below.
            }
        }

        let prev = cache.get(&key).map(|e| e.content.clone());

        let result = match (&prev, mode) {
            (Some(prev_content), ReadMode::Auto) if *prev_content != content => {
                let diff = diff_only(prev_content, &content);
                let diff_tokens = estimate_tokens(&diff);
                let full_tokens = original_tokens;
                // P1.2 auto-delta threshold: if the diff is >= 60% of the full file,
                // the delta is so noisy that the full file is actually cheaper to ship.
                // This guards against the "rewrote 90% of the file" case where the diff
                // would just be the new file with a few `-` lines at the top.
                if diff_tokens >= (full_tokens as f64 * 0.6) as usize {
                    let mut note = format!("full file; {lines} lines; handle {}", hash.short());
                    if lines > 40 && outline::supported(&path) {
                        note.push_str("; try mode=signatures for a structural outline");
                    }
                    ReadResult {
                        path: key.clone(),
                        hash: hash.0.clone(),
                        handle: hash.short().to_string(),
                        status: ReadStatus::Full,
                        lines,
                        bytes: content.len(),
                        est_tokens: full_tokens,
                        view: content.clone(),
                        note,
                    }
                } else {
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
                        est_tokens: diff_tokens,
                        view: diff,
                        note,
                    }
                }
            }
            _ => {
                let mut note = format!("full file; {lines} lines; handle {}", hash.short());
                // Point the agent at the cheaper structural view when one is available.
                if lines > 40 && outline::supported(&path) {
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
            key.clone(),
            CacheEntry {
                mtime_ns,
                hash,
                content,
                lines,
            },
        );
        // Record this read with the bounce tracker (P1.7). Compressed modes will register
        // as compressed; the next full read within BOUNCE_WINDOW triggers a bounce.
        let mode_label = match mode {
            ReadMode::Auto => "auto",
            ReadMode::Full => "full",
            ReadMode::Signatures => "signatures",
            ReadMode::Map => "map",
        };
        self.bounce_tracker.lock().unwrap().record_read(
            &key,
            mode_label,
            result.est_tokens,
            original_tokens,
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

/// Resolve `path` against an optional workspace root, rejecting anything that escapes.
///
/// - If no root is configured, the path is canonicalized normally.
/// - If a root is configured, relative paths are joined to it, absolute paths are checked against
///   it, and `..` components and symlinks that resolve outside the root are rejected.
fn resolve_within_root(root: Option<&Path>, path: &Path) -> Result<PathBuf> {
    if let Some(root) = root {
        // Strip Windows extended-path prefix (\\?\) so the starts_with comparison works
        // consistently for both existing paths (canonicalized with \\?\) and non-existent
        // paths (resolved via normalize_path without \\?\).
        let root = strip_extended_prefix(canonical_or_normalized(root)?);
        let candidate = if path.is_absolute() {
            path.to_path_buf()
        } else {
            root.join(path)
        };
        let resolved = if candidate.exists() {
            strip_extended_prefix(std::fs::canonicalize(&candidate)?)
        } else {
            normalize_path(&candidate)
        };
        if resolved.starts_with(&root) {
            Ok(resolved)
        } else {
            Err(Error::WorkspaceEscape(resolved))
        }
    } else if path.exists() {
        std::fs::canonicalize(path).map_err(Into::into)
    } else {
        // No root is configured; accept the path as-given (canonicalization may fail for new files).
        Ok(path.to_path_buf())
    }
}

/// On Windows, `std::fs::canonicalize` returns paths with a `\\?\` extended-length prefix.
/// Strip it so comparisons work uniformly across existing and non-existing paths.
fn strip_extended_prefix(path: PathBuf) -> PathBuf {
    #[cfg(windows)]
    {
        let s = path.to_string_lossy();
        if let Some(stripped) = s.strip_prefix(r"\\?\") {
            return PathBuf::from(stripped);
        }
    }
    path
}

/// Canonicalize a path if it exists, otherwise return a normalized absolute form.
fn canonical_or_normalized(path: &Path) -> Result<PathBuf> {
    if path.exists() {
        std::fs::canonicalize(path).map_err(Into::into)
    } else {
        let mut base = std::env::current_dir()?;
        if path.is_absolute() {
            base = PathBuf::new();
        }
        Ok(normalize_path(&base.join(path)))
    }
}

/// Collapse `.` and `..` components without touching the filesystem.
fn normalize_path(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(_) | Component::RootDir | Component::Normal(_) => out.push(component),
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
        }
    }
    out
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
    use cairn_store::Store;
    use std::time::{Duration, SystemTime};

    /// `None` when `CAIRN_DB_URL` is unset (offline runs skip these). The temp dir is a scratch
    /// workspace for the test's files (separate from the store).
    fn engine() -> Option<(ContextEngine, tempfile::TempDir)> {
        let store = Arc::new(Store::open_for_test()?);
        let dir = tempfile::tempdir().unwrap();
        Some((ContextEngine::new(store), dir))
    }

    #[test]
    fn cached_reread_is_cheap_diff_works_and_expand_is_lossless() {
        let Some((eng, dir)) = engine() else { return };
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

        // Both versions' originals are retained - nothing is ever lost.
        assert_eq!(eng.expand(&r1.hash).unwrap().unwrap(), original);
        assert_eq!(eng.expand(&r3.hash).unwrap().unwrap(), changed);
    }

    /// P1.2: a near-total rewrite (diff >= 60% of full) must fall back to Full,
    /// not ship a diff that's noisier than the original.
    #[test]
    fn auto_delta_falls_back_to_full_when_diff_too_large() {
        let Some((eng, dir)) = engine() else { return };
        let file = dir.path().join("rewritten.txt");
        let original: String = (0..1000)
            .map(|i| format!("line {i}: lorem ipsum dolor sit amet\n"))
            .collect();
        std::fs::write(&file, &original).unwrap();

        // First read seeds the cache.
        let r1 = eng.read(&file, ReadMode::Auto).unwrap();
        assert_eq!(r1.status, ReadStatus::Full);

        // Rewrite ~90% of the lines - the diff will be larger than 60% of full.
        let rewritten: String = (0..1000)
            .map(|i| format!("line {i}: completely different content here now\n"))
            .collect();
        std::fs::write(&file, &rewritten).unwrap();
        std::fs::OpenOptions::new()
            .write(true)
            .open(&file)
            .unwrap()
            .set_modified(SystemTime::now() + Duration::from_secs(2))
            .unwrap();

        // Re-read must fall back to Full, not inflate with a giant diff.
        let r2 = eng.read(&file, ReadMode::Auto).unwrap();
        assert_eq!(
            r2.status,
            ReadStatus::Full,
            "diff >= 60% of full should fall back to Full"
        );
        assert_eq!(
            r2.est_tokens, r1.est_tokens,
            "Full fall-back should equal original tokens"
        );
        // The view must NOT contain the +/- diff markers - it must be the raw file.
        assert!(!r2.view.starts_with('+') && !r2.view.starts_with('-'));
    }

    /// P1.1: an outline that's no cheaper than the raw file must fall through to Full,
    /// not waste tokens shipping a "compression" that's actually larger.
    #[test]
    fn outline_falls_back_to_full_when_no_smaller() {
        let Some((eng, dir)) = engine() else { return };
        let file = dir.path().join("tiny.rs");
        // A single-line file - the outline of one item is unlikely to be smaller than
        // the raw file with all its whitespace. The anti-inflation guard should kick in
        // and return Full instead of an inflated Outline.
        let src = "pub fn tiny() -> i32 { 42 }\n";
        std::fs::write(&file, src).unwrap();

        let r = eng.read(&file, ReadMode::Signatures).unwrap();
        // Either Outline (if outline() returned something cheaper) or Full (anti-inflation).
        // The invariant we care about: est_tokens <= estimate_tokens(src) AND
        // the original is recoverable.
        assert!(
            r.est_tokens <= estimate_tokens(src),
            "anti-inflation guard: shipping est_tokens={} for raw of {} tokens",
            r.est_tokens,
            estimate_tokens(src)
        );
        assert_eq!(eng.expand(&r.hash).unwrap().unwrap(), src);
    }

    #[test]
    fn signatures_mode_outlines_rust_and_stays_lossless() {
        let Some((eng, dir)) = engine() else { return };
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
        let Some((eng, dir)) = engine() else { return };
        let file = dir.path().join("notes.txt");
        let body = "plain prose, no code here\nsecond line\n";
        std::fs::write(&file, body).unwrap();

        // Unsupported language -> graceful fall-through to a full read.
        let r = eng.read(&file, ReadMode::Signatures).unwrap();
        assert_eq!(r.status, ReadStatus::Full);
        assert_eq!(r.view, body);
    }

    /// A workspace-rooted engine, skipped when the database is offline.
    fn rooted_engine() -> Option<(ContextEngine, tempfile::TempDir)> {
        let store = Arc::new(Store::open_for_test()?);
        let dir = tempfile::tempdir().unwrap();
        Some((
            ContextEngine::new_with_root(store, Some(dir.path().to_path_buf())),
            dir,
        ))
    }

    #[test]
    fn rooted_read_allows_inside_and_rejects_outside() {
        let Some((eng, dir)) = rooted_engine() else {
            return;
        };
        let inside = dir.path().join("inside.txt");
        std::fs::write(&inside, "ok").unwrap();

        assert!(
            eng.read(&inside, ReadMode::Full).is_ok(),
            "path inside root should be allowed"
        );

        // Keep the TempDir alive in a named binding so the directory is not deleted before
        // the read attempt (Windows deletes the directory immediately on TempDir drop).
        // The file need not exist - the engine rejects outside paths before any filesystem op.
        let _outside_dir = tempfile::tempdir().unwrap();
        let outside = _outside_dir.path().join("outside.txt");
        let err = eng
            .read(&outside, ReadMode::Full)
            .expect_err("absolute outside path must be rejected");
        assert!(
            err.to_string().contains("escapes workspace root"),
            "was: {err}"
        );

        let traversal = dir.path().join("../escape_attempt.txt");
        let err = eng
            .read(&traversal, ReadMode::Full)
            .expect_err("../ traversal must be rejected");
        assert!(
            err.to_string().contains("escapes workspace root"),
            "was: {err}"
        );
    }

    #[test]
    fn rooted_write_allows_inside_and_rejects_traversal() {
        let Some((eng, dir)) = rooted_engine() else {
            return;
        };
        let inside = dir.path().join("new.txt");
        eng.write(&inside, "hello\n")
            .expect("write inside root should succeed");
        assert_eq!(std::fs::read_to_string(&inside).unwrap(), "hello\n");

        let traversal = dir.path().join("../../escape.txt");
        let err = eng
            .write(&traversal, "x")
            .expect_err("../ traversal must be rejected");
        assert!(
            err.to_string().contains("escapes workspace root"),
            "was: {err}"
        );
    }

    #[test]
    #[cfg(unix)]
    fn rooted_read_rejects_symlink_escape() {
        let Some((eng, dir)) = rooted_engine() else {
            return;
        };
        let outside_dir = tempfile::tempdir().unwrap();
        let outside = outside_dir.path().join("target.txt");
        std::fs::write(&outside, "escaped").unwrap();
        let link = dir.path().join("link.txt");
        std::os::unix::fs::symlink(&outside, &link).unwrap();

        let err = eng
            .read(&link, ReadMode::Full)
            .expect_err("symlink escaping root must be rejected");
        assert!(
            err.to_string().contains("escapes workspace root"),
            "was: {err}"
        );
    }

    #[test]
    fn resolve_within_root_unit_tests() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let file = root.join("file.txt");
        std::fs::write(&file, "x").unwrap();

        // Relative inside root resolves to an absolute path under root.
        let resolved = resolve_within_root(Some(root), Path::new("file.txt")).unwrap();
        // strip_extended_prefix mirrors what resolve_within_root does so both sides
        // of the comparison are in the same (no \\?\) form on Windows.
        let canonical_root = super::strip_extended_prefix(std::fs::canonicalize(root).unwrap());
        assert!(
            resolved.starts_with(&canonical_root),
            "resolved {:?} should start with {:?}",
            resolved,
            canonical_root
        );

        // Absolute outside root is rejected.
        let outside_dir = tempfile::tempdir().unwrap();
        let outside = outside_dir.path().join("outside.txt");
        std::fs::write(&outside, "y").unwrap();
        let err = resolve_within_root(Some(root), &outside).unwrap_err();
        assert!(
            err.to_string().contains("escapes workspace root"),
            "was: {err}"
        );

        // Traversal via .. is rejected even when the target does not exist.
        let err = resolve_within_root(Some(root), Path::new("../nope.txt")).unwrap_err();
        assert!(
            err.to_string().contains("escapes workspace root"),
            "was: {err}"
        );

        // No root configured: normal canonicalization.
        let resolved = resolve_within_root(None, &file).unwrap();
        assert!(resolved.exists());
    }
}
