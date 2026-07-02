//! 09 — Session persistence: save/load round-trip via tempdir,
//! drift event ordering, anchor block content.

use cairn_session::{
    Decision, DriftEvent, DriftStatus, Finding, Session, SessionStore, Task, TouchedFile,
};

#[test]
fn session_save_and_load_round_trip_via_tempdir() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = SessionStore::new(dir.path());
    let session = Session {
        id: "sess-test-1".into(),
        project_hash: "proj-abc".into(),
        started_at: chrono::Utc::now(),
        ended_at: None,
        tasks: vec![Task {
            id: "t1".into(),
            title: "ship v0.7.0".into(),
            progress: "in progress".into(),
        }],
        findings: vec![Finding {
            text: "added p4.2 reranker".into(),
            source_file: None,
            confidence: 0.5,
        }],
        decisions: vec![Decision {
            text: "use NullReranker default".into(),
            rationale: "model is opt-in".into(),
            confidence: 0.7,
        }],
        touched_files: vec![TouchedFile {
            path: "crates/cairn-rerank/src/lib.rs".into(),
            mode: "edit".into(),
            handle: None,
        }],
        next_steps: vec!["wire up the dashboard".into()],
        memory_ids: vec!["mem-1".into()],
    };
    let path = store.save(&session).expect("save");
    assert!(path.exists());
    let loaded = store.load(&session.id).expect("load").expect("found");
    assert_eq!(loaded.id, session.id);
    assert_eq!(loaded.tasks.len(), 1);
    assert_eq!(loaded.findings[0].text, "added p4.2 reranker");
    assert_eq!(loaded.decisions[0].text, "use NullReranker default");
    assert_eq!(
        loaded.touched_files[0].path,
        "crates/cairn-rerank/src/lib.rs"
    );
    assert_eq!(loaded.memory_ids, vec!["mem-1".to_string()]);
}

#[test]
fn session_block_round_trips_via_as_block() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = SessionStore::new(dir.path());
    let session = Session {
        id: "sess-block".into(),
        project_hash: "proj".into(),
        started_at: chrono::Utc::now(),
        ended_at: None,
        tasks: vec![Task {
            id: "t1".into(),
            title: "task A".into(),
            progress: "started".into(),
        }],
        findings: vec![],
        decisions: vec![],
        touched_files: vec![],
        next_steps: vec![],
        memory_ids: vec![],
    };
    let block = session.as_block();
    assert!(block.contains("task A"));
    assert!(block.contains("## Tasks"));
    // Round-trip via store.
    store.save(&session).expect("save");
    let latest = store.latest_block().expect("latest block returns");
    assert!(latest.contains("task A"));
}

#[test]
fn session_new_assigns_unique_id() {
    let a = Session::new("p");
    let b = Session::new("p");
    assert_ne!(a.id, b.id);
    assert_eq!(a.project_hash, "p");
}

#[test]
fn drift_status_default_is_pending() {
    // The dashboard renders new drift events as "pending" until the user
    // approves/rejects. The default is part of the public contract.
    let s: DriftStatus = Default::default();
    assert_eq!(s, DriftStatus::Pending);
    // Serde round-trip.
    let json = serde_json::to_string(&s).unwrap();
    assert_eq!(json, "\"pending\"");
}

#[test]
fn drift_event_struct_holds_all_required_fields() {
    let e = DriftEvent {
        id: 1,
        ts: chrono::Utc::now(),
        path: "crates/foo.rs".into(),
        risk: "high".into(),
        kind: "edit".into(),
        detail: "silent corruption".into(),
        status: DriftStatus::Pending,
    };
    let s = serde_json::to_string(&e).expect("serialize");
    let back: DriftEvent = serde_json::from_str(&s).expect("deserialize");
    assert_eq!(back.id, 1);
    assert_eq!(back.path, "crates/foo.rs");
    assert_eq!(back.status, DriftStatus::Pending);
}

#[test]
fn session_list_returns_all_saved_ids() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = SessionStore::new(dir.path());
    for i in 0..3 {
        let session = Session {
            id: format!("sess-{i}"),
            project_hash: "p".into(),
            started_at: chrono::Utc::now(),
            ended_at: None,
            tasks: vec![],
            findings: vec![],
            decisions: vec![],
            touched_files: vec![],
            next_steps: vec![],
            memory_ids: vec![],
        };
        store.save(&session).expect("save");
    }
    let ids = store.list().expect("list");
    assert!(ids.contains(&"sess-0".to_string()));
    assert!(ids.contains(&"sess-1".to_string()));
    assert!(ids.contains(&"sess-2".to_string()));
    assert_eq!(ids.len(), 3);
}

#[test]
fn drift_append_and_approve_lifecycle() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = SessionStore::new(dir.path());
    let e1 = DriftEvent {
        id: 0,
        ts: chrono::Utc::now(),
        path: "crates/cairn-store/src/db.rs".into(),
        risk: "high".into(),
        kind: "edit".into(),
        detail: "silent corruption suspected".into(),
        status: DriftStatus::Pending,
    };
    let id1 = store.append_drift(&e1).expect("append 1");
    let e2 = DriftEvent {
        id: 0,
        ts: chrono::Utc::now(),
        path: "crates/cairn-api/src/lib.rs".into(),
        risk: "low".into(),
        kind: "edit".into(),
        detail: "clean edit".into(),
        status: DriftStatus::Pending,
    };
    let id2 = store.append_drift(&e2).expect("append 2");

    // recent_drift returns newest first (by id desc).
    let recent = store.recent_drift(10, None).expect("recent");
    assert_eq!(recent.len(), 2);
    assert_eq!(recent[0].id, id2);
    assert_eq!(recent[1].id, id1);

    // Approve id1; the next recent_drift must reflect the new status.
    assert!(store
        .set_drift_status(id1, DriftStatus::Approved)
        .expect("approve"));
    let after = store.recent_drift(10, None).expect("recent");
    let approved = after.iter().find(|e| e.id == id1).expect("id1 present");
    assert_eq!(approved.status, DriftStatus::Approved);

    // Rejecting an unknown id is a no-op (returns false).
    assert!(!store
        .set_drift_status(9999, DriftStatus::Rejected)
        .expect("reject unknown"));
}

#[test]
fn latest_id_tracks_most_recent_session() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = SessionStore::new(dir.path());
    assert!(store.latest_id().is_none());
    let s = Session::new("p");
    store.save(&s).expect("save");
    let s2 = Session::new("p");
    store.save(&s2).expect("save");
    assert_eq!(store.latest_id().as_deref(), Some(s2.id.as_str()));
}

#[test]
fn task_default_fields_are_optional() {
    let t: Task = serde_json::from_str(r#"{"id": "t1", "title": "x"}"#).unwrap();
    assert_eq!(t.id, "t1");
    assert_eq!(t.title, "x");
    assert_eq!(t.progress, "");
}
