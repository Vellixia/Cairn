//! T023/T050: secure resume-token input (FR-029, analysis U3).
//! Resolution order: --resume-token-stdin → CAIRN_RESUME_TOKEN → file.
//! Tokens are NEVER accepted as ordinary argv and never echoed.

use std::io::Read;
use std::path::Path;

use cairn_protocol::{ErrorBody, ErrorCode};

pub fn resolve(stdin_flag: bool, file: Option<&Path>) -> Result<String, ErrorBody> {
    if stdin_flag {
        let mut buf = String::new();
        std::io::stdin()
            .read_to_string(&mut buf)
            .map_err(|e| usage(format!("cannot read token from stdin: {e}")))?;
        let token = buf.lines().next().unwrap_or("").trim().to_string();
        if token.is_empty() {
            return Err(usage("empty resume token on stdin".into()));
        }
        return Ok(token);
    }
    if let Ok(token) = std::env::var("CAIRN_RESUME_TOKEN") {
        let token = token.trim().to_string();
        if !token.is_empty() {
            return Ok(token);
        }
    }
    if let Some(path) = file {
        let token = std::fs::read_to_string(path)
            .map_err(|e| usage(format!("cannot read token file: {e}")))?
            .trim()
            .to_string();
        if token.is_empty() {
            return Err(usage("token file is empty".into()));
        }
        return Ok(token);
    }
    Err(usage(
        "resume token required: use --resume-token-stdin, CAIRN_RESUME_TOKEN, or --resume-token-file"
            .into(),
    ))
}

/// Optional variant for commands where the token is only sometimes needed.
pub fn resolve_optional(stdin_flag: bool, file: Option<&Path>) -> Option<String> {
    resolve(stdin_flag, file).ok()
}

fn usage(message: String) -> ErrorBody {
    ErrorBody {
        code: ErrorCode::Usage,
        message,
        data: None,
    }
}
