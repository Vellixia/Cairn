//! Cross-platform process introspection and signalling.
//!
//! The test suite needs to identify `cairnd` processes by the `CAIRN_SOCKET`
//! they were started with and hard-kill them, so it can exercise the daemon
//! recovery path (FR-009, FR-047, H2) on every platform. Unix has `pgrep`/`ps`
//! and `SIGKILL`; Windows has `CreateToolhelp32Snapshot` and `TerminateProcess`.
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
    fn nonexistent_pid_is_not_running() {
        // A PID that does not exist. Picking a very large value keeps this
        // true even on systems that recycle PIDs aggressively.
        let dead = 2_000_000;
        assert!(
            !super::is_running(dead),
            "a phantom PID must not report running"
        );
    }
}
