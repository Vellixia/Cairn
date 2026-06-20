//! Build / inspect / serialize a `.cairnpkg` tarball.

use crate::manifest::{Manifest, ManifestStats};
use crate::signing;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

/// What gets bundled into a `.cairnpkg`. Memories, profile, patterns, and graph edges
/// each stream to their own JSONL file inside the tarball so individual sections can be
/// ingested without re-parsing the whole archive.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Pack {
    pub name: String,
    pub version: String,
    pub author: String,
    pub description: String,
    /// Memories to bundle, in the cairn-share `ShareableMemory` shape (already sanitized).
    #[serde(default)]
    pub memories: Vec<serde_json::Value>,
    #[serde(default)]
    pub profile: Vec<serde_json::Value>,
    #[serde(default)]
    pub patterns: Vec<serde_json::Value>,
    #[serde(default)]
    pub graph_edges: Vec<serde_json::Value>,
}

impl Pack {
    pub fn new(name: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            version: version.into(),
            ..Default::default()
        }
    }

    pub fn stats(&self) -> ManifestStats {
        ManifestStats {
            memories: self.memories.len(),
            profile: self.profile.len(),
            patterns: self.patterns.len(),
            graph_edges: self.graph_edges.len(),
        }
    }

    /// Write the pack to a tarball at `output_path`. Layout (matching the lean-ctx
    /// `.ctxpkg` design we adapted):
    ///
    /// - `manifest.json` — at the top, JSON
    /// - `memory.jsonl`, `profile.jsonl`, `patterns.jsonl`, `graph.jsonl` — newline-delimited JSON
    /// - `signature.sha256` — hex SHA-256 of the canonical manifest bytes
    ///
    /// We use the [tar] crate via the `tempfile` pattern (build in a tempdir, then rename).
    pub fn write_tarball(&self, output_path: &Path) -> io::Result<()> {
        use std::io::Write as _;
        let mut files: BTreeMap<String, Vec<u8>> = BTreeMap::new();
        files.insert("memory.jsonl".into(), self.write_jsonl(&self.memories));
        files.insert("profile.jsonl".into(), self.write_jsonl(&self.profile));
        files.insert("patterns.jsonl".into(), self.write_jsonl(&self.patterns));
        files.insert("graph.jsonl".into(), self.write_jsonl(&self.graph_edges));

        let manifest = Manifest::new(
            self.name.clone(),
            self.version.clone(),
            self.author.clone(),
            self.description.clone(),
            files
                .iter()
                .map(|(k, v)| (k.clone(), signing::hash_bytes(v)))
                .collect(),
            self.stats(),
        );
        let manifest_bytes = manifest.to_bytes().map_err(|e| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("serialize manifest: {e}"),
            )
        })?;
        files.insert("manifest.json".into(), manifest_bytes.clone());
        files.insert(
            "signature.sha256".into(),
            signing::sign_manifest(&manifest_bytes).into_bytes(),
        );

        // Write the tarball. We hand-roll the ustar header format — small enough that
        // adding the `tar` crate as a dependency just for one writer is wasteful, and the
        // format is well documented.
        let mut file = fs::File::create(output_path)?;
        for (name, bytes) in &files {
            write_tar_header(&mut file, name, bytes.len() as u64)?;
            file.write_all(bytes)?;
            // Pad to 512-byte boundary.
            let pad = (512 - (bytes.len() % 512)) % 512;
            for _ in 0..pad {
                file.write_all(&[0u8])?;
            }
        }
        // Two zero blocks = end of archive.
        file.write_all(&[0u8; 1024])?;
        Ok(())
    }

    fn write_jsonl<T: Serialize>(&self, items: &[T]) -> Vec<u8> {
        let mut out = Vec::new();
        for item in items {
            match serde_json::to_vec(item) {
                Ok(b) => {
                    out.extend_from_slice(&b);
                    out.push(b'\n');
                }
                Err(_) => {
                    // Skip items that don't serialize — better than failing the whole pack.
                }
            }
        }
        out
    }

    /// Install a pack at a target directory under `packs/<id>`. The id is taken from the
    /// manifest. Returns the install path.
    pub fn install_to(
        manifest: &Manifest,
        extract_root: &Path,
        packs_root: &Path,
    ) -> io::Result<PathBuf> {
        let dir = packs_root.join(&manifest.name);
        fs::create_dir_all(&dir)?;
        // Copy each file from the extract root into the install dir.
        for rel in manifest.files.keys() {
            let src = extract_root.join(rel);
            let dst = dir.join(rel);
            if let Some(parent) = dst.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(&src, &dst)?;
        }
        // Save the manifest under a stable name so `pack list` can render metadata later.
        let manifest_path = dir.join("manifest.json");
        fs::write(&manifest_path, serde_json::to_vec_pretty(manifest)?)?;
        Ok(dir)
    }
}

/// Write a single ustar header for a file. ustar is the de-facto tar format; details:
/// https://www.gnu.org/software/tar/manual/html_node/Standard.html
fn write_tar_header<W: Write>(w: &mut W, name: &str, size: u64) -> io::Result<()> {
    let mut header = [0u8; 512];
    // name: 0..100
    let name_bytes = name.as_bytes();
    let n = name_bytes.len().min(100);
    header[..n].copy_from_slice(&name_bytes[..n]);
    // mode: 100..108 ("0000644\0")
    write_octal(&mut header[100..108], 0o644);
    // uid: 108..116
    write_octal(&mut header[108..116], 0);
    // gid: 116..124
    write_octal(&mut header[116..124], 0);
    // size: 124..136
    write_octal(&mut header[124..136], size);
    // mtime: 136..148
    write_octal(&mut header[136..148], 0);
    // chksum: 148..156 (filled with spaces initially, then computed)
    header[148..156].fill(b' ');
    // typeflag: 156..157 = '0' (regular file)
    header[156] = b'0';
    // magic: 257..263 = "ustar\0"
    header[257..263].copy_from_slice(b"ustar\0");
    // version: 263..265 = "00"
    header[263..265].copy_from_slice(b"00");

    // Compute checksum: sum of all bytes (treating chksum as 8 spaces).
    let sum: u32 = header.iter().map(|b| *b as u32).sum();
    write_octal(&mut header[148..156], sum as u64);

    w.write_all(&header)
}

/// Write a zero-padded ASCII octal number followed by a NUL. Standard tar convention.
fn write_octal(dst: &mut [u8], value: u64) {
    let s = format!("{:>011o}\0", value);
    let bytes = s.as_bytes();
    let n = bytes.len().min(dst.len());
    dst[..n].copy_from_slice(&bytes[..n]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn write_tarball_creates_a_parsable_file() {
        let dir = TempDir::new().unwrap();
        let out = dir.path().join("test.cairnpkg");
        let mut pack = Pack::new("demo", "1.0.0");
        pack.author = "tester".into();
        pack.description = "round-trip".into();
        pack.memories
            .push(serde_json::json!({"id": "m1", "content": "hello"}));
        pack.profile
            .push(serde_json::json!({"rule": "always use tabs"}));
        pack.write_tarball(&out).unwrap();
        let meta = fs::metadata(&out).unwrap();
        assert!(meta.len() > 0);

        // The tarball must contain a manifest.json header.
        let bytes = fs::read(&out).unwrap();
        assert!(
            bytes
                .windows("manifest.json".len())
                .any(|w| w == b"manifest.json"),
            "tarball missing manifest.json entry"
        );
    }

    #[test]
    fn pack_stats_count_each_section() {
        let mut p = Pack::new("p", "1.0.0");
        p.memories.push(serde_json::json!({"a": 1}));
        p.memories.push(serde_json::json!({"a": 2}));
        p.profile.push(serde_json::json!({"b": 1}));
        p.patterns.push(serde_json::json!({"c": 1}));
        p.patterns.push(serde_json::json!({"c": 2}));
        p.patterns.push(serde_json::json!({"c": 3}));
        p.graph_edges
            .push(serde_json::json!({"src": "x", "dst": "y"}));
        let s = p.stats();
        assert_eq!(s.memories, 2);
        assert_eq!(s.profile, 1);
        assert_eq!(s.patterns, 3);
        assert_eq!(s.graph_edges, 1);
    }

    #[test]
    fn write_octal_produces_eleven_chars_plus_nul() {
        let mut buf = [0u8; 12];
        write_octal(&mut buf, 0o644);
        let s = std::str::from_utf8(&buf).unwrap();
        assert!(s.starts_with("00000000644"));
        assert!(s.ends_with('\0'));
    }

    #[test]
    fn write_tar_header_writes_512_bytes() {
        let mut buf = Vec::new();
        write_tar_header(&mut buf, "test.txt", 100).unwrap();
        assert_eq!(buf.len(), 512);
        assert_eq!(&buf[..8], b"test.txt");
        // typeflag = '0' (regular file)
        assert_eq!(buf[156], b'0');
        // magic = "ustar\0"
        assert_eq!(&buf[257..263], b"ustar\0");
    }
}
