//! SSE event stream listener — receives live proxy events.
//!
//! Connects to `GET /admin/events/stream` and forwards deserialized
//! events to the [`App`](crate::app::App) state via a channel.
//!
//! TODO: Implement SSE background task that spawns on dashboard launch,
//! reads from the `api::DashboardApi::event_stream()`, and pushes
//! events into the app state.

use crate::api;
use crate::app::App;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Start listening to the SSE event stream in a background task.
///
/// Events are pushed into the app's event timeline as they arrive.
pub async fn start_event_listener(
    app: Arc<Mutex<App>>,
    port: u16,
) {
    let api = api::DashboardApi::new(port);

    tokio::spawn(async move {
        loop {
            match api.event_stream().await {
                Ok(stream) => {
                    use futures::StreamExt;
                    let mut pinned = Box::pin(stream);
                    while let Some(event) = pinned.next().await {
                        let mut app = app.lock().await;
                        app.push_event(event);
                    }
                }
                Err(e) => {
                    eprintln!("[dedroom-dash] SSE connection failed: {e}");
                }
            }
            // Retry after 5 seconds
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        }
    });
}
