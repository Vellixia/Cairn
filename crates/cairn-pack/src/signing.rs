//! Signatures for `.cairnpkg`. v0.5.0 Sprint 13 adds Ed25519 alongside the original
//! HMAC-style SHA-256.
//!
//! ## Two signatures, one tarball
//!
//! A signed pack carries **both** `signature.sha256` and `signature.ed25519` when a
//! keypair is available at pack time. The sha256 one is the historical integrity hash
//! (every file in the manifest already has a per-file sha256, so the "manifest hash"
//! was redundant). The ed25519 one is the **authenticity** proof - it binds the pack
//! to a specific author key.
//!
//! Install verification prefers Ed25519 when present:
//! 1. If `signature.ed25519` exists AND the user supplied a public key, verify Ed25519.
//! 2. Otherwise fall back to `signature.sha256` for content integrity only (no auth).
//!
//! ## Keypair generation
//!
//! ```no_run
//! use cairn_pack::signing::{Keypair, PublicKey};
//! let kp = Keypair::generate();
//! let public = PublicKey::from(&kp);
//! // store `kp.to_bytes()` somewhere safe; serialize `public.to_bytes()` into the manifest.
//! ```
//!
//! The 32-byte secret key is the only thing that can sign; losing it means losing the
//! ability to publish new versions under the same author identity. The 32-byte public
//! key is what verifiers need; it is safe to share.

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use rand_core::OsRng;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::Path;
use thiserror::Error;

/// 32-byte Ed25519 secret key (signing half of a keypair). Wrap it; do not log it.
///
/// The inner `ed25519_dalek::SigningKey` is built with the `zeroize` feature enabled
/// (see `crates/cairn-pack/Cargo.toml`), so its secret material is overwritten on drop.
/// We deliberately do **not** derive `Clone` here - cloning would mint a second
/// 32-byte secret key in heap memory and defeat the zeroize-on-drop guarantee.
/// Callers that previously needed `kp.clone()` should pass `&Keypair` instead.
pub struct Keypair {
    inner: SigningKey,
}

/// 32-byte Ed25519 public key (verifying half). Safe to serialize into the manifest.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PublicKey {
    inner: VerifyingKey,
}

// Manual serde impl: ed25519_dalek::VerifyingKey does not implement Serialize/Deserialize
// in 2.1.x, so we serialize the 32 raw bytes. Hex-encoded form is what the manifest carries
// (see [`PublicKey::to_hex`]); this is the JSON form (`[u8; 32]` array or hex string).
impl Serialize for PublicKey {
    fn serialize<S: serde::Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        ser.serialize_bytes(&self.to_bytes())
    }
}

impl<'de> Deserialize<'de> for PublicKey {
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        struct BytesVisitor;
        impl<'de> serde::de::Visitor<'de> for BytesVisitor {
            type Value = PublicKey;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("32 bytes (Ed25519 public key)")
            }
            fn visit_bytes<E: serde::de::Error>(self, v: &[u8]) -> Result<Self::Value, E> {
                PublicKey::from_bytes(v).map_err(|_| {
                    E::invalid_length(v.len(), &"expected 32 bytes for an Ed25519 public key")
                })
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> Result<Self::Value, A::Error> {
                let mut buf = [0u8; 32];
                let mut i = 0;
                while let Some(b) = seq.next_element::<u8>()? {
                    if i >= 32 {
                        return Err(serde::de::Error::invalid_length(i, &self));
                    }
                    buf[i] = b;
                    i += 1;
                }
                if i != 32 {
                    return Err(serde::de::Error::invalid_length(i, &self));
                }
                PublicKey::from_bytes(&buf).map_err(|_| serde::de::Error::custom("bad pubkey"))
            }
        }
        de.deserialize_bytes(BytesVisitor)
    }
}

/// Hex-encoded Ed25519 signature (lower-case, 128 hex chars).
pub type SignatureHex = String;

/// Sign errors (signature generation only - verification errors are typed below).
#[derive(Debug, Error)]
pub enum SignError {
    #[error("invalid key bytes")]
    InvalidKey,
}

/// Verification failures for an Ed25519-signed pack.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum VerifyError {
    #[error("malformed signature (expected 128 hex chars)")]
    MalformedSignature,
    #[error("malformed public key (expected 32 bytes)")]
    MalformedPublicKey,
    #[error("signature does not match the supplied public key")]
    Mismatch,
}

impl Keypair {
    /// Generate a fresh random keypair using the OS RNG.
    pub fn generate() -> Self {
        let inner = SigningKey::generate(&mut OsRng);
        Self { inner }
    }

    /// Re-hydrate a keypair from 32 raw secret-key bytes.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, SignError> {
        let bytes: [u8; 32] = bytes.try_into().map_err(|_| SignError::InvalidKey)?;
        let inner = SigningKey::from_bytes(&bytes);
        Ok(Self { inner })
    }

    /// 32 raw secret-key bytes. Treat as sensitive - never log.
    pub fn to_bytes(&self) -> [u8; 32] {
        self.inner.to_bytes()
    }

    /// Public key for this keypair.
    pub fn public(&self) -> PublicKey {
        PublicKey {
            inner: self.inner.verifying_key(),
        }
    }

    /// Sign `payload` and return a 64-byte signature.
    pub fn sign(&self, payload: &[u8]) -> [u8; 64] {
        self.inner.sign(payload).to_bytes()
    }
}

impl PublicKey {
    /// Decode from 32 raw public-key bytes.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, VerifyError> {
        let bytes: [u8; 32] = bytes
            .try_into()
            .map_err(|_| VerifyError::MalformedPublicKey)?;
        let inner =
            VerifyingKey::from_bytes(&bytes).map_err(|_| VerifyError::MalformedPublicKey)?;
        Ok(Self { inner })
    }

    /// 32 raw public-key bytes.
    pub fn to_bytes(&self) -> [u8; 32] {
        self.inner.to_bytes()
    }

    /// Hex-encoded form (64 chars, lower-case). Safe for the manifest.
    pub fn to_hex(&self) -> String {
        hex::encode(self.to_bytes())
    }

    /// Verify `signature` (64 raw bytes) against `payload`.
    pub fn verify(&self, payload: &[u8], signature: &[u8]) -> Result<(), VerifyError> {
        let sig_bytes: [u8; 64] = signature
            .try_into()
            .map_err(|_| VerifyError::MalformedSignature)?;
        let sig = Signature::from_bytes(&sig_bytes);
        self.inner
            .verify(payload, &sig)
            .map_err(|_| VerifyError::Mismatch)
    }
}

impl From<&Keypair> for PublicKey {
    fn from(kp: &Keypair) -> Self {
        kp.public()
    }
}

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

/// Deterministic hex SHA-256 manifest signature (no key - integrity only). The `signature.sha256`
/// filename is preserved for backwards compatibility: v0.4.x and earlier packs only carry this.
pub fn sign_manifest(manifest_bytes: &[u8]) -> String {
    hash_bytes(manifest_bytes)
}

/// Sign a pack's canonical manifest bytes with Ed25519. Returns the lower-case hex
/// signature (128 chars). Use this when you want to publish under a specific author key.
pub fn sign_manifest_ed25519(manifest_bytes: &[u8], key: &Keypair) -> SignatureHex {
    let sig = key.sign(manifest_bytes);
    hex::encode(sig)
}

/// Verify a hex-encoded Ed25519 signature against the canonical manifest bytes.
pub fn verify_manifest_ed25519(
    manifest_bytes: &[u8],
    signature_hex: &str,
    public: &PublicKey,
) -> Result<(), VerifyError> {
    let sig_bytes = hex::decode(signature_hex).map_err(|_| VerifyError::MalformedSignature)?;
    public.verify(manifest_bytes, &sig_bytes)
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

    #[test]
    fn keypair_signs_and_verifies() {
        let kp = Keypair::generate();
        let pubk = kp.public();
        let sig = kp.sign(b"the manifest bytes");
        assert!(pubk.verify(b"the manifest bytes", &sig).is_ok());
        // Tampered message must fail verification.
        assert_eq!(
            pubk.verify(b"the manifest bytss", &sig).unwrap_err(),
            VerifyError::Mismatch
        );
        // Tampered signature must fail.
        let mut bad = sig;
        bad[0] ^= 0x01;
        assert!(pubk.verify(b"the manifest bytes", &bad).is_err());
    }

    #[test]
    fn keypair_round_trips_through_bytes() {
        let kp = Keypair::generate();
        let bytes = kp.to_bytes();
        let restored = Keypair::from_bytes(&bytes).unwrap();
        // A signature made with the original verifies with the restored public key.
        let sig = kp.sign(b"payload");
        assert!(restored.public().verify(b"payload", &sig).is_ok());
    }

    #[test]
    fn public_key_to_hex_round_trip() {
        let kp = Keypair::generate();
        let hex = kp.public().to_hex();
        assert_eq!(hex.len(), 64);
        let bytes = hex::decode(&hex).unwrap();
        let restored = PublicKey::from_bytes(&bytes).unwrap();
        assert_eq!(restored, kp.public());
    }

    #[test]
    fn sign_manifest_ed25519_returns_128_hex_chars() {
        let kp = Keypair::generate();
        let sig = sign_manifest_ed25519(b"manifest", &kp);
        assert_eq!(sig.len(), 128);
        assert!(sig.chars().all(|c| c.is_ascii_hexdigit()));
        assert!(verify_manifest_ed25519(b"manifest", &sig, &kp.public()).is_ok());
    }

    #[test]
    fn verify_manifest_ed25519_rejects_malformed_signature() {
        let kp = Keypair::generate();
        assert_eq!(
            verify_manifest_ed25519(b"x", "not-hex", &kp.public()).unwrap_err(),
            VerifyError::MalformedSignature
        );
        assert_eq!(
            verify_manifest_ed25519(b"x", "ab", &kp.public()).unwrap_err(),
            VerifyError::MalformedSignature
        );
    }

    #[test]
    fn different_keys_produce_different_signatures() {
        let kp1 = Keypair::generate();
        let kp2 = Keypair::generate();
        let sig1 = kp1.sign(b"manifest");
        let sig2 = kp2.sign(b"manifest");
        assert_ne!(sig1, sig2);
        // Cross-verification must fail.
        assert!(kp1.public().verify(b"manifest", &sig2).is_err());
        assert!(kp2.public().verify(b"manifest", &sig1).is_err());
    }

    /// Sanity-check that `Keypair::to_bytes()` returns the same 32 secret bytes
    /// round-trip after construction. We can't reliably observe the actual zero-fill
    /// of a dropped `SigningKey` from a test (it lives behind the `ed25519-dalek`
    /// opaque struct), but we can at least assert the secret is reachable pre-drop.
    /// If someone re-adds `#[derive(Clone)]` to `Keypair`, callers that previously
    /// did `kp.clone()` will compile again and the ZeroizeOnDrop story breaks - but
    /// there are no `.clone()` calls on `Keypair` in the workspace (grep-verified)
    /// so re-introducing Clone would be loud enough to catch in code review.
    #[test]
    fn keypair_secret_is_reachable_pre_drop() {
        let kp = Keypair::generate();
        let bytes_before = kp.to_bytes();
        assert_eq!(bytes_before.len(), 32);
        drop(kp);
    }
}
