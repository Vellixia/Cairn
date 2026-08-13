//! What a daemon that failed to start leaves behind for the CLI to read.
//!
//! `cairn` spawns `cairnd` detached with its standard handles on the null
//! device, so an error the daemon returns from `main` is printed to nothing at
//! all. From outside, a store the daemon could not open was indistinguishable
//! from a daemon that never ran: the CLI saw only that nothing bound the
//! socket, and reported `daemon_unavailable: cairnd did not start` — true, but
//! it names the symptom and implies a remedy (start a daemon) that does not
//! apply.
//!
//! The daemon's log file is the one channel that survives the spawn, so the
//! reason goes there behind a marker the CLI recognises. That marker is the
//! contract between the two, which is why it lives here rather than in either
//! binary.

/// Prefix of the log line a daemon writes when the local store cannot be
/// opened. The underlying error follows it as `": <reason>"`, on one line.
///
/// The daemon writes this line whatever `CAIRN_LOG` says. A user who silenced
/// the log has not asked to be told less about why nothing works.
pub const STORE_OPEN_FAILED: &str = "cairnd could not open the local store";
