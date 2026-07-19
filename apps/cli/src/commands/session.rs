//! `cairn session ...` (T037 start/show/stop, T050 heartbeat/reattach).
//! Resume tokens travel only via secure input (stdin/env/file) — never argv.

use std::str::FromStr;

use cairn_protocol::*;

use crate::{ipc, output, token, SessionCommand};

fn usage(message: String) -> ErrorBody {
    ErrorBody {
        code: ErrorCode::Usage,
        message,
        data: None,
    }
}

fn parse_instance(s: Option<String>) -> Result<Option<AgentInstanceId>, ErrorBody> {
    match s {
        None => Ok(None),
        Some(v) => AgentInstanceId::from_str(&v)
            .map(Some)
            .map_err(|e| usage(format!("invalid agent instance id: {e}"))),
    }
}

fn parse_session_id(s: &str) -> Result<SessionId, ErrorBody> {
    SessionId::from_str(s).map_err(|e| usage(format!("invalid session id: {e}")))
}

pub async fn run(json: bool, cmd: SessionCommand) -> i32 {
    match cmd {
        SessionCommand::Start {
            agent,
            agent_instance,
            agent_pid,
        } => {
            let instance = match parse_instance(agent_instance) {
                Ok(v) => v,
                Err(e) => return output::emit("session.start", json, Err(e)),
            };
            // Generate an instance id when the caller has none; it is
            // reported back in data.session.agent_instance_id.
            let agent_instance_id =
                instance.unwrap_or_else(|| AgentInstanceId(uuid::Uuid::new_v4()));
            let params = SessionStartParams {
                path: Some(super::cwd()),
                repository_id: None,
                agent_type: agent,
                agent_instance_id,
                agent_pid,
            };
            let result = ipc::call(methods::SESSION_START, &params).await;
            output::emit("session.start", json, result)
        }
        SessionCommand::Show {
            session,
            agent_instance,
            agent_type,
        } => {
            let instance = match parse_instance(agent_instance) {
                Ok(v) => v,
                Err(e) => return output::emit("session.show", json, Err(e)),
            };
            let session_id = match session.as_deref().map(parse_session_id).transpose() {
                Ok(v) => v,
                Err(e) => return output::emit("session.show", json, Err(e)),
            };
            let params = SessionGetParams {
                path: Some(super::cwd()),
                repository_id: None,
                session_id,
                agent_instance_id: instance,
                agent_type,
            };
            let result = ipc::call(methods::SESSION_GET, &params).await;
            output::emit("session.show", json, result)
        }
        SessionCommand::Heartbeat {
            session,
            agent_instance,
            resume_token_stdin,
            resume_token_file,
        } => {
            let resume_token =
                match token::resolve(resume_token_stdin, resume_token_file.as_deref()) {
                    Ok(t) => t,
                    Err(e) => return output::emit("session.heartbeat", json, Err(e)),
                };
            let session_id = match parse_session_id(&session) {
                Ok(v) => v,
                Err(e) => return output::emit("session.heartbeat", json, Err(e)),
            };
            let instance = match require_instance(agent_instance) {
                Ok(v) => v,
                Err(e) => return output::emit("session.heartbeat", json, Err(e)),
            };
            let params = SessionHeartbeatParams {
                session_id,
                agent_instance_id: instance,
                resume_token,
            };
            let result = ipc::call(methods::SESSION_HEARTBEAT, &params).await;
            output::emit("session.heartbeat", json, result)
        }
        SessionCommand::Reattach {
            session,
            agent_instance,
            resume_token_stdin,
            resume_token_file,
        } => {
            let resume_token =
                match token::resolve(resume_token_stdin, resume_token_file.as_deref()) {
                    Ok(t) => t,
                    Err(e) => return output::emit("session.reattach", json, Err(e)),
                };
            let session_id = match parse_session_id(&session) {
                Ok(v) => v,
                Err(e) => return output::emit("session.reattach", json, Err(e)),
            };
            let instance = match require_instance(agent_instance) {
                Ok(v) => v,
                Err(e) => return output::emit("session.reattach", json, Err(e)),
            };
            let params = SessionReattachParams {
                session_id,
                agent_instance_id: instance,
                resume_token,
            };
            let result = ipc::call(methods::SESSION_REATTACH, &params).await;
            output::emit("session.reattach", json, result)
        }
        SessionCommand::Stop {
            session,
            agent_instance,
            resume_token_stdin,
            resume_token_file,
        } => {
            let instance = match parse_instance(agent_instance) {
                Ok(v) => v,
                Err(e) => return output::emit("session.stop", json, Err(e)),
            };
            let session_id = match session.as_deref().map(parse_session_id).transpose() {
                Ok(v) => v,
                Err(e) => return output::emit("session.stop", json, Err(e)),
            };
            let params = SessionStopParams {
                session_id,
                repository_id: None,
                path: Some(super::cwd()),
                agent_instance_id: instance,
                resume_token: token::resolve_optional(
                    resume_token_stdin,
                    resume_token_file.as_deref(),
                ),
            };
            let result = ipc::call(methods::SESSION_STOP, &params).await;
            output::emit("session.stop", json, result)
        }
    }
}

fn require_instance(agent_instance: Option<String>) -> Result<AgentInstanceId, ErrorBody> {
    parse_instance(agent_instance)?.ok_or_else(|| {
        usage("agent instance id required (--agent-instance or CAIRN_AGENT_INSTANCE)".into())
    })
}
