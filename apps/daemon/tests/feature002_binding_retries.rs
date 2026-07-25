mod support;

use cairn_protocol::*;
use cairn_storage_local::{events, open_pool_at};
use support::binding::{retry_iterations, BindingFixture};
use support::TestDaemon;

#[tokio::test(flavor = "multi_thread")]
async fn configurable_identical_binding_retries_return_one_immutable_result() {
    let daemon = TestDaemon::start().await;
    let fixture = BindingFixture::create(&daemon).await;
    let key = IdempotencyKey::new_v7();
    let params = fixture.bind_params(key);
    let mut original = None;
    let iterations = retry_iterations();
    for _ in 0..iterations {
        let result: SessionBindResult =
            serde_json::from_value(daemon.call(methods::SESSION_BIND, &params).await.unwrap())
                .unwrap();
        assert!(result.created);
        match original.as_ref() {
            Some(original) => assert_eq!(&result, original),
            None => original = Some(result),
        }
    }
    let pool = open_pool_at(&daemon.db_path()).await.unwrap();
    let (registry_count,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM operation_idempotency WHERE idempotency_key=? AND method='session.bind'",
    )
    .bind(key.to_string())
    .fetch_one(&pool)
    .await
    .unwrap();
    let (projection_count,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM session_bindings WHERE session_id=?")
            .bind(fixture.session_id.to_string())
            .fetch_one(&pool)
            .await
            .unwrap();
    let event_count = events::list_events(
        &pool,
        None,
        None,
        Some(&fixture.session_id.to_string()),
        None,
        100,
    )
    .await
    .unwrap()
    .into_iter()
    .filter(|event| event.event_type == "session.bound")
    .count();
    assert_eq!(registry_count, 1);
    assert_eq!(projection_count, 1);
    assert_eq!(event_count, 1);
    println!(
        "binding_retries={{\"configured\":{iterations},\"completed\":{iterations},\"events\":{event_count},\"projections\":{projection_count},\"registry_results\":{registry_count}}}"
    );
    pool.close().await;
    drop(fixture);
    daemon.stop().await;
}
