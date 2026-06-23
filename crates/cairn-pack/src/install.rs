//! Install a `.cairnpkg` tarball into the local store.
//!
//! The install path mirrors the lean-ctx design we adopted: hash-verify every file
//! against the manifest, extract to a temp dir, then promote to `packs/<name>/` under
//! `data_dir`. On any failure we roll back (delete the temp dir).
//!
//! Ingest into the cairn-store happens after extraction via the cairn-share
//! `ShareBundle` round-trip — that's what makes a `.cairnpkg` *Cairn*-shaped and not just
//! a generic tarball.

use std::{
    fs,
    io::{self},
    path::{Path, PathBuf},
};

use crate::manifest::{verify_extracted, Manifest};
use crate::signing::{self, PublicKey, VerifyError};
use crate::MAX_UNCOMPRESSED_BYTES;

/// Extract a `.cairnpkg` to `extract_root`. Returns the parsed manifest.
///
/// We use the [tar] crate's ustar reader when available; otherwise we hand-roll a small
/// extractor. The format is documented: 512-byte header per file, then data, then
/// 512-byte padding, then two zero blocks to mark EOF.
pub fn install(tarball: &Path, extract_root: &Path) -> io::Result<Manifest> {
    // Read the entire tarball up front (sized — refuse anything past MAX_UNCOMPRESSED_BYTES).
    let bytes = fs::read(tarball)?;
    if bytes.len() as u64 > MAX_UNCOMPRESSED_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "tarball {} bytes exceeds limit {}",
                bytes.len(),
                MAX_UNCOMPRESSED_BYTES
            ),
        ));
    }
    let entries = parse_tar(&bytes)?;
    if entries.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "tarball has no entries",
        ));
    }

    // Find the manifest — required.
    let manifest_entry = entries
        .iter()
        .find(|e| e.name == "manifest.json")
        .ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "tarball missing manifest.json")
        })?;
    let manifest: Manifest = serde_json::from_slice(&manifest_entry.body).map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("manifest not valid JSON: {e}"),
        )
    })?;

    // Atomic write: stage to a sibling temp dir, then rename on success.
    let staging = extract_root.join(format!(".pack-staging-{}", uuid::Uuid::new_v4().simple()));
    fs::create_dir_all(&staging)?;

    for entry in &entries {
        let dst = staging.join(&entry.name);
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent)?;
        }
        // Verify the entry's bytes against the manifest before writing — defense in depth
        // against a tampered tarball whose headers check out but whose content is wrong.
        if let Some(expected) = manifest.files.get(&entry.name) {
            let actual = signing::hash_bytes(&entry.body);
            if &actual != expected {
                let _ = fs::remove_dir_all(&staging);
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "hash mismatch for {}: expected {}, got {}",
                        entry.name, expected, actual
                    ),
                ));
            }
        }
        fs::write(&dst, &entry.body)?;
    }

    // Final pass: re-verify the entire extract against the manifest.
    if let Err(e) = verify_extracted(&manifest, &staging) {
        let _ = fs::remove_dir_all(&staging);
        return Err(io::Error::new(io::ErrorKind::InvalidData, e));
    }

    // Promote: rename the staging dir to its final name (within `extract_root`).
    let final_dir = extract_root.join(&manifest.name);
    if final_dir.exists() {
        // Replace prior install — cairnpkg semantics are "newer version wins".
        fs::remove_dir_all(&final_dir)?;
    }
    fs::rename(&staging, &final_dir)?;

    Ok(manifest)
}

/// One entry from a parsed tarball.
pub struct TarEntry {
    pub name: String,
    pub body: Vec<u8>,
}

/// Verify the Ed25519 signature on a parsed tarball, if present. Returns:
/// - `Ok(true)` — a signature was present and verified against one of the supplied keys.
/// - `Ok(false)` — the tarball has no `signature.ed25519` entry (legacy / unsigned pack).
///   Integrity is still ensured by the per-file SHA-256 hashes; only authenticity is missing.
/// - `Err(_)` — a signature was present but invalid, or referenced a key not in `trusted_keys`.
///
/// `trusted_keys` is the set of author public keys the local user has chosen to trust —
/// usually pinned once and stored alongside the registry config.
pub fn verify_ed25519_signature(
    entries: &[TarEntry],
    manifest_bytes: &[u8],
    trusted_keys: &[PublicKey],
) -> std::result::Result<bool, VerifyError> {
    let Some(sig_entry) = entries.iter().find(|e| e.name == "signature.ed25519") else {
        return Ok(false);
    };
    let sig_hex =
        std::str::from_utf8(&sig_entry.body).map_err(|_| VerifyError::MalformedSignature)?;
    for key in trusted_keys {
        if signing::verify_manifest_ed25519(manifest_bytes, sig_hex.trim(), key).is_ok() {
            return Ok(true);
        }
    }
    Err(VerifyError::Mismatch)
}

/// Minimal ustar parser. Handles plain regular files; everything else is treated as a
/// zero-length entry and skipped (we don't need symlinks / dirs / pax extensions for
/// `.cairnpkg`).
pub fn parse_tar(bytes: &[u8]) -> io::Result<Vec<TarEntry>> {
    if bytes.len() < 1024 {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "tarball shorter than minimum (1024 bytes)",
        ));
    }
    let mut out = Vec::new();
    let mut cursor = 0usize;
    while cursor + 512 <= bytes.len() {
        let header = &bytes[cursor..cursor + 512];
        // All-zero block = end of archive.
        if header.iter().all(|b| *b == 0) {
            break;
        }
        let name = read_name(header);
        if name.is_empty() {
            // Probably a zero-padded block; stop.
            break;
        }
        let size = read_octal(&header[124..136]) as usize;
        let typeflag = header[156];
        cursor += 512;
        if typeflag != b'0' && typeflag != 0 {
            // Skip non-regular entries (we don't emit any in our own writer).
            let padded = size.div_ceil(512) * 512;
            cursor += padded;
            continue;
        }
        if cursor + size > bytes.len() {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                format!("entry {name} truncated"),
            ));
        }
        let body = bytes[cursor..cursor + size].to_vec();
        let padded = size.div_ceil(512) * 512;
        cursor += padded;
        if !name.is_empty() {
            out.push(TarEntry { name, body });
        }
    }
    Ok(out)
}

fn read_name(header: &[u8]) -> String {
    let end = header[..100].iter().position(|b| *b == 0).unwrap_or(100);
    String::from_utf8_lossy(&header[..end]).into_owned()
}

fn read_octal(slice: &[u8]) -> u64 {
    let end = slice
        .iter()
        .position(|b| *b == 0 || *b == b' ')
        .unwrap_or(slice.len());
    std::str::from_utf8(&slice[..end])
        .ok()
        .and_then(|s| u64::from_str_radix(s.trim(), 8).ok())
        .unwrap_or(0)
}

/// The path where packs are installed under a given data dir. Convention: `<data_dir>/packs`.
pub fn packs_root(data_dir: &Path) -> PathBuf {
    data_dir.join("packs")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pack::Pack;
    use tempfile::TempDir;

    #[test]
    fn install_extracts_and_verifies_a_round_trip_pack() {
        let dir = TempDir::new().unwrap();
        let tarball = dir.path().join("demo.cairnpkg");
        let mut pack = Pack::new("demo", "1.0.0");
        pack.author = "tester".into();
        pack.description = "round-trip install".into();
        pack.memories
            .push(serde_json::json!({"id": "m1", "content": "alpha"}));
        pack.memories
            .push(serde_json::json!({"id": "m2", "content": "beta"}));
        pack.write_tarball(&tarball).unwrap();

        let extract = dir.path().join("extract");
        fs::create_dir_all(&extract).unwrap();
        let m = install(&tarball, &extract).unwrap();
        assert_eq!(m.name, "demo");
        assert_eq!(m.stats.memories, 2);

        // The extracted dir contains the canonical files.
        let installed = extract.join("demo");
        assert!(installed.join("manifest.json").exists());
        assert!(installed.join("memory.jsonl").exists());
        assert!(installed.join("signature.sha256").exists());

        // Reinstall replaces the prior install.
        let m2 = install(&tarball, &extract).unwrap();
        assert_eq!(m2.id, m.id);
    }

    #[test]
    fn install_rejects_oversized_tarball() {
        let dir = TempDir::new().unwrap();
        let tarball = dir.path().join("big.cairnpkg");
        // Make a tarball > MAX_UNCOMPRESSED_BYTES. We do this by padding memory.jsonl.
        let mut pack = Pack::new("big", "1.0.0");
        // 4 MiB per entry; 5 entries → 20 MiB.
        for _ in 0..5 {
            let big = "x".repeat(4 * 1024 * 1024);
            pack.memories.push(serde_json::json!({"content": big}));
        }
        pack.write_tarball(&tarball).unwrap();

        let extract = dir.path().join("extract");
        fs::create_dir_all(&extract).unwrap();
        let r = install(&tarball, &extract);
        assert!(r.is_err(), "expected oversized tarball to be rejected");
    }

    #[test]
    fn install_rejects_tampered_tarball() {
        let dir = TempDir::new().unwrap();
        let tarball = dir.path().join("demo.cairnpkg");
        let mut pack = Pack::new("demo", "1.0.0");
        pack.memories
            .push(serde_json::json!({"id": "m1", "content": "original"}));
        pack.write_tarball(&tarball).unwrap();

        // Flip a byte in the middle of the tarball to simulate a corrupted download.
        let mut bytes = fs::read(&tarball).unwrap();
        let middle = bytes.len() / 2;
        bytes[middle] ^= 0x01;
        fs::write(&tarball, &bytes).unwrap();

        let extract = dir.path().join("extract");
        fs::create_dir_all(&extract).unwrap();
        let r = install(&tarball, &extract);
        assert!(r.is_err(), "expected tampered tarball to be rejected");
    }
}
