//! Session timeline — zoomable view of events over time.
//!
//! Shows a unicode dot chart of events (allowed vs blocked) over time,
//! with zoom controls to adjust the time window.

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Margin, Rect},
    style::{Color, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, BorderType, Paragraph},
};

use crate::app::App;
use crate::api::ProxyEventItem;

/// Render the session timeline.
pub fn render_timeline(frame: &mut Frame, area: Rect, app: &App, zoom_level: usize) {
    let vert = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(3), Constraint::Length(3)])
        .split(area.inner(Margin { horizontal: 1, vertical: 1 }));

    // Zoom controls
    render_zoom_controls(frame, vert[0], zoom_level);
    // Timeline dots
    render_timeline_dots(frame, vert[1], app, zoom_level);
    // Legend and latest
    render_timeline_info(frame, vert[2], app);
}

/// Available zoom windows (in seconds).
pub const ZOOM_OPTIONS: &[u64] = &[60, 300, 900, 3600];
pub const ZOOM_LABELS: &[&str] = &["1m", "5m", "15m", "1h"];

fn render_zoom_controls(frame: &mut Frame, area: Rect, zoom_level: usize) {
    let mut spans = vec![Span::styled("Zoom: ", Style::default().fg(Color::DarkGray).bold())];

    for (i, label) in ZOOM_LABELS.iter().enumerate() {
        if i == zoom_level {
            spans.push(Span::styled(
                format!(" [{label}] "),
                Style::default().fg(Color::Black).bg(Color::Cyan).bold(),
            ));
        } else {
            spans.push(Span::styled(
                format!(" [{label}] "),
                Style::default().fg(Color::DarkGray),
            ));
        }
    }

    let para = Paragraph::new(Line::from(spans));
    frame.render_widget(para, area);
}

fn render_timeline_dots(frame: &mut Frame, area: Rect, app: &App, zoom_level: usize) {
    let window_secs = ZOOM_OPTIONS.get(zoom_level).copied().unwrap_or(300);
    let events = app.events_in_window(window_secs);

    // Build a compact dot chart: each block character represents 1+ events
    let max_dots = area.width as usize;
    let dot_chart = build_dot_chart(&events, max_dots);

    let block = Block::bordered()
        .title(" Session Timeline ")
        .border_type(BorderType::Rounded)
        .border_style(Color::Cyan);

    let para = Paragraph::new(Line::from(
        Span::styled(dot_chart, Style::default().fg(Color::Cyan)),
    ))
    .block(block);

    frame.render_widget(para, area);
}

/// Build a compact dot chart string.
///
/// Divides the time window into `max_dots` buckets. Each bucket shows:
/// - `●` (green) if most events were allowed
/// - `○` (yellow) if warnings
/// - `×` (red) if most were blocked
fn build_dot_chart(events: &[&ProxyEventItem], max_dots: usize) -> String {
    if events.is_empty() || max_dots == 0 {
        return String::new();
    }

    // Group events into buckets
    if events.len() <= max_dots {
        // Enough space — show each event individually
        let mut chars = Vec::with_capacity(events.len());
        for event in events {
            match event.verdict.as_str() {
                "block" => chars.push('×'),
                "warn" => chars.push('○'),
                _ => chars.push('●'),
            }
        }
        return chars.into_iter().collect();
    }

    // Downsample: group events into max_dots buckets
    let bucket_size = events.len() / max_dots;
    let mut result = String::with_capacity(max_dots);

    for i in 0..max_dots {
        let start = i * bucket_size;
        let end = if i == max_dots - 1 {
            events.len()
        } else {
            start + bucket_size
        };
        let bucket = &events[start..end];

        let blocked_count = bucket.iter().filter(|e| e.verdict == "block").count();
        let allowed_count = bucket.iter().filter(|e| e.verdict == "allow").count();

        let ch = if blocked_count > allowed_count {
            '×'
        } else if blocked_count > 0 {
            '○'
        } else {
            '●'
        };
        result.push(ch);
    }

    result
}

fn render_timeline_info(frame: &mut Frame, area: Rect, app: &App) {
    let latest = app.recent_events(1).first().cloned();
    let blocked_in_window = app.events_in_window(ZOOM_OPTIONS[0])
        .iter()
        .filter(|e| e.verdict == "block")
        .count();

    let allowed_in_window = app.events_in_window(ZOOM_OPTIONS[0])
        .iter()
        .filter(|e| e.verdict == "allow")
        .count();

    let mut spans = vec![
        Span::styled("Legend: ", Style::default().fg(Color::DarkGray)),
        Span::styled("● allow ", Style::default().fg(Color::Green)),
        Span::styled("○ warn ", Style::default().fg(Color::Yellow)),
        Span::styled("× block ", Style::default().fg(Color::Red)),
    ];

    if let Some(event) = latest {
        spans.push(Span::styled(
            format!(" │ Latest: {} ", event.tool_name),
            Style::default().fg(Color::White).bold(),
        ));
        match event.verdict.as_str() {
            "block" => {
                spans.push(Span::styled("BLOCKED", Style::default().fg(Color::Red).bold()));
            }
            _ => {
                spans.push(Span::styled("allowed", Style::default().fg(Color::Green)));
            }
        }
    }

    spans.push(Span::styled(
        format!(" │ 1m: {allowed_in_window} allow, {blocked_in_window} block"),
        Style::default().fg(Color::DarkGray),
    ));

    let para = Paragraph::new(Line::from(spans));
    frame.render_widget(para, area);
}
