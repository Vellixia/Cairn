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
    #[cfg(target_os = "linux")]
    {
        if let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) {
            // `pid (comm) state ...`. The comm field is arbitrary and may
            // contain both spaces and parentheses, so the state letter is
            // found after the *last* ')', not by splitting on whitespace.
            if let Some((_, rest)) = stat.rsplit_once(')') {
                if rest.split_whitespace().next() == Some("Z") {
                    return false;
                }
            }
        }
    }
    // SAFETY: `kill(2)` with signal 0 performs no signal delivery; it is the
    // standard liveness check. `ESRCH` means "no such process".
    unsafe {
        libc::kill(pid as i32, 0) == 0
            || std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
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
        // The cmdline file is NUL-separated argv. Read a small prefix only;
        // we just need to match the executable name.
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
                let s = String::from_utf8_lossy(record);
                if s.starts_with(&format!("{key}=")) && s.as_ref() == format!("{key}={value}") {
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
            stdout
                .split_whitespace()
                .any(|entry| entry == format!("{key}={value}"))
        }
        Err(_) => false,
    }
}
