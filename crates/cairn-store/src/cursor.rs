//! Server-instance binding retained for typed spool identities.

use crate::{Result, Store, StoreError};
use uuid::Uuid;

/// Return the one deployment to which this edge was bound before the old
/// namespace runtime was removed. Multiple bindings fail closed.
pub async fn bound_server_instance(store: &Store) -> Result<Option<Uuid>> {
    let keys: Vec<String> = sqlx::query_scalar(
        "SELECT namespace FROM sync_cursor WHERE namespace LIKE 'team:%' ORDER BY namespace",
    )
    .fetch_all(store.pool())
    .await?;
    let mut bound = None;
    for key in keys {
        let Some(raw) = key.strip_prefix("team:") else {
            continue;
        };
        let instance = Uuid::parse_str(raw)
            .map_err(|error| StoreError::Corrupt(format!("invalid team binding {key}: {error}")))?;
        match bound {
            Some(existing) if existing != instance => {
                return Err(StoreError::Corrupt(format!(
                    "this store holds team bindings for {existing} and {instance}"
                )))
            }
            _ => bound = Some(instance),
        }
    }
    Ok(bound)
}
