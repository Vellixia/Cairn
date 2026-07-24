use cairn_events::{
    ProjectCreatedPayload, ProjectRepositoryAssociatedPayload, ProjectUpdatedPayload,
};
use schemars::schema_for;

#[test]
fn project_event_payload_schemas_remain_typed_and_replay_complete() {
    let created = serde_json::to_string(&schema_for!(ProjectCreatedPayload)).unwrap();
    let updated = serde_json::to_string(&schema_for!(ProjectUpdatedPayload)).unwrap();
    let associated =
        serde_json::to_string(&schema_for!(ProjectRepositoryAssociatedPayload)).unwrap();

    for stable_field in [
        "project_id",
        "name",
        "description",
        "status",
        "created_at",
        "updated_at",
    ] {
        assert!(created.contains(stable_field));
        assert!(updated.contains(stable_field));
    }
    assert!(updated.contains("changed_fields"));
    for stable_field in [
        "association_id",
        "project_id",
        "repository_id",
        "associated_at",
    ] {
        assert!(associated.contains(stable_field));
    }
    assert!(!associated.contains("canonical_path"));
    assert!(!associated.contains("remote_url"));
}
