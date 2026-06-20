//! HMAC-style keyed signature for `.cairnpkg`. v0.5.0 ships SHA-256 over a canonical
//! representation; production deployments can swap in Ed25519 (see ADR-014) without
//! changing the on-disk format (the `signature.sha256` filename will just become
//! `signature.ed25519` when the signer changes).
//!
//! **Why this isn't true PKI yet:** Cairn packages are publicly shared between trusted
//! peers (e.g. a team sharing internal patterns). Trust is anchored in the upstream
//! registry URL the user installs from, not in a CA. A future iteration can layer on
//! cosign-style attestations for higher assurance.

use sha2::{Digest, Sha256};
use std::path::Path;

/// Hex-encoded SHA-256 of the file's bytes (lowercase).
pub fn hash_bytes(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    hex::encode(h.finalize())
}

/// Hex-encoded SHA-256 of a file's contents. Used when building the manifest's `files` map.
pub fn hash_file(path: &Path) -> std::io::Result<String> {
    let bytes = std::fs::read(path)?;
    Ok(hash_bytes(&bytes))
}

/// Deterministic hex SHA-256 signature over the manifest's canonical form. Today this is
/// just another hash; the `signature.sha256` filename leaves room to upgrade to a
/// keyed signature without an on-disk format change.
pub fn sign_manifest(manifest_bytes: &[u8]) -> String {
    hash_bytes(manifest_bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_bytes_is_deterministic_and_64_hex_chars() {
        let a = hash_bytes(b"hello");
        let b = hash_bytes(b"hello");
        assert_eq!(a, b);
        assert_eq!(a.len(), 64);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        // Known-answer test (sha256 of "hello").
        assert_eq!(
            a,
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }
}
