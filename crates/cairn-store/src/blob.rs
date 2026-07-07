//! Content-addressed blob store on the filesystem.
//!
//! Files are stored under `blobs/<first-2-hex>/<full-hash>`, sharded to keep directories small.
//! Writes are idempotent (same content -> same path), so storing an original twice is free.

use cairn_core::{ContentHash, Result};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct BlobStore {
    root: PathBuf,
}

impl BlobStore {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    fn path_for(&self, hash: &ContentHash) -> PathBuf {
        let s = hash.as_str();
        self.root.join(&s[..2]).join(s)
    }

    /// Store bytes, returning their content hash. Idempotent.
    pub fn put(&self, bytes: &[u8]) -> Result<ContentHash> {
        let hash = ContentHash::of(bytes);
        let path = self.path_for(&hash);
        if !path.exists() {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&path, bytes)?;
        }
        Ok(hash)
    }

    pub fn put_str(&self, s: &str) -> Result<ContentHash> {
        self.put(s.as_bytes())
    }

    /// Fetch the exact original bytes for a hash, if present.
    pub fn get(&self, hash: &ContentHash) -> Result<Option<Vec<u8>>> {
        match fs::read(self.path_for(hash)) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn get_str(&self, hash: &ContentHash) -> Result<Option<String>> {
        Ok(self
            .get(hash)?
            .map(|b| String::from_utf8_lossy(&b).into_owned()))
    }

    pub fn has(&self, hash: &ContentHash) -> bool {
        self.path_for(hash).exists()
    }

    /// Resolve a short handle (prefix of the full content hash) to the full
    /// `ContentHash`, scanning the shard directory. Returns `None` when no blob
    /// matches the prefix. Handles shorter than 2 chars cannot map to a shard
    /// and are rejected.
    ///
    /// `read` advertises `handle = hash.short()` (12 chars) as the value to pass
    /// to `expand`, while `expand` looks up by full hash. This bridges the gap.
    pub fn resolve_short(&self, prefix: &str) -> Result<Option<ContentHash>> {
        if prefix.len() < 2 {
            return Ok(None);
        }
        let shard = &prefix[..2];
        let dir = self.root.join(shard);
        if !dir.exists() {
            return Ok(None);
        }
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with(prefix) {
                return Ok(Some(ContentHash(name.into_owned())));
            }
        }
        Ok(None)
    }
}
