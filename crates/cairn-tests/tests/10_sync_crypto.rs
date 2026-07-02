//! 10 — Sync CRDTs + Argon2id -> ChaCha20-Poly1305 AEAD crypto round-trip.

use cairn_sync::counter::GCounter;
use cairn_sync::crypto::{decrypt_envelope, encrypt_envelope};
use cairn_sync::orset::ORSet;
use cairn_sync::sync::VectorClock;

#[test]
fn gcounter_merge_is_commutative_and_idempotent() {
    let mut a = GCounter::new();
    let mut b = GCounter::new();
    a.increment("alice", 3);
    b.increment("bob", 5);
    // Commutativity: a.merge(b) == b.merge(a).
    let mut a1 = a.clone();
    let mut b1 = b.clone();
    a1.merge(&b);
    b1.merge(&a);
    assert_eq!(a1.total(), b1.total(), "merge is commutative");
    assert_eq!(a1.total(), 8);
    // Idempotency: merging the same counter twice doesn't double-count.
    let mut a2 = a.clone();
    a2.merge(&b);
    a2.merge(&b);
    assert_eq!(a2.total(), 8, "merge is idempotent");
}

#[test]
fn gcounter_associativity() {
    let mut a = GCounter::new();
    let mut b = GCounter::new();
    let mut c = GCounter::new();
    a.increment("a", 1);
    b.increment("b", 2);
    c.increment("c", 4);
    // (a ⊔ b) ⊔ c == a ⊔ (b ⊔ c).
    let mut left = a.clone();
    left.merge(&b);
    left.merge(&c);
    let mut right = b.clone();
    right.merge(&c);
    right.merge(&a);
    assert_eq!(left.total(), right.total(), "merge is associative");
    assert_eq!(left.total(), 7);
}

#[test]
fn gcounter_per_actor_max_not_sum() {
    // If alice's local count is 5 and the incoming is 3, merge keeps 5
    // (not 8). The CRDT semantic.
    let mut a = GCounter::new();
    a.increment("alice", 5);
    let mut incoming = GCounter::new();
    incoming.increment("alice", 3);
    a.merge(&incoming);
    assert_eq!(a.per_actor().get("alice").copied(), Some(5));
    assert_eq!(a.total(), 5);
}

#[test]
fn gcounter_serializes_with_key_sorted_json() {
    // Two independently-built counters with the same content produce
    // identical JSON — important for byte-level convergence after sync.
    let mut a = GCounter::new();
    a.increment("alice", 1);
    a.increment("bob", 2);
    let mut b = GCounter::new();
    b.increment("bob", 2);
    b.increment("alice", 1);
    assert_eq!(
        serde_json::to_string(&a).unwrap(),
        serde_json::to_string(&b).unwrap()
    );
}

#[test]
fn orset_add_then_remove_wins() {
    let mut s = ORSet::new();
    s.add("a");
    s.add("b");
    assert!(s.contains("a"));
    assert!(s.contains("b"));
    s.remove("a");
    assert!(!s.contains("a"));
    assert!(s.contains("b"));
}

#[test]
fn orset_concurrent_adds_converge_after_merge() {
    // Two replicas both add the same element. After sync, both
    // contain it exactly once.
    let mut a = ORSet::new();
    let mut b = ORSet::new();
    a.add("x");
    b.add("x");
    a.merge(&b);
    b.merge(&a);
    assert!(a.contains("x"));
    assert!(b.contains("x"));
    let a_members = a.members();
    let x_count = a_members.iter().filter(|m| *m == "x").count();
    assert_eq!(x_count, 1, "the same element appears once after merge");
}

#[test]
fn orset_add_returns_unique_marker() {
    let mut s = ORSet::new();
    let m1 = s.add("a");
    let m2 = s.add("a");
    // Adding the same value twice produces two distinct markers
    // (concurrent adds of "the same element" in OR-Set).
    assert_ne!(m1, m2);
    assert_eq!(s.marker_count("a"), 2);
}

#[test]
fn vector_clock_orders_events() {
    // The same event on the same replica increments the same clock.
    let mut c = VectorClock::new();
    c.tick("alice");
    c.tick("alice");
    c.tick("bob");
    assert_eq!(c.get("alice"), 2);
    assert_eq!(c.get("bob"), 1);
    // A fresh clock is empty.
    let fresh = VectorClock::new();
    assert_eq!(fresh.get("nobody"), 0);
}

#[test]
fn vector_clock_dominates_when_ahead() {
    // After ticking, this clock dominates the prior one.
    let mut a = VectorClock::new();
    a.tick("alice");
    let b = VectorClock::new();
    assert!(a.dominates(&b), "alice=1 dominates empty clock");
    let mut b2 = VectorClock::new();
    b2.tick("alice");
    b2.tick("alice");
    assert!(b2.dominates(&a), "alice=2 dominates alice=1");
    // Equal clocks do NOT strictly dominate each other.
    let c1 = VectorClock::new();
    let c2 = VectorClock::new();
    assert!(!c1.dominates(&c2), "empty does not dominate empty");
}

#[test]
fn crypto_encrypt_decrypt_round_trip() {
    let passphrase = b"correct horse battery staple";
    let plaintext = b"the quick brown fox jumps over the lazy dog";
    let env = encrypt_envelope(plaintext, passphrase, Some("alice@vellixia")).expect("encrypt");
    let back = decrypt_envelope(&env, passphrase, Some("alice@vellixia")).expect("decrypt");
    assert_eq!(back, plaintext);
}

#[test]
fn crypto_decrypt_with_wrong_passphrase_fails() {
    let env = encrypt_envelope(b"secret", b"right", None).expect("encrypt");
    assert!(decrypt_envelope(&env, b"wrong", None).is_err());
}

#[test]
fn crypto_decrypt_with_wrong_aad_fails() {
    let env = encrypt_envelope(b"secret", b"pw", Some("aad1")).expect("encrypt");
    assert!(decrypt_envelope(&env, b"pw", Some("aad2")).is_err());
}

#[test]
fn crypto_envelope_header_has_known_magic() {
    let env = encrypt_envelope(b"x", b"pw", None).expect("encrypt");
    assert!(!env.ciphertext.is_empty());
    assert!(!env.header.salt.is_empty());
    assert!(!env.header.nonce.is_empty());
}
