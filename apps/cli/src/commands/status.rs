//! `cairn status [--ignored [--cursor C]]` (T030, FR-035).

use cairn_protocol::{methods, IgnoredFilesParams, InspectParams};

use crate::{ipc, output};

pub async fn run(json: bool, ignored: bool, cursor: Option<String>) -> i32 {
    let params = InspectParams {
        path: Some(super::cwd()),
        repository_id: None,
    };
    if !ignored {
        let result = ipc::call(methods::REPOSITORY_INSPECT, &params).await;
        return output::emit("status", json, result);
    }

    // Drill-down: resolve repository id via inspect, then page ignored files.
    let mut client = match ipc::connect().await {
        Ok(c) => c,
        Err(e) => return output::emit("status.ignored", json, Err(e)),
    };
    let inspect = match client.call(methods::REPOSITORY_INSPECT, &params).await {
        Ok(v) => v,
        Err(e) => return output::emit("status.ignored", json, Err(e)),
    };
    // The inspection payload carries the worktree; the repository id comes
    // from a register round-trip which is idempotent and cheap.
    let register = match client
        .call(
            methods::REPOSITORY_REGISTER,
            &cairn_protocol::RegisterParams { path: super::cwd() },
        )
        .await
    {
        Ok(v) => v,
        Err(e) => return output::emit("status.ignored", json, Err(e)),
    };
    let _ = inspect;
    let repository_id = register["repository"]["repository_id"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    let page_params = IgnoredFilesParams {
        repository_id,
        cursor,
        limit: Some(1000),
        glob: None,
    };
    let result = client
        .call(methods::REPOSITORY_IGNORED_FILES, &page_params)
        .await;
    output::emit("status.ignored", json, result)
}
