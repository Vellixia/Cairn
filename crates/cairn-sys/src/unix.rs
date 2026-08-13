//! Unix process introspection and signalling.
//!
//! Identifies `cairnd` processes by the `CAIRN_SOCKET` value in their
//! environment, the same way the original `pgrep -f cairnd | ps eww -p <pid>`
//! pipeline did, but without spawning subprocesses. Reading another process's
//! environment from `/proc` is Linux-specific; on macOS we fall back to the
//! `ps eww -p <pid>` command, which is the same approach the test used before
//! and which works on Darwin because `ps` is allowed to read another
//! process's environment there.

use std::path::Path;

/// PIDs of processes running `cairnd` whose `CAIRN_SOCKET` matches `socket`.
///
/// Returns an empty vector if none are found, or if enumeration fails — the
/// caller (a test) asserts non-empty when it expects a daemon to be running.
pub fn daemons_for_socket(socket: &Path) -> Vec<i64> {
    let needle = socket.display().to_string();
    list_processes_named("cairnd")
        .into_iter()
        .filter(|pid| env_contains(*pid, "CAIRN_SOCKET", &needle))
        .collect()
}

/// Hard-kill a process the way a crash does.
///
/// `SIGKILL` is uncatchable and unstoppably terminates the target. Returns
/// `true` if the signal was sent; the process may still be dying.
pub fn kill(pid: i64) -> bool {
    // SAFETY: `kill(2)` is signal-safe and POSIX-defined. A stale PID is
    // harmless: the kernel reports `ESRCH` and we return `false`.
    unsafe { libc::kill(pid as i32, libc::SIGKILL) == 0 }
}

/// Whether `pid` is currently running.
///
/// A zombie counts as *not* running. It has already exited and only lingers
/// as an unreaped table entry, but `kill(pid, 0)` still succeeds for it — so
/// checking the signal alone reports a hard-killed daemon as alive forever.
/// That is not hypothetical here: `cairn` spawns the daemon and exits
/// immediately, orphaning it, so whether anything reaps it depends on whether
/// this host's init does. On one that does not, `wait_for_exit` would never
/// return true and a working `SIGKILL` would look broken.
pub fn is_running(pid: i64) -> bool {
    if is_zombie(pid) {
        return false;
    }
    // SAFETY: `kill(2)` with signal 0 performs no signal delivery; it is the
    // standard liveness check. `ESRCH` means "no such process".
    unsafe {
        libc::kill(pid as i32, 0) == 0
            || std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
    }
}

/// Whether `pid` has exited but not yet been reaped.
///
/// Implemented for every Unix, not only Linux. The hazard is the same
/// everywhere — `kill(pid, 0)` succeeds for a zombie, so a hard-killed
/// daemon reads as alive forever and `wait_for_exit` never returns true —
/// and macOS is the platform where it is most likely to bite, because it is
/// the one the local agent is developed on. Linux reads `/proc/<pid>/stat`;
/// elsewhere `ps` reports the state directly.
fn is_zombie(pid: i64) -> bool {
    #[cfg(target_os = "linux")]
    {
        if let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) {
            // `pid (comm) state ...`. The comm field is arbitrary and may
            // contain both spaces and parentheses, so the state letter is
            // found after the *last* ')', not by splitting on whitespace.
            if let Some((_, rest)) = stat.rsplit_once(')') {
                return rest.split_whitespace().next() == Some("Z");
            }
        }
        false
    }
    #[cfg(not(target_os = "linux"))]
    {
        // `ps -o state=` prints just the state column. On Darwin and the BSDs
        // a zombie is `Z`, possibly with a suffix flag such as `Z+`.
        let out = std::process::Command::new("ps")
            .args(["-o", "state=", "-p", &pid.to_string()])
            .output();
        match out {
            Ok(o) => String::from_utf8_lossy(&o.stdout).trim().starts_with('Z'),
            Err(_) => false,
        }
    }
}

/// Wait briefly for `pid` to exit, polling at 20 ms up to `deadline`.
pub fn wait_for_exit(pid: i64, deadline: std::time::Duration) -> bool {
    let end = std::time::Instant::now() + deadline;
    while std::time::Instant::now() < end {
        if !is_running(pid) {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    false
}

/// Enumerate processes whose executable name contains `name_substring`.
///
/// On Linux this walks `/proc`; on macOS and other Unices it shells out to
/// `pgrep -f`, which is the same approach the test suite already took.
fn list_processes_named(name_substring: &str) -> Vec<i64> {
    #[cfg(target_os = "linux")]
    {
        list_proc_linux(name_substring)
    }
    #[cfg(not(target_os = "linux"))]
    {
        list_via_pgrep(name_substring)
    }
}

#[cfg(target_os = "linux")]
fn list_proc_linux(name_substring: &str) -> Vec<i64> {
    let mut out = Vec::new();
    let entries = match std::fs::read_dir("/proc") {
        Ok(e) => e,
        Err(_) => return out,
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let pid: i64 = match name.to_string_lossy().parse() {
            Ok(p) => p,
            Err(_) => continue,
        };
        // The cmdline file is NUL-separated argv, and short — argv[0] is all
        // we match on, but reading the whole file is cheaper than seeking
        // inside procfs.
        let cmdline = match std::fs::read(format!("/proc/{pid}/cmdline")) {
            Ok(b) => b,
            Err(_) => continue,
        };
        let cmdline = String::from_utf8_lossy(&cmdline);
        if cmdline.starts_with(name_substring) || cmdline.contains(&format!("/{name_substring}")) {
            out.push(pid);
        }
    }
    out
}

#[cfg(not(target_os = "linux"))]
fn list_via_pgrep(name_substring: &str) -> Vec<i64> {
    let out = std::process::Command::new("pgrep")
        .args(["-f", name_substring])
        .output();
    match out {
        Ok(o) => String::from_utf8_lossy(&o.stdout)
            .lines()
            .filter_map(|l| l.trim().parse::<i64>().ok())
            .collect(),
        Err(_) => Vec::new(),
    }
}

/// Whether the environment of `pid` contains `key=value`.
///
/// Linux reads `/proc/<pid>/environ` directly. Other Unices fall back to
/// `ps eww -p <pid>`, which prints the environment as `KEY=value` pairs
/// separated by whitespace. The latter is the same approach the test suite
/// already took on macOS.
fn env_contains(pid: i64, key: &str, value: &str) -> bool {
    #[cfg(target_os = "linux")]
    {
        if let Ok(bytes) = std::fs::read(format!("/proc/{pid}/environ")) {
            // `/proc/<pid>/environ` is NUL-separated `KEY=VALUE` records.
            for record in bytes.split(|b| *b == 0) {
                if String::from_utf8_lossy(record) == format!("{key}={value}") {
                    return true;
                }
            }
            return false;
        }
    }
    // Fallback for macOS and any other Unix without `/proc`.
    let out = std::process::Command::new("ps")
        .args(["eww", "-p", &pid.to_string()])
        .output();
    match out {
        Ok(o) => {
            let stdout = String::from_utf8_lossy(&o.stdout);
            contains_token(&stdout, &format!("{key}={value}"))
        }
        Err(_) => false,
    }
}

/// Whether `haystack` contains `needle` as a whitespace-delimited token.
///
/// `ps eww` prints the environment space-separated, with no quoting, so there
/// is no way to split it back into records: a value containing a space would
/// be torn in two. Matching the whole `KEY=VALUE` needle and only checking
/// its *boundaries* sidesteps that — the spaces inside the value are part of
/// what we are looking for. Temporary directories with a space in the path
/// are the case this exists for.
fn contains_token(haystack: &str, needle: &str) -> bool {
    let bytes = haystack.as_bytes();
    let mut from = 0;
    while let Some(offset) = haystack[from..].find(needle) {
        let start = from + offset;
        let end = start + needle.len();
        let before_ok = start == 0 || bytes[start - 1].is_ascii_whitespace();
        let after_ok = end == bytes.len() || bytes[end].is_ascii_whitespace();
        if before_ok && after_ok {
            return true;
        }
        from = start + 1;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_token_is_matched_only_at_its_boundaries() {
        assert!(contains_token("A=1 B=2", "A=1"));
        assert!(contains_token("A=1 B=2", "B=2"));
        assert!(contains_token("A=1", "A=1"));
        // A prefix of a longer value must not match: this is what stops one
        // sandbox's socket matching another whose path extends it.
        assert!(!contains_token("SOCK=/tmp/a-1", "SOCK=/tmp/a"));
        assert!(!contains_token("XA=1", "A=1"));
    }

    #[test]
    fn a_value_containing_a_space_still_matches() {
        // The case `split_whitespace` got wrong.
        assert!(contains_token(
            "HOME=/var/my folder/x SHELL=/bin/zsh",
            "HOME=/var/my folder/x"
        ));
    }

    #[test]
    fn a_killed_but_unreaped_child_is_not_running() {
        // A zombie: exited, but still in the process table because nothing
        // has reaped it. `kill(pid, 0)` succeeds for one, so without the
        // zombie probe this would report the process alive forever and
        // `wait_for_exit` would never return.
        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("spawn a long-lived child");
        let pid = child.id() as i64;
        assert!(is_running(pid), "the child should start out running");

        assert!(kill(pid), "SIGKILL should be delivered");

        // Deliberately not reaped yet, so the entry lingers as a zombie.
        assert!(
            wait_for_exit(pid, std::time::Duration::from_secs(2)),
            "an unreaped, killed child must be reported as not running"
        );

        child.wait().expect("reap the child");
    }
}
