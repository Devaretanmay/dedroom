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
}

impl CcrEntry {
    pub fn new(original: String, was_error: bool, ttl: Duration) -> Self {
        Self {
            original,
            was_error,
            expires_at: Instant::now() + ttl,
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

// ── SQLite backend (behind `sqlite` feature) ────────────────────────────────

/// SQLite-backed CCR store for persistent on-disk storage.
///
/// Creates a `ccr_entries` table with BLOB keys, TEXT values, and epoch-
/// second expiry. Uses `tokio::sync::Mutex` internally because `rusqlite::Connection`
/// is `Send` but not `Sync`.
#[cfg(feature = "sqlite")]
#[derive(Debug)]
pub struct SqliteStore {
    conn: tokio::sync::Mutex<rusqlite::Connection>,
    default_ttl: Duration,
    /// Put counter. Only prunes expired entries every 100 puts.
    put_counter: std::sync::atomic::AtomicU64,
}

#[cfg(feature = "sqlite")]
impl SqliteStore {
    /// Open or create a database at `path`.
    ///
    /// Pass `":memory:"` for an in-memory database (useful in tests).
    pub fn new(path: &str, ttl_seconds: u64) -> rusqlite::Result<Self> {
        let conn = rusqlite::Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "wal")?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS ccr_entries (
                key_hash   BLOB    PRIMARY KEY NOT NULL,
                original   TEXT    NOT NULL,
                was_error  INTEGER NOT NULL DEFAULT 0,
                expires_at INTEGER NOT NULL,
                tool       TEXT,
                args_hash  BLOB
            )"
        )?;
        Ok(Self {
            conn: tokio::sync::Mutex::new(conn),
            default_ttl: Duration::from_secs(ttl_seconds),
            put_counter: std::sync::atomic::AtomicU64::new(1),
        })
    }

    /// Create an in-memory SQLite store (convenience for tests).
    pub fn new_in_memory(ttl_seconds: u64) -> rusqlite::Result<Self> {
        Self::new(":memory:", ttl_seconds)
    }

    /// Delete expired entries to keep the DB bounded.
    fn prune_expired(&self) {
        let now_epoch = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        if let Ok(conn) = self.conn.try_lock() {
            let _ = conn.execute(
                "DELETE FROM ccr_entries WHERE expires_at <= ?1",
                rusqlite::params![now_epoch],
            );
        }
    }
}

#[async_trait::async_trait]
#[cfg(feature = "sqlite")]
impl CcrBackend for SqliteStore {
    async fn put(&self, key: Hash, entry: CcrEntry) {
        let now = Instant::now();
        let remaining = if entry.expires_at > now {
            entry.expires_at - now
        } else {
            self.default_ttl
        };
        let expires_at_epoch = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64
            + remaining.as_secs() as i64;

        {
            let conn = self.conn.lock().await;
            let _ = conn.execute(
                "INSERT OR REPLACE INTO ccr_entries (key_hash, original, was_error, expires_at)
                 VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![
                    key.as_bytes() as &[u8],
                    entry.original,
                    entry.was_error as i32,
                    expires_at_epoch,
                ],
            );
        }

        // Amortize expired-entry cleanup: only prune every 100 puts.
        let prev = self.put_counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if prev.is_multiple_of(100) {
            self.prune_expired();
        }
    }

    async fn get(&self, key: &Hash) -> Option<CcrEntry> {
        let key_bytes = key.as_bytes() as &[u8];
        let conn = self.conn.lock().await;
        let mut stmt = conn
            .prepare(
                "SELECT original, was_error, expires_at
                 FROM ccr_entries WHERE key_hash = ?1",
            )
            .ok()?;

        let row = stmt
            .query_row(rusqlite::params![key_bytes], |row| {
                let original: String = row.get(0)?;
                let was_error: bool = row.get::<_, i32>(1)? != 0;
                let expires_at_epoch: i64 = row.get(2)?;
                Ok((original, was_error, expires_at_epoch))
            })
            .ok()?;

        let (original, was_error, expires_at_epoch) = row;

        // Check expiry
        let now_epoch = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        if expires_at_epoch <= now_epoch {
            return None;
        }

        // Reconstruct Instant from remaining duration
        let remaining_secs = (expires_at_epoch - now_epoch) as u64;
        let expires_at = Instant::now() + Duration::from_secs(remaining_secs);

        Some(CcrEntry {
            original,
            was_error,
            expires_at,
        })
    }

    async fn exists_with_error(&self, key: &Hash) -> Option<bool> {
        let key_bytes = key.as_bytes() as &[u8];
        let now_epoch = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        let conn = self.conn.lock().await;
        let mut stmt = conn
            .prepare(
                "SELECT was_error FROM ccr_entries WHERE key_hash = ?1 AND expires_at > ?2",
            )
            .ok()?;

        stmt.query_row(rusqlite::params![key_bytes, now_epoch], |row| {
            let was_error: bool = row.get::<_, i32>(0)? != 0;
            Ok(was_error)
        })
        .ok()
    }

    async fn prune(&self) {
        let now_epoch = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        let conn = self.conn.lock().await;
        let _ = conn.execute(
            "DELETE FROM ccr_entries WHERE expires_at <= ?1",
            rusqlite::params![now_epoch],
        );
    }
}

// ── Top-level store wrapper ─────────────────────────────────────────────────

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

    // ── SQLite backend tests ─────────────────────────────────────────────

    #[cfg(feature = "sqlite")]
    #[tokio::test]
    async fn test_sqlite_put_get() {
        let store = SqliteStore::new_in_memory(60).unwrap();
        let key = blake3::hash(b"test data");
        store
            .put(key, CcrEntry::new("hello".into(), false, Duration::from_secs(60)))
            .await;
        let entry = store.get(&key).await;
        assert!(entry.is_some());
        assert_eq!(entry.unwrap().original, "hello");
    }

    #[cfg(feature = "sqlite")]
    #[tokio::test]
    async fn test_sqlite_expiry() {
        let store = SqliteStore::new_in_memory(0).unwrap();
        let key = blake3::hash(b"test data");
        store
            .put(key, CcrEntry::new("hello".into(), false, Duration::from_secs(0)))
            .await;
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
        let entry = store.get(&key).await;
        assert!(entry.is_none(), "entry should have expired");
    }

    #[cfg(feature = "sqlite")]
    #[tokio::test]
    async fn test_sqlite_exists_with_error() {
        let store = SqliteStore::new_in_memory(60).unwrap();
        let key = blake3::hash(b"failed call");
        store
            .put(key, CcrEntry::new("error".into(), true, Duration::from_secs(60)))
            .await;
        assert_eq!(store.exists_with_error(&key).await, Some(true));
    }

    #[cfg(feature = "sqlite")]
    #[tokio::test]
    async fn test_sqlite_prune() {
        let store = SqliteStore::new_in_memory(0).unwrap();
        let key = blake3::hash(b"expired entry");
        store
            .put(key, CcrEntry::new("gone".into(), false, Duration::from_secs(0)))
            .await;
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
        store.prune().await;
        // After prune, getting should return None
        let entry = store.get(&key).await;
        assert!(entry.is_none());
    }



    #[cfg(feature = "sqlite")]
    #[tokio::test]
    async fn test_sqlite_overwrite() {
        let store = SqliteStore::new_in_memory(60).unwrap();
        let key = blake3::hash(b"same key");

        store
            .put(key, CcrEntry::new("first".into(), false, Duration::from_secs(60)))
            .await;
        store
            .put(key, CcrEntry::new("second".into(), false, Duration::from_secs(60)))
            .await;

        let entry = store.get(&key).await.unwrap();
        assert_eq!(entry.original, "second");
    }

    #[cfg(feature = "sqlite")]
    #[tokio::test]
    async fn test_sqlite_persistence() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_ccr.db");
        let path_str = db_path.to_str().unwrap().to_string();

        let key = blake3::hash(b"persist me");

        // First connection: write
        {
            let store = SqliteStore::new(&path_str, 60).unwrap();
            store
                .put(key, CcrEntry::new("stored".into(), true, Duration::from_secs(60)))
                .await;
        }

        // Second connection: read back — data should persist
        {
            let store = SqliteStore::new(&path_str, 60).unwrap();
            let entry = store.get(&key).await;
            assert!(entry.is_some(), "data should persist across connections");
            let entry = entry.unwrap();
            assert_eq!(entry.original, "stored");
            assert!(entry.was_error);
            assert!(entry.expires_at > Instant::now());
        }
    }
}
