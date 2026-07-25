#![allow(dead_code)]

use cairn_domain::{
    EventId, GoalContractV1, Project, ProjectId, ProjectRepositoryAssociationId, SessionId, Task,
    TaskId, TaskRevision, TaskRevisionId, Timestamp,
};
use cairn_events::{
    EventBuilder, ProjectAssociationEvent, ProjectChangedField, ProjectCreatedPayload,
    ProjectRepositoryAssociatedPayload, ProjectUpdatedPayload, SessionBindingEvent,
    SessionBoundPayload, SessionStartedPayload, TaskCreatedEvent, TaskCreatedPayload,
    TaskEventState, TaskRevisionCreatedPayload, TaskRevisionEvent,
};
use cairn_storage_local::{EventRow, NewEvent};

pub struct MixedFixture {
    pub rows: Vec<EventRow>,
    pub repository_id: String,
    pub worktree_id: String,
    pub historical_session_id: SessionId,
    /// Migrated Feature 001 session that never binds and stays `local_unbound`.
    pub unbound_session_id: SessionId,
    pub bound_start_session_id: SessionId,
    pub project: Project,
    pub task: Task,
    pub revision_one: TaskRevision,
    pub revision_two: TaskRevision,
}

impl MixedFixture {
    /// Position of the `nth` (zero-based) row of `event_type` in the ordered ledger.
    /// Tests address rows by meaning so that extending the fixture cannot silently
    /// retarget a corruption case.
    pub fn index_of(&self, event_type: &str, nth: usize) -> usize {
        self.rows
            .iter()
            .enumerate()
            .filter(|(_, row)| row.event_type == event_type)
            .map(|(index, _)| index)
            .nth(nth)
            .unwrap_or_else(|| panic!("mixed fixture has no {event_type} row {nth}"))
    }
}

pub fn mixed_fixture() -> MixedFixture {
    let repository_id = "repository-mixed-001".to_string();
    let worktree_id = "worktree-mixed-001".to_string();
    let historical_session_id = SessionId::new_v7();
    let unbound_session_id = SessionId::new_v7();
    let bound_start_session_id = SessionId::new_v7();
    let project_id = ProjectId::new_v7();
    let task_id = TaskId::new_v7();
    let revision_one_id = TaskRevisionId::new_v7();
    let revision_two_id = TaskRevisionId::new_v7();
    let created_at = timestamp("2026-07-22T01:00:00.000Z");
    let archived_at = timestamp("2026-07-22T01:01:00.000Z");
    let restored_at = timestamp("2026-07-22T01:02:00.000Z");
    let revision_two_at = timestamp("2026-07-22T01:03:00.000Z");

    let project_created = Project::new(project_id, "Mixed project", None, created_at).unwrap();
    let mut project_archived = project_created.clone();
    project_archived.set_status(cairn_domain::ProjectStatus::Archived, archived_at);
    let mut project_restored = project_archived.clone();
    project_restored.set_status(cairn_domain::ProjectStatus::Active, restored_at);

    let mut task_revision_one = Task::new(task_id, project_id, "Mixed task", created_at).unwrap();
    let revision_one = TaskRevision::new(
        revision_one_id,
        task_id,
        1,
        None,
        contract("revision one"),
        created_at,
    )
    .unwrap();
    task_revision_one.latest_revision_number = 1;
    let revision_two = TaskRevision::new(
        revision_two_id,
        task_id,
        2,
        Some(revision_one_id),
        contract("revision two"),
        revision_two_at,
    )
    .unwrap();
    let mut task_revision_two = task_revision_one.clone();
    task_revision_two.latest_revision_number = 2;
    task_revision_two.updated_at = revision_two_at;

    let mut rows = Vec::new();
    let mut push = |event: NewEvent, aggregate_seq: Option<i64>| {
        let seq = i64::try_from(rows.len() + 1).unwrap();
        rows.push(event_row(event, seq, aggregate_seq));
    };
    push(
        EventBuilder::repository_registered(
            &repository_id,
            "repository-uuid-mixed-001",
            "/fixture/repository",
            None,
        ),
        None,
    );
    push(
        EventBuilder::worktree_registered(
            &repository_id,
            &worktree_id,
            "worktree-uuid-mixed-001",
            "/fixture/repository",
            true,
        ),
        None,
    );
    push(
        EventBuilder::session_started(
            &repository_id,
            &worktree_id,
            &historical_session_id.to_string(),
            &SessionStartedPayload {
                agent_type: "historical".into(),
                agent_instance_id: "historical-agent".into(),
                start_snapshot_id: "snapshot-historical".into(),
                local_user: "local".into(),
            },
        ),
        None,
    );
    // A migrated historical session that stays local_unbound for the whole ledger, with
    // legacy null-aggregate interruption and stop rows.
    push(
        EventBuilder::session_started(
            &repository_id,
            &worktree_id,
            &unbound_session_id.to_string(),
            &SessionStartedPayload {
                agent_type: "migrated-unbound".into(),
                agent_instance_id: "migrated-unbound-agent".into(),
                start_snapshot_id: "snapshot-unbound".into(),
                local_user: "local".into(),
            },
        ),
        None,
    );
    push(
        EventBuilder::session_interrupted(
            &repository_id,
            &worktree_id,
            &unbound_session_id.to_string(),
            "liveness_lost",
            "bounded",
        ),
        None,
    );
    push(
        EventBuilder::session_stopped(
            &repository_id,
            &worktree_id,
            &unbound_session_id.to_string(),
            "snapshot-unbound-final",
        ),
        None,
    );
    push(
        EventBuilder::project_created(
            EventId::new_v7(),
            "project-create",
            &ProjectCreatedPayload {
                schema_version: 1,
                project: project_created.clone().into(),
            },
        ),
        Some(1),
    );
    push(
        EventBuilder::project_repository_associated(
            EventId::new_v7(),
            "project-associate",
            &ProjectRepositoryAssociatedPayload {
                schema_version: 1,
                association: ProjectAssociationEvent {
                    association_id: ProjectRepositoryAssociationId::new_v7(),
                    project_id,
                    repository_id: repository_id.clone(),
                    associated_at: created_at,
                },
            },
        ),
        Some(1),
    );
    push(
        EventBuilder::task_created(
            EventId::new_v7(),
            "task-create",
            &TaskCreatedPayload {
                schema_version: 1,
                task: TaskCreatedEvent::from(task_revision_one.clone()),
            },
        ),
        Some(1),
    );
    push(
        EventBuilder::task_revision_created(
            EventId::new_v7(),
            "task-create",
            cairn_events::EventOperationMethod::TaskCreate,
            1,
            &TaskRevisionCreatedPayload {
                schema_version: 1,
                revision: TaskRevisionEvent::from(revision_one.clone()),
                task: TaskEventState::from(task_revision_one.clone()),
            },
        ),
        Some(2),
    );
    push(
        EventBuilder::session_bound(
            EventId::new_v7(),
            "bind-historical",
            &SessionBoundPayload {
                schema_version: 1,
                binding: SessionBindingEvent {
                    session_id: historical_session_id,
                    repository_id: repository_id.clone(),
                    worktree_id: worktree_id.clone(),
                    project_id,
                    task_id,
                    task_revision_id: revision_one_id,
                    bound_at: created_at,
                },
            },
        ),
        Some(1),
    );
    push(
        EventBuilder::session_recovered(
            &repository_id,
            &worktree_id,
            &historical_session_id.to_string(),
            "snapshot-recovered",
        ),
        Some(2),
    );
    push(
        EventBuilder::session_watcher_start_failed(
            &repository_id,
            &worktree_id,
            &historical_session_id.to_string(),
            cairn_domain::WatcherStartStage::Reconcile,
        ),
        Some(3),
    );
    push(
        EventBuilder::project_updated(
            EventId::new_v7(),
            "project-archive",
            &ProjectUpdatedPayload {
                schema_version: 1,
                project: project_archived.into(),
                changed_fields: vec![ProjectChangedField::Status],
            },
        ),
        Some(2),
    );
    push(
        EventBuilder::project_updated(
            EventId::new_v7(),
            "project-restore",
            &ProjectUpdatedPayload {
                schema_version: 1,
                project: project_restored.clone().into(),
                changed_fields: vec![ProjectChangedField::Status],
            },
        ),
        Some(3),
    );
    push(
        EventBuilder::task_revision_created(
            EventId::new_v7(),
            "task-revise",
            cairn_events::EventOperationMethod::TaskRevise,
            0,
            &TaskRevisionCreatedPayload {
                schema_version: 1,
                revision: TaskRevisionEvent::from(revision_two.clone()),
                task: TaskEventState::from(task_revision_two.clone()),
            },
        ),
        Some(3),
    );
    push(
        EventBuilder::session_started(
            &repository_id,
            &worktree_id,
            &bound_start_session_id.to_string(),
            &SessionStartedPayload {
                agent_type: "bound-start".into(),
                agent_instance_id: "bound-start-agent".into(),
                start_snapshot_id: "snapshot-bound".into(),
                local_user: "local".into(),
            },
        ),
        Some(1),
    );
    push(
        EventBuilder::session_bound_for_start(
            EventId::new_v7(),
            &bound_start_session_id.to_string(),
            &SessionBoundPayload {
                schema_version: 1,
                binding: SessionBindingEvent {
                    session_id: bound_start_session_id,
                    repository_id: repository_id.clone(),
                    worktree_id: worktree_id.clone(),
                    project_id,
                    task_id,
                    task_revision_id: revision_two_id,
                    bound_at: revision_two_at,
                },
            },
        ),
        Some(2),
    );

    MixedFixture {
        rows,
        repository_id,
        worktree_id,
        historical_session_id,
        unbound_session_id,
        bound_start_session_id,
        project: project_restored,
        task: task_revision_two,
        revision_one,
        revision_two,
    }
}

fn event_row(event: NewEvent, seq: i64, aggregate_seq: Option<i64>) -> EventRow {
    let legacy = aggregate_seq.is_none();
    EventRow {
        seq,
        id: event.id,
        idempotency_key: event.idempotency_key,
        event_type: event.event_type,
        repository_id: event.repository_id,
        worktree_id: event.worktree_id,
        session_id: event.session_id,
        snapshot_id: event.snapshot_id,
        payload: serde_json::to_string(&event.payload).unwrap(),
        recorded_at: event.recorded_at,
        aggregate_type: (!legacy).then_some(event.aggregate_type),
        aggregate_id: (!legacy).then_some(event.aggregate_id),
        aggregate_seq,
    }
}

fn timestamp(value: &str) -> Timestamp {
    Timestamp::parse(value).unwrap()
}

fn contract(goal: &str) -> GoalContractV1 {
    GoalContractV1::new(
        goal.into(),
        vec!["included".into()],
        vec!["excluded".into()],
        vec!["accepted".into()],
        vec!["constraint".into()],
    )
    .unwrap()
}
