//! On-disk pack registry store. See [`crate`] for the layout and HTTP API surface.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    sync::Mutex,
};
use thiserror::Error;

use cairn_pack::{Manifest, PublicKey};

/// Errors that the registry can return to the HTTP layer. The [`From`] impls let callers
/// `?`-propagate `io::Error` and `serde_json::Error` cleanly.
#[derive(Debug, Error)]
pub enum RegistryError {
    #[error("io: {0}")]
    Io(#[from] io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("pack format: {0}")]
    Pack(String),
    #[error("Ed25519 signature did not match any trusted key")]
    InvalidSignature,
    #[error("not found: {0}")]
    NotFound(String),
    #[error("already exists: {0}")]
    AlreadyExists(String),
    #[error("malformed trusted key: {0}")]
    BadKey(String),
    /// Pack's declared scope is wider than any trust grant allows. Surfaced with the
    /// pack's scope and the granted scopes so the operator can fix the configuration.
    #[error(
        "pack scope {pack_scope:?} not allowed by any trust grant (granted: {granted_scopes:?})"
    )]
    ScopeDenied {
        pack_scope: TrustScope,
        granted_scopes: Vec<TrustScope>,
    },
}

/// What `POST /registry/packs` returns — captures both the verification path taken and
/// the stored metadata, so the publisher can confirm where their pack ended up.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublishReceipt {
    pub pack_id: String,
    pub name: String,
    pub version: String,
    pub signed_by: Option<String>, // hex pubkey if a trusted signer was matched
    pub status: PublishStatus,
    pub stored_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PublishStatus {
    /// Pack carried a `signature.ed25519` entry AND it verified against one of the
    /// trusted keys.
    Signed,
    /// Pack had no `signature.ed25519` entry. Stored because integrity hashes still
    /// match — but no author authenticity is asserted. The CLI / federation layer is
    /// responsible for warning the user when this happens.
    Unsigned,
}

/// What we keep in `index.json` — the fields a `GET /registry/packs` caller wants.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackMeta {
    pub id: String,
    pub name: String,
    pub version: String,
    pub author: String,
    pub description: String,
    pub created_at: DateTime<Utc>,
    pub stored_at: DateTime<Utc>,
    pub size_bytes: u64,
    /// Hex-encoded author pubkey if this pack was signed by a trusted signer at
    /// publish time. `None` for unsigned packs.
    pub signer_pubkey: Option<String>,
    pub has_ed25519_signature: bool,
    pub memory_count: usize,
    pub download_count: u64,
    /// How widely this pack is meant to be shared (Sprint 14). Defaults to `public`.
    #[serde(default)]
    pub scope: TrustScope,
    /// Origin registry URL for federation-replicated packs. `None` for locally-published.
    /// When set, the federation sync layer uses this to avoid loops and to verify the
    /// pack's provenance.
    #[serde(default)]
    pub origin: Option<String>,
    /// Number of provenance graph edges carried in this pack (Sprint 14c). The edges
    /// themselves live in `graph.jsonl` inside the tarball — `GET /registry/packs/:name`
    /// returns the count and `GET /registry/packs/:name/:version/manifest.json` returns
    /// the full graph.
    #[serde(default)]
    pub provenance_edge_count: usize,
}

/// How widely a pack is meant to be shared. The scope is asserted by the publisher (in
/// the manifest) and enforced by the federation sync layer (Sprint 14). A `local` pack
/// shouldn't propagate to other registries; a `team` pack goes to the team registry
/// only; `public` is the default for open sharing.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TrustScope {
    /// Only this registry should ever hold it. Reject any incoming federation sync that
    /// claims this pack is from a different origin.
    Local,
    /// Share with the team registry (the operator's own organization). Other teams
    /// shouldn't see it via public registry discovery.
    Team,
    /// Share with the world. No scope-based filtering applies.
    #[default]
    Public,
}

/// A trusted author public key, with the scope the local operator allows. When a pack
/// arrives signed by `key`, the registry checks that the pack's own scope is at most
/// as wide as `allows`. A `Local`-scoped trust entry won't accept `Public`-scoped packs,
/// even if the signature verifies.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustGrant {
    pub key: PublicKey,
    /// Highest scope this key is allowed to publish at.
    pub allows: TrustScope,
    /// Free-form display label (e.g. "alice@vellixia" or "vellixia-org").
    pub label: Option<String>,
    #[serde(default = "Utc::now")]
    pub granted_at: DateTime<Utc>,
}

/// Append-only log entry recording that an operator (or federation peer) revoked a
/// pack version. The federation sync layer (Sprint 14) replays these to subscribers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RevocationEvent {
    pub name: String,
    pub version: String,
    pub revoked_at: DateTime<Utc>,
    pub reason: Option<String>,
}

/// Registry's only mutable state. Cheap to wrap in `Arc` — the disk is the source of
/// truth, this is a coarse write lock to keep index/keys/revocations consistent.
pub struct Registry {
    root: PathBuf,
    state: Mutex<RegistryState>,
}

#[derive(Default)]
struct RegistryState {
    index: Vec<PackMeta>,
    trusted_keys: Vec<TrustGrant>,
    revocations: Vec<RevocationEvent>,
}

impl Registry {
    /// Open (or create) a registry rooted at `<data_dir>/registry/`. If the directory
    /// doesn't exist, it's created. Existing `index.json`, `trusted_keys.json`, and
    /// `revocations.jsonl` are loaded if present.
    pub fn open(data_dir: &Path) -> Result<Self, RegistryError> {
        let root = data_dir.join("registry");
        fs::create_dir_all(root.join("packs"))?;
        let index: Vec<PackMeta> = match fs::read(root.join("index.json")) {
            Ok(b) => serde_json::from_slice(&b)?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(e.into()),
        };
        // trusted_keys.json may be in either the legacy `Vec<PublicKey>` shape (v0.5.0
        // Sprint 13) or the v0.5.0 Sprint 14 `Vec<TrustGrant>` shape. We probe both.
        let trusted_keys: Vec<TrustGrant> = match fs::read(root.join("trusted_keys.json")) {
            Ok(b) => match serde_json::from_slice::<Vec<TrustGrant>>(&b) {
                Ok(v) => v,
                Err(_) => {
                    // Fall back to legacy: list of plain public keys, treat them as
                    // `public`-scope grants with no label.
                    let legacy: Vec<PublicKey> = serde_json::from_slice(&b).unwrap_or_default();
                    legacy
                        .into_iter()
                        .map(|key| TrustGrant {
                            key,
                            allows: TrustScope::Public,
                            label: None,
                            granted_at: Utc::now(),
                        })
                        .collect()
                }
            },
            Err(e) if e.kind() == io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(e.into()),
        };
        let revocations: Vec<RevocationEvent> = match fs::read(root.join("revocations.jsonl")) {
            Ok(b) => b
                .split(|c| *c == b'\n')
                .filter(|l| !l.is_empty())
                .map(serde_json::from_slice)
                .collect::<Result<Vec<_>, _>>()?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(e.into()),
        };
        Ok(Self {
            root,
            state: Mutex::new(RegistryState {
                index,
                trusted_keys,
                revocations,
            }),
        })
    }

    /// List all packs, newest first.
    pub fn list_all(&self) -> Result<Vec<PackMeta>, RegistryError> {
        let g = self.state.lock().expect("registry lock poisoned");
        let mut out = g.index.clone();
        out.sort_by_key(|m| std::cmp::Reverse(m.stored_at));
        Ok(out)
    }

    /// List versions of a single pack (any order — caller can sort by `created_at`).
    pub fn list_versions(&self, name: &str) -> Result<Vec<PackMeta>, RegistryError> {
        let g = self.state.lock().expect("registry lock poisoned");
        Ok(g.index.iter().filter(|m| m.name == name).cloned().collect())
    }

    /// Search case-insensitively across `name + description + author`.
    pub fn search(&self, query: &str) -> Result<Vec<PackMeta>, RegistryError> {
        let q = query.to_ascii_lowercase();
        if q.is_empty() {
            return self.list_all();
        }
        let g = self.state.lock().expect("registry lock poisoned");
        Ok(g.index
            .iter()
            .filter(|m| {
                m.name.to_ascii_lowercase().contains(&q)
                    || m.description.to_ascii_lowercase().contains(&q)
                    || m.author.to_ascii_lowercase().contains(&q)
            })
            .cloned()
            .collect())
    }

    /// Return the trust grants (key + scope) this registry will accept signatures from.
    pub fn trust_grants(&self) -> Result<Vec<TrustGrant>, RegistryError> {
        Ok(self
            .state
            .lock()
            .expect("registry lock poisoned")
            .trusted_keys
            .clone())
    }

    /// Add a trust grant (key + scope). If the key is already trusted, the grant is
    /// updated to the new scope/label rather than duplicated. Used by the CLI's
    /// `pack trust <hex> --scope <local|team|public>` command or by the registry's
    /// first-run admin bootstrap.
    pub fn trust(
        &self,
        key: PublicKey,
        scope: TrustScope,
        label: Option<String>,
    ) -> Result<(), RegistryError> {
        let mut g = self.state.lock().expect("registry lock poisoned");
        if let Some(existing) = g.trusted_keys.iter_mut().find(|t| t.key == key) {
            existing.allows = scope;
            existing.label = label.or(existing.label.clone());
            existing.granted_at = Utc::now();
        } else {
            g.trusted_keys.push(TrustGrant {
                key,
                allows: scope,
                label,
                granted_at: Utc::now(),
            });
        }
        self.write_trusted_keys(&g.trusted_keys)?;
        Ok(())
    }

    /// Drop a trust grant by key. No-op if the key isn't trusted.
    pub fn untrust(&self, key: &PublicKey) -> Result<bool, RegistryError> {
        let mut g = self.state.lock().expect("registry lock poisoned");
        let before = g.trusted_keys.len();
        g.trusted_keys.retain(|t| &t.key != key);
        let removed = g.trusted_keys.len() != before;
        if removed {
            self.write_trusted_keys(&g.trusted_keys)?;
        }
        Ok(removed)
    }

    /// List the revocation log (chronological order).
    pub fn list_revocations(&self) -> Result<Vec<RevocationEvent>, RegistryError> {
        Ok(self
            .state
            .lock()
            .expect("registry lock poisoned")
            .revocations
            .clone())
    }

    /// The newest revocation timestamp known to this registry — the high-water mark for
    /// federation sync. `None` if no revocations have happened yet (a fresh subscriber
    /// should pass `since=0` to get the full log).
    pub fn last_revocation_ts(&self) -> Option<DateTime<Utc>> {
        self.state
            .lock()
            .expect("registry lock poisoned")
            .revocations
            .iter()
            .map(|r| r.revoked_at)
            .max()
    }

    /// Revocation events strictly newer than `since` (RFC3339 comparison). Used by the
    /// federation sync layer to pull only what the subscriber hasn't seen.
    pub fn revocations_since(
        &self,
        since: DateTime<Utc>,
    ) -> Result<Vec<RevocationEvent>, RegistryError> {
        Ok(self
            .state
            .lock()
            .expect("registry lock poisoned")
            .revocations
            .iter()
            .filter(|r| r.revoked_at > since)
            .cloned()
            .collect())
    }

    /// Publish a tarball. Returns the receipt (including which path: signed/unsigned).
    ///
    /// **Verification policy:** if the tarball contains `signature.ed25519`, at least one
    /// of `trusted_keys` (or the per-call override) must verify the signature. If the
    /// tarball has no Ed25519 signature, it's stored anyway — the per-file SHA-256s are
    /// still integrity-checked at install time.
    ///
    /// **Scope policy (Sprint 14):** when a pack's manifest declares a `scope`, the
    /// matching trust grant must allow a scope at least as wide. A grant with
    /// `allows = Team` can sign `local` or `team` packs, but not `public`. Mismatched
    /// scopes fail with [`RegistryError::ScopeDenied`] — the publisher should narrow
    /// the pack's scope or request a wider grant.
    pub fn publish(
        &self,
        tarball: &[u8],
        trusted_override: Option<&str>,
    ) -> Result<PublishReceipt, RegistryError> {
        // Parse the tarball to extract the manifest + signature entry.
        let entries = cairn_pack::tar(tarball).map_err(|e| RegistryError::Pack(e.to_string()))?;
        let manifest_entry = entries
            .iter()
            .find(|e| e.name == "manifest.json")
            .ok_or_else(|| RegistryError::Pack("missing manifest.json".into()))?;
        let manifest: Manifest =
            serde_json::from_slice(&manifest_entry.body).map_err(RegistryError::Json)?;
        let pack_scope = manifest_scope(&manifest);

        // Decide which trusted-key set applies. The override is treated as a
        // public-scope grant.
        let grants: Vec<TrustGrant> = match trusted_override {
            Some(hex) => vec![TrustGrant {
                key: parse_pubkey(hex)?,
                allows: TrustScope::Public,
                label: Some("one-off override".into()),
                granted_at: Utc::now(),
            }],
            None => self
                .state
                .lock()
                .expect("registry lock poisoned")
                .trusted_keys
                .clone(),
        };

        // Try each grant; the first one whose key matches AND whose scope allows this
        // pack's scope wins.
        let mut matched_signer: Option<PublicKey> = None;
        for grant in &grants {
            if !scope_allows(grant.allows, pack_scope) {
                continue;
            }
            if cairn_pack::install::verify_ed25519_signature(
                &entries,
                &manifest_entry.body,
                &[grant.key],
            )
            .ok()
            .unwrap_or(false)
            {
                matched_signer = Some(grant.key);
                break;
            }
        }

        // Decide signed vs unsigned based on the manifest's signature entry.
        let has_signature = entries.iter().any(|e| e.name == "signature.ed25519");
        let (status, signer_hex) = match (has_signature, matched_signer) {
            (true, Some(k)) => (PublishStatus::Signed, Some(k.to_hex())),
            (true, None) => {
                // Signed but no grant accepted the signature. Either no trust or scope
                // mismatch — surface the latter explicitly so the operator can fix it.
                let has_compatible_grant =
                    grants.iter().any(|g| scope_allows(g.allows, pack_scope));
                if has_compatible_grant {
                    return Err(RegistryError::InvalidSignature);
                }
                return Err(RegistryError::ScopeDenied {
                    pack_scope,
                    granted_scopes: grants.iter().map(|g| g.allows).collect(),
                });
            }
            (false, _) => (PublishStatus::Unsigned, None),
        };

        // Stage the tarball to packs/<name>/<version>.cairnpkg. Use OpenOptions::create_new
        // (which fails with AlreadyExists if the file already exists) so two concurrent
        // publishes of the same name+version can no longer both succeed and clobber each
        // other. The previous `exists()` + `fs::write()` pair was a TOCTOU race; the
        // `create_new` open is atomic on POSIX and Windows.
        let pack_path = self
            .root
            .join("packs")
            .join(&manifest.name)
            .join(format!("{}.cairnpkg", manifest.version));
        if let Some(parent) = pack_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut pack_file = match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&pack_path)
        {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                return Err(RegistryError::AlreadyExists(format!(
                    "{}-{}",
                    manifest.name, manifest.version
                )));
            }
            Err(e) => return Err(RegistryError::Io(e)),
        };
        use std::io::Write;
        pack_file.write_all(tarball)?;
        pack_file.sync_all()?;
        drop(pack_file);
        let size_bytes = fs::metadata(&pack_path)?.len();

        // Cache the manifest under a sibling filename so a quick `find` over the registry
        // can render metadata without unpacking. Use the canonical
        // `packs/<name>/<version>.manifest.json` shape (not `<version>.cairnpkg.manifest.json`)
        // so the `download_manifest` HTTP endpoint and a future `find` index agree on
        // the same path. Same create_new semantics — never clobber a cached manifest.
        let manifest_cache = pack_path.with_extension("manifest.json");
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&manifest_cache)
        {
            Ok(mut f) => {
                f.write_all(&serde_json::to_vec_pretty(&manifest)?)?;
                f.sync_all()?;
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                // Manifest cache for this version already exists from a prior publish
                // that wrote the tarball but never landed in the index (e.g. a crash).
                // Don't fail the publish — the cache is a hint, not the source of truth.
                tracing::warn!(
                    path = %manifest_cache.display(),
                    "manifest cache already existed; overwriting"
                );
                fs::write(&manifest_cache, serde_json::to_vec_pretty(&manifest)?)?;
            }
            Err(e) => return Err(RegistryError::Io(e)),
        }

        // Append to the index.
        let meta = PackMeta {
            id: manifest.id.clone(),
            name: manifest.name.clone(),
            version: manifest.version.clone(),
            author: manifest.author.clone(),
            description: manifest.description.clone(),
            created_at: manifest.created_at,
            stored_at: Utc::now(),
            size_bytes,
            signer_pubkey: signer_hex.clone(),
            has_ed25519_signature: status == PublishStatus::Signed,
            memory_count: manifest.stats.memories,
            download_count: 0,
            scope: pack_scope,
            origin: None,
            provenance_edge_count: manifest.stats.graph_edges,
        };
        {
            let mut g = self.state.lock().expect("registry lock poisoned");
            g.index
                .retain(|m| !(m.name == meta.name && m.version == meta.version));
            g.index.push(meta.clone());
            self.write_index(&g.index)?;
        }

        Ok(PublishReceipt {
            pack_id: manifest.id,
            name: manifest.name,
            version: manifest.version,
            signed_by: signer_hex,
            status,
            stored_at: meta.stored_at,
        })
    }

    /// Read the raw tarball bytes for a stored pack.
    pub fn download_bytes(&self, name: &str, version: &str) -> Result<Vec<u8>, RegistryError> {
        let path = self
            .root
            .join("packs")
            .join(name)
            .join(format!("{version}.cairnpkg"));
        if !path.exists() {
            return Err(RegistryError::NotFound(format!("{name}-{version}")));
        }
        Ok(fs::read(&path)?)
    }

    /// Read the cached manifest for a stored pack (Sprint 14c). The cached file lives
    /// beside the tarball as `<version>.manifest.json` and contains the full pack
    /// metadata — including the graph.jsonl contents (when the publisher included
    /// provenance edges). Useful for `/dashboard/registry` to render the provenance
    /// chain without unpacking the tarball.
    pub fn download_manifest(&self, name: &str, version: &str) -> Result<Vec<u8>, RegistryError> {
        let path = self
            .root
            .join("packs")
            .join(name)
            .join(format!("{version}.manifest.json"));
        if !path.exists() {
            return Err(RegistryError::NotFound(format!("{name}-{version}")));
        }
        Ok(fs::read(&path)?)
    }

    /// Revoke (unpublish) a pack. Bumps the revocations log so federation peers see it.
    pub fn revoke(&self, name: &str, version: &str) -> Result<RevocationEvent, RegistryError> {
        let path = self
            .root
            .join("packs")
            .join(name)
            .join(format!("{version}.cairnpkg"));
        let manifest_path = self
            .root
            .join("packs")
            .join(name)
            .join(format!("{version}.manifest.json"));
        if !path.exists() {
            return Err(RegistryError::NotFound(format!("{name}-{version}")));
        }
        fs::remove_file(&path)?;
        let _ = fs::remove_file(&manifest_path);

        let event = RevocationEvent {
            name: name.to_string(),
            version: version.to_string(),
            revoked_at: Utc::now(),
            reason: None,
        };
        {
            let mut g = self.state.lock().expect("registry lock poisoned");
            g.index
                .retain(|m| !(m.name == name && m.version == version));
            self.write_index(&g.index)?;
            g.revocations.push(event.clone());
            self.write_revocations(&g.revocations)?;
        }
        Ok(event)
    }

    /// Federation-cascade revoke: append a revocation event without requiring a local
    /// pack tarball to be present. Used by [`crate::federation::sync_from`] when
    /// propagating a peer registry's revocation to a subscriber that may never have
    /// installed the pack locally. Returns the event that was recorded.
    pub fn revoke_if_exists(
        &self,
        name: &str,
        version: &str,
    ) -> Result<RevocationEvent, RegistryError> {
        let event = RevocationEvent {
            name: name.to_string(),
            version: version.to_string(),
            revoked_at: Utc::now(),
            reason: Some("cascade from peer".into()),
        };
        let mut g = self.state.lock().expect("registry lock poisoned");
        g.index
            .retain(|m| !(m.name == name && m.version == version));
        self.write_index(&g.index)?;
        g.revocations.push(event.clone());
        self.write_revocations(&g.revocations)?;
        Ok(event)
    }

    fn write_index(&self, index: &[PackMeta]) -> Result<(), RegistryError> {
        let tmp = self.root.join("index.json.tmp");
        {
            let mut f = fs::File::create(&tmp)?;
            let json = serde_json::to_vec_pretty(index)?;
            f.write_all(&json)?;
            f.sync_all()?;
        }
        fs::rename(tmp, self.root.join("index.json"))?;
        Ok(())
    }

    fn write_trusted_keys(&self, keys: &[TrustGrant]) -> Result<(), RegistryError> {
        let tmp = self.root.join("trusted_keys.json.tmp");
        {
            let mut f = fs::File::create(&tmp)?;
            let json = serde_json::to_vec_pretty(keys)?;
            f.write_all(&json)?;
            f.sync_all()?;
        }
        fs::rename(tmp, self.root.join("trusted_keys.json"))?;
        Ok(())
    }

    fn write_revocations(&self, revs: &[RevocationEvent]) -> Result<(), RegistryError> {
        let tmp = self.root.join("revocations.jsonl.tmp");
        {
            let mut f = fs::File::create(&tmp)?;
            for r in revs {
                let json = serde_json::to_vec(r)?;
                f.write_all(&json)?;
                f.write_all(b"\n")?;
            }
            f.sync_all()?;
        }
        fs::rename(tmp, self.root.join("revocations.jsonl"))?;
        Ok(())
    }
}

/// Parse a hex-encoded 32-byte Ed25519 public key.
fn parse_pubkey(hex_str: &str) -> Result<PublicKey, RegistryError> {
    let bytes = hex::decode(hex_str.trim()).map_err(|_| RegistryError::BadKey(hex_str.into()))?;
    PublicKey::from_bytes(&bytes).map_err(|_| RegistryError::BadKey(hex_str.into()))
}

/// Read the pack's declared scope from its manifest. The current `.cairnpkg` manifest
/// doesn't carry an explicit `scope` field — we infer it from the `description` field's
/// `scope: <local|team|public>` prefix when present, falling back to `public`.
///
/// **SECURITY CAVEAT:** This is a substring match on a description string that the
/// publisher controls and that is *not* part of the manifest's signed payload
/// (`signature.ed25519` covers the manifest JSON, not its rendered description text in
/// the registry). A malicious publisher can ship a pack whose description reads
/// "Public release, please redistribute" with the literal phrase `scope: local`
/// anywhere in it, and the registry will classify it as local (and refuse public
/// re-publish). The first match in the fixed-order probe list wins, so the attack is
/// trivial. Until the schema is upgraded, treat `manifest_scope` as best-effort and
/// require a `trust_override` on publish for any non-`Public` scope.
// FIXME: v0.6 — replace description-prefix parsing with a first-class `scope` field in
// `cairn_pack::Manifest` and include it in the canonical signed payload. Track the
// schema bump alongside the other v0.6 ledger + manifest migrations.
fn manifest_scope(manifest: &cairn_pack::Manifest) -> TrustScope {
    let desc = &manifest.description;
    for (marker, scope) in [
        ("scope: local", TrustScope::Local),
        ("scope: team", TrustScope::Team),
        ("scope: public", TrustScope::Public),
    ] {
        if desc.to_ascii_lowercase().contains(marker) {
            return scope;
        }
    }
    TrustScope::Public
}

/// True when a trust grant with `granted_scope` is allowed to publish packs whose
/// declared scope is `pack_scope`. Local ⊂ Team ⊂ Public (i.e. a wider grant can
/// publish a narrower-scoped pack, but not the other way around).
fn scope_allows(granted: TrustScope, pack: TrustScope) -> bool {
    fn rank(s: TrustScope) -> u8 {
        match s {
            TrustScope::Local => 0,
            TrustScope::Team => 1,
            TrustScope::Public => 2,
        }
    }
    rank(granted) >= rank(pack)
}
#[cfg(test)]
mod tests {
    use super::*;
    use cairn_pack::{Keypair, Pack};
    use tempfile::TempDir;

    fn signed_pack_bytes(kp: &Keypair) -> Vec<u8> {
        let dir = TempDir::new().unwrap();
        let out = dir.path().join("p.cairnpkg");
        let mut pack = Pack::new("alpha", "1.0.0");
        pack.author = "tester".into();
        pack.description = "searchable text".into();
        pack.memories
            .push(serde_json::json!({"id": "m1", "content": "x"}));
        pack.write_tarball_signed(&out, kp).unwrap();
        std::fs::read(&out).unwrap()
    }

    #[test]
    fn publish_unsigned_pack_succeeds_and_is_listable() {
        let dir = TempDir::new().unwrap();
        let reg = Registry::open(dir.path()).unwrap();

        let bytes = {
            let td = TempDir::new().unwrap();
            let out = td.path().join("u.cairnpkg");
            let mut pack = Pack::new("plain", "0.1.0");
            pack.memories
                .push(serde_json::json!({"id": "m1", "content": "x"}));
            pack.write_tarball(&out).unwrap();
            std::fs::read(&out).unwrap()
        };
        let receipt = reg.publish(&bytes, None).unwrap();
        assert_eq!(receipt.status, PublishStatus::Unsigned);
        assert!(receipt.signed_by.is_none());

        let all = reg.list_all().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].name, "plain");
        assert!(!all[0].has_ed25519_signature);
    }

    #[test]
    fn publish_signed_pack_with_trusted_key_returns_signed() {
        let dir = TempDir::new().unwrap();
        let reg = Registry::open(dir.path()).unwrap();
        let kp = Keypair::generate();
        reg.trust(kp.public(), TrustScope::Public, None).unwrap();

        let bytes = signed_pack_bytes(&kp);
        let receipt = reg.publish(&bytes, None).unwrap();
        assert_eq!(receipt.status, PublishStatus::Signed);
        assert_eq!(
            receipt.signed_by.as_deref(),
            Some(kp.public().to_hex().as_str())
        );
    }

    #[test]
    fn publish_signed_pack_without_trusted_key_rejects() {
        let dir = TempDir::new().unwrap();
        let reg = Registry::open(dir.path()).unwrap();
        // Note: no trust() call.
        let kp = Keypair::generate();
        let bytes = signed_pack_bytes(&kp);
        let err = reg.publish(&bytes, None).unwrap_err();
        // Empty grants list → pack's public scope is denied by every grant (none).
        assert!(
            matches!(err, RegistryError::ScopeDenied { .. }),
            "got {err:?}"
        );
    }

    #[test]
    fn publish_pack_with_team_scope_rejected_by_local_only_grant() {
        let dir = TempDir::new().unwrap();
        let reg = Registry::open(dir.path()).unwrap();
        let kp = Keypair::generate();
        // Grant only allows Local — pack's team scope should be rejected.
        reg.trust(kp.public(), TrustScope::Local, None).unwrap();

        let mut pack = Pack::new("team-pack", "1.0.0");
        pack.description = "scope: team — shared with the team".into();
        pack.memories
            .push(serde_json::json!({"id": "m1", "content": "x"}));
        let td = TempDir::new().unwrap();
        let out = td.path().join("team.cairnpkg");
        pack.write_tarball_signed(&out, &kp).unwrap();
        let bytes = std::fs::read(&out).unwrap();

        let err = reg.publish(&bytes, None).unwrap_err();
        match err {
            RegistryError::ScopeDenied {
                pack_scope: TrustScope::Team,
                granted_scopes,
            } => {
                assert_eq!(granted_scopes, vec![TrustScope::Local]);
            }
            other => panic!("expected ScopeDenied, got {other:?}"),
        }
    }

    #[test]
    fn publish_team_scoped_pack_with_team_grant_succeeds() {
        let dir = TempDir::new().unwrap();
        let reg = Registry::open(dir.path()).unwrap();
        let kp = Keypair::generate();
        reg.trust(kp.public(), TrustScope::Team, Some("team-bot".into()))
            .unwrap();

        let mut pack = Pack::new("team-ok", "1.0.0");
        pack.description = "scope: team".into();
        pack.memories
            .push(serde_json::json!({"id": "m1", "content": "x"}));
        let td = TempDir::new().unwrap();
        let out = td.path().join("team.cairnpkg");
        pack.write_tarball_signed(&out, &kp).unwrap();
        let bytes = std::fs::read(&out).unwrap();

        let receipt = reg.publish(&bytes, None).unwrap();
        assert_eq!(receipt.status, PublishStatus::Signed);

        // Listing must report the scope on the PackMeta.
        let all = reg.list_all().unwrap();
        assert_eq!(all[0].scope, TrustScope::Team);
        // And the trust grant's label must be retrievable.
        let grants = reg.trust_grants().unwrap();
        assert_eq!(grants.len(), 1);
        assert_eq!(grants[0].label.as_deref(), Some("team-bot"));
    }

    #[test]
    fn untrust_removes_a_key_and_blocks_future_publishes() {
        let dir = TempDir::new().unwrap();
        let reg = Registry::open(dir.path()).unwrap();
        let kp = Keypair::generate();
        reg.trust(kp.public(), TrustScope::Public, None).unwrap();
        assert_eq!(reg.trust_grants().unwrap().len(), 1);

        assert!(reg.untrust(&kp.public()).unwrap());
        assert!(
            !reg.untrust(&kp.public()).unwrap(),
            "second untrust is a no-op"
        );
        assert!(reg.trust_grants().unwrap().is_empty());

        // Publish now fails because no grant is configured.
        let bytes = signed_pack_bytes(&kp);
        let err = reg.publish(&bytes, None).unwrap_err();
        assert!(
            matches!(err, RegistryError::ScopeDenied { .. }),
            "got {err:?}"
        );
    }

    #[test]
    fn trust_updating_an_existing_key_replaces_scope_and_preserves_label() {
        let dir = TempDir::new().unwrap();
        let reg = Registry::open(dir.path()).unwrap();
        let kp = Keypair::generate();
        reg.trust(kp.public(), TrustScope::Team, Some("alice".into()))
            .unwrap();
        reg.trust(kp.public(), TrustScope::Public, None).unwrap();

        let grants = reg.trust_grants().unwrap();
        assert_eq!(grants.len(), 1, "should update in place, not duplicate");
        assert_eq!(grants[0].allows, TrustScope::Public);
        assert_eq!(grants[0].label.as_deref(), Some("alice"), "label preserved");
    }

    #[test]
    fn scope_marker_in_description_is_detected() {
        // Sanity-check the description parser used by the publish path.
        let m_local = cairn_pack::Manifest::new(
            "x",
            "1",
            "a",
            "scope: local — for me only",
            Default::default(),
            Default::default(),
        );
        assert_eq!(manifest_scope(&m_local), TrustScope::Local);
        let m_default = cairn_pack::Manifest::new(
            "x",
            "1",
            "a",
            "no scope here",
            Default::default(),
            Default::default(),
        );
        assert_eq!(manifest_scope(&m_default), TrustScope::Public);
    }

    /// Two concurrent publishes of the same name+version must produce exactly one success
    /// and one AlreadyExists failure. Without the atomic create_new open, both writers
    /// would race past the `exists()` check and clobber each other.
    #[test]
    fn concurrent_publish_of_same_version_only_one_wins() {
        use std::sync::Arc;
        use std::thread;

        let dir = TempDir::new().unwrap();
        let reg = Arc::new(Registry::open(dir.path()).unwrap());

        // Pre-build the tarball bytes once so the threads share identical inputs.
        let kp = Keypair::generate();
        reg.trust(kp.public(), TrustScope::Public, None).unwrap();
        let bytes = Arc::new(signed_pack_bytes(&kp));

        let mut handles = Vec::new();
        for _ in 0..4 {
            let reg = Arc::clone(&reg);
            let bytes = Arc::clone(&bytes);
            handles.push(thread::spawn(move || reg.publish(&bytes, None)));
        }
        let mut successes = 0;
        let mut already_exists = 0;
        for h in handles {
            match h.join().unwrap() {
                Ok(_) => successes += 1,
                Err(RegistryError::AlreadyExists(_)) => already_exists += 1,
                Err(e) => panic!("unexpected error: {e:?}"),
            }
        }
        assert_eq!(successes, 1, "exactly one publish must succeed");
        assert_eq!(
            already_exists, 3,
            "all other concurrent publishes must see AlreadyExists"
        );

        let listed = reg.list_all().unwrap();
        assert_eq!(listed.len(), 1, "index must contain exactly one pack");
    }

    #[test]
    fn publish_with_trust_override_accepts_only_the_overridden_key() {
        let dir = TempDir::new().unwrap();
        let reg = Registry::open(dir.path()).unwrap();
        let kp = Keypair::generate();
        let bytes = signed_pack_bytes(&kp);
        let receipt = reg.publish(&bytes, Some(&kp.public().to_hex())).unwrap();
        assert_eq!(receipt.status, PublishStatus::Signed);
    }

    #[test]
    fn download_bytes_round_trips_a_published_pack() {
        let dir = TempDir::new().unwrap();
        let reg = Registry::open(dir.path()).unwrap();
        let bytes = {
            let td = TempDir::new().unwrap();
            let out = td.path().join("r.cairnpkg");
            let mut pack = Pack::new("rt", "1.0.0");
            pack.memories
                .push(serde_json::json!({"id": "m1", "content": "x"}));
            pack.write_tarball(&out).unwrap();
            std::fs::read(&out).unwrap()
        };
        reg.publish(&bytes, None).unwrap();
        let downloaded = reg.download_bytes("rt", "1.0.0").unwrap();
        assert_eq!(downloaded, bytes);
    }

    #[test]
    fn publish_records_provenance_edge_count_in_pack_meta() {
        let dir = TempDir::new().unwrap();
        let reg = Registry::open(dir.path()).unwrap();
        let bytes = {
            let td = TempDir::new().unwrap();
            let out = td.path().join("prov.cairnpkg");
            let mut pack = Pack::new("prov", "1.0.0");
            pack.memories
                .push(serde_json::json!({"id": "m1", "content": "alpha"}));
            pack.graph_edges.push(serde_json::json!({
                "src": "m1",
                "dst": "m2",
                "kind": "derived_from",
            }));
            pack.graph_edges.push(serde_json::json!({
                "src": "m1",
                "dst": "src/foo.rs",
                "kind": "applies_to",
            }));
            pack.write_tarball(&out).unwrap();
            std::fs::read(&out).unwrap()
        };
        reg.publish(&bytes, None).unwrap();

        let meta = reg.list_all().unwrap();
        assert_eq!(meta[0].provenance_edge_count, 2);

        // The cached manifest must be readable via download_manifest and include the
        // graph edge count.
        let manifest_bytes = reg.download_manifest("prov", "1.0.0").unwrap();
        let m: Manifest = serde_json::from_slice(&manifest_bytes).unwrap();
        assert_eq!(m.stats.graph_edges, 2);
    }

    #[test]
    fn revoke_removes_the_pack_and_appends_to_log() {
        let dir = TempDir::new().unwrap();
        let reg = Registry::open(dir.path()).unwrap();
        let bytes = {
            let td = TempDir::new().unwrap();
            let out = td.path().join("r.cairnpkg");
            let mut pack = Pack::new("killme", "1.0.0");
            pack.memories
                .push(serde_json::json!({"id": "m1", "content": "x"}));
            pack.write_tarball(&out).unwrap();
            std::fs::read(&out).unwrap()
        };
        reg.publish(&bytes, None).unwrap();
        let event = reg.revoke("killme", "1.0.0").unwrap();
        assert_eq!(event.name, "killme");
        assert_eq!(event.version, "1.0.0");

        // Subsequent download must fail.
        assert!(reg.download_bytes("killme", "1.0.0").is_err());
        // And the revocation log must include it.
        assert_eq!(reg.list_revocations().unwrap().len(), 1);
    }

    #[test]
    fn search_matches_name_description_and_author_case_insensitively() {
        let dir = TempDir::new().unwrap();
        let reg = Registry::open(dir.path()).unwrap();
        let kp = Keypair::generate();
        reg.trust(kp.public(), TrustScope::Public, None).unwrap();

        let by_name = signed_pack_bytes(&kp); // alpha, author "tester"
        let bytes2 = {
            let td = TempDir::new().unwrap();
            let out = td.path().join("z.cairnpkg");
            let mut pack = Pack::new("zebra-stripes", "2.0.0");
            pack.author = "another-author".into();
            pack.description = "all about horses".into();
            pack.memories
                .push(serde_json::json!({"id": "m1", "content": "x"}));
            pack.write_tarball(&out).unwrap();
            std::fs::read(&out).unwrap()
        };
        reg.publish(&by_name, None).unwrap();
        reg.publish(&bytes2, None).unwrap();

        let hits = reg.search("ALPHA").unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].name, "alpha");
        let hits = reg.search("horses").unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].name, "zebra-stripes");
        let hits = reg.search("tester").unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].name, "alpha");
        let hits = reg.search("another-author").unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].name, "zebra-stripes");
    }

    #[test]
    fn list_versions_returns_only_that_pack() {
        let dir = TempDir::new().unwrap();
        let reg = Registry::open(dir.path()).unwrap();
        for v in ["1.0.0", "1.1.0", "2.0.0"] {
            let td = TempDir::new().unwrap();
            let out = td.path().join(format!("p-{v}.cairnpkg"));
            let mut pack = Pack::new("multi", v);
            pack.memories
                .push(serde_json::json!({"id": "m1", "content": "x"}));
            pack.write_tarball(&out).unwrap();
            reg.publish(&std::fs::read(&out).unwrap(), None).unwrap();
        }
        let versions = reg.list_versions("multi").unwrap();
        assert_eq!(versions.len(), 3);
        let names: Vec<&str> = versions.iter().map(|v| v.version.as_str()).collect();
        assert!(names.contains(&"1.0.0"));
        assert!(names.contains(&"1.1.0"));
        assert!(names.contains(&"2.0.0"));
    }
}
