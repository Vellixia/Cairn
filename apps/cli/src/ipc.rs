//! T022: IPC client with daemon auto-spawn (research R11).

use std::time::Duration;

use cairn_protocol::{ErrorBody, ErrorCode, Request, Response};
use serde::Serialize;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

pub struct Client {
    #[cfg(unix)]
    stream: tokio::net::UnixStream,
    #[cfg(windows)]
    stream: tokio::net::windows::named_pipe::NamedPipeClient,
}

fn spawn_daemon() -> Result<(), ErrorBody> {
    let bin = std::env::var("CAIRN_DAEMON_BIN")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            let mut p = std::env::current_exe().unwrap_or_default();
            p.set_file_name(if cfg!(windows) {
                "cairnd.exe"
            } else {
                "cairnd"
            });
            p
        });
    std::process::Command::new(&bin)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| ErrorBody {
            code: ErrorCode::DaemonUnavailable,
            message: format!("cannot spawn daemon {}: {e}", bin.display()),
            data: None,
        })
}

async fn try_connect() -> std::io::Result<Client> {
    #[cfg(unix)]
    {
        let path = std::env::var("CAIRN_SOCKET_PATH")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| {
                if let Ok(runtime) = std::env::var("XDG_RUNTIME_DIR") {
                    return std::path::PathBuf::from(runtime)
                        .join("cairn")
                        .join("daemon.sock");
                }
                cairn_data_dir().join("daemon.sock")
            });
        let stream = tokio::net::UnixStream::connect(path).await?;
        Ok(Client { stream })
    }
    #[cfg(windows)]
    {
        let name = std::env::var("CAIRN_PIPE_NAME").unwrap_or_else(|_| {
            format!(
                "cairn-{}-daemon",
                std::env::var("USERNAME").unwrap_or_else(|_| "user".into())
            )
        });
        let addr = format!(r"\\.\pipe\{name}");
        let stream = tokio::net::windows::named_pipe::ClientOptions::new().open(&addr)?;
        Ok(Client { stream })
    }
}

#[cfg(unix)]
fn cairn_data_dir() -> std::path::PathBuf {
    if let Ok(dir) = std::env::var("CAIRN_DATA_DIR") {
        return std::path::PathBuf::from(dir);
    }
    dirs_data_local().join("cairn")
}

#[cfg(unix)]
fn dirs_data_local() -> std::path::PathBuf {
    if let Ok(x) = std::env::var("XDG_DATA_HOME") {
        return x.into();
    }
    std::env::var("HOME")
        .map(|h| std::path::PathBuf::from(h).join(".local/share"))
        .unwrap_or_else(|_| ".".into())
}

/// Connect, auto-spawning the daemon when absent (~3 s retry budget).
/// `CAIRN_NO_SPAWN=1` disables auto-spawn (tests, supervised setups).
pub async fn connect() -> Result<Client, ErrorBody> {
    if let Ok(c) = try_connect().await {
        return Ok(c);
    }
    if std::env::var("CAIRN_NO_SPAWN").is_ok() {
        return Err(ErrorBody {
            code: ErrorCode::DaemonUnavailable,
            message: "daemon not running and auto-spawn disabled (CAIRN_NO_SPAWN)".into(),
            data: None,
        });
    }
    spawn_daemon()?;
    let mut delay = Duration::from_millis(50);
    for _ in 0..8 {
        tokio::time::sleep(delay).await;
        if let Ok(c) = try_connect().await {
            return Ok(c);
        }
        delay = (delay * 2).min(Duration::from_millis(800));
    }
    Err(ErrorBody {
        code: ErrorCode::DaemonUnavailable,
        message: "daemon did not become reachable within retry budget".into(),
        data: None,
    })
}

impl Client {
    pub async fn call<P: Serialize>(
        &mut self,
        method: &str,
        params: &P,
    ) -> Result<serde_json::Value, ErrorBody> {
        let req = Request {
            id: uuid::Uuid::new_v4().to_string(),
            method: method.to_string(),
            params: serde_json::to_value(params).expect("serializable params"),
        };
        let mut line = serde_json::to_string(&req).expect("serializable request");
        line.push('\n');

        let io_err = |e: std::io::Error| ErrorBody {
            code: ErrorCode::DaemonUnavailable,
            message: format!("ipc failure: {e}"),
            data: None,
        };

        self.stream
            .write_all(line.as_bytes())
            .await
            .map_err(io_err)?;
        let mut reader = BufReader::new(&mut self.stream);
        let mut buf = String::new();
        reader.read_line(&mut buf).await.map_err(io_err)?;
        let resp: Response = serde_json::from_str(&buf).map_err(|e| ErrorBody {
            code: ErrorCode::Internal,
            message: format!("malformed daemon response: {e}"),
            data: None,
        })?;
        match (resp.result, resp.error) {
            (Some(v), None) => Ok(v),
            (_, Some(e)) => Err(e),
            _ => Err(ErrorBody {
                code: ErrorCode::Internal,
                message: "daemon response had neither result nor error".into(),
                data: None,
            }),
        }
    }
}

/// One-shot convenience.
pub async fn call<P: Serialize>(method: &str, params: &P) -> Result<serde_json::Value, ErrorBody> {
    let mut client = connect().await?;
    client.call(method, params).await
}
