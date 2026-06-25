//! `.cairnpkg` - Cairn's portable context package format (v0.5.0 Phase 4.0 Sprint 11).
//!
//! A `.cairnpkg` is a single tarball containing:
//!
//! - `manifest.json` - metadata (id, version, author, sha256 of every other file)
//! - `memory.jsonl` - newline-delimited memories in `cairn-share`'s `ShareableMemory` shape
//! - `profile.jsonl` - newline-delimited preferences
//! - `patterns.jsonl` - newline-delimited reusable patterns (agentmemory's "lesson" pattern)
//! - `graph.jsonl` - newline-delimited provenance edges
//! - `signature.sha256` - HMAC-like keyed signature over the manifest (see [`signing`])
//!
//! Adopted from the lean-ctx `.ctxpkg` design (SHA-256 integrity, atomic writes, knowledge
//! merge, graph overlay). The `.cairnpkg` extension is canonical; `.ctxpkg` is accepted as
//! an import alias for lean-ctx interop per the v0.5.0 plan S10.
//!
//! ## CLI surface (per Sprint 11)
//!
//! - `cairn pack create [NAME]` - bundle current memories/profile/patterns into a `.cairnpkg`
//! - `cairn pack install <file>` - import a `.cairnpkg` into the local store
//! - `cairn pack info <file>` - print the manifest
//! - `cairn pack list` - list installed packs
//! - `cairn pack remove <name>` - uninstall
//! - `cairn pack export <name> <file>` - write a pack to disk
//! - `cairn pack import <file>` - ingest a `.cairnpkg` (alias for install with merging)
//! - `cairn pack auto-load` - turn on the auto-load list
//! - `cairn pack publish <file> --registry <url>` - push to a pack registry (HTTP)
//!
//! See [`pack`] for the in-memory model and [`install`] for the unpack path.

pub mod install;
pub mod manifest;
pub mod pack;
pub mod signing;

pub use install::parse_tar as tar;
pub use manifest::Manifest;
pub use pack::Pack;
pub use signing::{
    sign_manifest, sign_manifest_ed25519, verify_manifest_ed25519, Keypair, PublicKey, SignError,
    VerifyError,
};

/// Canonical extension for Cairn context packages. The lean-ctx interop alias `.ctxpkg`
/// is also accepted as an import format - see [`manifest::is_supported_extension`].
pub const EXTENSION: &str = "cairnpkg";
pub const ALT_EXTENSION: &str = "ctxpkg";

/// Tarball MIME hint (informational; tarballs don't have a true MIME).
pub const MIME: &str = "application/x-cairnpkg";

/// Maximum accepted package size (16 MiB uncompressed). Anything bigger is rejected by
/// [`manifest::Manifest::read`] before we unpack a single byte.
pub const MAX_UNCOMPRESSED_BYTES: u64 = 16 * 1024 * 1024;
