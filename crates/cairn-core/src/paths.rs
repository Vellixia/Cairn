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
///
/// Creation is atomic, and it has to be. The daemon and the CLI are separate
/// processes over one state directory, so on a fresh machine two of them can
/// reach "the salt is not there yet" at the same moment. A plain write lets both
/// generate a salt and the later one win — leaving whoever computed an
/// `origin_ref` from the losing salt with a reference that no longer matches its
/// own project. A pattern would then stop recognizing the project it came from,
/// and validate itself there (FR-402).
///
/// So the salt is written to a temporary file and **linked** into place, which
/// fails rather than overwrites if somebody else got there first. The loser
/// reads the winner's value, and every process agrees.
pub fn machine_salt() -> std::io::Result<String> {
    let path = machine_salt_path();
    let read_existing = || -> Option<String> {
        let existing = std::fs::read_to_string(&path).ok()?;
        let trimmed = existing.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    };
    if let Some(salt) = read_existing() {
        return Ok(salt);
    }

    ensure_home()?;
    let salt = crate::domain::new_id().to_string() + &crate::domain::new_id().to_string();

    // Unique per attempt: a stale temporary file from a killed process must not
    // make every later attempt fail.
    let tmp = path.with_file_name(format!(
        "machine-salt.{}.{}",
        std::process::id(),
        crate::domain::new_id()
    ));
    std::fs::write(&tmp, &salt)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
    }

    let won = std::fs::hard_link(&tmp, &path).is_ok();
    let _ = std::fs::remove_file(&tmp);
    if won {
        return Ok(salt);
    }

    // Somebody else linked theirs in first — or the filesystem cannot link, in
    // which case the pre-existing behaviour is still the best available.
    read_existing().map_or_else(
        || {
            std::fs::write(&path, &salt)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
            }
            Ok(salt.clone())
        },
        Ok,
    )
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

    /// Every caller that races to create the salt ends up with the same one.
    ///
    /// The daemon and the CLI are separate processes over one state directory,
    /// so on a fresh machine both can find no salt at the same moment. A plain
    /// write let both invent one and the later win, and a caller holding the
    /// losing salt computed an `origin_ref` that no longer matched its own
    /// project — so a pattern stopped recognizing where it came from and
    /// validated itself there (FR-402). It surfaced as a test that failed about
    /// one run in three, which is exactly how long a race like this survives.
    #[test]
    fn a_racing_salt_creation_agrees_on_one_value() {
        let prev = std::env::var("CAIRN_HOME").ok();
        let dir = std::env::temp_dir().join(format!("cairn-salt-race-{}", crate::domain::new_id()));
        std::env::set_var("CAIRN_HOME", &dir);

        let salts: Vec<String> = std::thread::scope(|s| {
            let handles: Vec<_> = (0..8)
                .map(|_| s.spawn(|| machine_salt().expect("salt")))
                .collect();
            handles
                .into_iter()
                .map(|h| h.join().expect("joined"))
                .collect()
        });

        let first = &salts[0];
        assert!(
            salts.iter().all(|s| s == first),
            "racing callers produced different salts: {salts:?}"
        );
        assert_eq!(
            std::fs::read_to_string(machine_salt_path())
                .expect("salt file")
                .trim(),
            first.as_str(),
            "the stored salt is not the one the callers were given"
        );
        // No temporary file is left behind for the next caller to trip over.
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .expect("home")
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n != "machine-salt")
            .collect();
        assert!(leftovers.is_empty(), "left behind: {leftovers:?}");

        let _ = std::fs::remove_dir_all(&dir);
        match prev {
            Some(v) => std::env::set_var("CAIRN_HOME", v),
            None => std::env::remove_var("CAIRN_HOME"),
        }
    }
}
