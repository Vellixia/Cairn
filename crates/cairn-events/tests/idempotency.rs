use cairn_domain::WatcherStartStage;
use cairn_events::FEATURE002_EVENT_TYPES;
use cairn_events::{derive_event_key, DerivedEventKeyInput, EventOperationMethod};
use cairn_events::{AggregateEnvelope, AggregateIdentity, AggregateType};
use cairn_events::{
    BranchChangedPayload, EventBuilder, SessionStartedPayload, StateChangedPayload,
};

#[test]
fn aggregate_envelopes_require_real_identity_positive_sequence_and_version_one() {
    let identity = AggregateIdentity::new(AggregateType::Project, "project-id").unwrap();
    let envelope = AggregateEnvelope::new(identity, 1).unwrap();
    envelope.validate().unwrap();
    assert_eq!(envelope.aggregate_type.as_str(), "project");
    assert!(AggregateIdentity::new(AggregateType::Task, " ").is_err());
    assert!(AggregateEnvelope::new(
        AggregateIdentity::new(AggregateType::Task, "task-id").unwrap(),
        0
    )
    .is_err());
    assert_eq!(FEATURE002_EVENT_TYPES.len(), 6);
}

#[test]
fn derived_event_keys_are_stable_and_position_specific() {
    let input = |position, event_type| DerivedEventKeyInput {
        operation_identity: "01900000-0000-7000-8000-000000000001",
        method: EventOperationMethod::TaskCreate,
        event_position: position,
        event_type,
    };
    let task_created = derive_event_key(&input(0, "task.created"));
    let revision_created = derive_event_key(&input(1, "task.revision_created"));
    assert_eq!(task_created.len(), 64);
    assert!(task_created
        .chars()
        .all(|character| character.is_ascii_hexdigit()));
    assert_eq!(task_created, derive_event_key(&input(0, "task.created")));
    assert_ne!(task_created, revision_created);
}

#[test]
fn method_event_type_and_operation_identity_are_domain_separated() {
    let base = DerivedEventKeyInput {
        operation_identity: "01900000-0000-7000-8000-000000000001",
        method: EventOperationMethod::SessionBind,
        event_position: 0,
        event_type: "session.bound",
    };
    let expected = derive_event_key(&base);
    for changed in [
        DerivedEventKeyInput {
            method: EventOperationMethod::SessionStart,
            ..base.clone()
        },
        DerivedEventKeyInput {
            operation_identity: "01900000-0000-7000-8000-000000000002",
            ..base.clone()
        },
        DerivedEventKeyInput {
            event_type: "session.started",
            ..base.clone()
        },
    ] {
        assert_ne!(expected, derive_event_key(&changed));
    }
}

#[test]
fn bound_start_uses_stable_session_identity_without_a_new_raw_key() {
    let session_id = "01900000-0000-7000-8000-000000000099";
    let started = derive_event_key(&DerivedEventKeyInput {
        operation_identity: session_id,
        method: EventOperationMethod::SessionStart,
        event_position: 0,
        event_type: "session.started",
    });
    let bound = derive_event_key(&DerivedEventKeyInput {
        operation_identity: session_id,
        method: EventOperationMethod::SessionStart,
        event_position: 1,
        event_type: "session.bound",
    });
    assert_ne!(started, bound);
    assert_eq!(
        bound,
        derive_event_key(&DerivedEventKeyInput {
            operation_identity: session_id,
            method: EventOperationMethod::SessionStart,
            event_position: 1,
            event_type: "session.bound",
        })
    );
}

#[test]
fn every_feature001_builder_supplies_its_real_post_migration_aggregate() {
    let started_payload = SessionStartedPayload {
        agent_type: "agent".into(),
        agent_instance_id: "instance".into(),
        start_snapshot_id: "snapshot".into(),
        local_user: "user".into(),
    };
    let state_payload = StateChangedPayload {
        worktree_id: "worktree".into(),
        from_snapshot_id: None,
        to_snapshot_id: "snapshot".into(),
    };
    let branch_payload = BranchChangedPayload {
        from_branch: None,
        to_branch: Some("main".into()),
        from_head: None,
        to_head: "head".into(),
    };
    let events = vec![
        (
            EventBuilder::repository_registered("repo", "uuid", "/repo", None),
            "repository",
            "repo",
        ),
        (
            EventBuilder::worktree_registered("repo", "worktree", "uuid", "/repo", true),
            "worktree",
            "worktree",
        ),
        (
            EventBuilder::snapshot_created(
                "repo",
                "worktree",
                "snapshot",
                "fp",
                Some("main"),
                "head",
            ),
            "worktree",
            "worktree",
        ),
        (
            EventBuilder::session_started("repo", "worktree", "session", &started_payload),
            "session",
            "session",
        ),
        (
            EventBuilder::repository_state_changed("repo", "worktree", &state_payload),
            "worktree",
            "worktree",
        ),
        (
            EventBuilder::branch_changed("repo", "worktree", &branch_payload),
            "worktree",
            "worktree",
        ),
        (
            EventBuilder::session_stopped("repo", "worktree", "session", "snapshot"),
            "session",
            "session",
        ),
        (
            EventBuilder::session_interrupted("repo", "worktree", "session", "reason", "detail"),
            "session",
            "session",
        ),
        (
            EventBuilder::session_watcher_start_failed(
                "repo",
                "worktree",
                "session",
                WatcherStartStage::Install,
            ),
            "session",
            "session",
        ),
        (
            EventBuilder::session_recovered("repo", "worktree", "session", "snapshot"),
            "session",
            "session",
        ),
        (
            EventBuilder::session_reattach_rejected(
                "repo", "worktree", "session", "instance", "reason", "attempt",
            ),
            "session",
            "session",
        ),
        (
            EventBuilder::identity_marker_restored("repo", Some("worktree"), "marker"),
            "repository",
            "repo",
        ),
    ];
    for (event, aggregate_type, aggregate_id) in events {
        assert_eq!(event.aggregate_type, aggregate_type);
        assert_eq!(event.aggregate_id, aggregate_id);
        assert!(!event.aggregate_id.starts_with("__"));
    }
}
