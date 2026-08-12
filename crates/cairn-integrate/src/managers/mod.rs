//! Integration managers.
//!
//! A manager distributes Cairn resources to agents on the developer's behalf.
//! It is never an agent: it produces no sessions, no observations and no
//! lifecycle events, and it is reported in its own section rather than in the
//! agent list (FR-101).
//!
//! Exactly one exists in this feature (FR-103).

pub mod cc_switch;
