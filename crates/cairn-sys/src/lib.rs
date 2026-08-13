//! Cross-platform process introspection and signalling.
//!
//! The test suite needs to identify `cairnd` processes by the `CAIRN_SOCKET`
//! they were started with and hard-kill them, so it can exercise the daemon
//! recovery path (FR-009, FR-047, H2) on every platform. Unix matches on the
//! process environment (`/proc`, or `ps eww` where there is none) and signals
//! with `SIGKILL`; Windows asks the pipe itself who owns it, via
//! `GetNamedPipeServerProcessId`, and ends it with `TerminateProcess`.
//! This crate hides that difference behind one small surface so the tests do
//! not grow their own per-platform scaffolding.
//!
//! Nothing here is production code: every function exists for the test suite,
//! and every function is best-effort rather than fallible. A test that asks
//! for "daemons serving this socket" and gets an empty list fails loudly,
//! which is what we want; silent partial failure would mask a broken fixture.

#[cfg(unix)]
mod unix;
#[cfg(unix)]
pub use unix::*;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::*;

#[cfg(test)]
mod tests {
    //! The only thing that is platform-independent enough to assert here is
    //! that a `kill` of the current process is reported as no longer running
    //! once it has been reaped. The cross-platform surface itself is exercised
    //! end-to-end by `tests/tests/us1_sessions.rs`.

    #[test]
    fn alive_pid_is_running() {
        let me = std::process::id() as i64;
        assert!(
            super::is_running(me),
            "the test process must report running"
        );
    }

    #[test]
    fn a_reaped_child_is_not_running() {
        // Use a PID known to be dead rather than one assumed to be unused.
        // A large constant is not safe to assume free: Linux allows `pid_max`
        // up to 4,194,304, so the number could belong to a real process on a
        // busy host and fail this test for the wrong reason. Spawning a child
        // and reaping it leaves a PID that is genuinely finished.
        let mut child = std::process::Command::new(if cfg!(windows) { "cmd" } else { "true" })
            .args(if cfg!(windows) {
                vec!["/C", "exit"]
            } else {
                vec![]
            })
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("spawn a short-lived child");
        let pid = child.id() as i64;
        child.wait().expect("reap the child");

        assert!(
            !super::is_running(pid),
            "a reaped child ({pid}) must not report running"
        );
    }
}
