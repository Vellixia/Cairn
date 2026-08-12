//! Windows process introspection and signalling.
//!
//! Identifies `cairnd.exe` processes serving a named pipe and terminates
//! them with `TerminateProcess` — the nearest analogue of `SIGKILL` on
//! Windows: uncatchable, immediate, not subject to cooperative shutdown
//! handlers.
//!
//! The FFI surface is the minimum that needs it: `OpenProcess`,
//! `TerminateProcess`, and `GetExitCodeProcess` for liveness. Everything
//! else (process enumeration, pipe-to-PID attribution) is delegated to
//! `tasklist` and a small PowerShell probe, the same way the Unix side of
//! this crate shells out to `pgrep`/`ps`. Keeps the FFI surface small and
//! pushes snapshotting quirks onto the OS.

use std::path::Path;
use std::process::Command;

/// PIDs of `cairnd.exe` processes serving `socket`.
///
/// On Windows `socket` is a named pipe path (`\\.\pipe\cairnd-…`). Each
/// sandbox gets a unique pipe name, so the only `cairnd.exe` that holds a
/// given pipe is the one this sandbox started. We open the pipe and ask
/// `GetNamedPipeServerProcessId` who owns it — no process enumeration
/// needed.
pub fn daemons_for_socket(socket: &Path) -> Vec<i64> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::System::Pipes::GetNamedPipeServerProcessId;

    let file = match std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(socket)
    {
        Ok(f) => f,
        // ERROR_PIPE_BUSY: every instance is taken, which still means alive.
        // We cannot get the server PID in this state, so fall back to
        // enumerating cairnd.exe processes and returning any that are alive.
        Err(e) if e.raw_os_error() == Some(231) => return list_cairnd_pids(),
        Err(_) => return Vec::new(),
    };
    let mut server_pid: u32 = 0;
    let ok =
        unsafe { GetNamedPipeServerProcessId(file.as_raw_handle() as _, &mut server_pid) != 0 };
    if ok && server_pid > 0 {
        vec![server_pid as i64]
    } else {
        Vec::new()
    }
}

/// `TerminateProcess` — the Windows analogue of `SIGKILL`.
///
/// Uncatchable, immediate, no shutdown handlers run. Returns `true` if the
/// process was terminated. The process may still be in the middle of dying;
/// use [`is_running`] to poll for exit.
pub fn kill(pid: i64) -> bool {
    // SAFETY: `OpenProcess` returns a handle that we close after use.
    // `TerminateProcess` with exit code 1 is the documented hard-kill.
    unsafe {
        use windows_sys::Win32::System::Threading::{
            OpenProcess, TerminateProcess, PROCESS_TERMINATE,
        };
        let handle = OpenProcess(PROCESS_TERMINATE, 0, pid as u32);
        if handle.is_null() {
            return false;
        }
        let ok = TerminateProcess(handle, 1) != 0;
        windows_sys::Win32::Foundation::CloseHandle(handle);
        ok
    }
}

/// Whether `pid` is still running.
///
/// `GetExitCodeProcess` returns `STILL_ACTIVE` (259) for a live process.
pub fn is_running(pid: i64) -> bool {
    unsafe {
        use windows_sys::Win32::System::Threading::{
            GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
        };
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid as u32);
        if handle.is_null() {
            return false;
        }
        let mut code: u32 = 0;
        let ok = GetExitCodeProcess(handle, &mut code) != 0;
        windows_sys::Win32::Foundation::CloseHandle(handle);
        ok && code == 259 /* STILL_ACTIVE */
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

/// Enumerate every `cairnd.exe` process on the system via `tasklist`.
///
/// `tasklist /FI "IMAGENAME eq cairnd.exe" /FO CSV /NH` prints one CSV row
/// per matching process: `"cairnd.exe","1234","Console","1","12,345 K"`.
/// We parse the second field.
fn list_cairnd_pids() -> Vec<i64> {
    let out = Command::new("tasklist")
        .args(["/FI", "IMAGENAME eq cairnd.exe", "/FO", "CSV", "/NH"])
        .output();
    let out = match out {
        Ok(o) if o.status.success() => o,
        _ => return Vec::new(),
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let mut pids = Vec::new();
    for line in text.lines() {
        let fields: Vec<&str> = line.split(',').collect();
        if fields.len() < 2 {
            continue;
        }
        let pid_text = fields[1].trim_matches('"');
        if let Ok(p) = pid_text.parse::<i64>() {
            pids.push(p);
        }
    }
    pids
}
