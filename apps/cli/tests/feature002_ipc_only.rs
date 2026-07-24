use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use cairn_protocol::{CliEnvelope, ErrorCode};

#[test]
fn cli_dependency_and_source_tripwire_forbids_storage_access() {
    let manifest =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml")).unwrap();
    for forbidden in ["sqlx", "cairn-storage-local", "cairn_storage_local"] {
        assert!(
            !manifest.contains(forbidden),
            "CLI dependency added: {forbidden}"
        );
    }

    let mut files = Vec::new();
    collect_rs(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut files,
    );
    for path in files {
        let source = std::fs::read_to_string(&path).unwrap();
        for forbidden in [
            "sqlx::",
            "SqlitePool",
            "SqliteConnection",
            "cairn_storage_local",
            "open_pool(",
            "BEGIN IMMEDIATE",
        ] {
            assert!(
                !source.contains(forbidden),
                "{} imports or opens storage through {forbidden}",
                path.display()
            );
        }
    }

    for command in ["project.rs", "task.rs", "session.rs"] {
        let source = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("src/commands")
                .join(command),
        )
        .unwrap();
        assert!(
            source.contains("ipc::call("),
            "{command} bypasses daemon IPC"
        );
    }
}

#[test]
fn daemon_unavailability_is_one_typed_json_envelope_and_exit_five() {
    let dir = tempfile::TempDir::new().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_cairn"))
        .args(["daemon", "status", "--json"])
        .env("CAIRN_NO_SPAWN", "1")
        .env("CAIRN_SOCKET_PATH", dir.path().join("missing.sock"))
        .env("CAIRN_PIPE_NAME", "cairn-definitely-missing-pipe")
        .env("CAIRN_DATA_DIR", dir.path().join("data"))
        .output()
        .unwrap();
    assert_envelope(output, ErrorCode::DaemonUnavailable, 5);
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn malformed_daemon_response_fails_closed_as_one_json_envelope() {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    let dir = tempfile::TempDir::new().unwrap();
    let socket = dir.path().join("malformed.sock");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut line = String::new();
        BufReader::new(&mut stream)
            .read_line(&mut line)
            .await
            .unwrap();
        assert!(line.contains("v1.daemon.status"));
        stream.write_all(b"not-json\n").await.unwrap();
    });

    let data_dir = dir.path().join("data");
    let socket_for_child = socket.clone();
    let output = tokio::task::spawn_blocking(move || {
        Command::new(env!("CARGO_BIN_EXE_cairn"))
            .args(["daemon", "status", "--json"])
            .env("CAIRN_NO_SPAWN", "1")
            .env("CAIRN_SOCKET_PATH", socket_for_child)
            .env("CAIRN_DATA_DIR", data_dir)
            .output()
            .unwrap()
    })
    .await
    .unwrap();
    server.await.unwrap();
    assert_envelope(output, ErrorCode::Internal, 1);
}

fn assert_envelope(output: Output, code: ErrorCode, exit: i32) {
    assert_eq!(output.status.code(), Some(exit));
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(stdout.lines().count(), 1, "stdout={stdout:?}");
    let envelope: CliEnvelope = serde_json::from_str(stdout.trim()).unwrap();
    let error = envelope.error.expect("error envelope");
    assert_eq!(error.code, code);
    assert!(envelope.data.is_none());
    let rendered = serde_json::to_string(&error).unwrap();
    for forbidden in ["token-value", "CAIRN_SECRET", "private-goal-sentinel"] {
        assert!(!rendered.contains(forbidden));
    }
}

fn collect_rs(root: &Path, files: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(root).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            collect_rs(&path, files);
        } else if path.extension().and_then(|value| value.to_str()) == Some("rs") {
            files.push(path);
        }
    }
}
