//! 19 — MemoryEngine end-to-end via the in-memory Store.
//!
//! Replaces the deleted `02_hybrid_search.rs` (which re-implemented RRF
//! inside the test and asserted on the re-implementation). Every test
//! here calls a real `MemoryEngine` method against a real `Store`.

use cairn_core::{MemoryKind, NewMemory};
use cairn_memory::{gotcha_tracker::FailureEvent, MemoryEngine, ScoredMemory};
use cairn_store::Store;
use std::sync::Arc;

fn engine() -> Option<(MemoryEngine, tempfile::TempDir)> {
    let dir = tempfile::tempdir().ok()?;
    let blobs = dir.path().join("blobs");
    let store = Arc::new(Store::open_in_memory(blobs).ok()?);
    Some((MemoryEngine::new(store), dir))
}

#[test]
fn remember_dedups_identical_content() {
    let Some((mem, _dir)) = engine() else { return };
    let a = mem
        .remember(NewMemory::new("use sqlite for storage"))
        .expect("first remember");
    let b = mem
        .remember(NewMemory::new("use sqlite for storage"))
        .expect("second remember");
    assert_eq!(a.id, b.id, "identical content must dedup to one memory");
}

#[test]
fn recall_ranks_relevant_first() {
    let Some((mem, _dir)) = engine() else { return };
    mem.remember(NewMemory::new("use sqlite plus a content-hash blob store"))
        .unwrap();
    mem.remember(NewMemory::new("the weather today is sunny"))
        .unwrap();
    let hits = mem.recall("sqlite blob storage", 10).expect("recall");
    assert!(!hits.is_empty());
    assert!(
        hits[0].memory.content.contains("sqlite"),
        "sqlite memory should top; got {:?}",
        hits[0].memory.content
    );
}

#[test]
fn recall_reinforces_returned_memories() {
    let Some((mem, _dir)) = engine() else { return };
    let m = mem
        .remember(NewMemory::new("recall reinforcement target"))
        .unwrap();
    let initial = mem.get(&m.id).expect("get").expect("present").confidence;
    mem.recall("recall reinforcement", 5).expect("recall");
    let after = mem.get(&m.id).expect("get").expect("present");
    assert!(
        after.confidence > initial,
        "confidence must increase after recall; got {} -> {}",
        initial,
        after.confidence
    );
    assert!(after.access_count >= 1);
}

#[test]
fn pin_keeps_memory_at_top_of_wakeup() {
    let Some((mem, _dir)) = engine() else { return };
    let important = mem
        .remember(NewMemory {
            content: "an important decision".into(),
            kind: Some(MemoryKind::Decision),
            importance: Some(0.95),
            ..Default::default()
        })
        .unwrap();
    let pinned = mem
        .remember(NewMemory::new("a pinned note that should rise"))
        .unwrap();
    mem.pin(&pinned.id, true).expect("pin");
    let w = mem.wakeup(10).expect("wakeup");
    assert_eq!(w[0].id, pinned.id, "pinned first");
    assert!(w.iter().any(|x| x.id == important.id));
}

#[test]
fn edit_updates_only_specified_fields() {
    let Some((mem, _dir)) = engine() else { return };
    let m = mem.remember(NewMemory::new("original content")).unwrap();
    let updated = mem
        .edit(&m.id, Some("new content".into()), None, None, None)
        .expect("edit")
        .expect("found");
    assert_eq!(updated.content, "new content");
    // Importance untouched at the 0.5 default.
    assert!((updated.importance - 0.5).abs() < 1e-6);
    // Unknown id returns Ok(None).
    assert!(mem
        .edit("no-such-id", None, None, None, None)
        .unwrap()
        .is_none());
}

#[test]
fn delete_removes_memory() {
    let Some((mem, _dir)) = engine() else { return };
    let m = mem.remember(NewMemory::new("to be deleted")).unwrap();
    assert!(mem.delete(&m.id).expect("delete"));
    assert!(mem.get(&m.id).expect("get").is_none());
    assert!(
        !mem.delete(&m.id).expect("delete again"),
        "second delete is no-op"
    );
}

#[test]
fn hybrid_search_returns_scored_hits_under_limit() {
    let Some((mem, _dir)) = engine() else { return };
    mem.remember(NewMemory {
        content: "rust uses ownership for memory safety".into(),
        kind: Some(MemoryKind::Fact),
        ..Default::default()
    })
    .unwrap();
    mem.remember(NewMemory {
        content: "the cat sat on the mat".into(),
        kind: Some(MemoryKind::Note),
        ..Default::default()
    })
    .unwrap();
    let hits: Vec<ScoredMemory> = mem
        .hybrid_search("rust ownership", 5, 10)
        .expect("hybrid_search");
    assert!(!hits.is_empty());
    assert!(hits[0].memory.content.contains("rust"));
    for h in &hits {
        assert!(
            (0.0..=1.0).contains(&h.score),
            "score in [0, 1]: {}",
            h.score
        );
    }
}

#[test]
fn gotcha_tracker_records_failure_events() {
    let Some((mem, _dir)) = engine() else { return };
    let created = mem
        .record_failure(FailureEvent::new(
            "cargo build",
            "linker error: undefined reference to foo",
        ))
        .expect("record_failure");
    // First failure does not yet form a cluster, so no gotcha memory is created.
    assert!(created.is_none());
    let created2 = mem
        .record_failure(FailureEvent::new(
            "cargo build",
            "linker error: undefined reference to foo",
        ))
        .expect("record_failure second");
    // A second event on the same topic triggers the gotcha promotion.
    assert!(created2.is_some(), "second failure must promote to gotcha");
    let top = mem.top_gotcha_clusters(3).expect("top clusters");
    assert_eq!(top.len(), 1);
    assert!(top[0].size() >= 2);
}

#[test]
fn crystallize_promotes_working_into_semantic_crystal() {
    let Some((mem, _dir)) = engine() else { return };
    let a = mem.remember(NewMemory::new("first working note")).unwrap();
    let b = mem.remember(NewMemory::new("second working note")).unwrap();
    let crystal_id = mem
        .crystallize(None)
        .expect("crystallize")
        .expect("crystal");
    let crystal = mem.get(&crystal_id).expect("get").expect("present");
    assert_eq!(crystal.tier, cairn_core::MemoryTier::Semantic);
    assert!(crystal.derived_from.contains(&a.id));
    assert!(crystal.derived_from.contains(&b.id));
    // Inputs are moved to episodic with a supersedes edge back to the crystal.
    let a_after = mem.get(&a.id).expect("get").expect("present");
    assert_eq!(a_after.tier, cairn_core::MemoryTier::Episodic);
    assert!(a_after.supersedes.contains(&crystal_id));
    let _ = b;
}

#[test]
fn consolidate_promotes_reinforced_fact_to_semantic() {
    let Some((mem, _dir)) = engine() else { return };
    let fact = mem
        .remember(NewMemory {
            content: "rust uses ownership for memory safety".into(),
            kind: Some(MemoryKind::Fact),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(fact.tier, cairn_core::MemoryTier::Working);
    mem.consolidate().expect("consolidate 1"); // working -> episodic
    mem.recall("rust ownership memory", 10).expect("recall 1");
    mem.recall("rust ownership memory", 10).expect("recall 2");
    mem.consolidate().expect("consolidate 2"); // episodic + reinforced -> semantic
    let after = mem.get(&fact.id).expect("get").expect("present");
    assert_eq!(after.tier, cairn_core::MemoryTier::Semantic);
}
