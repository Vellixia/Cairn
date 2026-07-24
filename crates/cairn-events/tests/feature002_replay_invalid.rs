mod support;

use cairn_events::catalog::{
    PROJECT_CREATED, PROJECT_REPOSITORY_ASSOCIATED, PROJECT_UPDATED, SESSION_BOUND, TASK_CREATED,
    TASK_REVISION_CREATED,
};
use cairn_events::replay::{replay_mixed_rows, MixedReplayError};

fn assert_fails_without_mutating(
    rows: &mut [cairn_storage_local::EventRow],
    expected: MixedReplayError,
) {
    let before = rows
        .iter()
        .map(|event| (event.seq, event.id.clone(), event.payload.clone()))
        .collect::<Vec<_>>();
    assert_eq!(replay_mixed_rows(rows).unwrap_err(), expected);
    let after = rows
        .iter()
        .map(|event| (event.seq, event.id.clone(), event.payload.clone()))
        .collect::<Vec<_>>();
    assert_eq!(after, before, "strict replay must never mutate the ledger");
}

/// Rewrite one row's JSON payload through `edit`.
fn corrupt_payload(
    fixture: &mut support::MixedFixture,
    index: usize,
    edit: impl FnOnce(&mut serde_json::Value),
) {
    let mut payload: serde_json::Value =
        serde_json::from_str(&fixture.rows[index].payload).unwrap();
    edit(&mut payload);
    fixture.rows[index].payload = serde_json::to_string(&payload).unwrap();
}

#[test]
fn unknown_type_version_malformed_payload_and_global_order_fail_closed() {
    let mut fixture = support::mixed_fixture();
    let project_created = fixture.index_of(PROJECT_CREATED, 0);
    fixture.rows[project_created].event_type = "feature002.unknown".into();
    assert_fails_without_mutating(&mut fixture.rows, MixedReplayError::UnsupportedEventType);

    let mut fixture = support::mixed_fixture();
    let project_created = fixture.index_of(PROJECT_CREATED, 0);
    corrupt_payload(&mut fixture, project_created, |payload| {
        payload["schema_version"] = serde_json::json!(2);
    });
    assert_fails_without_mutating(
        &mut fixture.rows,
        MixedReplayError::UnsupportedPayloadVersion,
    );

    let mut fixture = support::mixed_fixture();
    let revision_one = fixture.index_of(TASK_REVISION_CREATED, 0);
    fixture.rows[revision_one].payload = "{not-json".into();
    assert_fails_without_mutating(&mut fixture.rows, MixedReplayError::MalformedPayload);

    let mut fixture = support::mixed_fixture();
    let task_created = fixture.index_of(TASK_CREATED, 0);
    fixture.rows[task_created].seq = fixture.rows[task_created - 1].seq;
    assert_fails_without_mutating(&mut fixture.rows, MixedReplayError::InvalidGlobalOrder);
}

#[test]
fn aggregate_gaps_fake_worktree_scope_and_missing_references_fail_closed() {
    let mut fixture = support::mixed_fixture();
    let revision_one = fixture.index_of(TASK_REVISION_CREATED, 0);
    fixture.rows[revision_one].aggregate_seq = Some(4);
    assert_fails_without_mutating(
        &mut fixture.rows,
        MixedReplayError::InvalidAggregateSequence,
    );

    let mut fixture = support::mixed_fixture();
    let project_created = fixture.index_of(PROJECT_CREATED, 0);
    fixture.rows[project_created].worktree_id = Some("fake-worktree".into());
    assert_fails_without_mutating(&mut fixture.rows, MixedReplayError::InvalidAggregateScope);

    let mut fixture = support::mixed_fixture();
    let associated = fixture.index_of(PROJECT_REPOSITORY_ASSOCIATED, 0);
    corrupt_payload(&mut fixture, associated, |payload| {
        payload["association"]["repository_id"] = serde_json::json!("missing-repository");
    });
    assert_fails_without_mutating(&mut fixture.rows, MixedReplayError::InvalidReference);

    let mut fixture = support::mixed_fixture();
    let task_created = fixture.index_of(TASK_CREATED, 0);
    corrupt_payload(&mut fixture, task_created, |payload| {
        payload["task"]["project_id"] = serde_json::json!(cairn_domain::ProjectId::new_v7());
    });
    assert_fails_without_mutating(&mut fixture.rows, MixedReplayError::InvalidReference);
}

#[test]
fn revision_binding_fingerprint_and_post_state_corruption_fail_closed() {
    let mut fixture = support::mixed_fixture();
    let revision_one = fixture.index_of(TASK_REVISION_CREATED, 0);
    corrupt_payload(&mut fixture, revision_one, |payload| {
        payload["revision"]["goal_contract_fingerprint"] = serde_json::json!("0".repeat(64));
    });
    assert_fails_without_mutating(&mut fixture.rows, MixedReplayError::InvalidReference);

    let mut fixture = support::mixed_fixture();
    let revision_two = fixture.index_of(TASK_REVISION_CREATED, 1);
    corrupt_payload(&mut fixture, revision_two, |payload| {
        payload["task"]["latest_revision_number"] = serde_json::json!(9);
    });
    assert_fails_without_mutating(&mut fixture.rows, MixedReplayError::InvalidReference);

    let mut fixture = support::mixed_fixture();
    let bound = fixture.index_of(SESSION_BOUND, 0);
    corrupt_payload(&mut fixture, bound, |payload| {
        payload["binding"]["task_revision_id"] =
            serde_json::json!(cairn_domain::TaskRevisionId::new_v7());
    });
    assert_fails_without_mutating(&mut fixture.rows, MixedReplayError::InvalidReference);

    let mut fixture = support::mixed_fixture();
    let bound = fixture.index_of(SESSION_BOUND, 0);
    let conflicting = fixture.rows[bound].clone();
    fixture.rows.insert(bound + 1, conflicting);
    for (index, event) in fixture.rows.iter_mut().enumerate() {
        event.seq = i64::try_from(index + 1).unwrap();
    }
    fixture.rows[bound + 1].aggregate_seq = Some(2);
    assert_fails_without_mutating(&mut fixture.rows, MixedReplayError::ConflictingBinding);
}

#[test]
fn duplicate_aggregate_sequence_and_absent_parents_fail_closed() {
    // A second row reusing an already-consumed aggregate sequence with different
    // content is rejected rather than silently overwriting the earlier projection.
    let mut fixture = support::mixed_fixture();
    let archived = fixture.index_of(PROJECT_UPDATED, 0);
    fixture.rows[archived].aggregate_seq = Some(1);
    assert_fails_without_mutating(
        &mut fixture.rows,
        MixedReplayError::InvalidAggregateSequence,
    );

    // A revision whose parent task does not exist. The aggregate scope is retargeted
    // with the payload so the row is internally consistent and the rejection can only
    // come from the missing task, not from a scope mismatch.
    let mut fixture = support::mixed_fixture();
    let revision_one = fixture.index_of(TASK_REVISION_CREATED, 0);
    let absent_task = cairn_domain::TaskId::new_v7();
    corrupt_payload(&mut fixture, revision_one, |payload| {
        payload["revision"]["task_id"] = serde_json::json!(absent_task);
        payload["task"]["task_id"] = serde_json::json!(absent_task);
    });
    fixture.rows[revision_one].aggregate_id = Some(absent_task.to_string());
    fixture.rows[revision_one].aggregate_seq = Some(1);
    assert_fails_without_mutating(&mut fixture.rows, MixedReplayError::InvalidReference);

    // A binding naming a project that the ledger never created.
    let mut fixture = support::mixed_fixture();
    let bound = fixture.index_of(SESSION_BOUND, 0);
    corrupt_payload(&mut fixture, bound, |payload| {
        payload["binding"]["project_id"] = serde_json::json!(cairn_domain::ProjectId::new_v7());
    });
    assert_fails_without_mutating(&mut fixture.rows, MixedReplayError::InvalidReference);
}

#[test]
fn verification_mismatch_is_a_closed_content_free_error() {
    assert_eq!(
        MixedReplayError::ProjectionMismatch.to_string(),
        "mixed replay differs from live projections"
    );
    for forbidden in ["SELECT", "/Users/", "token", "revision one"] {
        assert!(!MixedReplayError::InvalidReference
            .to_string()
            .contains(forbidden));
    }
}
