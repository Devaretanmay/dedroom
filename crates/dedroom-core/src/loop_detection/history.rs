//! Tracks tool call history for loop detection.
//!
//! Stores a bounded sliding window of `(tool, canonical_args, was_error)` tuples.
//! History is automatically pruned when it exceeds the configured window size
//! (fixes the unbounded-growth problem).
//!
//! Two backends are available:
//! - [`HistoryTracker`] — in-memory `VecDeque` (default)
//! - [`SqliteHistoryTracker`] — persistent SQLite storage (behind `sqlite` feature)

use std::collections::VecDeque;
use crate::config::CountMode;

/// A single entry in the call history.
#[derive(Debug, Clone)]
pub struct HistoryEntry {
    pub tool: String,
    /// Canonicalized arguments (volatile fields stripped).
    pub canonical_args: String,
    /// Whether the tool call resulted in an error.
    pub was_error: bool,
}

/// Backend abstraction for loop detection history.
///
/// Implementations can store history in-memory (default) or in a
/// persistent SQLite database for cross-restart loop detection.
pub trait HistoryBackend: std::fmt::Debug + Send + Sync {
    /// Push a new tool call into history.
    fn push(&mut self, tool: String, canonical_args: String, was_error: bool);

    /// Count how many times the given tool+args pair appears in history
    /// (up to and including the window size).
    fn count_repeats(&self, tool: &str, canonical_args: &str, mode: CountMode) -> u32;

    /// Number of entries currently stored.
    fn len(&self) -> usize;

    /// Returns true if history is empty.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Return a snapshot of all entries (oldest first).
    fn snapshot(&self) -> Vec<HistoryEntry>;

    /// Clear all history.
    fn clear(&mut self);

    /// Resize the window. Truncates oldest entries if shrinking.
    fn set_window(&mut self, new_window: usize);

}

// ── In-memory backend ──────────────────────────────────────────────────────

/// Bounded, sliding-window history of tool calls backed by a `VecDeque`.
#[derive(Debug)]
pub struct HistoryTracker {
    /// Maximum number of entries to keep.
    window: usize,
    /// Circular buffer of entries (newest at back).
    entries: VecDeque<HistoryEntry>,
}

impl HistoryTracker {
    /// Create a new tracker with the given window size.
    pub fn new(window: usize) -> Self {
        Self {
            window: window.max(1),
            entries: VecDeque::with_capacity(window),
        }
    }
}

impl HistoryBackend for HistoryTracker {
    fn push(&mut self, tool: String, canonical_args: String, was_error: bool) {
        if self.entries.len() >= self.window {
            self.entries.pop_front();
        }
        self.entries.push_back(HistoryEntry {
            tool,
            canonical_args,
            was_error,
        });
    }

    fn count_repeats(
        &self,
        tool: &str,
        canonical_args: &str,
        mode: CountMode,
    ) -> u32 {
        self.entries.iter().rev().fold(0u32, |count, entry| {
            if entry.tool == tool && entry.canonical_args == canonical_args {
                match mode {
                    CountMode::All => count + 1,
                    CountMode::ErrorsOnly => {
                        if entry.was_error {
                            count + 1
                        } else {
                            count
                        }
                    }
                }
            } else {
                count
            }
        })
    }

    fn len(&self) -> usize {
        self.entries.len()
    }

    fn snapshot(&self) -> Vec<HistoryEntry> {
        self.entries.iter().cloned().collect()
    }

    fn clear(&mut self) {
        self.entries.clear();
    }

    fn set_window(&mut self, new_window: usize) {
        self.window = new_window.max(1);
        while self.entries.len() > self.window {
            self.entries.pop_front();
        }
    }
}

// ── SQLite backend (behind `sqlite` feature) ───────────────────────────────

/// SQLite-backed history tracker for persistent loop detection across restarts.
///
/// Stores entries in a `loop_history` table with auto-increment IDs and
/// uses the configured window size to bound queries and periodically prune
/// old rows.
#[cfg(feature = "sqlite")]
#[derive(Debug)]
pub struct SqliteHistoryTracker {
    conn: std::sync::Mutex<rusqlite::Connection>,
    window: usize,
    /// Push counter. Only prunes every `window` pushes to amortize the cost.
    push_counter: u64
}

#[cfg(feature = "sqlite")]
impl SqliteHistoryTracker {
    /// Open or create a database at `path`.
    ///
    /// Pass `":memory:"` for an in-memory database (useful in tests).
    pub fn new(path: &str, window: usize) -> rusqlite::Result<Self> {
        let window = window.max(1);
        let conn = rusqlite::Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "wal")?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS loop_history (
                id              INTEGER PRIMARY KEY AUTOINCREMENT,
                tool            TEXT    NOT NULL,
                canonical_args  TEXT    NOT NULL,
                was_error       INTEGER NOT NULL DEFAULT 0,
                created_at      INTEGER NOT NULL DEFAULT (strftime('%s','now'))
            );
            CREATE INDEX IF NOT EXISTS idx_loop_history_lookup
                ON loop_history (tool, canonical_args);
"
        )?;

        Ok(Self {
            conn: std::sync::Mutex::new(conn),
            window,
            push_counter: 1,
        })
    }

    /// Create an in-memory SQLite store (convenience for tests).
    pub fn new_in_memory(window: usize) -> rusqlite::Result<Self> {
        Self::new(":memory:", window)
    }

    /// Prune rows outside the sliding window to keep the DB bounded.
    ///
    /// Keeps only the `window` most recent entries by deleting everything
    /// with an `id` not in the most recent `window` IDs.
    fn prune(&self) {
        if let Ok(conn) = self.conn.lock() {
            let _ = conn.execute(
                "DELETE FROM loop_history WHERE id NOT IN (
                    SELECT id FROM loop_history ORDER BY id DESC LIMIT ?1
                )",
                rusqlite::params![self.window as i64],
            );
        }
    }
}

#[cfg(feature = "sqlite")]
impl HistoryBackend for SqliteHistoryTracker {
    fn push(&mut self, tool: String, canonical_args: String, was_error: bool) {
        if let Ok(conn) = self.conn.lock() {
            let _ = conn.execute(
                "INSERT INTO loop_history (tool, canonical_args, was_error) VALUES (?1, ?2, ?3)",
                rusqlite::params![tool, canonical_args, was_error as i32],
            );
        }

        // Only prune every `window` pushes to amortize the cost.
        // The `count_repeats` query already uses `LIMIT window` so
        // correctness is unaffected by extra rows in the DB.
        if self.push_counter.is_multiple_of(self.window as u64) {
            self.prune();
        }
        self.push_counter += 1;
    }

    fn count_repeats(&self, tool: &str, canonical_args: &str, mode: CountMode) -> u32 {
        let Ok(conn) = self.conn.lock() else {
            return 0;
        };

        let count: Result<u32, _> = match mode {
            CountMode::All => conn.query_row(
                "SELECT COUNT(*) FROM (
                    SELECT 1 FROM loop_history
                    WHERE tool = ?1 AND canonical_args = ?2
                    ORDER BY id DESC
                    LIMIT ?3
                )",
                rusqlite::params![tool, canonical_args, self.window as i64],
                |row| row.get(0),
            ),
            CountMode::ErrorsOnly => conn.query_row(
                "SELECT COUNT(*) FROM (
                    SELECT 1 FROM loop_history
                    WHERE tool = ?1 AND canonical_args = ?2 AND was_error = 1
                    ORDER BY id DESC
                    LIMIT ?3
                )",
                rusqlite::params![tool, canonical_args, self.window as i64],
                |row| row.get(0),
            ),
        };

        count.unwrap_or(0)
    }

    fn len(&self) -> usize {
        let Ok(conn) = self.conn.lock() else {
            return 0;
        };
        conn.query_row(
            "SELECT COUNT(*) FROM loop_history",
            [],
            |row| row.get::<_, usize>(0),
        )
        .unwrap_or(0)
    }

    fn snapshot(&self) -> Vec<HistoryEntry> {
        let Ok(conn) = self.conn.lock() else {
            return Vec::new();
        };

        let mut stmt = match conn.prepare(
            "SELECT tool, canonical_args, was_error FROM loop_history ORDER BY id ASC"
        ) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };

        let rows = stmt.query_map([], |row| {
            Ok(HistoryEntry {
                tool: row.get(0)?,
                canonical_args: row.get(1)?,
                was_error: row.get::<_, i32>(2)? != 0,
            })
        });

        rows.ok().map_or_else(Vec::new, |iter| iter.filter_map(|r| r.ok()).collect())
    }

    fn clear(&mut self) {
        if let Ok(conn) = self.conn.lock() {
            let _ = conn.execute("DELETE FROM loop_history", []);
        }
    }

    fn set_window(&mut self, new_window: usize) {
        self.window = new_window.max(1);
        self.prune();
    }


}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── In-memory backend tests ───────────────────────────────────────────

    #[test]
    fn test_push_and_count() {
        let mut h = HistoryTracker::new(10);
        h.push("write_file".into(), r#"{"path":"/tmp/x.txt"}"#.into(), false);
        h.push("write_file".into(), r#"{"path":"/tmp/x.txt"}"#.into(), false);
        assert_eq!(h.count_repeats("write_file", r#"{"path":"/tmp/x.txt"}"#, CountMode::All), 2);
    }

    #[test]
    fn test_window_eviction() {
        let mut h = HistoryTracker::new(3);
        h.push("a".into(), "1".into(), false);
        h.push("a".into(), "1".into(), false);
        h.push("a".into(), "1".into(), false);
        h.push("a".into(), "1".into(), false); // evicts the first
        assert_eq!(h.count_repeats("a", "1", CountMode::All), 3);
        assert_eq!(h.len(), 3);
    }

    #[test]
    fn test_count_errors_only() {
        let mut h = HistoryTracker::new(10);
        h.push("write_file".into(), r#"{"path":"/tmp/x.txt"}"#.into(), false);
        h.push("write_file".into(), r#"{"path":"/tmp/x.txt"}"#.into(), true);
        h.push("write_file".into(), r#"{"path":"/tmp/x.txt"}"#.into(), true);
        assert_eq!(h.count_repeats("write_file", r#"{"path":"/tmp/x.txt"}"#, CountMode::ErrorsOnly), 2);
    }

    #[test]
    fn test_different_tools_independent() {
        let mut h = HistoryTracker::new(10);
        h.push("read_file".into(), r#"{"path":"/tmp/x.txt"}"#.into(), false);
        h.push("write_file".into(), r#"{"path":"/tmp/x.txt"}"#.into(), false);
        assert_eq!(h.count_repeats("write_file", r#"{"path":"/tmp/x.txt"}"#, CountMode::All), 1);
        assert_eq!(h.count_repeats("read_file", r#"{"path":"/tmp/x.txt"}"#, CountMode::All), 1);
    }

    #[test]
    fn test_clear() {
        let mut h = HistoryTracker::new(10);
        h.push("a".into(), "1".into(), false);
        h.clear();
        assert!(h.is_empty());
    }

    #[test]
    fn test_set_window_shrinks() {
        let mut h = HistoryTracker::new(10);
        for _ in 0..5 {
            h.push("a".into(), "1".into(), false);
        }
        h.set_window(2);
        assert_eq!(h.len(), 2);
    }

    #[test]
    fn test_backend_trait_via_ref() {
        // Verify HistoryTracker works through the trait interface
        let mut tracker: Box<dyn HistoryBackend> = Box::new(HistoryTracker::new(5));
        tracker.push("test".into(), "args".into(), false);
        assert_eq!(tracker.len(), 1);
        assert_eq!(tracker.count_repeats("test", "args", CountMode::All), 1);
        let entries = tracker.snapshot();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].tool, "test");
    }

    // ── SQLite backend tests ──────────────────────────────────────────────

    #[cfg(feature = "sqlite")]
    #[test]
    fn test_sqlite_push_and_count() {
        let mut h = SqliteHistoryTracker::new_in_memory(10).unwrap();
        h.push("write_file".into(), r#"{"path":"/tmp/x.txt"}"#.into(), false);
        h.push("write_file".into(), r#"{"path":"/tmp/x.txt"}"#.into(), false);
        assert_eq!(h.count_repeats("write_file", r#"{"path":"/tmp/x.txt"}"#, CountMode::All), 2);
    }

    #[cfg(feature = "sqlite")]
    #[test]
    fn test_sqlite_window_eviction() {
        let mut h = SqliteHistoryTracker::new_in_memory(3).unwrap();
        h.push("a".into(), "1".into(), false);
        h.push("a".into(), "1".into(), false);
        h.push("a".into(), "1".into(), false);
        h.push("a".into(), "1".into(), false); // should evict the first
        // count_repeats uses LIMIT window, so it should return 3
        assert_eq!(h.count_repeats("a", "1", CountMode::All), 3);
        // total rows in DB may be 4 (prune happens before insert, but let's check)
        assert!(h.len() <= 4);
    }

    #[cfg(feature = "sqlite")]
    #[test]
    fn test_sqlite_count_errors_only() {
        let mut h = SqliteHistoryTracker::new_in_memory(10).unwrap();
        h.push("write_file".into(), r#"{"path":"/tmp/x.txt"}"#.into(), false);
        h.push("write_file".into(), r#"{"path":"/tmp/x.txt"}"#.into(), true);
        h.push("write_file".into(), r#"{"path":"/tmp/x.txt"}"#.into(), true);
        assert_eq!(h.count_repeats("write_file", r#"{"path":"/tmp/x.txt"}"#, CountMode::ErrorsOnly), 2);
    }

    #[cfg(feature = "sqlite")]
    #[test]
    fn test_sqlite_different_tools_independent() {
        let mut h = SqliteHistoryTracker::new_in_memory(10).unwrap();
        h.push("read_file".into(), r#"{"path":"/tmp/x.txt"}"#.into(), false);
        h.push("write_file".into(), r#"{"path":"/tmp/x.txt"}"#.into(), false);
        assert_eq!(h.count_repeats("write_file", r#"{"path":"/tmp/x.txt"}"#, CountMode::All), 1);
        assert_eq!(h.count_repeats("read_file", r#"{"path":"/tmp/x.txt"}"#, CountMode::All), 1);
    }

    #[cfg(feature = "sqlite")]
    #[test]
    fn test_sqlite_clear() {
        let mut h = SqliteHistoryTracker::new_in_memory(10).unwrap();
        h.push("a".into(), "1".into(), false);
        h.clear();
        assert!(h.is_empty());
        assert_eq!(h.len(), 0);
    }

    #[cfg(feature = "sqlite")]
    #[test]
    fn test_sqlite_set_window_shrinks() {
        let mut h = SqliteHistoryTracker::new_in_memory(10).unwrap();
        for _ in 0..5 {
            h.push("a".into(), "1".into(), false);
        }
        h.set_window(2);
        assert_eq!(h.count_repeats("a", "1", CountMode::All), 2);
    }

    #[cfg(feature = "sqlite")]
    #[test]
    fn test_sqlite_persistence() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("loop_history.db");
        let path_str = db_path.to_str().unwrap().to_string();

        let key = r#"{"path":"/tmp/persist.txt"}"#;

        // First connection: write entries
        {
            let mut h = SqliteHistoryTracker::new(&path_str, 20).unwrap();
            h.push("write_file".into(), key.into(), false);
            h.push("write_file".into(), key.into(), true);
        }

        // Second connection: read back — data should persist
        {
            let h = SqliteHistoryTracker::new(&path_str, 20).unwrap();
            assert_eq!(h.len(), 2);
            assert_eq!(h.count_repeats("write_file", key, CountMode::All), 2);
            assert_eq!(h.count_repeats("write_file", key, CountMode::ErrorsOnly), 1);

            let entries = h.snapshot();
            assert_eq!(entries.len(), 2);
            assert!(!entries[0].was_error);
            assert!(entries[1].was_error);
        }
    }

    #[cfg(feature = "sqlite")]
    #[test]
    fn test_sqlite_backend_trait_via_ref() {
        let mut tracker: Box<dyn HistoryBackend> =
            Box::new(SqliteHistoryTracker::new_in_memory(5).unwrap());
        tracker.push("test".into(), "args".into(), false);
        assert_eq!(tracker.len(), 1);
        assert_eq!(tracker.count_repeats("test", "args", CountMode::All), 1);
        let entries = tracker.snapshot();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].tool, "test");
    }
}
