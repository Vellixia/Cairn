//! Cairn core: shared domain types for the Cairn context & reliability engine.
//!
//! Everything here is deliberately storage- and transport-agnostic so it can be reused by the
//! store, context engine, memory engine, API, MCP server, and CLI without circular deps.

pub mod config;
pub mod error;
pub mod hash;
pub mod model;

pub use config::{Config, EmbedConfig, TlsConfig};
pub use error::{Error, Result};
pub use hash::ContentHash;
pub use model::{DeviceToken, Memory, MemoryKind, MemoryTier, NewMemory};
