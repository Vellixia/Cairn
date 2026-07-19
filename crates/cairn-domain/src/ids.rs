//! Identity newtypes. UUIDv7 everywhere a Cairn-owned identity is minted
//! (creation-time ordered, research R12); agent instance ids are accepted as
//! any RFC 4122 UUID because the caller generates them.

use std::fmt;
use std::str::FromStr;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

macro_rules! uuid_newtype {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord,
            Serialize, Deserialize, JsonSchema,
        )]
        #[serde(transparent)]
        pub struct $name(pub Uuid);

        impl $name {
            /// Mint a new time-ordered (UUIDv7) identity.
            pub fn new_v7() -> Self {
                Self(Uuid::now_v7())
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }

        impl FromStr for $name {
            type Err = uuid::Error;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Ok(Self(Uuid::parse_str(s)?))
            }
        }
    };
}

uuid_newtype!(
    /// Stable repository identity stored in Git-private metadata (FR-002).
    RepoUuid
);
uuid_newtype!(
    /// Per-worktree identity stored in the worktree's private Git dir.
    WorktreeUuid
);
uuid_newtype!(
    /// Stable session identifier (FR-014).
    SessionId
);
uuid_newtype!(
    /// Persisted snapshot identity.
    SnapshotId
);
uuid_newtype!(
    /// Append-only event identity.
    EventId
);
uuid_newtype!(
    /// Caller-generated per-agent-instance identity (clarification Q1).
    AgentInstanceId
);
