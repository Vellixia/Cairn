//! Source-preserving configuration editing (FR-152, FR-153, D37).
//!
//! Cairn must preserve every setting it does not own — entries, ordering where
//! the format is order-significant, comments, and formatting where the format
//! carries them. SC-103 and SC-104 assert *byte identity* for non-Cairn
//! content, which a parse-and-reserialize editor cannot deliver in either
//! format: `serde_json` discards indentation, escaping and layout; `toml`
//! discards comments. Both need a tree that retains source spans.
//!
//! So: `jsonc-parser`'s CST for JSON and JSONC, `toml_edit` for TOML, and a
//! marker splice for Markdown. Hand-built string substitution into a
//! structured configuration file is prohibited (FR-153), and there is
//! deliberately no fallback path that pretty-prints a whole file.
//!
//! Malformed input fails closed and writes nothing (FR-137).

pub mod json;
pub mod markdown;
pub mod toml;

use serde::{Deserialize, Serialize};

/// The outcome of an edit. `Unchanged` is what makes every operation
/// idempotent (FR-157).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    Unchanged,
    Written(String),
}

impl Change {
    pub fn text(&self) -> Option<&str> {
        match self {
            Change::Unchanged => None,
            Change::Written(s) => Some(s),
        }
    }
    pub fn is_changed(&self) -> bool {
        matches!(self, Change::Written(_))
    }
}

/// Why an edit could not be performed. Every variant means *nothing was
/// written*.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EditError {
    /// The enclosing file could not be parsed. Cairn reports it and refuses to
    /// rewrite it, rather than replacing it with a valid file of its own
    /// construction (FR-137, US7 #3).
    Malformed { path: String, detail: String },
    /// The structure is not what the format requires — an array where an
    /// object belongs, for instance.
    UnexpectedShape { path: String, detail: String },
    /// Cairn's markers are missing, unbalanced or mismatched.
    DamagedMarkers { path: String, detail: String },
}

impl EditError {
    pub fn path(&self) -> &str {
        match self {
            EditError::Malformed { path, .. }
            | EditError::UnexpectedShape { path, .. }
            | EditError::DamagedMarkers { path, .. } => path,
        }
    }
    /// The health condition this failure maps to.
    pub fn condition(&self) -> crate::model::HealthCondition {
        match self {
            EditError::Malformed { .. } | EditError::UnexpectedShape { .. } => {
                crate::model::HealthCondition::MalformedConfig
            }
            EditError::DamagedMarkers { .. } => crate::model::HealthCondition::DamagedMarkers,
        }
    }
}

impl std::fmt::Display for EditError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EditError::Malformed { path, detail } => {
                write!(f, "{path}: could not be parsed: {detail}")
            }
            EditError::UnexpectedShape { path, detail } => write!(f, "{path}: {detail}"),
            EditError::DamagedMarkers { path, detail } => write!(f, "{path}: {detail}"),
        }
    }
}

impl std::error::Error for EditError {}
