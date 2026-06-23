//! Optional end-to-end encryption for sync envelopes (v0.5.0 Sprint 15b).
//!
//! When a user opts in to E2E encryption, every [`SyncEnvelope`] they send is encrypted
//! with a key derived from their passphrase via **Argon2id** (the OWASP-recommended
//! password hashing function). The peer never sees the passphrase, only the ciphertext
//! + the salt + the algorithm parameters needed to derive the same key.
//!
//! Threat model covered:
//!
//! - **Passive observer** (compromised log aggregator, MitM who only records traffic):
//!   sees ciphertext + salt + nonce but cannot derive the key without the passphrase.
//! - **Active MitM** swapping envelopes for their own: the receiver's key derivation
//!   fails because the passphrase is wrong, OR the AEAD authentication tag rejects
//!   the forgery.
//!
//! Threat model NOT covered (deliberately out of scope for v0.5.0):
//!
//! - **Compromised endpoint**: if an attacker has the user's device, they have the
//!   passphrase + plaintext. Argon2id can't help.
//! - **Forward secrecy**: a single key encrypts every envelope; rotating keys
//!   requires re-encrypting everything. A future iteration can layer an ephemeral
//!   ECDH exchange on top for per-session keys.
//! - **Deniability**: envelope headers are signed-with-passphrase not signed-as-someone.
//!   A peer can prove the sender had the passphrase but not which user. v0.6 ADR.

use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};

/// Magic string + version prefix for our envelope header. Lets us evolve the format
/// without breaking older peers.
const HEADER_MAGIC: &str = "cairn-sync-e2e";
const HEADER_VERSION: u8 = 1;

/// Ciphertext envelope as it appears on the wire. The `header` is plaintext — it
/// carries the algorithm name, salt, and nonce — and is followed by the encrypted
/// payload + the AEAD authentication tag.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncryptedEnvelope {
    pub header: Header,
    /// ChaCha20-Poly1305 ciphertext + tag (appended).
    pub ciphertext: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Header {
    pub magic: String,
    pub version: u8,
    pub kdf: String,
    pub kdf_params: KdfParams,
    /// 16-byte random salt; mixed into the Argon2id derivation.
    pub salt: Vec<u8>,
    /// 12-byte random nonce for ChaCha20-Poly1305.
    pub nonce: Vec<u8>,
    /// Optional AAD (associated data) — bound to the ciphertext so the receiver
    /// can't be tricked into decrypting an envelope intended for a different peer.
    /// In v0.5.0 we use `from + to` actor names.
    pub aad: Option<String>,
}

/// Argon2id parameters. Memory cost is the OWASP-recommended minimum for interactive
/// use (64 MiB); time cost is 3 iterations; parallelism = 1.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KdfParams {
    pub m_cost_kib: u32,
    pub t_cost: u32,
    pub p_cost: u32,
}

impl Default for KdfParams {
    fn default() -> Self {
        Self {
            m_cost_kib: 64 * 1024,
            t_cost: 3,
            p_cost: 1,
        }
    }
}

impl KdfParams {
    /// Clamp peer-supplied KDF parameters to the OWASP-recommended minimums before handing
    /// them to `argon2::Params::new`. Without this, a malicious peer could ship
    /// `m_cost_kib=1, t_cost=1, p_cost=1` in the envelope header and coerce us into a
    /// 4 KiB / 1-iteration derivation — fast enough to be a denial-of-service vector even
    /// though the AEAD tag still rejects forgery. The `t_cost >= 3` floor matches the
    /// minimum we use ourselves; the `p_cost <= 4` cap blocks attackers from spinning up
    /// unbounded argon2 threads.
    fn to_argon2(&self) -> Result<Params, CryptoError> {
        const MIN_M_COST_KIB: u32 = 64 * 1024;
        const MIN_T_COST: u32 = 3;
        const MAX_P_COST: u32 = 4;
        if self.m_cost_kib < MIN_M_COST_KIB {
            return Err(CryptoError::Argon2(format!(
                "m_cost_kib {} below minimum {}",
                self.m_cost_kib, MIN_M_COST_KIB
            )));
        }
        if self.t_cost < MIN_T_COST {
            return Err(CryptoError::Argon2(format!(
                "t_cost {} below minimum {}",
                self.t_cost, MIN_T_COST
            )));
        }
        if self.p_cost < 1 || self.p_cost > MAX_P_COST {
            return Err(CryptoError::Argon2(format!(
                "p_cost {} outside [1, {}]",
                self.p_cost, MAX_P_COST
            )));
        }
        Params::new(self.m_cost_kib, self.t_cost, self.p_cost, Some(32))
            .map_err(|e| CryptoError::Argon2(e.to_string()))
    }
}

/// Errors the crypto layer can return.
#[derive(Debug, thiserror::Error)]
pub enum CryptoError {
    #[error("argon2: {0}")]
    Argon2(String),
    #[error("AEAD: {0}")]
    Aead(String),
    #[error("malformed envelope: {0}")]
    Malformed(String),
    #[error("envelope version mismatch (got {got}, expected {expected})")]
    VersionMismatch { got: u8, expected: u8 },
    #[error("unknown kdf: {0}")]
    UnknownKdf(String),
    #[error("invalid header magic: {0:?}")]
    BadMagic(String),
}

/// Derive a 32-byte ChaCha20-Poly1305 key from the user's passphrase using Argon2id.
fn derive_key(passphrase: &[u8], salt: &[u8], params: &KdfParams) -> Result<[u8; 32], CryptoError> {
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params.to_argon2()?);
    let mut out = [0u8; 32];
    argon
        .hash_password_into(passphrase, salt, &mut out)
        .map_err(|e| CryptoError::Argon2(e.to_string()))?;
    Ok(out)
}

/// Encrypt a sync envelope's body. The header is plaintext; the body (the actual
/// `SyncEnvelope` JSON) is encrypted with the passphrase-derived key.
pub fn encrypt_envelope(
    envelope_bytes: &[u8],
    passphrase: &[u8],
    aad: Option<&str>,
) -> Result<EncryptedEnvelope, CryptoError> {
    let params = KdfParams::default();
    let mut salt = [0u8; 16];
    OsRng.fill_bytes(&mut salt);
    let mut nonce_bytes = [0u8; 12];
    OsRng.fill_bytes(&mut nonce_bytes);

    let key = derive_key(passphrase, &salt, &params)?;
    let cipher = ChaCha20Poly1305::new(Key::from_slice(&key));
    let nonce = Nonce::from_slice(&nonce_bytes);
    let payload = Payload {
        msg: envelope_bytes,
        aad: aad.map(str::as_bytes).unwrap_or(&[]),
    };
    let ciphertext = cipher
        .encrypt(nonce, payload)
        .map_err(|e| CryptoError::Aead(e.to_string()))?;

    Ok(EncryptedEnvelope {
        header: Header {
            magic: HEADER_MAGIC.to_string(),
            version: HEADER_VERSION,
            kdf: "argon2id".to_string(),
            kdf_params: params,
            salt: salt.to_vec(),
            nonce: nonce_bytes.to_vec(),
            aad: aad.map(str::to_string),
        },
        ciphertext,
    })
}

/// Decrypt a previously-encrypted envelope. Returns the plaintext `SyncEnvelope`
/// bytes — the caller is responsible for deserializing them.
pub fn decrypt_envelope(
    env: &EncryptedEnvelope,
    passphrase: &[u8],
    expected_aad: Option<&str>,
) -> Result<Vec<u8>, CryptoError> {
    if env.header.magic != HEADER_MAGIC {
        return Err(CryptoError::BadMagic(env.header.magic.clone()));
    }
    if env.header.version != HEADER_VERSION {
        return Err(CryptoError::VersionMismatch {
            got: env.header.version,
            expected: HEADER_VERSION,
        });
    }
    if env.header.kdf != "argon2id" {
        return Err(CryptoError::UnknownKdf(env.header.kdf.clone()));
    }
    if env.header.aad.as_deref() != expected_aad {
        return Err(CryptoError::Malformed(format!(
            "AAD mismatch: envelope says {:?}, caller expected {:?}",
            env.header.aad, expected_aad
        )));
    }
    if env.header.salt.len() != 16 {
        return Err(CryptoError::Malformed("salt must be 16 bytes".into()));
    }
    if env.header.nonce.len() != 12 {
        return Err(CryptoError::Malformed("nonce must be 12 bytes".into()));
    }
    let key = derive_key(passphrase, &env.header.salt, &env.header.kdf_params)?;
    let cipher = ChaCha20Poly1305::new(Key::from_slice(&key));
    let nonce = Nonce::from_slice(&env.header.nonce);
    let payload = Payload {
        msg: &env.ciphertext,
        aad: expected_aad.map(str::as_bytes).unwrap_or(&[]),
    };
    cipher
        .decrypt(nonce, payload)
        .map_err(|e| CryptoError::Aead(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encrypt_then_decrypt_round_trips() {
        let plaintext = b"hello, cairn";
        let pw = b"correct horse battery staple";
        let env = encrypt_envelope(plaintext, pw, Some("alice->bob")).unwrap();
        let back = decrypt_envelope(&env, pw, Some("alice->bob")).unwrap();
        assert_eq!(back, plaintext);
    }

    #[test]
    fn wrong_passphrase_fails_to_decrypt() {
        let plaintext = b"top secret memory";
        let env = encrypt_envelope(plaintext, b"right passphrase", Some("a->b")).unwrap();
        let err = decrypt_envelope(&env, b"wrong passphrase", Some("a->b"))
            .expect_err("decryption must fail");
        // Argon2 itself succeeds (it always produces a key), but the AEAD
        // authentication fails because the wrong key produces garbage plaintext.
        match err {
            CryptoError::Aead(_) => {}
            other => panic!("expected Aead error, got {other:?}"),
        }
    }

    #[test]
    fn wrong_aad_fails_to_decrypt() {
        let plaintext = b"memory for alice";
        let env = encrypt_envelope(plaintext, b"pw", Some("alice->bob")).unwrap();
        let err =
            decrypt_envelope(&env, b"pw", Some("bob->alice")).expect_err("wrong AAD must reject");
        // The AAD check happens before AEAD; we expect Malformed here.
        match err {
            CryptoError::Malformed(_) => {}
            other => panic!("expected Malformed, got {other:?}"),
        }
    }

    #[test]
    fn tampered_ciphertext_is_rejected() {
        let plaintext = b"trust me";
        let pw = b"pw";
        let mut env = encrypt_envelope(plaintext, pw, None).unwrap();
        // Flip a bit in the middle of the ciphertext.
        let mid = env.ciphertext.len() / 2;
        env.ciphertext[mid] ^= 0x80;
        let err = decrypt_envelope(&env, pw, None).expect_err("tampered must fail");
        assert!(matches!(err, CryptoError::Aead(_)), "got {err:?}");
    }

    #[test]
    fn bad_magic_is_rejected() {
        let mut env = encrypt_envelope(b"x", b"pw", None).unwrap();
        env.header.magic = "not cairn".into();
        let err = decrypt_envelope(&env, b"pw", None).expect_err("bad magic must fail");
        assert!(matches!(err, CryptoError::BadMagic(_)));
    }

    #[test]
    fn version_mismatch_is_rejected() {
        let mut env = encrypt_envelope(b"x", b"pw", None).unwrap();
        env.header.version = 99;
        let err = decrypt_envelope(&env, b"pw", None).expect_err("bad version must fail");
        assert!(matches!(err, CryptoError::VersionMismatch { .. }));
    }

    /// Pre-fix regression: a malicious peer could ship `m_cost_kib=1, t_cost=1, p_cost=1`
    /// in the envelope header and coerce the receiver into a 4 KiB / 1-iteration Argon2id
    /// derivation. The receiver's AEAD would still reject forgery, but the cheap
    /// derivation is a denial-of-service vector — an attacker can force us to spend
    /// negligible CPU per envelope. The fix clamps `KdfParams` to OWASP-recommended
    /// minimums before calling `Params::new`.
    #[test]
    fn kdf_params_below_minimum_are_rejected_on_decrypt() {
        let mut env = encrypt_envelope(b"x", b"pw", None).unwrap();
        // m_cost_kib below the 64 MiB floor
        env.header.kdf_params = KdfParams {
            m_cost_kib: 1,
            t_cost: 3,
            p_cost: 1,
        };
        let err = decrypt_envelope(&env, b"pw", None).expect_err("weak m_cost must reject");
        assert!(matches!(err, CryptoError::Argon2(_)), "got {err:?}");

        // t_cost below the 3-iteration floor
        env.header.kdf_params = KdfParams {
            m_cost_kib: 64 * 1024,
            t_cost: 1,
            p_cost: 1,
        };
        let err = decrypt_envelope(&env, b"pw", None).expect_err("weak t_cost must reject");
        assert!(matches!(err, CryptoError::Argon2(_)), "got {err:?}");

        // p_cost above the 4-thread cap
        env.header.kdf_params = KdfParams {
            m_cost_kib: 64 * 1024,
            t_cost: 3,
            p_cost: 64,
        };
        let err = decrypt_envelope(&env, b"pw", None).expect_err("excessive p_cost must reject");
        assert!(matches!(err, CryptoError::Argon2(_)), "got {err:?}");
    }

    #[test]
    fn kdf_params_at_default_still_round_trip() {
        // Sanity-check that the default params (which the clamp accepts) still produce
        // a valid round-trip after the clamp was introduced. encrypt_envelope always
        // writes the default, so this should be a no-op for the happy path.
        let plaintext = b"kdf clamp ok";
        let env = encrypt_envelope(plaintext, b"pw", None).unwrap();
        assert_eq!(env.header.kdf_params.m_cost_kib, 64 * 1024);
        assert_eq!(env.header.kdf_params.t_cost, 3);
        assert_eq!(env.header.kdf_params.p_cost, 1);
        let back = decrypt_envelope(&env, b"pw", None).unwrap();
        assert_eq!(back, plaintext);
    }
}
