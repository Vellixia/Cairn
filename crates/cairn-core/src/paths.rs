//! Where Cairn keeps its local state.
//!
//! Every path honours `CAIRN_HOME`, which is what lets tests run fully
//! isolated against a real SQLite file and a real socket.

use std::path::PathBuf;

/// Root of Cairn's local state.
///
/// `CAIRN_HOME` wins; otherwise the platform data directory.
pub fn home() -> PathBuf {
    if let Ok(dir) = std::env::var("CAIRN_HOME") {
        if !dir.is_empty() {
            return PathBuf::from(dir);
        }
    }
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("cairn")
}

/// The local SQLite database (D2).
pub fn db_path() -> PathBuf {
    home().join("cairn.sqlite3")
}

/// Daemon control endpoint: a Unix domain socket path on Unix, a named pipe
/// name on Windows. `CAIRN_SOCKET` always wins, which is what lets tests run
/// several daemons side by side.
#[cfg(unix)]
pub fn socket_path() -> PathBuf {
    if let Ok(p) = std::env::var("CAIRN_SOCKET") {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    // Unix socket paths are length-limited, so this is kept short.
    home().join("cairnd.sock")
}

/// Windows has no filesystem-addressed local socket; named pipes live in
/// their own `\\.\pipe\` namespace instead. The name is derived from `home()`
/// (which is already per-user, via `%APPDATA%` or `CAIRN_HOME`) so two users
/// on the same machine never collide on one pipe.
#[cfg(windows)]
pub fn socket_path() -> PathBuf {
    if let Ok(p) = std::env::var("CAIRN_SOCKET") {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(home().to_string_lossy().as_bytes());
    let digest = hex::encode(hasher.finalize());
    PathBuf::from(format!(r"\\.\pipe\cairnd-{}", &digest[..16]))
}

pub fn config_path() -> PathBuf {
    home().join("config.json")
}

/// Server API token, stored 0600 on Unix (D10).
///
/// Windows has no mode bits, so there the file is only as private as the
/// directory holding it — which is under the user's own profile, and so is
/// not readable by other unprivileged users, but carries no explicit ACL of
/// its own.
pub fn token_path() -> PathBuf {
    home().join("token")
}

pub fn log_path() -> PathBuf {
    home().join("cairn.log")
}

/// The daemon's log.
///
/// Separate from `cairn.log`, which the hook writes to from the agent's
/// process: the two have different writers and different lifetimes.
pub fn daemon_log_path() -> PathBuf {
    home().join("cairnd.log")
}

/// Create the state directory if it does not exist.
pub fn ensure_home() -> std::io::Result<PathBuf> {
    let h = home();
    std::fs::create_dir_all(&h)?;
    Ok(h)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cairn_home_overrides_everything() {
        // Serialized within this test only; env is process-global.
        let prev = std::env::var("CAIRN_HOME").ok();
        std::env::set_var("CAIRN_HOME", "/tmp/cairn-test-home");
        assert_eq!(home(), PathBuf::from("/tmp/cairn-test-home"));
        assert!(db_path().starts_with("/tmp/cairn-test-home"));
        match prev {
            Some(v) => std::env::set_var("CAIRN_HOME", v),
            None => std::env::remove_var("CAIRN_HOME"),
        }
    }
}
