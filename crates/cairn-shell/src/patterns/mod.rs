//! Named shell-output compression patterns (P4.4). Each category exports a `PATTERNS` slice
//! that the [`crate::registry::REGISTRY`] aggregates. A pattern declares:
//!
//! - what tokens must be present in the command line (substring match, word-bounded)
//! - the category it belongs to
//! - optional `keep` / `drop` filter lists applied before dedup + truncate
//!
//! Adding a new pattern: drop a `Pattern` literal into the appropriate `patterns/*.rs`,
//! add tests, done. No dispatch logic to touch.

pub mod build;
pub mod dirlist;
pub mod fileread;
pub mod http;
pub mod infra;
pub mod lint;
pub mod package;
pub mod search;
pub mod vcs;
