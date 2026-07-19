//! `cairn init` (T030).

use cairn_protocol::{methods, RegisterParams};

use crate::{ipc, output};

pub async fn run(json: bool) -> i32 {
    let params = RegisterParams { path: super::cwd() };
    let result = ipc::call(methods::REPOSITORY_REGISTER, &params).await;
    output::emit("init", json, result)
}
