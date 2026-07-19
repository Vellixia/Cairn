//! Liveness policy (analysis A1): lease expiry is the authoritative staleness
//! clock; a verifiably dead PID may confirm staleness earlier; a missing or
//! unverifiable PID NEVER implies death.

use cairn_domain::{LivenessReason, Timestamp};

/// Result of evaluating a live session's health.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Health {
    Healthy,
    Stale(LivenessReason),
}

/// Check whether an OS process is verifiably alive.
/// Returns None when liveness cannot be determined (process_unknown).
pub fn process_alive(pid: Option<i64>) -> Option<bool> {
    let pid = pid?;
    if pid <= 0 {
        return None;
    }
    #[cfg(unix)]
    {
        // kill(pid, 0): 0 => alive; EPERM => alive but not ours; ESRCH => dead.
        let rc = unsafe { libc::kill(pid as libc::pid_t, 0) };
        if rc == 0 {
            return Some(true);
        }
        let errno = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
        if errno == libc::EPERM {
            return Some(true);
        }
        return Some(false);
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
        use windows_sys::Win32::System::Threading::{
            GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
        };
        unsafe {
            let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid as u32);
            if handle.is_null() {
                // Access denied or gone: cannot verify => unknown, never "dead".
                return None;
            }
            let mut code: u32 = 0;
            let ok = GetExitCodeProcess(handle, &mut code);
            CloseHandle(handle);
            if ok == 0 {
                return None;
            }
            return Some(code == STILL_ACTIVE as u32);
        }
    }
    #[allow(unreachable_code)]
    None
}

/// Evaluate session health at `now` (FR-034 policy).
pub fn evaluate(lease_expires_at: Timestamp, agent_pid: Option<i64>, now: Timestamp) -> Health {
    let lease_expired = now > lease_expires_at;
    match process_alive(agent_pid) {
        Some(false) => Health::Stale(LivenessReason::ProcessDead),
        Some(true) => {
            if lease_expired {
                Health::Stale(LivenessReason::HeartbeatExpired)
            } else {
                Health::Healthy
            }
        }
        None => {
            // PID unknown/unverifiable: never implies death. Lease expiry is
            // the only staleness signal, recorded as process_unknown context.
            if lease_expired {
                Health::Stale(LivenessReason::ProcessUnknown)
            } else {
                Health::Healthy
            }
        }
    }
}
