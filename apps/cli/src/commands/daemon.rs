//! `cairn daemon status` (T030).

use cairn_protocol::methods;

use crate::{ipc, output};

pub async fn run(json: bool) -> i32 {
    let result = ipc::call(methods::DAEMON_STATUS, &serde_json::json!({})).await;
    output::emit("daemon.status", json, result)
}
