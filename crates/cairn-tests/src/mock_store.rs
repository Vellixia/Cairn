//! In-memory mock of `cairn-store` for tests that need a real `Store`
//! reference but cannot talk to HelixDB.
//!
//! Most integration tests in this crate construct `Memory` / `NewMemory`
//! values directly and exercise crate functions that accept typed input
//! without going through a `Store`. The few cases that *do* need a store
//! (e.g. `cairn-context` reading from a workspace dir, or
//! `cairn-session` writing drift events) use [`MockStore`] — a thin
//! `HashMap`-backed shim that satisfies the small slice of the
//! `cairn-store::Store` API the tests touch.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::Result;
use chrono::{DateTime, Utc};

/// Tiny in-memory store. The methods exposed are exactly the ones the
/// integration tests in this crate call — anything else is deliberately
/// absent so we notice if production code starts using a different
/// surface.
#[derive(Debug, Default)]
pub struct MockStore {
    inner: Arc<Mutex<MockStoreInner>>,
    workspace_root: PathBuf,
}

#[derive(Debug, Default)]
struct MockStoreInner {
    /// Path -> bytes for `read_file`-style lookups.
    files: HashMap<PathBuf, Vec<u8>>,
    /// Audit ledger: append-only list of (timestamp, message, hmac) tuples.
    audit: Vec<AuditEntry>,
}

#[derive(Debug, Clone)]
pub struct AuditEntry {
    pub ts: DateTime<Utc>,
    pub message: String,
    pub hmac: [u8; 32],
}

impl MockStore {
    /// New empty mock rooted at `workspace_root` (used to satisfy
    /// `read_file` lookups for the in-memory files).
    pub fn new(workspace_root: impl Into<PathBuf>) -> Self {
        Self {
            inner: Arc::new(Mutex::new(MockStoreInner::default())),
            workspace_root: workspace_root.into(),
        }
    }

    /// Insert a fake file so the read modes in cairn-context have something
    /// to operate on. `path` is resolved relative to the workspace root.
    pub fn put_file(&self, path: impl AsRef<Path>, bytes: impl Into<Vec<u8>>) {
        let mut g = self.inner.lock().expect("mock_store poisoned");
        g.files.insert(path.as_ref().to_path_buf(), bytes.into());
    }

    /// Read a file by absolute path, or `None` if the mock has not been
    /// seeded with it.
    pub fn read_file(&self, path: impl AsRef<Path>) -> Option<Vec<u8>> {
        let g = self.inner.lock().expect("mock_store poisoned");
        g.files.get(path.as_ref()).cloned()
    }

    /// Workspace root. Returned for tests that exercise the path-rewrite
    /// surface of `cairn-context` and need to know the absolute prefix.
    pub fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    /// Append an audit entry. The HMAC is computed deterministically from
    /// the message + a fixed test key so tests can assert on it.
    pub fn append_audit(&self, message: &str) -> AuditEntry {
        use hmac::{Hmac, Mac};
        use sha2::Sha256;
        let key = b"cairn-test-fixture-key-do-not-use-in-prod";
        let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(key).expect("hmac key length is fixed");
        mac.update(message.as_bytes());
        let result = mac.finalize().into_bytes();
        let mut hmac = [0u8; 32];
        hmac.copy_from_slice(&result);
        let entry = AuditEntry {
            ts: Utc::now(),
            message: message.to_string(),
            hmac,
        };
        let mut g = self.inner.lock().expect("mock_store poisoned");
        g.audit.push(entry.clone());
        entry
    }

    /// All audit entries written so far, in append order.
    pub fn audit_log(&self) -> Vec<AuditEntry> {
        let g = self.inner.lock().expect("mock_store poisoned");
        g.audit.clone()
    }

    /// Verify the audit log's HMACs (no-op on a corrupted line: returns the
    /// index of the first bad line, or `None` if all lines verify).
    pub fn audit_first_corruption(&self) -> Option<usize> {
        use hmac::{Hmac, Mac};
        use sha2::Sha256;
        let key = b"cairn-test-fixture-key-do-not-use-in-prod";
        let g = self.inner.lock().expect("mock_store poisoned");
        for (i, e) in g.audit.iter().enumerate() {
            let mut mac =
                <Hmac<Sha256> as Mac>::new_from_slice(key).expect("hmac key length is fixed");
            mac.update(e.message.as_bytes());
            if mac.verify_slice(&e.hmac).is_err() {
                return Some(i);
            }
        }
        None
    }
}

/// Convenience: a function that takes a `&cairn_store::Store` cannot accept
/// this directly. The few tests that need a real `Store` build one with
/// `tempfile::tempdir()` and copy the seeded files into it; this shim is
/// purely a fixture to make wiring fast.
pub fn seed_minimal_workspace() -> MockStore {
    let store = MockStore::new("/tmp/cairn-test-ws");
    store.put_file("README.md", b"# cairn test\n");
    store
}

/// `Result` re-export so test files don't need to import anyhow.
pub type MockResult<T> = Result<T>;
