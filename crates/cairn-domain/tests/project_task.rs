use std::str::FromStr;

use cairn_domain::{
    GoalContractV1, Project, ProjectId, ProjectRepositoryAssociation,
    ProjectRepositoryAssociationId, ProjectStatus, Task, TaskId, TaskRevision, TaskRevisionId,
    Timestamp,
};
use uuid::{Uuid, Version};

fn now() -> Timestamp {
    Timestamp::parse("2026-07-22T00:00:00Z").unwrap()
}

fn contract() -> GoalContractV1 {
    GoalContractV1::new("Goal".into(), vec![], vec![], vec![], vec![]).unwrap()
}

#[test]
fn feature002_ids_round_trip_and_new_ids_are_v7() {
    let ids = [
        ProjectId::new_v7().to_string(),
        ProjectRepositoryAssociationId::new_v7().to_string(),
        TaskId::new_v7().to_string(),
        TaskRevisionId::new_v7().to_string(),
    ];
    for id in ids {
        assert_eq!(
            Uuid::parse_str(&id).unwrap().get_version(),
            Some(Version::SortRand)
        );
    }
    let project = ProjectId::new_v7();
    assert_eq!(ProjectId::from_str(&project.to_string()).unwrap(), project);
}

#[test]
fn duplicate_project_names_and_task_titles_are_valid_but_ids_differ() {
    let first = Project::new(ProjectId::new_v7(), "Same", None, now()).unwrap();
    let second = Project::new(ProjectId::new_v7(), "Same", None, now()).unwrap();
    assert_eq!(first.name, second.name);
    assert_ne!(first.id, second.id);

    let task_a = Task::new(TaskId::new_v7(), first.id, "Same task", now()).unwrap();
    let task_b = Task::new(TaskId::new_v7(), first.id, "Same task", now()).unwrap();
    assert_eq!(task_a.title, task_b.title);
    assert_ne!(task_a.id, task_b.id);
}

#[test]
fn project_archive_and_restore_are_explicit_without_identity_change() {
    let mut project = Project::new(ProjectId::new_v7(), " Project\r\n", None, now()).unwrap();
    let id = project.id;
    assert_eq!(project.name, "Project");
    assert!(project.status.accepts_mutations());
    project.set_status(
        ProjectStatus::Archived,
        Timestamp::parse("2026-07-22T00:01:00Z").unwrap(),
    );
    assert!(!project.status.accepts_mutations());
    project.set_status(
        ProjectStatus::Active,
        Timestamp::parse("2026-07-22T00:02:00Z").unwrap(),
    );
    assert_eq!(project.id, id);
    assert!(project.status.accepts_mutations());
}

#[test]
fn repository_association_uses_only_stable_repository_id() {
    let project_id = ProjectId::new_v7();
    let association = ProjectRepositoryAssociation::new(
        ProjectRepositoryAssociationId::new_v7(),
        project_id,
        "repository-row-id",
        now(),
        7,
    )
    .unwrap();
    assert_eq!(association.project_id, project_id);
    assert_eq!(association.repository_id, "repository-row-id");
    assert_eq!(association.event_seq, 7);
}

#[test]
fn revisions_are_positive_immutable_values_with_same_task_earlier_parents() {
    let project = ProjectId::new_v7();
    let task = Task::new(TaskId::new_v7(), project, "Task", now()).unwrap();
    let revision_one = TaskRevision::new(
        TaskRevisionId::new_v7(),
        task.id,
        1,
        None,
        contract(),
        now(),
    )
    .unwrap();
    revision_one.validate_parent(None).unwrap();

    let revision_two = TaskRevision::new(
        TaskRevisionId::new_v7(),
        task.id,
        2,
        Some(revision_one.id),
        contract(),
        now(),
    )
    .unwrap();
    revision_two.validate_parent(Some(&revision_one)).unwrap();

    let other_task_parent = TaskRevision::new(
        TaskRevisionId::new_v7(),
        TaskId::new_v7(),
        1,
        None,
        contract(),
        now(),
    )
    .unwrap();
    assert!(revision_two
        .validate_parent(Some(&other_task_parent))
        .is_err());
    assert_eq!(task.project_id, project, "task ownership remains permanent");
}
