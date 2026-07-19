//! Pure domain types and logic for Cairn's local session foundation.
//!
//! No IO, no SQL, no transport, no Git invocation (constitution: module
//! ownership map). Everything here is deterministic and unit-testable.

pub mod ids;
pub mod session;
pub mod snapshot;
pub mod time;

pub use ids::*;
pub use session::*;
pub use snapshot::*;
pub use time::Timestamp;
