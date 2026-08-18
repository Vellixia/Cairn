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

/// The per-machine salt behind a pattern's `origin_ref` (FR-393).
pub fn machine_salt_path() -> PathBuf {
    home().join("machine-salt")
}

/// Create the state directory if it does not exist.
pub fn ensure_home() -> std::io::Result<PathBuf> {
    let h = home();
    std::fs::create_dir_all(&h)?;
    Ok(h)
}

/// A stable random value this machine keeps to itself.
///
/// A reusable pattern holds **no project identity** — no name, no path, no
/// remote, no id. What it holds instead is `origin_ref`, a digest of the source
/// project salted with this value, which answers "did these two patterns come
/// from the same project?" without answering "which project?".
///
/// Salted rather than a bare digest because a project id is a UUID and a bare
/// digest of one is a lookup away from being reversed by anyone holding the
/// projects table — and because two machines must not produce the same
/// `origin_ref` for the same project, which would correlate them across a
/// boundary patterns never cross (FR-508).
///
/// Created on first use, 0600 on Unix, and never transmitted. A machine that
/// loses it produces new references for the same project, which makes older
/// patterns' origins unmatched rather than wrong.
pub fn machine_salt() -> std::io::Result<String> {
    let path = machine_salt_path();
    if let Ok(existing) = std::fs::read_to_string(&path) {
        let trimmed = existing.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
    }

    ensure_home()?;
    let salt = crate::domain::new_id().to_string() + &crate::domain::new_id().to_string();
    std::fs::write(&path, &salt)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(salt)
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
