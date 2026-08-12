//! Atomic application and verification (FR-154, FR-155, FR-156, FR-196).
//!
//! Each individual configuration file is replaced atomically, so an
//! interrupted write can never leave a truncated or partially written file.
//! Recoverability comes from that atomic replacement, **not** from copying the
//! developer's files: those routinely carry tokens, provider credentials and
//! environment secrets Cairn is forbidden to hold (FR-156, FR-197, FR-200).
//!
//! Where recovery content genuinely is needed — a forced repair over content a
//! developer edited — only the Cairn-owned block, entry, or wholly
//! Cairn-generated file is preserved, never the enclosing file (FR-238).
//!
//! Configuration operations do not fail soft. A multi-file change that partly
//! lands reports failure naming both halves; it is never reported as success
//! (FR-155, FR-196).

use crate::edit::EditError;
use crate::model::{canonical_hash, AgentId, ResourceKind};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};

/// One file write, ready to apply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileWrite {
    pub path: PathBuf,
    pub contents: String,
    pub agent: AgentId,
    pub kind: ResourceKind,
}

/// One whole directory Cairn generated, written or removed as a unit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeWrite {
    pub root: PathBuf,
    /// Relative path → contents.
    pub files: Vec<(String, String)>,
    pub agent: AgentId,
    pub kind: ResourceKind,
}

/// What an operation did, per resource.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Applied {
    pub agent: AgentId,
    pub kind: ResourceKind,
    pub target: String,
}

/// What it could not do, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotApplied {
    pub agent: AgentId,
    pub kind: ResourceKind,
    pub target: String,
    pub reason: String,
}

/// The outcome of applying a plan.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApplyResult {
    pub applied: Vec<Applied>,
    pub not_applied: Vec<NotApplied>,
    pub verified: bool,
    /// Paths only — never their content (FR-239).
    pub recovery_artifacts: Vec<String>,
}

impl ApplyResult {
    /// A partial application is never reported as success (FR-155).
    pub fn is_partial(&self) -> bool {
        !self.not_applied.is_empty() && !self.applied.is_empty()
    }
    pub fn failed(&self) -> bool {
        !self.not_applied.is_empty()
    }
}

/// Why a write could not be performed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplyError {
    PermissionDenied {
        path: String,
        detail: String,
    },
    Io {
        path: String,
        detail: String,
    },
    Edit(EditError),
    /// The change was applied but re-inspection did not observe it.
    VerificationFailed {
        path: String,
        detail: String,
    },
}

impl std::fmt::Display for ApplyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApplyError::PermissionDenied { path, detail } => {
                write!(f, "permission_denied: {path}: {detail}")
            }
            ApplyError::Io { path, detail } => write!(f, "{path}: {detail}"),
            ApplyError::Edit(e) => write!(f, "{e}"),
            ApplyError::VerificationFailed { path, detail } => {
                write!(f, "verification_failed: {path}: {detail}")
            }
        }
    }
}

impl ApplyError {
    /// The CLI error code this maps to.
    pub fn code(&self) -> &'static str {
        match self {
            ApplyError::PermissionDenied { .. } => "permission_denied",
            ApplyError::Io { .. } => "invalid_request",
            ApplyError::Edit(e) => match e {
                EditError::DamagedMarkers { .. } => "damaged_markers",
                _ => "malformed_config",
            },
            ApplyError::VerificationFailed { .. } => "verification_failed",
        }
    }
}

fn io_error(path: &Path, e: std::io::Error) -> ApplyError {
    let path = path.display().to_string();
    if e.kind() == std::io::ErrorKind::PermissionDenied {
        ApplyError::PermissionDenied {
            path,
            detail: e.to_string(),
        }
    } else {
        ApplyError::Io {
            path,
            detail: e.to_string(),
        }
    }
}

/// Replace one file atomically.
///
/// The original stays intact and readable until the replacement is complete,
/// so an interrupted operation leaves the prior state in place (FR-154,
/// FR-156). The temporary is created in the destination directory so the
/// rename cannot cross a filesystem boundary.
pub fn write_atomic(path: &Path, contents: &str) -> Result<(), ApplyError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| io_error(parent, e))?;
    }
    let dir = path.parent().unwrap_or(Path::new("."));
    let stem = path
        .file_name()
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_else(|| "cairn".into());
    let tmp = dir.join(format!(".{stem}.cairn-{}.tmp", std::process::id()));

    let result = (|| -> std::io::Result<()> {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(contents.as_bytes())?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&tmp, path)
    })();

    if let Err(e) = result {
        let _ = std::fs::remove_file(&tmp);
        return Err(io_error(path, e));
    }
    Ok(())
}

/// Read a file, treating absence as empty rather than as an error.
pub fn read_or_empty(path: &Path) -> Result<String, ApplyError> {
    match std::fs::read_to_string(path) {
        Ok(s) => Ok(s),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(io_error(path, e)),
    }
}

/// Write a directory Cairn generated in full — the Skill tree.
///
/// Whole-file mutation is legitimate here precisely because Cairn owns every
/// byte of every file in it (FR-238).
pub fn write_tree(tree: &TreeWrite) -> Result<(), ApplyError> {
    for (rel, contents) in &tree.files {
        let path = tree.root.join(rel);
        write_atomic(&path, contents)?;
    }
    Ok(())
}

/// Remove a directory Cairn generated in full.
pub fn remove_tree(root: &Path) -> Result<(), ApplyError> {
    match std::fs::remove_dir_all(root) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(io_error(root, e)),
    }
}

/// Verify that what was written is what is now on disk (FR-151's last step).
pub fn verify_file(path: &Path, expected: &str) -> Result<(), ApplyError> {
    let found = read_or_empty(path)?;
    if canonical_hash(&found) == canonical_hash(expected) {
        Ok(())
    } else {
        Err(ApplyError::VerificationFailed {
            path: path.display().to_string(),
            detail: "the file does not contain what Cairn just wrote".into(),
        })
    }
}

/// Preserve Cairn-owned content before a forced change (FR-222, FR-238, D39).
///
/// What is preserved is the Cairn-owned block, entry, or wholly
/// Cairn-generated file — **never** the enclosing configuration file, and
/// never a setting Cairn does not own. If the owned content cannot be isolated
/// from the rest of the file, the caller reports the condition and changes
/// nothing rather than copying the whole file.
pub struct RecoveryWrite {
    pub agent: AgentId,
    pub kind: ResourceKind,
    pub source_path: PathBuf,
    /// Cairn-owned prior content only.
    pub owned_content: String,
}

/// Where recovery artifacts live, and how many are kept.
pub const RECOVERY_RETENTION: usize = 10;

/// Write one recovery artifact and prune older ones.
///
/// Returns its path. The artifact's *content* is never logged and never enters
/// diagnostics; only its path is ever printed (FR-239).
pub fn write_recovery(
    cairn_home: &Path,
    now: &str,
    write: &RecoveryWrite,
) -> Result<PathBuf, ApplyError> {
    let dir = cairn_home
        .join("recovery")
        .join(write.agent.as_str())
        .join(write.kind.as_str());
    std::fs::create_dir_all(&dir).map_err(|e| io_error(&dir, e))?;
    let hash = canonical_hash(&write.owned_content);
    let path = dir.join(format!("{now}-{hash}.txt"));
    write_atomic(&path, &write.owned_content)?;
    prune_recovery(&dir)?;
    Ok(path)
}

/// Keep the ten most recent artifacts per `(agent, kind)`.
fn prune_recovery(dir: &Path) -> Result<(), ApplyError> {
    let mut entries: Vec<PathBuf> = match std::fs::read_dir(dir) {
        Ok(rd) => rd.filter_map(|e| e.ok().map(|e| e.path())).collect(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(io_error(dir, e)),
    };
    if entries.len() <= RECOVERY_RETENTION {
        return Ok(());
    }
    // The timestamp prefix sorts lexicographically, which is why it is written
    // that way.
    entries.sort();
    let excess = entries.len() - RECOVERY_RETENTION;
    for path in entries.into_iter().take(excess) {
        let _ = std::fs::remove_file(path);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> tempfile::TempDir {
        tempfile::tempdir().expect("tempdir")
    }

    #[test]
    fn a_write_replaces_the_file_and_leaves_no_temporary_behind() {
        let d = tmp();
        let p = d.path().join("nested").join("config.json");
        write_atomic(&p, "{\n}\n").unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "{\n}\n");
        let strays: Vec<_> = std::fs::read_dir(p.parent().unwrap())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains("tmp"))
            .collect();
        assert!(strays.is_empty(), "a temporary file was left behind");
    }

    #[test]
    fn verification_catches_a_write_that_did_not_land() {
        let d = tmp();
        let p = d.path().join("c.json");
        write_atomic(&p, "one\n").unwrap();
        assert!(verify_file(&p, "one\n").is_ok());
        assert!(matches!(
            verify_file(&p, "two\n"),
            Err(ApplyError::VerificationFailed { .. })
        ));
    }

    #[test]
    fn a_partial_apply_is_never_success() {
        // FR-155.
        let r = ApplyResult {
            applied: vec![Applied {
                agent: AgentId::Codex,
                kind: ResourceKind::Mcp,
                target: "a".into(),
            }],
            not_applied: vec![NotApplied {
                agent: AgentId::Codex,
                kind: ResourceKind::Lifecycle,
                target: "b".into(),
                reason: "permission_denied".into(),
            }],
            verified: false,
            recovery_artifacts: vec![],
        };
        assert!(r.is_partial());
        assert!(r.failed());
    }

    #[test]
    fn a_recovery_artifact_holds_only_the_owned_content() {
        // FR-238, SC-133: never the enclosing file.
        let d = tmp();
        let path = write_recovery(
            d.path(),
            "2026-08-12T09-14-02Z",
            &RecoveryWrite {
                agent: AgentId::ClaudeCode,
                kind: ResourceKind::Instructions,
                source_path: "/repo/CLAUDE.md".into(),
                owned_content: "## Cairn\n\n1. Rule.\n".into(),
            },
        )
        .unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, "## Cairn\n\n1. Rule.\n");
        assert!(path.starts_with(d.path().join("recovery")));
    }

    #[test]
    fn recovery_artifacts_are_capped_at_ten_per_resource() {
        let d = tmp();
        for i in 0..15 {
            write_recovery(
                d.path(),
                &format!("2026-08-12T09-{i:02}-00Z"),
                &RecoveryWrite {
                    agent: AgentId::ClaudeCode,
                    kind: ResourceKind::Instructions,
                    source_path: "/repo/CLAUDE.md".into(),
                    owned_content: format!("body {i}\n"),
                },
            )
            .unwrap();
        }
        let dir = d
            .path()
            .join("recovery")
            .join("claude-code")
            .join("instructions");
        assert_eq!(std::fs::read_dir(dir).unwrap().count(), RECOVERY_RETENTION);
    }

    #[test]
    fn a_tree_is_written_and_removed_as_a_unit() {
        let d = tmp();
        let root = d.path().join("skills").join("cairn");
        write_tree(&TreeWrite {
            root: root.clone(),
            files: vec![
                ("SKILL.md".into(), "---\n---\n".into()),
                ("references/a.md".into(), "a\n".into()),
            ],
            agent: AgentId::ClaudeCode,
            kind: ResourceKind::Skill,
        })
        .unwrap();
        assert!(root.join("references/a.md").exists());
        remove_tree(&root).unwrap();
        assert!(!root.exists());
        // Removing what is already gone is not an error — disconnect stays
        // idempotent (FR-157).
        remove_tree(&root).unwrap();
    }

    #[test]
    fn a_read_only_file_fails_with_the_reason_and_leaves_the_original() {
        let d = tmp();
        let p = d.path().join("ro.json");
        write_atomic(&p, "original\n").unwrap();
        let mut perms = std::fs::metadata(d.path()).unwrap().permissions();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            perms.set_mode(0o500);
            std::fs::set_permissions(d.path(), perms.clone()).unwrap();
            let outcome = write_atomic(&p, "replacement\n");
            perms.set_mode(0o700);
            std::fs::set_permissions(d.path(), perms).unwrap();
            match outcome {
                Err(e) => {
                    assert!(matches!(e, ApplyError::PermissionDenied { .. }), "{e}");
                    assert_eq!(std::fs::read_to_string(&p).unwrap(), "original\n");
                }
                // Running with privileges that bypass file permissions (root
                // in a container, for instance). There is nothing to assert
                // about a restriction the kernel is not applying.
                Ok(()) => eprintln!("skipped: file permissions are not enforced for this user"),
            }
        }
    }
}
