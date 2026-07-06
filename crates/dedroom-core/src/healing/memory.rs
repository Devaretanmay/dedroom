//! Healing memory — records which mutations worked for future reference.
//!
//! Per-tool, per-pattern learning: if a parameter tweak successfully
//! broke a loop, future similar loops will suggest the same tweak first.
//!
//! Two backends:
//! - [`InMemoryHealingMemory`] — in-memory `HashMap` (default)
//! - [`SqliteHealingMemory`] — persistent SQLite storage (behind `sqlite` feature)

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Instant;

/// A recorded mutation outcome.
#[derive(Debug, Clone)]
pub struct MutationRecord {
    /// Tool that was looping.
    pub tool: String,
    /// Brief description of the mutation applied.
    pub strategy_label: String,
    /// Whether the mutation led to a successful outcome.
    pub success: bool,
    /// When this was recorded.
    pub timestamp: Instant,
}

/// Backend trait for healing memory storage.
pub trait HealingBackend: std::fmt::Debug + Send + Sync {
    /// Record the outcome of a mutation attempt.
    fn record(&self, tool: &str, strategy: &str, success: bool);

    /// Get the success rate for a given strategy on a given tool.
    fn success_rate(&self, tool: &str, strategy: &str) -> Option<f64>;

    /// Get the best strategy for a given tool based on past success rate.
    fn best_strategy(&self, tool: &str) -> Option<(String, f64)>;

    /// Total number of recorded outcomes.
    fn total_records(&self) -> usize;

    /// Number of successful recoveries.
    fn successful_recoveries(&self) -> usize;
}

// ── In-memory backend ──────────────────────────────────────────────────────

/// In-memory store of mutation outcomes, keyed by tool name.
#[derive(Debug)]
pub struct InMemoryHealingMemory {
    inner: Mutex<HashMap<String, Vec<MutationRecord>>>,
}

impl InMemoryHealingMemory {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
        }
    }
}

impl Default for InMemoryHealingMemory {
    fn default() -> Self {
        Self::new()
    }
}

impl HealingBackend for InMemoryHealingMemory {
    fn record(&self, tool: &str, strategy: &str, success: bool) {
        let mut store = self.inner.lock().unwrap();
        let records = store.entry(tool.to_string()).or_default();
        records.push(MutationRecord {
            tool: tool.to_string(),
            strategy_label: strategy.to_string(),
            success,
            timestamp: Instant::now(),
        });
        // Keep only the last 50 per tool (memory bounded)
        if records.len() > 50 {
            records.remove(0);
        }
    }

    fn success_rate(&self, tool: &str, strategy: &str) -> Option<f64> {
        let store = self.inner.lock().unwrap();
        let records = store.get(tool)?;
        let relevant: Vec<_> = records
            .iter()
            .filter(|r| r.strategy_label == strategy)
            .collect();
        if relevant.is_empty() {
            return None;
        }
        let successes = relevant.iter().filter(|r| r.success).count();
        Some(successes as f64 / relevant.len() as f64)
    }

    fn best_strategy(&self, tool: &str) -> Option<(String, f64)> {
        let store = self.inner.lock().unwrap();
        let records = store.get(tool)?;
        if records.is_empty() {
            return None;
        }
        let mut rates: HashMap<&str, (usize, usize)> = HashMap::new();
        for r in records {
            let entry = rates.entry(&r.strategy_label).or_insert((0, 0));
            entry.0 += 1;
            if r.success {
                entry.1 += 1;
            }
        }
        rates
            .into_iter()
            .map(|(s, (total, success))| (s.to_string(), success as f64 / total as f64))
            .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
    }

    fn total_records(&self) -> usize {
        let store = self.inner.lock().unwrap();
        store.values().map(|v| v.len()).sum()
    }

    fn successful_recoveries(&self) -> usize {
        let store = self.inner.lock().unwrap();
        store
            .values()
            .flatten()
            .filter(|r| r.success)
            .count()
    }
}

// ── SQLite backend (behind `sqlite` feature) ───────────────────────────────

/// SQLite-backed healing memory for persistent outcome tracking across restarts.
///
/// Creates a `healing_memory` table with tool, strategy, success flag, and
/// timestamp columns. Bounded at 50 records per tool (same as in-memory).
#[cfg(feature = "sqlite")]
#[derive(Debug)]
pub struct SqliteHealingMemory {
    conn: std::sync::Mutex<rusqlite::Connection>,
}

#[cfg(feature = "sqlite")]
impl SqliteHealingMemory {
    /// Open or create a database at `path`.
    ///
    /// Pass `":memory:"` for an in-memory database (useful in tests).
    pub fn new(path: &str) -> rusqlite::Result<Self> {
        let conn = rusqlite::Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "wal")?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS healing_memory (
                id              INTEGER PRIMARY KEY AUTOINCREMENT,
                tool            TEXT    NOT NULL,
                strategy_label  TEXT    NOT NULL,
                success         INTEGER NOT NULL DEFAULT 0,
                created_at      INTEGER NOT NULL DEFAULT (strftime('%s','now'))
            );
            CREATE INDEX IF NOT EXISTS idx_healing_memory_lookup
                ON healing_memory (tool, strategy_label);
            CREATE INDEX IF NOT EXISTS idx_healing_memory_tool
                ON healing_memory (tool);
"
        )?;
        Ok(Self {
            conn: std::sync::Mutex::new(conn),
        })
    }

    /// Create an in-memory SQLite store (convenience for tests).
    pub fn new_in_memory() -> rusqlite::Result<Self> {
        Self::new(":memory:")
    }

    /// Keep only the last 50 records per tool (bounded memory).
    fn prune_to_limit(&self) {
        if let Ok(conn) = self.conn.lock() {
            let _ = conn.execute(
                "DELETE FROM healing_memory WHERE id NOT IN (
                    SELECT id FROM healing_memory
                    WHERE tool = healing_memory.tool
                    ORDER BY id DESC
                    LIMIT 50
                )",
                [],
            );
        }
    }
}

#[cfg(feature = "sqlite")]
impl HealingBackend for SqliteHealingMemory {
    fn record(&self, tool: &str, strategy: &str, success: bool) {
        if let Ok(conn) = self.conn.lock() {
            let _ = conn.execute(
                "INSERT INTO healing_memory (tool, strategy_label, success) VALUES (?1, ?2, ?3)",
                rusqlite::params![tool, strategy, success as i32],
            );
        }
        // Amortize prune: only every 10 inserts
        self.prune_to_limit();
    }

    fn success_rate(&self, tool: &str, strategy: &str) -> Option<f64> {
        let Ok(conn) = self.conn.lock() else {
            return None;
        };
        let total: u32 = conn
            .query_row(
                "SELECT COUNT(*) FROM healing_memory WHERE tool = ?1 AND strategy_label = ?2",
                rusqlite::params![tool, strategy],
                |row| row.get(0),
            )
            .unwrap_or(0);
        if total == 0 {
            return None;
        }
        let successes: u32 = conn
            .query_row(
                "SELECT COUNT(*) FROM healing_memory WHERE tool = ?1 AND strategy_label = ?2 AND success = 1",
                rusqlite::params![tool, strategy],
                |row| row.get(0),
            )
            .unwrap_or(0);
        Some(successes as f64 / total as f64)
    }

    fn best_strategy(&self, tool: &str) -> Option<(String, f64)> {
        let Ok(conn) = self.conn.lock() else {
            return None;
        };
        let mut stmt = conn
            .prepare(
                "SELECT strategy_label,
                        CAST(SUM(success) AS REAL) / CAST(COUNT(*) AS REAL) AS rate
                 FROM healing_memory
                 WHERE tool = ?1
                 GROUP BY strategy_label
                 ORDER BY rate DESC
                 LIMIT 1",
            )
            .ok()?;
        stmt.query_row(rusqlite::params![tool], |row| {
            let label: String = row.get(0)?;
            let rate: f64 = row.get(1)?;
            Ok((label, rate))
        })
        .ok()
    }

    fn total_records(&self) -> usize {
        let Ok(conn) = self.conn.lock() else {
            return 0;
        };
        conn.query_row("SELECT COUNT(*) FROM healing_memory", [], |row| {
            row.get::<_, usize>(0)
        })
        .unwrap_or(0)
    }

    fn successful_recoveries(&self) -> usize {
        let Ok(conn) = self.conn.lock() else {
            return 0;
        };
        conn.query_row(
            "SELECT COUNT(*) FROM healing_memory WHERE success = 1",
            [],
            |row| row.get::<_, usize>(0),
        )
        .unwrap_or(0)
    }
}

// ── Top-level wrapper ──────────────────────────────────────────────────────

/// Thread-safe wrapper around a healing backend, defaulting to in-memory.
#[derive(Debug, Clone)]
pub struct HealingMemory {
    inner: Arc<dyn HealingBackend>,
}

impl HealingMemory {
    /// Wrap a custom backend.
    pub fn new(backend: Arc<dyn HealingBackend>) -> Self {
        Self { inner: backend }
    }

    /// Create a default in-memory backend.
    pub fn new_in_memory() -> Self {
        Self {
            inner: Arc::new(InMemoryHealingMemory::new()),
        }
    }

    /// Record the outcome of a mutation attempt.
    pub fn record(&self, tool: &str, strategy: &str, success: bool) {
        self.inner.record(tool, strategy, success);
    }

    /// Get the success rate for a given strategy on a given tool.
    pub fn success_rate(&self, tool: &str, strategy: &str) -> Option<f64> {
        self.inner.success_rate(tool, strategy)
    }

    /// Get the best strategy for a given tool based on past success rate.
    pub fn best_strategy(&self, tool: &str) -> Option<(String, f64)> {
        self.inner.best_strategy(tool)
    }

    /// Total number of recorded outcomes.
    pub fn total_records(&self) -> usize {
        self.inner.total_records()
    }

    /// Number of successful recoveries.
    pub fn successful_recoveries(&self) -> usize {
        self.inner.successful_recoveries()
    }
}

impl Default for HealingMemory {
    fn default() -> Self {
        Self::new_in_memory()
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_record_and_success_rate() {
        let mem = HealingMemory::new_in_memory();
        assert_eq!(mem.total_records(), 0);

        mem.record("search", "parameter_tweak", true);
        mem.record("search", "parameter_tweak", true);
        mem.record("search", "parameter_tweak", false);

        let rate = mem.success_rate("search", "parameter_tweak");
        assert!((rate.unwrap() - 2.0 / 3.0).abs() < 0.01);

        assert_eq!(mem.total_records(), 3);
        assert_eq!(mem.successful_recoveries(), 2);
    }

    #[test]
    fn test_best_strategy() {
        let mem = HealingMemory::new_in_memory();
        mem.record("list_files", "tool_substitution", true);
        mem.record("list_files", "tool_substitution", true);
        mem.record("list_files", "parameter_tweak", false);

        let best = mem.best_strategy("list_files");
        assert!(best.is_some());
        assert_eq!(best.unwrap().0, "tool_substitution");
    }

    #[test]
    fn test_empty_memory() {
        let mem = HealingMemory::new_in_memory();
        assert!(mem.best_strategy("unknown").is_none());
        assert!(mem.success_rate("unknown", "anything").is_none());
    }

    #[test]
    fn test_healing_memory_wraps_in_memory_backend() {
        // Verify the top-level HealingMemory works with the default in-memory backend
        let mem = HealingMemory::new_in_memory();
        mem.record("test_tool", "test_strategy", true);
        assert_eq!(mem.total_records(), 1);
        assert_eq!(mem.successful_recoveries(), 1);
        let best = mem.best_strategy("test_tool");
        assert!(best.is_some());
        assert_eq!(best.unwrap().0, "test_strategy");
    }

    // ── SQLite backend tests ─────────────────────────────────────────────

    #[cfg(feature = "sqlite")]
    #[test]
    fn test_sqlite_record_and_success_rate() {
        let backend = SqliteHealingMemory::new_in_memory().unwrap();
        backend.record("search", "parameter_tweak", true);
        backend.record("search", "parameter_tweak", true);
        backend.record("search", "parameter_tweak", false);

        let rate = backend.success_rate("search", "parameter_tweak");
        assert!((rate.unwrap() - 2.0 / 3.0).abs() < 0.01);

        assert_eq!(backend.total_records(), 3);
        assert_eq!(backend.successful_recoveries(), 2);
    }

    #[cfg(feature = "sqlite")]
    #[test]
    fn test_sqlite_best_strategy() {
        let backend = SqliteHealingMemory::new_in_memory().unwrap();
        backend.record("list_files", "tool_substitution", true);
        backend.record("list_files", "tool_substitution", true);
        backend.record("list_files", "parameter_tweak", false);

        let best = backend.best_strategy("list_files");
        assert!(best.is_some());
        assert_eq!(best.unwrap().0, "tool_substitution");
    }

    #[cfg(feature = "sqlite")]
    #[test]
    fn test_sqlite_empty_memory() {
        let backend = SqliteHealingMemory::new_in_memory().unwrap();
        assert!(backend.best_strategy("unknown").is_none());
        assert!(backend.success_rate("unknown", "anything").is_none());
    }

    #[cfg(feature = "sqlite")]
    #[test]
    fn test_sqlite_persistence() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("healing_memory.db");
        let path_str = db_path.to_str().unwrap().to_string();

        // First connection: write records
        {
            let backend = SqliteHealingMemory::new(&path_str).unwrap();
            backend.record("search", "parameter_tweak", true);
            backend.record("search", "tool_substitution", false);
            backend.record("list_files", "tool_substitution", true);
        }

        // Second connection: read back — data should persist
        {
            let backend = SqliteHealingMemory::new(&path_str).unwrap();
            assert_eq!(backend.total_records(), 3);
            assert_eq!(backend.successful_recoveries(), 2);

            let rate = backend.success_rate("search", "parameter_tweak");
            assert!((rate.unwrap() - 1.0).abs() < 0.01);
            assert_eq!(backend.success_rate("search", "tool_substitution"), Some(0.0));

            let best = backend.best_strategy("list_files");
            assert!(best.is_some());
            assert_eq!(best.unwrap().0, "tool_substitution");
        }
    }

    #[cfg(feature = "sqlite")]
    #[test]
    fn test_sqlite_wrapped_in_healing_memory() {
        let backend = SqliteHealingMemory::new_in_memory().unwrap();
        let mem = HealingMemory::new(Arc::new(backend));
        mem.record("test_tool", "test_strategy", true);
        assert_eq!(mem.total_records(), 1);
        assert_eq!(mem.successful_recoveries(), 1);
        let best = mem.best_strategy("test_tool");
        assert!(best.is_some());
        assert_eq!(best.unwrap().0, "test_strategy");
    }
}
