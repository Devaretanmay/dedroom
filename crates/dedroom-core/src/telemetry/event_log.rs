//! Non-blocking NDJSON event stream for the proxy.
//!
//! Every request event (loop verdict, compression result) is written as a
//! single JSON line to `~/.dedroom/events.ndjson` via a background writer
//! thread, so the main proxy thread is never blocked on I/O.
//!
//! Events are also broadcast via a `tokio::sync::broadcast` channel so that
//! SSE subscribers (e.g. `/admin/events/stream`) receive events in real-time.
//! An in-memory ring buffer holds the last N events for fast polling access
//! without re-parsing the NDJSON file on every request.
//!
//! # Design
//!
//! - `EventLog` wraps an `mpsc::SyncSender` (file writer) + `broadcast::Sender`
//!   (SSE subscribers) + ring buffer — cheaply `Clone`-able.
//! - The background thread owns the file handle and drains the mpsc channel.
//! - `record()` never blocks the caller (uses `try_send` — drops on overflow).
//! - Channel capacity is 4096 events for the file writer, 256 for broadcast.

use serde::Serialize;
use std::collections::VecDeque;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::broadcast;

/// Default ring-buffer capacity.
const RING_CAPACITY: usize = 1000;

/// A single proxy event, written as one NDJSON line.
#[derive(Debug, Clone, Serialize)]
pub struct ProxyEvent {
    /// Unix millisecond timestamp.
    pub timestamp: u64,
    /// Session identifier (from `x-session-id` header).
    pub session_id: Option<String>,
    /// Agent identifier (from `x-agent-id` header or `User-Agent`).
    pub agent_id: Option<String>,
    /// The tool that was called.
    pub tool_name: String,
    /// BLAKE3 hex hash of `(tool, args)` — deterministic dedup key.
    pub args_hash: Option<String>,
    /// Verdict from the loop detector: `"allow"`, `"block"`, or `"inject"`.
    pub verdict: String,
    /// Compression ratio (`1.0 - compressed/original`), if compression ran.
    pub compression_ratio: Option<f64>,
    /// Token count before compression.
    pub original_tokens: Option<u64>,
    /// Token count after compression.
    pub compressed_tokens: Option<u64>,
    /// How close the tool is to being blocked (`0.0`–`1.0`).
    pub tilt_index: Option<f64>,
    /// Microseconds spent in the pipeline for this tool call.
    pub latency_us: u64,
}

/// Handle to the background NDJSON event logger with SSE broadcast support
/// and an in-memory ring buffer of recent events.
///
/// Clone and cheaply share across request handlers. Drops silently if the
/// background thread has exited (the mpsc channel closes and events are
/// lost from the file, but the broadcast still works for SSE subscribers).
#[derive(Debug, Clone)]
pub struct EventLog {
    sender: mpsc::SyncSender<ProxyEvent>,
    /// Broadcast channel for live SSE subscribers.
    broadcast: broadcast::Sender<ProxyEvent>,
    /// Path to the NDJSON file, stored so the admin endpoint can read it.
    log_path: PathBuf,
    /// In-memory ring buffer of recent events (fast-path for /admin/events).
    ring_buffer: Arc<Mutex<VecDeque<ProxyEvent>>>,
    /// Total events recorded (monotonically increasing, for metadata).
    total_events: Arc<AtomicU64>,
}

impl EventLog {
    /// Start the background logger, writing to `~/.dedroom/events.ndjson`.
    /// Creates the directory and file if they don't exist.
    pub fn start() -> Self {
        Self::start_with_path(Self::data_path())
    }

    /// Start with a specific path (useful for tests or custom dirs).
    pub fn start_with_path(path: PathBuf) -> Self {
        // Ensure the parent directory exists
        let _ = path.parent().map(fs::create_dir_all);

        let (tx, rx) = mpsc::sync_channel::<ProxyEvent>(4096);
        let (bcast_tx, _) = broadcast::channel::<ProxyEvent>(256);
        let writer_path = path.clone();

        thread::spawn(move || {
            let file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&writer_path);

            let mut file: File = match file {
                Ok(f) => f,
                Err(e) => {
                    eprintln!(
                        "[dedroom] Failed to open event log at {}: {e}",
                        writer_path.display()
                    );
                    return;
                }
            };

            // Drain the channel, writing one JSON line per event.
            for event in rx {
                if let Ok(line) = serde_json::to_string(&event)
                    && let Err(e) = writeln!(file, "{line}")
                {
                    eprintln!("[dedroom] Failed to write event: {e}");
                    break;
                }
            }

            // Flush on shutdown.
            let _ = file.flush();
        });

        Self {
            sender: tx,
            broadcast: bcast_tx,
            log_path: path,
            ring_buffer: Arc::new(Mutex::new(VecDeque::with_capacity(RING_CAPACITY + 1))),
            total_events: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Path: `$HOME/.dedroom/events.ndjson` on macOS/Linux,
    /// `%APPDATA%/DedrooM/events.ndjson` on Windows,
    /// fallback to `/tmp/dedroom-events.ndjson`.
    fn data_path() -> PathBuf {
        if let Some(home) = std::env::var_os("HOME") {
            PathBuf::from(home).join(".dedroom").join("events.ndjson")
        } else if let Some(appdata) = std::env::var_os("APPDATA") {
            PathBuf::from(appdata)
                .join("DedrooM")
                .join("events.ndjson")
        } else {
            PathBuf::from("/tmp/dedroom-events.ndjson")
        }
    }

    /// Record an event. Non-blocking — drops the event if the mpsc channel
    /// is full, protecting the proxy thread from back-pressure. Always
    /// broadcasts to SSE subscribers (drops if no subscribers or lagging).
    pub fn record(&self, event: ProxyEvent) {
        // Write to file — non-blocking, drops on overflow
        let _ = self.sender.try_send(event.clone());
        // Broadcast to SSE subscribers — drops if no subscribers or full
        let _ = self.broadcast.send(event.clone());

        // Push into the in-memory ring buffer
        if let Ok(mut buf) = self.ring_buffer.lock() {
            if buf.len() >= RING_CAPACITY {
                buf.pop_front();
            }
            buf.push_back(event);
        }

        self.total_events.fetch_add(1, Ordering::Relaxed);
    }

    /// Return the last `n` events from the in-memory ring buffer (fast path).
    ///
    /// This is O(n) and does no I/O — the ring buffer is a `VecDeque` that
    /// is populated synchronously during `record()`.
    pub fn recent_events(&self, n: usize) -> Vec<ProxyEvent> {
        if let Ok(buf) = self.ring_buffer.lock() {
            let len = buf.len();
            let take = n.min(len);
            buf.iter().rev().take(take).cloned().rev().collect()
        } else {
            Vec::new()
        }
    }

    /// Total number of events recorded since startup.
    pub fn event_count(&self) -> u64 {
        self.total_events.load(Ordering::Relaxed)
    }

    /// Get a receiver for the SSE event stream.
    pub fn subscribe(&self) -> broadcast::Receiver<ProxyEvent> {
        self.broadcast.subscribe()
    }

    /// Path to the NDJSON event log file.
    pub fn path(&self) -> &Path {
        &self.log_path
    }

    /// Current unix time in milliseconds.
    pub fn now_millis() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::BufRead;

    #[test]
    fn test_event_serialization() {
        let event = ProxyEvent {
            timestamp: 1_700_000_000_000,
            session_id: Some("sess-001".into()),
            agent_id: Some("claude-code".into()),
            tool_name: "write_file".into(),
            args_hash: Some("abc123".into()),
            verdict: "allow".into(),
            compression_ratio: Some(0.7),
            original_tokens: Some(1000),
            compressed_tokens: Some(300),
            tilt_index: Some(0.5),
            latency_us: 42,
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"tool_name\":\"write_file\""));
        assert!(json.contains("\"verdict\":\"allow\""));
        assert!(json.contains("\"compression_ratio\":0.7"));
    }

    #[test]
    fn test_event_log_writes_to_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.ndjson");
        let log = EventLog::start_with_path(path.clone());

        log.record(ProxyEvent {
            timestamp: 1,
            session_id: None,
            agent_id: None,
            tool_name: "search".into(),
            args_hash: None,
            verdict: "block".into(),
            compression_ratio: None,
            original_tokens: None,
            compressed_tokens: None,
            tilt_index: Some(0.9),
            latency_us: 100,
        });

        // Give the background thread time to flush
        std::thread::sleep(std::time::Duration::from_millis(100));

        let file = File::open(&path).unwrap();
        let reader = std::io::BufReader::new(file);
        let lines: Vec<String> = reader.lines().map_while(Result::ok).collect();
        assert_eq!(lines.len(), 1, "should have one event line");
        assert!(
            lines[0].contains("\"verdict\":\"block\""),
            "line: {}",
            lines[0]
        );
    }

    #[test]
    fn test_multiple_events() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("multi.ndjson");
        let log = EventLog::start_with_path(path.clone());

        for i in 0..5 {
            log.record(ProxyEvent {
                timestamp: i,
                session_id: None,
                agent_id: None,
                tool_name: format!("tool_{i}"),
                args_hash: None,
                verdict: "allow".into(),
                compression_ratio: None,
                original_tokens: None,
                compressed_tokens: None,
                tilt_index: Some(0.1 * i as f64),
                latency_us: i * 10,
            });
        }

        std::thread::sleep(std::time::Duration::from_millis(100));

        let file = File::open(&path).unwrap();
        let reader = std::io::BufReader::new(file);
        let lines: Vec<String> = reader.lines().map_while(Result::ok).collect();
        assert_eq!(lines.len(), 5);
        assert!(lines[4].contains("\"tool_name\":\"tool_4\""));
    }

    #[test]
    fn test_now_millis() {
        let now = EventLog::now_millis();
        assert!(now > 1_700_000_000_000);
        assert!(now < 2_000_000_000_000);
    }

    #[tokio::test]
    async fn test_broadcast_receives_event() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("broadcast.ndjson");
        let log = EventLog::start_with_path(path);
        let mut rx = log.subscribe();

        let event = ProxyEvent {
            timestamp: 42,
            session_id: Some("test".into()),
            agent_id: None,
            tool_name: "ssh".into(),
            args_hash: None,
            verdict: "allow".into(),
            compression_ratio: None,
            original_tokens: None,
            compressed_tokens: None,
            tilt_index: None,
            latency_us: 0,
        };

        log.record(event.clone());

        let received = tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv())
            .await
            .expect("should not time out")
            .expect("should not be closed");

        assert_eq!(received.tool_name, "ssh");
        assert_eq!(received.timestamp, 42);
    }

    #[test]
    fn test_ring_buffer_returns_recent_events() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ring.ndjson");
        let log = EventLog::start_with_path(path);

        // Record 5 events
        for i in 0..5 {
            log.record(ProxyEvent {
                timestamp: i,
                session_id: None,
                agent_id: None,
                tool_name: format!("tool_{i}"),
                args_hash: None,
                verdict: "allow".into(),
                compression_ratio: None,
                original_tokens: None,
                compressed_tokens: None,
                tilt_index: None,
                latency_us: 0,
            });
        }

        // Should get all 5 back
        let recent = log.recent_events(100);
        assert_eq!(recent.len(), 5, "should return all 5 events");
        assert_eq!(recent[0].tool_name, "tool_0");
        assert_eq!(recent[4].tool_name, "tool_4");
    }

    #[test]
    fn test_ring_buffer_caps_at_capacity() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ring-cap.ndjson");
        let log = EventLog::start_with_path(path);

        // Record way more than capacity
        for i in 0..RING_CAPACITY + 50 {
            log.record(ProxyEvent {
                timestamp: i as u64,
                session_id: None,
                agent_id: None,
                tool_name: "overflow".into(),
                args_hash: None,
                verdict: "allow".into(),
                compression_ratio: None,
                original_tokens: None,
                compressed_tokens: None,
                tilt_index: None,
                latency_us: 0,
            });
        }

        // Should cap at RING_CAPACITY
        let recent = log.recent_events(2000);
        assert_eq!(recent.len(), RING_CAPACITY, "ring buffer should cap at capacity");

        // The oldest events should have been dropped
        assert_eq!(
            recent[0].timestamp, 50,
            "oldest event should be the 51st (0-indexed: 50), since first 50 were evicted"
        );
        assert_eq!(
            recent[RING_CAPACITY - 1].timestamp,
            (RING_CAPACITY + 49) as u64,
            "newest event should be the last recorded"
        );
    }

    #[test]
    fn test_ring_buffer_subset() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ring-subset.ndjson");
        let log = EventLog::start_with_path(path);

        for i in 0..10 {
            log.record(ProxyEvent {
                timestamp: i,
                session_id: None,
                agent_id: None,
                tool_name: "subset".into(),
                args_hash: None,
                verdict: "allow".into(),
                compression_ratio: None,
                original_tokens: None,
                compressed_tokens: None,
                tilt_index: None,
                latency_us: 0,
            });
        }

        // Request only 3
        let recent = log.recent_events(3);
        assert_eq!(recent.len(), 3);
        assert_eq!(recent[0].timestamp, 7);
        assert_eq!(recent[2].timestamp, 9);
    }

    #[test]
    fn test_event_count_tracks_total() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("count.ndjson");
        let log = EventLog::start_with_path(path);

        assert_eq!(log.event_count(), 0);

        for i in 0..10 {
            log.record(ProxyEvent {
                timestamp: i,
                session_id: None,
                agent_id: None,
                tool_name: "counter".into(),
                args_hash: None,
                verdict: "allow".into(),
                compression_ratio: None,
                original_tokens: None,
                compressed_tokens: None,
                tilt_index: None,
                latency_us: 0,
            });
        }

        assert_eq!(log.event_count(), 10, "total events should be 10 even if buffer is full");
    }
}
