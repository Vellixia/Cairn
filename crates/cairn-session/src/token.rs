//! Resume tokens: 256-bit random, delivered once, stored only as BLAKE3 hash
//! (research R8, Principle X). Raw tokens must never be logged or persisted.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use rand::RngCore;

/// Generate a fresh resume token (base64url, no padding) and its stored hash.
pub fn generate_resume_token() -> (String, String) {
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    let token = URL_SAFE_NO_PAD.encode(bytes);
    let hash = hash_resume_token(&token);
    (token, hash)
}

/// BLAKE3 hex of a presented token (constant-content comparison key).
pub fn hash_resume_token(token: &str) -> String {
    blake3::hash(token.as_bytes()).to_hex().to_string()
}

/// Verify a presented token against the stored hash (timing-safe compare).
pub fn verify_resume_token(presented: &str, stored_hash: &str) -> bool {
    let presented_hash = hash_resume_token(presented);
    // blake3 output is fixed-length hex; constant-time compare over bytes.
    let a = presented_hash.as_bytes();
    let b = stored_hash.as_bytes();
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}
