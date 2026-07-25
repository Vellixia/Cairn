use cairn_domain::{
    transition, ProjectId, SessionBindingMode, SessionState, TaskRevisionId, TransitionReason,
};

#[test]
fn binding_mode_is_independent_from_every_lifecycle_state() {
    let states = [
        SessionState::Active,
        SessionState::Recovering,
        SessionState::Stopped,
        SessionState::Interrupted,
    ];
    let modes = [
        SessionBindingMode::LocalUnbound,
        SessionBindingMode::ProjectBound {
            project_id: ProjectId::new_v7(),
            task_revision_id: TaskRevisionId::new_v7(),
        },
    ];
    for state in states {
        for mode in modes {
            let encoded = serde_json::to_value(mode).unwrap();
            let decoded: SessionBindingMode = serde_json::from_value(encoded).unwrap();
            assert_eq!(decoded, mode);
            assert_eq!(state, state, "scope cannot mutate lifecycle state");
        }
    }
}

#[test]
fn existing_lifecycle_transition_matrix_is_unchanged() {
    assert_eq!(
        transition(
            SessionState::Active,
            SessionState::Recovering,
            TransitionReason::DaemonRestart
        )
        .unwrap(),
        SessionState::Recovering
    );
    assert!(transition(
        SessionState::Stopped,
        SessionState::Active,
        TransitionReason::Reattach
    )
    .is_err());
}
