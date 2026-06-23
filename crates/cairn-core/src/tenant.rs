//! Multi-tenant identity (v0.5.0 Sprint 19).
//!
//! An [`OrgId`] is the tenant boundary for a multi-tenant cairn-server. Every
//! [`Memory`](crate::Memory) carries an org id; queries scope by the caller's
//! org id (extracted from the bearer token). When `Config::multi_tenant` is
//! `false` (the default for self-hosted installs), every org id collapses to
//! [`OrgId::default()`] — the on-disk schema is identical to the v0.4.0 shape
//! and existing users see no change.
//!
//! Org ids are short, lower-case, ASCII-only — easy to log and to type at the
//! CLI. They're *not* secrets: they're tenant identifiers, not credentials.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct OrgId(String);

impl Default for OrgId {
    /// The default org id is the implicit single-tenant id `OrgId::SINGLE_TENANT`.
    fn default() -> Self {
        Self(OrgId::SINGLE_TENANT.to_string())
    }
}

impl OrgId {
    /// The implicit single-tenant org id used when `Config::multi_tenant = false`.
    pub const SINGLE_TENANT: &'static str = "default";

    /// Build an org id from a string. Returns `Err` if the string is empty, too long,
    /// or contains characters outside `[a-z0-9_-]` (we want log-friendly ids).
    pub fn new(s: impl Into<String>) -> Result<Self, OrgIdError> {
        let s: String = s.into();
        if s.is_empty() {
            return Err(OrgIdError::Empty);
        }
        if s.len() > 64 {
            return Err(OrgIdError::TooLong);
        }
        if !s
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
        {
            return Err(OrgIdError::InvalidChar);
        }
        Ok(Self(s))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// True when this org id is the implicit single-tenant id.
    pub fn is_default(&self) -> bool {
        self.0 == Self::SINGLE_TENANT
    }
}

impl fmt::Display for OrgId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for OrgId {
    type Err = OrgIdError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum OrgIdError {
    #[error("org id must not be empty")]
    Empty,
    #[error("org id exceeds 64 chars")]
    TooLong,
    #[error("org id may only contain [a-z0-9_-]")]
    InvalidChar,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_org_id_is_the_singleton() {
        let o = OrgId::default();
        assert_eq!(o.as_str(), OrgId::SINGLE_TENANT);
        assert!(o.is_default());
    }

    #[test]
    fn new_accepts_valid_org_ids() {
        for s in ["vellixia", "team-a", "acme_corp", "abc123"] {
            let o = OrgId::new(s).unwrap();
            assert_eq!(o.as_str(), s);
        }
    }

    #[test]
    fn new_rejects_invalid_org_ids() {
        assert_eq!(OrgId::new(""), Err(OrgIdError::Empty));
        assert_eq!(OrgId::new("a".repeat(65)), Err(OrgIdError::TooLong));
        for bad in ["ACME", "acme corp", "acme!", "acme/corp"] {
            assert_eq!(OrgId::new(bad), Err(OrgIdError::InvalidChar));
        }
    }

    #[test]
    fn org_id_round_trips_through_serde() {
        let o = OrgId::new("vellixia").unwrap();
        let json = serde_json::to_string(&o).unwrap();
        assert_eq!(json, "\"vellixia\"");
        let back: OrgId = serde_json::from_str(&json).unwrap();
        assert_eq!(back, o);
    }
}
