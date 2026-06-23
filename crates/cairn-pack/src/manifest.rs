//! The manifest that describes a `.cairnpkg` tarball.
//!
//! The manifest is a JSON file at `manifest.json` inside the tarball. It carries:
//!
//! - `id` — a stable UUID v4 (regenerated per import if you want a fresh local id).
//! - `name` — short slug, e.g. `rust-safety`.
//! - `version` — semver string; compares with `>` so newer packages upgrade.
//! - `author` — display name; not authenticated (no key infrastructure yet — see ADR-012).
//! - `description` — free text.
//! - `created_at` — RFC3339 timestamp.
//! - `files` — map of `<filename>` → sha256 hex digest. Verifies every other file in the
//!   tarball before we touch its bytes.
//! - `stats` — counts (memories, profile entries, patterns, edges) — informational, helps
//!   `pack info` render a one-line summary without unpacking the whole archive.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    io::{self, Read},
    path::Path,
};
use uuid::Uuid;

use crate::signing;
use crate::{ALT_EXTENSION, EXTENSION};

/// On-disk manifest schema. Mirrors the JSON we serialize into `manifest.json` at the top
/// of every `.cairnpkg`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Manifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub author: String,
    pub description: String,
    pub created_at: DateTime<Utc>,
    /// Map of in-archive path → sha256 hex digest.
    pub files: BTreeMap<String, String>,
    pub stats: ManifestStats,
    /// Author public keys whose Ed25519 signatures appear in `signature.ed25519` files
    /// inside the tarball. Empty for unsigned (legacy) packs.
    #[serde(default)]
    pub signers: Vec<crate::signing::PublicKey>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ManifestStats {
    #[serde(default)]
    pub memories: usize,
    #[serde(default)]
    pub profile: usize,
    #[serde(default)]
    pub patterns: usize,
    #[serde(default)]
    pub graph_edges: usize,
}

impl Manifest {
    /// Build a fresh manifest from the content that will go into a tarball. The caller
    /// supplies the file map (already-hashed by [`signing::hash_file`]) and stats.
    pub fn new(
        name: impl Into<String>,
        version: impl Into<String>,
        author: impl Into<String>,
        description: impl Into<String>,
        files: BTreeMap<String, String>,
        stats: ManifestStats,
    ) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            name: name.into(),
            version: version.into(),
            author: author.into(),
            description: description.into(),
            created_at: Utc::now(),
            files,
            stats,
            signers: Vec::new(),
        }
    }

    /// Serialize to pretty JSON.
    pub fn to_bytes(&self) -> serde_json::Result<Vec<u8>> {
        serde_json::to_vec_pretty(self)
    }

    /// Read a manifest from a file path. Used by `pack info` and by [`crate::install::install`]
    /// before extracting any other file.
    pub fn read(path: &Path) -> io::Result<Self> {
        let bytes = fs::read(path)?;
        let m: Self = serde_json::from_slice(&bytes).map_err(|e| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("manifest not valid JSON: {e}"),
            )
        })?;
        Ok(m)
    }

    /// Read from a tarball entry (used during `install`).
    pub fn from_entry<R: Read>(mut r: R) -> io::Result<Self> {
        let mut buf = Vec::new();
        r.read_to_end(&mut buf)?;
        serde_json::from_slice(&buf).map_err(|e| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("manifest not valid JSON: {e}"),
            )
        })
    }
}

/// True if `path` looks like a cairnpkg (by extension).
pub fn is_supported_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| {
            let lower = e.to_ascii_lowercase();
            lower == EXTENSION || lower == ALT_EXTENSION
        })
        .unwrap_or(false)
}

/// Validate a manifest's file map against an actual extracted tarball directory. Returns
/// `Ok(())` on match, `Err` describing the first mismatch. This is what `install` calls
/// after extraction to catch a partial / tampered download.
pub fn verify_extracted(
    manifest: &Manifest,
    extract_root: &Path,
) -> std::result::Result<(), String> {
    for (rel, expected) in &manifest.files {
        let path = extract_root.join(rel);
        let bytes = fs::read(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
        let actual = signing::hash_bytes(&bytes);
        if &actual != expected {
            return Err(format!(
                "hash mismatch for {rel}: expected {expected}, got {actual}"
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn manifest_round_trips_through_disk() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("manifest.json");
        let mut files = BTreeMap::new();
        files.insert("memory.jsonl".into(), "deadbeef".into());
        let m = Manifest::new(
            "test-pack",
            "1.0.0",
            "tester",
            "round-trip test",
            files,
            ManifestStats {
                memories: 3,
                ..Default::default()
            },
        );
        let json = serde_json::to_vec_pretty(&m).unwrap();
        fs::write(&path, &json).unwrap();
        let loaded = Manifest::read(&path).unwrap();
        assert_eq!(loaded, m);
    }

    #[test]
    fn is_supported_extension_accepts_both() {
        assert!(is_supported_extension(Path::new("/x/y.cairnpkg")));
        assert!(is_supported_extension(Path::new("/x/y.CAIRNPKG")));
        assert!(is_supported_extension(Path::new("/x/y.ctxpkg")));
        assert!(!is_supported_extension(Path::new("/x/y.zip")));
        assert!(!is_supported_extension(Path::new("/x/y")));
    }

    #[test]
    fn verify_extracted_reports_hash_mismatch() {
        let dir = TempDir::new().unwrap();
        let mut files = BTreeMap::new();
        files.insert("memory.jsonl".into(), "deadbeef".into());
        let m = Manifest::new("x", "1", "a", "b", files, ManifestStats::default());
        fs::write(dir.path().join("memory.jsonl"), b"actual content").unwrap();
        let e = verify_extracted(&m, dir.path()).unwrap_err();
        assert!(e.contains("hash mismatch"));
    }

    #[test]
    fn verify_extracted_passes_when_hashes_match() {
        let dir = TempDir::new().unwrap();
        let content = b"actual content";
        let hash = signing::hash_bytes(content);
        let mut files = BTreeMap::new();
        files.insert("memory.jsonl".into(), hash.clone());
        let m = Manifest::new("x", "1", "a", "b", files, ManifestStats::default());
        fs::write(dir.path().join("memory.jsonl"), content).unwrap();
        verify_extracted(&m, dir.path()).unwrap();
    }
}
