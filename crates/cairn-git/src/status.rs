//! T019: `git status --porcelain=v2 --branch --untracked-files=all -z` parser.
//!
//! Ignored enumeration is deliberately NOT requested on this path (analysis
//! I2); the ignore-crate walker in `ignored.rs` is authoritative for that.

use crate::error::GitError;
use crate::runner::GitRunner;

/// One changed path with its staged/unstaged classification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileChange {
    pub path: String,
    pub status: ChangeKind,
    pub orig_path: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    Added,
    Modified,
    Deleted,
    Renamed,
    Copied,
    TypeChanged,
    Unmerged,
}

impl ChangeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ChangeKind::Added => "added",
            ChangeKind::Modified => "modified",
            ChangeKind::Deleted => "deleted",
            ChangeKind::Renamed => "renamed",
            ChangeKind::Copied => "copied",
            ChangeKind::TypeChanged => "typechanged",
            ChangeKind::Unmerged => "unmerged",
        }
    }

    fn from_xy(c: u8) -> Option<Self> {
        Some(match c {
            b'A' => ChangeKind::Added,
            b'M' => ChangeKind::Modified,
            b'D' => ChangeKind::Deleted,
            b'R' => ChangeKind::Renamed,
            b'C' => ChangeKind::Copied,
            b'T' => ChangeKind::TypeChanged,
            b'U' => ChangeKind::Unmerged,
            _ => return None,
        })
    }
}

/// Parsed exact working-tree status (FR-006).
#[derive(Debug, Clone, Default)]
pub struct StatusReport {
    /// Current branch; `None` when HEAD is detached.
    pub branch: Option<String>,
    /// HEAD commit OID; `None` on an unborn branch.
    pub head_oid: Option<String>,
    pub staged: Vec<FileChange>,
    pub unstaged: Vec<FileChange>,
    pub untracked: Vec<String>,
}

/// Run and parse status for the repository at `runner.dir()`.
pub async fn status(runner: &GitRunner) -> Result<StatusReport, GitError> {
    let raw = runner
        .run(&[
            "status",
            "--porcelain=v2",
            "--branch",
            "--untracked-files=all",
            "-z",
        ])
        .await?;
    parse_porcelain_v2(&raw)
}

/// Parse NUL-terminated porcelain v2 output.
pub fn parse_porcelain_v2(raw: &[u8]) -> Result<StatusReport, GitError> {
    let mut report = StatusReport::default();
    let mut fields = raw.split(|&b| b == 0).peekable();

    while let Some(field) = fields.next() {
        if field.is_empty() {
            continue;
        }
        let line = String::from_utf8_lossy(field).into_owned();
        if let Some(rest) = line.strip_prefix("# ") {
            if let Some(oid) = rest.strip_prefix("branch.oid ") {
                if oid != "(initial)" {
                    report.head_oid = Some(oid.to_string());
                }
            } else if let Some(head) = rest.strip_prefix("branch.head ") {
                if head != "(detached)" {
                    report.branch = Some(head.to_string());
                }
            }
            continue;
        }

        let mut parts = line.splitn(2, ' ');
        let kind = parts.next().unwrap_or("");
        let rest = parts.next().unwrap_or("");
        match kind {
            "1" => parse_ordinary(rest, &mut report)?,
            "2" => {
                // Rename/copy: the NEXT NUL field is the original path.
                let orig = fields
                    .next()
                    .map(|f| String::from_utf8_lossy(f).into_owned())
                    .ok_or_else(|| GitError::Parse("rename entry missing orig path".into()))?;
                parse_rename(rest, orig, &mut report)?;
            }
            "u" => parse_unmerged(rest, &mut report)?,
            "?" => report.untracked.push(rest.to_string()),
            "!" => { /* ignored entries are never requested on this path */ }
            _ => { /* forward-compatible: skip unknown record types */ }
        }
    }
    Ok(report)
}

/// Ordinary entry: `XY sub mH mI mW hH hI path` (after the leading "1 ").
fn parse_ordinary(rest: &str, report: &mut StatusReport) -> Result<(), GitError> {
    let mut it = rest.splitn(9, ' ');
    let xy = it
        .next()
        .ok_or_else(|| GitError::Parse("missing XY".into()))?;
    // skip sub, mH, mI, mW, hH, hI
    for _ in 0..6 {
        it.next()
            .ok_or_else(|| GitError::Parse("short ordinary entry".into()))?;
    }
    let path = it
        .next()
        .ok_or_else(|| GitError::Parse("ordinary entry missing path".into()))?
        .to_string();
    let bytes = xy.as_bytes();
    if bytes.len() != 2 {
        return Err(GitError::Parse(format!("bad XY field: {xy}")));
    }
    if bytes[0] != b'.' {
        if let Some(status) = ChangeKind::from_xy(bytes[0]) {
            report.staged.push(FileChange {
                path: path.clone(),
                status,
                orig_path: None,
            });
        }
    }
    if bytes[1] != b'.' {
        if let Some(status) = ChangeKind::from_xy(bytes[1]) {
            report.unstaged.push(FileChange {
                path,
                status,
                orig_path: None,
            });
        }
    }
    Ok(())
}

/// Rename/copy entry: `XY sub mH mI mW hH hI Xscore path` + NUL + origPath.
fn parse_rename(rest: &str, orig: String, report: &mut StatusReport) -> Result<(), GitError> {
    let mut it = rest.splitn(10, ' ');
    let xy = it
        .next()
        .ok_or_else(|| GitError::Parse("missing XY".into()))?;
    for _ in 0..7 {
        it.next()
            .ok_or_else(|| GitError::Parse("short rename entry".into()))?;
    }
    let path = it
        .next()
        .ok_or_else(|| GitError::Parse("rename entry missing path".into()))?
        .to_string();
    let bytes = xy.as_bytes();
    if bytes.len() != 2 {
        return Err(GitError::Parse(format!("bad XY field: {xy}")));
    }
    if bytes[0] != b'.' {
        if let Some(status) = ChangeKind::from_xy(bytes[0]) {
            report.staged.push(FileChange {
                path: path.clone(),
                status,
                orig_path: Some(orig.clone()),
            });
        }
    }
    if bytes[1] != b'.' {
        if let Some(status) = ChangeKind::from_xy(bytes[1]) {
            report.unstaged.push(FileChange {
                path,
                status,
                orig_path: Some(orig),
            });
        }
    }
    Ok(())
}

/// Unmerged entry: `XY sub m1 m2 m3 mW h1 h2 h3 path`.
fn parse_unmerged(rest: &str, report: &mut StatusReport) -> Result<(), GitError> {
    let path = rest
        .rsplit(' ')
        .next()
        .ok_or_else(|| GitError::Parse("unmerged entry missing path".into()))?
        .to_string();
    report.unstaged.push(FileChange {
        path,
        status: ChangeKind::Unmerged,
        orig_path: None,
    });
    Ok(())
}

/// Parse `git ls-files -s -z` output into index entries for staged_fp.
pub fn parse_ls_files(raw: &[u8]) -> Result<Vec<cairn_domain::IndexEntry>, GitError> {
    let mut entries = Vec::new();
    for field in raw.split(|&b| b == 0) {
        if field.is_empty() {
            continue;
        }
        let line = String::from_utf8_lossy(field);
        // Format: "<mode> <oid> <stage>\t<path>"
        let (meta, path) = line
            .split_once('\t')
            .ok_or_else(|| GitError::Parse(format!("bad ls-files line: {line}")))?;
        let mut m = meta.split(' ');
        let mode = m
            .next()
            .ok_or_else(|| GitError::Parse("ls-files missing mode".into()))?;
        let oid = m
            .next()
            .ok_or_else(|| GitError::Parse("ls-files missing oid".into()))?;
        let stage: u8 = m
            .next()
            .ok_or_else(|| GitError::Parse("ls-files missing stage".into()))?
            .parse()
            .map_err(|_| GitError::Parse("bad stage number".into()))?;
        entries.push(cairn_domain::IndexEntry {
            mode: mode.to_string(),
            stage,
            oid: oid.to_string(),
            path: path.to_string(),
        });
    }
    Ok(entries)
}
