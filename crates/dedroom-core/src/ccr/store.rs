//! CCR storage backends.
//!
//! Stores compressed content originals keyed by BLAKE3 hash, with TTL
//! eviction. Shared between compression cache and loop detection memory.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use blake3::Hash;

/// A stored entry in the CCR.
#[derive(Debug, Clone)]
pub struct CcrEntry {
    /// The original (uncompressed) content.
    pub original: String,
    /// Whether the associated tool call resulted in an error.
    pub was_error: bool,
    /// Expiration time.
    pub expires_at: Instant,
    /// Optional tool name for lookup.
    pub tool: Option<String>,
    /// Optional canonical args hash for loop detection.
    pub args_hash: Option<Hash>,
}

impl CcrEntry {
    pub fn new(original: String, was_error: bool, ttl: Duration) -> Self {
        Self {
            original,
            was_error,
            expires_at: Instant::now() + ttl,
            tool: None,
            args_hash: None,
        }
    }
}

/// Backend trait for CCR storage.
#[async_trait::async_trait]
pub trait CcrBackend: Send + Sync + std::fmt::Debug {
    /// Store an entry.
    async fn put(&self, key: Hash, entry: CcrEntry);
    /// Retrieve an entry.
    async fn get(&self, key: &Hash) -> Option<CcrEntry>;
    /// Check if a key exists and return its error flag.
    async fn exists_with_error(&self, key: &Hash) -> Option<bool>;
    /// Remove expired entries.
    async fn prune(&self);
}

/// In-memory CCR store backed by a `HashMap`.
#[derive(Debug, Clone)]
pub struct InMemoryStore {
    inner: Arc<RwLock<HashMap<Hash, CcrEntry>>>,
    default_ttl: Duration,
}

impl InMemoryStore {
    pub fn new(ttl_seconds: u64) -> Self {
        Self {
            inner: Arc::new(RwLock::new(HashMap::new())),
            default_ttl: Duration::from_secs(ttl_seconds),
        }
    }
}

#[async_trait::async_trait]
impl CcrBackend for InMemoryStore {
    async fn put(&self, key: Hash, mut entry: CcrEntry) {
        if entry.expires_at <= Instant::now() {
            entry.expires_at = Instant::now() + self.default_ttl;
        }
        let mut map = self.inner.write().await;
        map.insert(key, entry);
    }

    async fn get(&self, key: &Hash) -> Option<CcrEntry> {
        let map = self.inner.read().await;
        map.get(key).and_then(|e| {
            if e.expires_at > Instant::now() {
                Some(e.clone())
            } else {
                None
            }
        })
    }

    async fn exists_with_error(&self, key: &Hash) -> Option<bool> {
        let map = self.inner.read().await;
        map.get(key).and_then(|e| {
            if e.expires_at > Instant::now() {
                Some(e.was_error)
            } else {
                None
            }
        })
    }

    async fn prune(&self) {
        let mut map = self.inner.write().await;
        map.retain(|_, e| e.expires_at > Instant::now());
    }
}

/// Compute a BLAKE3 hash for a tool call.
pub fn hash_tool_call(tool: &str, canonical_args: &str) -> Hash {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"tool_call");
    hasher.update(tool.as_bytes());
    hasher.update(canonical_args.as_bytes());
    hasher.finalize()
}

/// The top-level CCR store, wrapping a backend.
#[derive(Debug, Clone)]
pub struct CcrStore {
    backend: Arc<dyn CcrBackend>,
    default_ttl: Duration,
}

impl CcrStore {
    pub fn new(backend: Arc<dyn CcrBackend>, ttl_seconds: u64) -> Self {
        Self {
            backend,
            default_ttl: Duration::from_secs(ttl_seconds),
        }
    }

    pub async fn put(&self, key: Hash, original: String, was_error: bool) {
        let entry = CcrEntry::new(original, was_error, self.default_ttl);
        self.backend.put(key, entry).await;
    }

    pub async fn put_with_meta(
        &self,
        key: Hash,
        original: String,
        was_error: bool,
        tool: String,
        args_hash: Hash,
    ) {
        let mut entry = CcrEntry::new(original, was_error, self.default_ttl);
        entry.tool = Some(tool);
        entry.args_hash = Some(args_hash);
        self.backend.put(key, entry).await;
    }

    pub async fn get(&self, key: &Hash) -> Option<CcrEntry> {
        self.backend.get(key).await
    }

    pub async fn exists_with_error(&self, key: &Hash) -> Option<bool> {
        self.backend.exists_with_error(key).await
    }

    pub async fn prune(&self) {
        self.backend.prune().await;
    }

    /// Default TTL duration.
    pub fn default_ttl(&self) -> Duration {
        self.default_ttl
    }
}

// ── SQLite store (placeholder) ──────────────────────────────────────────────

/// SQLite-backed CCR store (requires `rusqlite`).
#[cfg(feature = "sqlite")]
#[derive(Debug)]
pub struct SqliteStore {
    // TODO: implement SQLite backend
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_in_memory_put_get() {
        let store = InMemoryStore::new(60);
        let key = blake3::hash(b"test data");
        store.put(key, CcrEntry::new("hello".into(), false, Duration::from_secs(60))).await;
        let entry = store.get(&key).await;
        assert!(entry.is_some());
        assert_eq!(entry.unwrap().original, "hello");
    }

    #[tokio::test]
    async fn test_in_memory_expiry() {
        let store = InMemoryStore::new(0); // 0 second TTL
        let key = blake3::hash(b"test data");
        store.put(key, CcrEntry::new("hello".into(), false, Duration::from_secs(0))).await;
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
        // Entry should be expired (TTL = 0)
        let entry = store.get(&key).await;
        assert!(entry.is_none());
    }

    #[tokio::test]
    async fn test_exists_with_error() {
        let store = InMemoryStore::new(60);
        let key = blake3::hash(b"failed call");
        store.put(key, CcrEntry::new("error".into(), true, Duration::from_secs(60))).await;
        assert_eq!(store.exists_with_error(&key).await, Some(true));
    }

    #[test]
    fn test_hash_tool_call_deterministic() {
        let h1 = hash_tool_call("write_file", r#"{"path":"/tmp/x.txt"}"#);
        let h2 = hash_tool_call("write_file", r#"{"path":"/tmp/x.txt"}"#);
        assert_eq!(h1, h2);
    }

    #[test]
    fn test_hash_tool_call_different_args() {
        let h1 = hash_tool_call("write_file", r#"{"path":"/tmp/a.txt"}"#);
        let h2 = hash_tool_call("write_file", r#"{"path":"/tmp/b.txt"}"#);
        assert_ne!(h1, h2);
    }
}
