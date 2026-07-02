//! 01 — 4-tier memory + LLM consolidator + followup/gotcha trackers + heatmap + arch report.
//!
//! Real crate surface only: every test calls a function exported from a
//! cairn-* crate (cairn-memory, cairn-core). The pure field-only asserts
//! used to live here but were tautological — they constructed a value and
//! read its fields back. They have been removed.

use cairn_core::{MemoryKind, MemoryTier};
use cairn_memory::gotcha_tracker::FailureEvent;
use cairn_memory::{
    analysis::{activity_heatmap, generate_architecture_report},
    FollowupTracker, GotchaTracker,
};
use cairn_tests::fixtures::{mock_new_memory, mock_session_with_n_memories};

#[test]
fn four_tiers_round_trip_through_new_memory_input() {
    // Serde round-trip is a real invariant: every NewMemory that goes through
    // the remember path is deserialized to the same shape on the other end.
    for tier in [
        MemoryTier::Working,
        MemoryTier::Episodic,
        MemoryTier::Semantic,
        MemoryTier::Procedural,
    ] {
        let mut nm = mock_new_memory("round-trip content", MemoryKind::Fact);
        nm.tier = Some(tier);
        let s = serde_json::to_string(&nm).expect("serialize");
        let back: cairn_core::NewMemory = serde_json::from_str(&s).expect("deserialize");
        assert_eq!(back.tier, Some(tier));
    }
}

#[test]
fn followup_tracker_surfaces_repeated_recall_queries() {
    let mut t = FollowupTracker::new();
    let first = t.record("how do I authenticate", &["m1".into()]);
    assert!(!first, "first record is not yet a followup");
    let second = t.record("how do I authenticate", &["m2".into()]);
    assert!(second, "disjoint result set on same query is a followup");
    let rate = t.followup_rate();
    assert!(
        (rate - 0.5).abs() < 1e-9,
        "rate is exactly followups/queries"
    );
    let third = t.record("how do I authenticate", &["m1".into(), "m2".into()]);
    assert!(!third, "overlapping result set is not a followup");
    t.reset();
    assert_eq!(t.followup_rate(), 0.0);
}

#[test]
fn gotcha_tracker_clusters_repeated_failures() {
    let mut t = GotchaTracker::new();
    let e = FailureEvent::new("cargo build", "linker error: undefined reference to foo");
    let first = t.record(e.clone());
    assert!(first.is_none(), "first failure does not form a cluster");
    let cluster = t.record(e.clone()).expect("cluster formed");
    assert_eq!(cluster.size(), 2);
    assert!(t.has_promotable());
    let top = t.top_clusters(3);
    assert_eq!(top.len(), 1);
    assert!(top[0].size() >= 2);
}

#[test]
fn activity_heatmap_counts_recent_memories_only() {
    let memories = mock_session_with_n_memories(5);
    let counts = activity_heatmap(&memories, 30);
    assert!(!counts.is_empty());
    assert!(counts.len() <= 5);
    let total: u32 = counts.values().sum();
    assert_eq!(total, 5);
}

#[test]
fn activity_heatmap_excludes_old_memories_outside_window() {
    use chrono::{Duration, Utc};
    let in_window: Vec<_> = (0..3)
        .map(|i| {
            cairn_tests::fixtures::mock_memory_at(
                &format!("recent-{i}"),
                MemoryKind::Note,
                MemoryTier::Working,
                "recent",
                Utc::now() - Duration::days(i as i64),
            )
        })
        .collect();
    let out_of_window: Vec<_> = (0..2)
        .map(|i| {
            cairn_tests::fixtures::mock_memory_at(
                &format!("old-{i}"),
                MemoryKind::Note,
                MemoryTier::Working,
                "ancient",
                Utc::now() - Duration::days(120 + i as i64),
            )
        })
        .collect();
    let mut all = in_window.clone();
    all.extend(out_of_window);
    let counts = activity_heatmap(&all, 30);
    let total: u32 = counts.values().sum();
    assert_eq!(total, in_window.len() as u32, "old memories are excluded");
}

#[test]
fn architecture_report_counts_nodes_and_edges() {
    let memories = mock_session_with_n_memories(4);
    let mut graph = cairn_memory::MemoryGraph {
        nodes: Vec::new(),
        edges: Vec::new(),
    };
    for m in &memories {
        graph.nodes.push(cairn_memory::MemoryGraphNode {
            id: m.id.clone(),
            kind: format!("{:?}", m.kind),
            tier: format!("{:?}", m.tier),
            content_preview: m.content.chars().take(80).collect(),
            confidence: m.confidence,
            pinned: m.pinned,
            importance: m.importance,
        });
    }
    for i in 1..memories.len() {
        graph.edges.push(cairn_memory::MemoryGraphEdge {
            source: memories[i - 1].id.clone(),
            target: memories[i].id.clone(),
            kind: "derived_from".into(),
        });
    }
    let report = generate_architecture_report(&graph);
    assert_eq!(report.file_count, 4);
    assert_eq!(report.edge_count, 3);
    assert!(report.markdown.contains("Architecture"));
}
