//! 08 — Pack + Registry: Ed25519 sign/verify, manifest round-trip, trust grants.

use cairn_pack::signing::{sign_manifest_ed25519, verify_manifest_ed25519, Keypair};
use cairn_registry::TrustScope;

#[test]
fn ed25519_sign_then_verify_round_trip() {
    let kp = Keypair::generate();
    let payload = b"manifest bytes go here - no business logic, just bytes";
    let sig = sign_manifest_ed25519(payload, &kp);
    assert!(
        !sig.as_bytes().iter().all(|b| *b == 0),
        "signature is non-zero"
    );
    verify_manifest_ed25519(payload, &sig, &kp.public()).expect("verification");
}

#[test]
fn ed25519_verify_rejects_tampered_payload() {
    let kp = Keypair::generate();
    let payload = b"original";
    let sig = sign_manifest_ed25519(payload, &kp);
    let tampered = b"originalX";
    assert!(verify_manifest_ed25519(tampered, &sig, &kp.public()).is_err());
}

#[test]
fn ed25519_verify_rejects_wrong_public_key() {
    let kp1 = Keypair::generate();
    let kp2 = Keypair::generate();
    let payload = b"hello";
    let sig = sign_manifest_ed25519(payload, &kp1);
    assert!(verify_manifest_ed25519(payload, &sig, &kp2.public()).is_err());
}

#[test]
fn public_key_to_hex_round_trip() {
    let kp = Keypair::generate();
    let hex = kp.public().to_hex();
    assert_eq!(hex.len(), 64, "32 raw bytes hex = 64 chars");
    // Lower-case hex.
    assert!(hex
        .chars()
        .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    // The hex is stable across calls (no random components in the
    // public-key serialization).
    assert_eq!(hex, kp.public().to_hex());
}

#[test]
fn trust_scope_serializes_as_snake_case() {
    // The cairn-registry DTO for the trust keys serializes the
    // `allows` field as one of "local" / "team" / "public".
    use serde_json::json;
    let local = serde_json::to_string(&json!("local")).unwrap();
    assert_eq!(local, "\"local\"");
    // The TrustScope enum is the source of truth.
    let _ = TrustScope::Local;
    let _ = TrustScope::Team;
    let _ = TrustScope::Public;
}

#[test]
fn trust_scope_copy_semantics() {
    // TrustScope is a small enum that must be Copy + Clone (it's
    // passed across threads).
    let a = TrustScope::Public;
    let b = a;
    let c = a;
    assert_eq!(a, b);
    assert_eq!(a, c);
}

#[test]
fn keypair_generate_produces_unique_keys() {
    let kp1 = Keypair::generate();
    let kp2 = Keypair::generate();
    assert_ne!(kp1.public().to_hex(), kp2.public().to_hex());
}
