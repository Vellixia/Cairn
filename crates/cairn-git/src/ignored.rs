//! T026: ignore-crate walker — the AUTHORITATIVE source for ignored-file
//! summaries (analysis I2, FR-035). Bounded output: collapsed roots, ≤20
//! samples, truncation indicator; full detail only via pagination.

use std::collections::BTreeMap;
use std::path::Path;

use ignore::gitignore::{Gitignore, GitignoreBuilder};

use crate::error::GitError;

pub const MAX_SAMPLES: usize = 20;
/// Enumeration cap for summary counting; beyond this `truncated=true`.
pub const COUNT_CAP: u64 = 200_000;
pub const MAX_PAGE: usize = 1000;

#[derive(Debug, Clone, Default)]
pub struct IgnoredSummary {
    pub total_count: u64,
    pub gitignore_count: u64,
    pub cairnignore_count: u64,
    pub collapsed_roots: Vec<(String, u64)>,
    pub samples: Vec<String>,
    pub truncated: bool,
}

/// Which rule stack matched a path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Source {
    GitIgnore,
    CairnIgnore,
}

/// Layered matcher: git ignore rules plus `.cairnignore` (FR-026).
pub struct IgnoreStack {
    git: Gitignore,
    cairn: Gitignore,
}

impl IgnoreStack {
    pub fn load(root: &Path) -> Result<Self, GitError> {
        let mut gb = GitignoreBuilder::new(root);
        gb.add(root.join(".gitignore"));
        // Repo-local exclude file participates in git ignore semantics.
        gb.add(root.join(".git").join("info").join("exclude"));
        let git = gb.build().map_err(|e| GitError::Parse(e.to_string()))?;

        let mut cb = GitignoreBuilder::new(root);
        cb.add(root.join(".cairnignore"));
        let cairn = cb.build().map_err(|e| GitError::Parse(e.to_string()))?;
        Ok(Self { git, cairn })
    }

    fn classify(&self, rel: &Path, is_dir: bool) -> Option<Source> {
        if self
            .git
            .matched_path_or_any_parents(rel, is_dir)
            .is_ignore()
        {
            Some(Source::GitIgnore)
        } else if self
            .cairn
            .matched_path_or_any_parents(rel, is_dir)
            .is_ignore()
        {
            Some(Source::CairnIgnore)
        } else {
            None
        }
    }

    /// True when a path must be excluded from observation/fingerprinting.
    pub fn is_excluded(&self, rel: &Path, is_dir: bool) -> bool {
        self.classify(rel, is_dir).is_some()
    }
}

fn walk_ignored(
    root: &Path,
    stack: &IgnoreStack,
    cap: u64,
    mut on_file: impl FnMut(&str, Source),
) -> Result<bool, GitError> {
    // Manual stack walk so ignored DIRECTORIES are descended (to count) while
    // .git itself is always skipped.
    let mut truncated = false;
    let mut seen: u64 = 0;
    let mut stack_dirs = vec![root.to_path_buf()];
    while let Some(dir) = stack_dirs.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue, // races with deletion are fine: advisory data
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(ft) = entry.file_type() else { continue };
            let rel = match path.strip_prefix(root) {
                Ok(r) => r.to_path_buf(),
                Err(_) => continue,
            };
            if rel
                .components()
                .next()
                .map(|c| c.as_os_str() == ".git")
                .unwrap_or(false)
            {
                continue;
            }
            if ft.is_dir() {
                stack_dirs.push(path);
            } else if ft.is_file() {
                if let Some(source) = stack.classify(&rel, false) {
                    seen += 1;
                    if seen > cap {
                        truncated = true;
                        return Ok(truncated);
                    }
                    let rel_str = rel.to_string_lossy().replace('\\', "/");
                    on_file(&rel_str, source);
                }
            }
        }
    }
    Ok(truncated)
}

/// Build the bounded ignored summary (FR-035).
pub fn ignored_summary(root: &Path) -> Result<IgnoredSummary, GitError> {
    let stack = IgnoreStack::load(root)?;
    let mut summary = IgnoredSummary::default();
    let mut roots: BTreeMap<String, u64> = BTreeMap::new();

    let truncated = walk_ignored(root, &stack, COUNT_CAP, |rel, source| {
        summary.total_count += 1;
        match source {
            Source::GitIgnore => summary.gitignore_count += 1,
            Source::CairnIgnore => summary.cairnignore_count += 1,
        }
        let top = rel.split('/').next().unwrap_or(rel).to_string();
        *roots.entry(top).or_insert(0) += 1;
        if summary.samples.len() < MAX_SAMPLES {
            summary.samples.push(rel.to_string());
        }
    })?;
    summary.truncated = truncated;
    summary.collapsed_roots = roots.into_iter().collect();
    Ok(summary)
}

/// Cursor-paginated ignored-file enumeration (FR-035 drill-down).
/// Cursor = last path of the previous page (paths are emitted in sorted order).
pub fn ignored_page(
    root: &Path,
    cursor: Option<&str>,
    limit: usize,
    glob: Option<&str>,
) -> Result<(Vec<String>, Option<String>), GitError> {
    let stack = IgnoreStack::load(root)?;
    let limit = limit.clamp(1, MAX_PAGE);
    let glob_matcher = match glob {
        Some(g) => Some(
            ignore::gitignore::GitignoreBuilder::new(root)
                .add_line(None, g)
                .map_err(|e| GitError::Parse(e.to_string()))?
                .build()
                .map_err(|e| GitError::Parse(e.to_string()))?,
        ),
        None => None,
    };

    let mut all: Vec<String> = Vec::new();
    walk_ignored(root, &stack, COUNT_CAP, |rel, _| {
        if let Some(m) = &glob_matcher {
            if !m.matched(Path::new(rel), false).is_ignore() {
                return;
            }
        }
        all.push(rel.to_string());
    })?;
    all.sort();

    let start = match cursor {
        Some(c) => all.partition_point(|p| p.as_str() <= c),
        None => 0,
    };
    let page: Vec<String> = all.iter().skip(start).take(limit).cloned().collect();
    let next = if start + page.len() < all.len() {
        page.last().cloned()
    } else {
        None
    };
    Ok((page, next))
}
