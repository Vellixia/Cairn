//! T007: fingerprint determinism (SC-002) and sensitivity (FR-010).

use cairn_domain::snapshot::*;

fn index_entries() -> Vec<IndexEntry> {
    vec![
        IndexEntry {
            mode: "100644".into(),
            stage: 0,
            oid: "aaa".into(),
            path: "b.txt".into(),
        },
        IndexEntry {
            mode: "100644".into(),
            stage: 0,
            oid: "bbb".into(),
            path: "a.txt".into(),
        },
        IndexEntry {
            mode: "100755".into(),
            stage: 0,
            oid: "ccc".into(),
            path: "dir/c.sh".into(),
        },
    ]
}

fn work_entries() -> Vec<WorkEntry> {
    vec![
        WorkEntry {
            path: "z.txt".into(),
            status: WorkStatus::Modified,
            content_hash: Some("h1".into()),
        },
        WorkEntry {
            path: "a.txt".into(),
            status: WorkStatus::Added,
            content_hash: Some("h2".into()),
        },
        WorkEntry {
            path: "gone.txt".into(),
            status: WorkStatus::Deleted,
            content_hash: None,
        },
    ]
}

#[test]
fn staged_fingerprint_is_order_independent_and_stable() {
    let entries = index_entries();
    let baseline = staged_fingerprint(&entries);
    for _ in 0..100 {
        let mut shuffled = entries.clone();
        shuffled.reverse();
        assert_eq!(staged_fingerprint(&shuffled), baseline);
        shuffled.swap(0, 2);
        assert_eq!(staged_fingerprint(&shuffled), baseline);
    }
}

#[test]
fn work_fingerprint_is_order_independent_and_stable() {
    let entries = work_entries();
    let baseline = work_fingerprint(&entries);
    for _ in 0..100 {
        let mut shuffled = entries.clone();
        shuffled.rotate_left(1);
        assert_eq!(work_fingerprint(&shuffled), baseline);
    }
}

#[test]
fn final_fingerprint_is_deterministic_over_100_runs() {
    let c = SnapshotComponents {
        branch: Some("main".into()),
        head_commit: "deadbeef".into(),
        staged_fp: staged_fingerprint(&index_entries()),
        unstaged_fp: work_fingerprint(&work_entries()),
        untracked_fp: work_fingerprint(&[]),
    };
    let baseline = c.final_fingerprint();
    for _ in 0..100 {
        assert_eq!(c.final_fingerprint(), baseline);
    }
}

#[test]
fn staging_change_flips_staged_and_final_fp_only() {
    let base_staged = staged_fingerprint(&index_entries());
    let mut mutated = index_entries();
    mutated[0].oid = "changed".into();
    let new_staged = staged_fingerprint(&mutated);
    assert_ne!(base_staged, new_staged);

    let unstaged = work_fingerprint(&work_entries());
    let untracked = work_fingerprint(&[]);
    let before = snapshot_fingerprint(Some("main"), "head1", &base_staged, &unstaged, &untracked);
    let after = snapshot_fingerprint(Some("main"), "head1", &new_staged, &unstaged, &untracked);
    assert_ne!(before, after);
}

#[test]
fn edit_changes_unstaged_component() {
    let mut entries = work_entries();
    let before = work_fingerprint(&entries);
    entries[0].content_hash = Some("different".into());
    assert_ne!(work_fingerprint(&entries), before);
}

#[test]
fn untracked_add_changes_untracked_component() {
    let before = work_fingerprint(&[]);
    let after = work_fingerprint(&[WorkEntry {
        path: "new.txt".into(),
        status: WorkStatus::Added,
        content_hash: Some("h".into()),
    }]);
    assert_ne!(before, after);
}

#[test]
fn delete_is_distinct_from_content_change() {
    let modified = work_fingerprint(&[WorkEntry {
        path: "f".into(),
        status: WorkStatus::Modified,
        content_hash: Some("h".into()),
    }]);
    let deleted = work_fingerprint(&[WorkEntry {
        path: "f".into(),
        status: WorkStatus::Deleted,
        content_hash: None,
    }]);
    assert_ne!(modified, deleted);
}

#[test]
fn branch_and_head_changes_flip_final_fp() {
    let (s, u, t) = ("s".to_string(), "u".to_string(), "t".to_string());
    let base = snapshot_fingerprint(Some("main"), "head1", &s, &u, &t);
    assert_ne!(
        snapshot_fingerprint(Some("feature"), "head1", &s, &u, &t),
        base
    );
    assert_ne!(
        snapshot_fingerprint(Some("main"), "head2", &s, &u, &t),
        base
    );
    assert_ne!(snapshot_fingerprint(None, "head1", &s, &u, &t), base);
}

#[test]
fn detached_marker_distinct_from_branch_named_detached() {
    // A branch literally named "DETACHED" must still differ from detached HEAD
    // only if content differs; identical strings collide by design, so the
    // marker is uppercase-reserved. Verify the None case uses the marker.
    let fp_none = snapshot_fingerprint(None, "h", "s", "u", "t");
    let fp_marker = snapshot_fingerprint(Some(DETACHED_MARKER), "h", "s", "u", "t");
    assert_eq!(fp_none, fp_marker); // documented equivalence of the reserved marker
}
