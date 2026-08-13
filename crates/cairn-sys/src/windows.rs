//! Windows process introspection and signalling.
//!
//! Identifies `cairnd.exe` processes serving a named pipe and terminates
//! them with `TerminateProcess` — the nearest analogue of `SIGKILL` on
//! Windows: uncatchable, immediate, not subject to cooperative shutdown
//! handlers.
//!
//! The FFI surface is the minimum that needs it: `GetNamedPipeServerProcessId`
//! to attribute a pipe to its owner, `OpenProcess` and `TerminateProcess` to
//! end it, and `GetExitCodeProcess` for liveness.
//!
//! Attribution is deliberately never widened to "find processes that look
//! like ours". The caller hard-kills what this reports, so a guess is a
//! guess about which process to destroy — see `daemons_for_socket`.

use std::path::Path;

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

    // A busy pipe is transient: the daemon opens the next instance as soon as
    // the pending one is taken, so a client mid-request clears in
    // milliseconds. Wait it out.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    let file = loop {
        match std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(socket)
        {
            Ok(f) => break f,
            // ERROR_PIPE_BUSY. Never widen this into "every cairnd.exe on the
            // machine": the caller hard-kills whatever this returns, and a
            // developer running the suite has their own daemon serving their
            // own repositories. Guessing costs them that daemon. Returning
            // nothing makes the test fail loudly instead, which is the safe
            // direction to be wrong in.
            Err(e) if e.raw_os_error() == Some(231) => {
                if std::time::Instant::now() >= deadline {
                    return Vec::new();
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            Err(_) => return Vec::new(),
        }
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
