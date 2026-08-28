//! Two devices of one person, converging (T099, T110, T111; FR-492, FR-493,
//! FR-583, SC-411, SC-427, SC-455).
//!
//! Cairn is local-first, so two machines write independently while offline and
//! reconcile afterwards. Personal knowledge is the first content where that
//! matters across machines rather than across branches, and it brings two
//! specific hazards with it.
//!
//! The first is a silent winner. Two devices recording contradicting notes about
//! the same subject must end up with a *standing conflict* — the disagreement is
//! the information, and a rule that picked one would delete it. Crucially the
//! outcome must not depend on which device synchronized first or on whose clock
//! was ahead, because neither of those facts says anything about which note is
//! right.
//!
//! The second is a silent loss. Two devices that independently produce a
//! byte-identical payload must both land. Before `writer_id` joined the
//! idempotency key, the second device's write collided at the transport layer and
//! was reported a duplicate — never reaching reconciliation, which is the only
//! thing entitled to decide two records say the same thing.
//!
//! Driven against `cairn-store` and `cairn-core` directly. Two devices are two
//! stores, and a merge is what a pull does; a server adds nothing to any of these
//! claims and would make each test an order of magnitude slower.

use cairn_core::domain::Reconciliation;
use cairn_core::domain::{new_id, MemoryType};
use cairn_store::global::{
    create_personal, merge_synced_personal, personal_subject, personal_writer_gaps,
    NewPersonalKnowledge, SyncedPersonalKnowledge,
};
use cairn_store::Store;
use chrono::{Duration, Utc};
use uuid::Uuid;

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime")
}

/// A device: its own store, with its own `writer_identity`.
async fn device() -> Store {
    Store::open_memory().await.unwrap()
}

/// Write one personal note on `store`, and return what a pull of it would carry.
///
/// Reading the row back rather than reconstructing the payload, so what travels
/// here is what the row actually holds — including the `writer_id` this store
/// minted, which is the field every assertion below turns on.
async fn write_and_read(
    store: &Store,
    owner: Uuid,
    content: &str,
    topic_key: Option<&str>,
    value_key: Option<&str>,
) -> SyncedPersonalKnowledge {
    let outcome = create_personal(
        store,
        NewPersonalKnowledge::direct(
            owner,
            MemoryType::Fact,
            content,
            topic_key,
            value_key,
            Vec::new(),
        ),
        &[],
    )
    .await
    .expect("create");
    let r = outcome.record;
    SyncedPersonalKnowledge {
        id: r.id,
        owner_user_id: owner,
        knowledge_type: r.knowledge_type,
        content: r.content,
        topic_key: r.topic_key,
        value_key: r.value_key,
        applicability: r.applicability,
        writer_id: writer_of(store, r.id).await,
        writer_seq: seq_of(store, r.id).await,
        created_at: r.created_at,
        superseded_by_id: None,
        forgotten_at: None,
    }
}

async fn writer_of(store: &Store, id: Uuid) -> Uuid {
    let raw: String = sqlx::query_scalar("SELECT writer_id FROM personal_knowledge WHERE id = ?1")
        .bind(id.to_string())
        .fetch_one(store.pool())
        .await
        .unwrap();
    Uuid::parse_str(&raw).unwrap()
}

async fn seq_of(store: &Store, id: Uuid) -> i64 {
    sqlx::query_scalar("SELECT writer_seq FROM personal_knowledge WHERE id = ?1")
        .bind(id.to_string())
        .fetch_one(store.pool())
        .await
        .unwrap()
}

// ---------------------------------------------------------------------------
// T110 / FR-493 / SC-411 — a standing conflict, whatever the order or the clock
// ---------------------------------------------------------------------------

/// Two devices, both offline, record contradicting personal knowledge. On
/// reconnection the result is a standing conflict — in **both** synchronization
/// orders and under clock skew in **both** directions.
///
/// Exercising one order would prove nothing about ordering: an implementation
/// that always preferred whichever record arrived second would pass it. Exercising
/// one clock direction would likewise pass on an implementation that sorted by
/// `created_at`, which is precisely why `MemoryFacts` has no such field.
///
/// Falsified by adding any ordering rule to `derive_subject` — a timestamp
/// comparison, a write-order comparison, or a `writer_seq` tiebreak.
#[test]
fn two_devices_recording_contradictions_converge_on_a_standing_conflict() {
    runtime().block_on(async {
        let owner = new_id();
        // (later-of-the-two, description) — the skew is applied to whichever
        // record the case says should look newer.
        for skew_favours_a in [true, false] {
            for a_arrives_first in [true, false] {
                let device_a = device().await;
                let device_b = device().await;

                let mut from_a = write_and_read(
                    &device_a,
                    owner,
                    "the retry budget is four attempts",
                    Some("retry.budget"),
                    Some("four"),
                )
                .await;
                let mut from_b = write_and_read(
                    &device_b,
                    owner,
                    "the retry budget is two attempts",
                    Some("retry.budget"),
                    Some("two"),
                )
                .await;

                // Clock skew, applied deliberately and in both directions. Two
                // machines' clocks disagree in practice, and a merge rule that
                // consulted them would be right by luck.
                let later = Utc::now() + Duration::hours(1);
                let earlier = Utc::now() - Duration::hours(1);
                if skew_favours_a {
                    from_a.created_at = later;
                    from_b.created_at = earlier;
                } else {
                    from_a.created_at = earlier;
                    from_b.created_at = later;
                }

                // A third store receives both, in the order under test.
                let convergence = device().await;
                let (first, second) = if a_arrives_first {
                    (from_a.clone_for_merge(), from_b.clone_for_merge())
                } else {
                    (from_b.clone_for_merge(), from_a.clone_for_merge())
                };
                merge_synced_personal(&convergence, first)
                    .await
                    .expect("first");
                merge_synced_personal(&convergence, second)
                    .await
                    .expect("second");

                let subject = personal_subject(&convergence, owner, "retry.budget")
                    .await
                    .expect("subject");
                assert_eq!(
                    subject.members.len(),
                    2,
                    "skew_favours_a={skew_favours_a} a_arrives_first={a_arrives_first}: \
                     one of two contradicting records was dropped"
                );
                assert!(
                    matches!(subject.view.reconciliation, Reconciliation::Conflicted),
                    "skew_favours_a={skew_favours_a} a_arrives_first={a_arrives_first}: \
                     expected a standing conflict, got {:?}",
                    subject.view.reconciliation
                );
                assert_eq!(
                    subject.view.answers.len(),
                    2,
                    "the conflict did not surface both answers"
                );
            }
        }
    });
}

// ---------------------------------------------------------------------------
// T111 / SC-427 — a byte-identical payload from two devices lands twice
// ---------------------------------------------------------------------------

/// Two devices emitting a byte-identical payload both land, rather than one being
/// discarded as a duplicate of the other's write.
///
/// The two halves are different claims and both are asserted. First, the
/// transport must not collapse them: `writer_id` joins the idempotency key, so
/// two stores producing the same content compute two distinct keys. Second,
/// reconciliation must then see them and call them duplicates — which is a
/// decision, recorded as a relation, and not at all the same outcome as one write
/// vanishing in transit with nothing recorded anywhere.
///
/// Falsified by removing `writer_id` from the key computation in
/// `outbox::enqueue_global`.
#[test]
fn two_devices_emitting_the_same_payload_both_land_and_reconcile() {
    runtime().block_on(async {
        let owner = new_id();
        let content = "prefer the workspace lockfile over a per-crate one";

        let device_a = device().await;
        let device_b = device().await;

        // Both lanes established, so both writes are actually queued and the
        // keys exist to be compared.
        let instance = Uuid::now_v7();
        let namespace = cairn_core::domain::SyncNamespace::Personal(instance, owner);
        for store in [&device_a, &device_b] {
            cairn_store::cursor::establish(store, &namespace)
                .await
                .unwrap();
        }

        let from_a = write_and_read(
            &device_a,
            owner,
            content,
            Some("build.lockfile"),
            Some("workspace"),
        )
        .await;
        let from_b = write_and_read(
            &device_b,
            owner,
            content,
            Some("build.lockfile"),
            Some("workspace"),
        )
        .await;

        assert_ne!(
            from_a.writer_id, from_b.writer_id,
            "two stores minted the same writer identity, so this test cannot \
             distinguish the case it is about"
        );

        let key_a: String = sqlx::query_scalar("SELECT idempotency_key FROM outbox LIMIT 1")
            .fetch_one(device_a.pool())
            .await
            .unwrap();
        let key_b: String = sqlx::query_scalar("SELECT idempotency_key FROM outbox LIMIT 1")
            .fetch_one(device_b.pool())
            .await
            .unwrap();
        assert_ne!(
            key_a, key_b,
            "two devices computed one idempotency key for the same content, so the \
             second device's write would be discarded in transit as a duplicate"
        );

        // Both reach a third store, and both are kept.
        let convergence = device().await;
        merge_synced_personal(&convergence, from_a.clone_for_merge())
            .await
            .expect("a");
        merge_synced_personal(&convergence, from_b.clone_for_merge())
            .await
            .expect("b");
        let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM personal_knowledge")
            .fetch_one(convergence.pool())
            .await
            .unwrap();
        assert_eq!(rows, 2, "one of two identical writes was discarded");

        // And reconciliation — not the transport — is what noticed they agree.
        let subject = personal_subject(&convergence, owner, "build.lockfile")
            .await
            .expect("subject");
        assert!(
            matches!(
                subject.view.reconciliation,
                Reconciliation::Settled | Reconciliation::Reinforced | Reconciliation::Corroborated
            ),
            "two identical records did not reconcile as agreement: {:?}",
            subject.view.reconciliation
        );
    });
}

// ---------------------------------------------------------------------------
// T099 / FR-492 / FR-583 / SC-455 — `writer_seq` is inert, and gaps are reported
// ---------------------------------------------------------------------------

/// One corpus, replayed under permuted, withheld and renumbered `writer_seq`
/// values, produces byte-identical derived output every time.
///
/// This is the behavioural half of "`writer_seq` is diagnostic only, never a
/// tiebreak". The structural half is that `MemoryFacts` has no field for it, so a
/// tiebreak that consulted one would not compile — and the two halves are both
/// needed: the structural one cannot stop a *caller* from pre-sorting its input
/// by sequence before handing it over, and this one can.
///
/// Falsified by adding `writer_seq` to `MemoryFacts` and sorting on it, or by any
/// caller ordering members by sequence before deriving.
#[test]
fn replaying_one_corpus_under_permuted_sequences_derives_the_same_answer() {
    runtime().block_on(async {
        let owner = new_id();
        let source = device().await;

        // Three notes on one subject, two agreeing and one differing, so the
        // derived answer is something more interesting than a single member.
        let mut corpus = Vec::new();
        for (content, value) in [
            ("the retry budget is four attempts", "four"),
            ("retry budget: four attempts", "four"),
            ("the retry budget is two attempts", "two"),
        ] {
            corpus.push(
                write_and_read(&source, owner, content, Some("retry.budget"), Some(value)).await,
            );
        }

        // Case 1: as written. Case 2: sequences reversed. Case 3: renumbered
        // into a completely different range. Case 4: the middle sequence
        // withheld, so the stream has a hole in it.
        let cases: Vec<(&str, Vec<i64>)> = vec![
            ("as written", vec![1, 2, 3]),
            ("reversed", vec![3, 2, 1]),
            ("renumbered", vec![900, 400, 7000]),
            ("with a hole", vec![1, 3, 4]),
        ];

        let mut expected: Option<String> = None;
        for (label, sequences) in &cases {
            let replay = device().await;
            for (row, seq) in corpus.iter().zip(sequences) {
                let mut incoming = row.clone_for_merge();
                incoming.writer_seq = *seq;
                merge_synced_personal(&replay, incoming).await.expect(label);
            }

            let subject = personal_subject(&replay, owner, "retry.budget")
                .await
                .expect("subject");
            // The answer ids are the corpus's own, which are stable across
            // replays, so this compares the derived *decision* rather than
            // incidental identifiers.
            let rendered = format!(
                "{:?}|{:?}|{:?}",
                subject.view.reconciliation,
                {
                    let mut a = subject.view.answers.clone();
                    a.sort();
                    a
                },
                subject.members.len()
            );
            match &expected {
                None => expected = Some(rendered),
                Some(first) => assert_eq!(
                    first, &rendered,
                    "replaying the corpus `{label}` changed the derived answer, so \
                     something is ordering on writer_seq"
                ),
            }
        }
    });
}

/// A withheld middle record is reported as a detected gap.
///
/// Separate from the replay test above, because they assert opposite things about
/// the same field and collapsing them would hide either failure behind the other:
/// the sequence must change *nothing* about what the records mean, and must still
/// be enough to notice that one never arrived. A gap nobody reports is
/// indistinguishable from a stream that had no gap.
///
/// Falsified by dropping the gap report, or by making it fire on a stream with no
/// hole in it.
#[test]
fn a_withheld_middle_record_is_reported_as_a_gap_and_a_complete_run_is_not() {
    runtime().block_on(async {
        let owner = new_id();
        let source = device().await;
        let mut corpus = Vec::new();
        for content in ["note one", "note two", "note three"] {
            corpus.push(write_and_read(&source, owner, content, None, None).await);
        }

        // The complete run reports nothing.
        let complete = device().await;
        for row in &corpus {
            merge_synced_personal(&complete, row.clone_for_merge())
                .await
                .expect("merge");
        }
        assert!(
            personal_writer_gaps(&complete, owner)
                .await
                .unwrap()
                .is_empty(),
            "a complete run was reported as having a gap"
        );

        // Withholding the middle one does.
        let with_hole = device().await;
        for row in [&corpus[0], &corpus[2]] {
            merge_synced_personal(&with_hole, row.clone_for_merge())
                .await
                .expect("merge");
        }
        let gaps = personal_writer_gaps(&with_hole, owner).await.unwrap();
        assert_eq!(gaps.len(), 1, "expected one writer's gap, got {gaps:?}");
        assert_eq!(gaps[0].writer_id, corpus[0].writer_id);
        assert_eq!(gaps[0].missing, vec![corpus[1].writer_seq]);
        assert_eq!(gaps[0].highest_seen, corpus[2].writer_seq);
    });
}

/// `SyncedPersonalKnowledge` is not `Clone` — it is a one-shot input to a merge
/// by design — so a test that replays one row builds a fresh copy each time.
trait CloneForMerge {
    fn clone_for_merge(&self) -> SyncedPersonalKnowledge;
}

impl CloneForMerge for SyncedPersonalKnowledge {
    fn clone_for_merge(&self) -> SyncedPersonalKnowledge {
        SyncedPersonalKnowledge {
            id: self.id,
            owner_user_id: self.owner_user_id,
            knowledge_type: self.knowledge_type,
            content: self.content.clone(),
            topic_key: self.topic_key.clone(),
            value_key: self.value_key.clone(),
            applicability: self.applicability.clone(),
            writer_id: self.writer_id,
            writer_seq: self.writer_seq,
            created_at: self.created_at,
            superseded_by_id: self.superseded_by_id,
            forgotten_at: self.forgotten_at,
        }
    }
}
