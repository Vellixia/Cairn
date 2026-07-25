//! Git CLI adapter: subprocess runner, porcelain v2 parsing, identity
//! markers, ignored-summary walker, fingerprint pipeline.

pub mod discover;
pub mod error;
pub mod fingerprint;
pub mod identity;
pub mod ignored;
pub mod runner;
pub mod status;

pub use discover::*;
pub use error::GitError;
pub use runner::GitRunner;
pub use status::*;
