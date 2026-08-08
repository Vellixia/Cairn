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

/// Daemon control socket. Kept short: Unix socket paths are length-limited.
pub fn socket_path() -> PathBuf {
    if let Ok(p) = std::env::var("CAIRN_SOCKET") {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    home().join("cairnd.sock")
}

pub fn config_path() -> PathBuf {
    home().join("config.json")
}

/// Server API token, stored 0600 (D10).
pub fn token_path() -> PathBuf {
    home().join("token")
}

pub fn log_path() -> PathBuf {
    home().join("cairn.log")
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
