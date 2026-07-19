//! Session lifecycle policy (T035): idempotent start, lease/heartbeat,
//! recovering-state machine, staleness with liveness reason codes.

pub mod liveness;
pub mod service;
pub mod token;

pub use service::*;
pub use token::*;
