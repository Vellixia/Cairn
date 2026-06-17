//! Core domain model: memories and their tiers/kinds.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Human-memory-inspired tiers. New observations land in `Working` and are consolidated upward
/// (episodic events -> semantic facts -> procedural how-to) over time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemoryTier {
    Working,
    Episodic,
    Semantic,
    Procedural,
}

impl MemoryTier {
    pub fn as_str(&self) -> &'static str {
        match self {
            MemoryTier::Working => "working",
            MemoryTier::Episodic => "episodic",
            MemoryTier::Semantic => "semantic",
            MemoryTier::Procedural => "procedural",
        }
    }
}

impl std::str::FromStr for MemoryTier {
    type Err = crate::Error;
    fn from_str(s: &str) -> crate::Result<Self> {
        Ok(match s {
            "working" => Self::Working,
            "episodic" => Self::Episodic,
            "semantic" => Self::Semantic,
            "procedural" => Self::Procedural,
            other => {
                return Err(crate::Error::Invalid(format!(
                    "unknown memory tier: {other}"
                )))
            }
        })
    }
}

/// What a memory represents. Drives recall ranking and consolidation rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemoryKind {
    Fact,
    Decision,
    Task,
    Preference,
    Gotcha,
    Note,
}

impl MemoryKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            MemoryKind::Fact => "fact",
            MemoryKind::Decision => "decision",
            MemoryKind::Task => "task",
            MemoryKind::Preference => "preference",
            MemoryKind::Gotcha => "gotcha",
            MemoryKind::Note => "note",
        }
    }
}

impl std::str::FromStr for MemoryKind {
    type Err = crate::Error;
    fn from_str(s: &str) -> crate::Result<Self> {
        Ok(match s {
            "fact" => Self::Fact,
            "decision" => Self::Decision,
            "task" => Self::Task,
            "preference" => Self::Preference,
            "gotcha" => Self::Gotcha,
            "note" => Self::Note,
            other => {
                return Err(crate::Error::Invalid(format!(
                    "unknown memory kind: {other}"
                )))
            }
        })
    }
}

/// A persisted memory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Memory {
    pub id: String,
    pub kind: MemoryKind,
    pub tier: MemoryTier,
    pub content: String,
    pub concepts: Vec<String>,
    pub files: Vec<String>,
    pub session_id: Option<String>,
    pub importance: f32,
    pub access_count: i64,
    #[serde(default)]
    pub suspicious: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Input for creating a memory. Optional fields fall back to sensible defaults.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NewMemory {
    pub content: String,
    #[serde(default)]
    pub kind: Option<MemoryKind>,
    #[serde(default)]
    pub tier: Option<MemoryTier>,
    #[serde(default)]
    pub concepts: Vec<String>,
    #[serde(default)]
    pub files: Vec<String>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub importance: Option<f32>,
    #[serde(default)]
    pub suspicious: Option<bool>,
}

impl NewMemory {
    pub fn new(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            ..Default::default()
        }
    }

    /// Materialize into a full [`Memory`] with id and timestamps.
    pub fn into_memory(self) -> Memory {
        let now = Utc::now();
        Memory {
            id: Uuid::new_v4().to_string(),
            kind: self.kind.unwrap_or(MemoryKind::Note),
            tier: self.tier.unwrap_or(MemoryTier::Working),
            content: self.content,
            concepts: self.concepts,
            files: self.files,
            session_id: self.session_id,
            importance: self.importance.unwrap_or(0.5).clamp(0.0, 1.0),
            access_count: 0,
            suspicious: self.suspicious.unwrap_or(false),
            created_at: now,
            updated_at: now,
        }
    }
}

/// A per-device access token for authenticating to a Cairn server.
/// `id` is the token identifier (stored in the backend). `token` is the opaque bearer value
/// (a signed JWT) returned to the user once, only at creation time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceToken {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    pub name: String,
    pub created_at: DateTime<Utc>,
}

impl DeviceToken {
    /// Metadata-only token (used by list/revoke flows that must never re-emit the bearer).
    pub fn meta(id: String, name: String, created_at: DateTime<Utc>) -> Self {
        Self {
            id,
            token: None,
            name,
            created_at,
        }
    }
}
