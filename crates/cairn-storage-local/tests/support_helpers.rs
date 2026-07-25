mod support;

#[tokio::test]
async fn database_builders_copy_frozen_fixture_and_open_independent_pools() {
    let fixture = support::TestDatabase::from_feature001_fixture().await;
    let independent =
        support::independent_pool(&fixture.path, std::time::Duration::from_millis(250)).await;
    let (repositories,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM repositories")
        .fetch_one(&independent)
        .await
        .unwrap();
    assert_eq!(repositories, 4);
    independent.close().await;

    let empty = support::TestDatabase::empty().await;
    let (repositories,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM repositories")
        .fetch_one(&empty.pool)
        .await
        .unwrap();
    assert_eq!(repositories, 0);
}
