mod support;

use cairn_domain::{ProjectStatus, SessionBindingMode};
use cairn_events::replay::replay_mixed_rows;

#[test]
fn mixed_legacy_and_feature002_ledger_replays_field_for_field() {
    let fixture = support::mixed_fixture();
    let state = replay_mixed_rows(&fixture.rows).expect("strict mixed replay");

    assert_eq!(state.repositories.len(), 1);
    assert!(state.repositories.contains(&fixture.repository_id));
    assert_eq!(
        state.worktrees.get(&fixture.worktree_id),
        Some(&fixture.repository_id)
    );
    assert_eq!(state.projects.projects.len(), 1);
    assert_eq!(
        state.projects.projects.get(&fixture.project.id),
        Some(&fixture.project)
    );
    assert_eq!(
        state.projects.projects[&fixture.project.id].status,
        ProjectStatus::Active
    );
    assert_eq!(state.projects.associations.len(), 1);
    assert_eq!(
        state.tasks.tasks,
        [(fixture.task.id, fixture.task.clone())].into()
    );
    assert_eq!(state.tasks.revisions.len(), 2);
    assert_eq!(
        state.tasks.revisions.get(&fixture.revision_one.id),
        Some(&fixture.revision_one)
    );
    assert_eq!(
        state.tasks.revisions.get(&fixture.revision_two.id),
        Some(&fixture.revision_two)
    );

    assert_eq!(state.sessions.len(), 3);
    assert_eq!(
        state.sessions[&fixture.historical_session_id.to_string()].state,
        "interrupted"
    );

    // The migrated historical session that never binds stays local_unbound through its
    // whole legacy lifecycle, and no lifecycle event implies a binding.
    assert_eq!(
        state.sessions[&fixture.unbound_session_id.to_string()].state,
        "stopped"
    );
    assert_eq!(
        state.session_bindings.scopes[&fixture.unbound_session_id],
        SessionBindingMode::LocalUnbound
    );
    assert!(!state
        .session_bindings
        .bindings
        .contains_key(&fixture.unbound_session_id));
    assert_eq!(
        state.session_bindings.scopes[&fixture.historical_session_id],
        SessionBindingMode::ProjectBound {
            project_id: fixture.project.id,
            task_revision_id: fixture.revision_one.id,
        }
    );
    assert_eq!(
        state.session_bindings.scopes[&fixture.bound_start_session_id],
        SessionBindingMode::ProjectBound {
            project_id: fixture.project.id,
            task_revision_id: fixture.revision_two.id,
        }
    );
    assert_eq!(state.session_bindings.bindings.len(), 2);
    assert_eq!(
        state.session_bindings.bindings[&fixture.historical_session_id].task_revision_id,
        fixture.revision_one.id,
        "newer revisions never advance an existing binding"
    );
    let bound_start = fixture
        .rows
        .iter()
        .filter(|event| {
            event.session_id.as_deref() == Some(fixture.bound_start_session_id.to_string().as_str())
        })
        .map(|event| event.event_type.as_str())
        .collect::<Vec<_>>();
    assert_eq!(bound_start, ["session.started", "session.bound"]);
    // Legacy Feature 001 rows carry null aggregate fields; post-migration rows carry
    // explicit ones, and both are interpreted from the one ordered ledger.
    let legacy = fixture.index_of("session.started", 0);
    assert!(fixture.rows[legacy].aggregate_type.is_none());
    assert!(fixture.rows[legacy].aggregate_seq.is_none());
    let post_migration = fixture.index_of("project.created", 0);
    assert!(fixture.rows[post_migration].aggregate_type.is_some());
    assert!(fixture.rows[post_migration].aggregate_seq.is_some());
    assert!(
        fixture
            .rows
            .iter()
            .any(|row| row.event_type == "session.stopped" && row.aggregate_type.is_none()),
        "the mixed ledger must include legacy stop rows"
    );
}
