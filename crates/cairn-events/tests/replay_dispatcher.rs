use cairn_events::replay::{ReplayDispatchError, ReplayDispatcher, ReplayHandler};
use serde_json::json;

struct CounterHandler;

impl ReplayHandler<u64> for CounterHandler {
    fn event_type(&self) -> &'static str {
        "foundation.test"
    }

    fn schema_version(&self) -> u16 {
        1
    }

    fn apply(
        &self,
        state: &mut u64,
        payload: &serde_json::Value,
    ) -> Result<(), ReplayDispatchError> {
        *state += payload
            .get("amount")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| ReplayDispatchError::InvalidPayload("foundation.test".into()))?;
        Ok(())
    }
}

#[test]
fn typed_dispatcher_rejects_unknown_types_and_versions_before_story_handlers() {
    let mut dispatcher = ReplayDispatcher::default();
    dispatcher.register(CounterHandler).unwrap();
    assert!(dispatcher.register(CounterHandler).is_err());

    let mut state = 0;
    dispatcher
        .dispatch(
            &mut state,
            "foundation.test",
            &json!({"schema_version":1,"amount":2}),
        )
        .unwrap();
    assert_eq!(state, 2);
    assert!(matches!(
        dispatcher.dispatch(
            &mut state,
            "foundation.test",
            &json!({"schema_version":2,"amount":2})
        ),
        Err(ReplayDispatchError::UnsupportedPayloadVersion { .. })
    ));
    assert!(matches!(
        dispatcher.dispatch(&mut state, "project.created", &json!({"schema_version":1})),
        Err(ReplayDispatchError::UnsupportedEventType(_))
    ));
}
