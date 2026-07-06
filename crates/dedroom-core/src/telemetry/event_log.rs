//! Non-blocking NDJSON event stream for the proxy.
//!
//! Every request event is written as a single JSON line to `~/.dedroom/events.ndjson`
//! via a background writer thread. Events are also broadcast via `tokio::sync::broadcast`
//! for SSE subscribers. No ring buffer — the file serves as the persistent record.

use serde::Serialize;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::sync::mpsc;
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::broadcast;

/// A single proxy event, written as one NDJSON line.
#[derive(Debug, Clone, Serialize)]
pub struct ProxyEvent {
    pub timestamp: u64,
    pub session_id: Option<String>,
    pub agent_id: Option<String>,
    pub tool_name: String,
    pub args_hash: Option<String>,
    pub verdict: String,
    pub compression_ratio: Option<f64>,
    pub original_tokens: Option<u64>,
    pub compressed_tokens: Option<u64>,
    pub tilt_index: Option<f64>,
    pub latency_us: u64,
}

/// Handle to the background NDJSON event logger with SSE broadcast support.
#[derive(Debug, Clone)]
pub struct EventLog {
    sender: mpsc::SyncSender<ProxyEvent>,
    broadcast: broadcast::Sender<ProxyEvent>,
    log_path: PathBuf,
    total_events: Arc<AtomicU64>,
}

impl EventLog {
    pub fn start() -> Self {
        Self::start_with_path(Self::data_path())
    }

    pub fn start_with_path(path: PathBuf) -> Self {
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
                    eprintln!("[dedroom] Failed to open event log at {}: {e}", writer_path.display());
                    return;
                }
            };
            for event in rx {
                if let Ok(line) = serde_json::to_string(&event)
                    && let Err(e) = writeln!(file, "{line}")
                {
                    eprintln!("[dedroom] Failed to write event: {e}");
                    break;
                }
            }
            let _ = file.flush();
        });

        Self {
            sender: tx,
            broadcast: bcast_tx,
            log_path: path,
            total_events: Arc::new(AtomicU64::new(0)),
        }
    }

    fn data_path() -> PathBuf {
        if let Some(home) = std::env::var_os("HOME") {
            PathBuf::from(home).join(".dedroom").join("events.ndjson")
        } else if let Some(appdata) = std::env::var_os("APPDATA") {
            PathBuf::from(appdata).join("DedrooM").join("events.ndjson")
        } else {
            PathBuf::from("/tmp/dedroom-events.ndjson")
        }
    }

    pub fn record(&self, event: ProxyEvent) {
        let _ = self.sender.try_send(event.clone());
        let _ = self.broadcast.send(event);
        self.total_events.fetch_add(1, Ordering::Relaxed);
    }

    pub fn event_count(&self) -> u64 {
        self.total_events.load(Ordering::Relaxed)
    }

    pub fn subscribe(&self) -> broadcast::Receiver<ProxyEvent> {
        self.broadcast.subscribe()
    }

    pub fn path(&self) -> &Path {
        &self.log_path
    }

    pub fn now_millis() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    }
}

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
    }

    #[test]
    fn test_event_log_writes_to_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.ndjson");
        let log = EventLog::start_with_path(path.clone());
        log.record(ProxyEvent {
            timestamp: 1, session_id: None, agent_id: None,
            tool_name: "search".into(), args_hash: None,
            verdict: "block".into(), compression_ratio: None,
            original_tokens: None, compressed_tokens: None,
            tilt_index: Some(0.9), latency_us: 100,
        });
        std::thread::sleep(std::time::Duration::from_millis(100));
        let file = File::open(&path).unwrap();
        let reader = std::io::BufReader::new(file);
        let lines: Vec<String> = reader.lines().map_while(Result::ok).collect();
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("\"verdict\":\"block\""));
    }

    #[test]
    fn test_multiple_events() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("multi.ndjson");
        let log = EventLog::start_with_path(path.clone());
        for i in 0..5 {
            log.record(ProxyEvent {
                timestamp: i, session_id: None, agent_id: None,
                tool_name: format!("tool_{i}"), args_hash: None,
                verdict: "allow".into(), compression_ratio: None,
                original_tokens: None, compressed_tokens: None,
                tilt_index: Some(0.1 * i as f64), latency_us: i * 10,
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
            timestamp: 42, session_id: Some("test".into()), agent_id: None,
            tool_name: "ssh".into(), args_hash: None, verdict: "allow".into(),
            compression_ratio: None, original_tokens: None, compressed_tokens: None,
            tilt_index: None, latency_us: 0,
        };
        log.record(event.clone());
        let received = tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv())
            .await.expect("should not time out").expect("should not be closed");
        assert_eq!(received.tool_name, "ssh");
        assert_eq!(received.timestamp, 42);
    }

    #[test]
    fn test_event_count() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("count.ndjson");
        let log = EventLog::start_with_path(path);
        assert_eq!(log.event_count(), 0);
        for i in 0..10 {
            log.record(ProxyEvent {
                timestamp: i, session_id: None, agent_id: None,
                tool_name: "counter".into(), args_hash: None,
                verdict: "allow".into(), compression_ratio: None,
                original_tokens: None, compressed_tokens: None,
                tilt_index: None, latency_us: 0,
            });
        }
        assert_eq!(log.event_count(), 10);
    }
}
