#[path = "support/feature002.rs"]
mod feature002;

#[test]
fn feature002_daemon_helpers_are_deterministic_and_use_the_frozen_fixture() {
    let first = feature002::deterministic_agent_instance(42);
    let second = feature002::deterministic_agent_instance(42);
    assert_eq!(first, second);
    assert!(feature002::feature001_database_fixture().is_file());
    let repository = feature002::repository();
    assert!(repository.root().join(".git").exists());
}
