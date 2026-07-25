//! Pure domain types and logic for Cairn's local session foundation.
//!
//! No IO, no SQL, no transport, no Git invocation (constitution: module
//! ownership map). Everything here is deterministic and unit-testable.

pub mod goal_contract;
pub mod ids;
pub mod project;
pub mod session;
pub mod snapshot;
pub mod task;
pub mod time;

pub use goal_contract::*;
pub use ids::*;
pub use project::*;
pub use session::*;
pub use snapshot::*;
pub use task::*;
pub use time::Timestamp;
