//! Top header bar — live status, uptime, event count, tab bar, and controls.

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Paragraph, Tabs},
};

use crate::app::{App, Tab};

/// Height of the header bar in terminal rows.
pub const HEADER_HEIGHT: u16 = 3;

/// Render the dashboard header, including status bar and tab bar.
pub fn render_header(frame: &mut Frame, area: Rect, app: &App) {
    let vert = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(2)])
        .split(area);

    // ── Status bar ──────────────────────────────────────────────────────
    render_status_bar(frame, vert[0], app);
    // ── Tab bar ─────────────────────────────────────────────────────────
    render_tab_bar(frame, vert[1], app);
}

fn render_status_bar(frame: &mut Frame, area: Rect, app: &App) {
    let connected = app.is_connected;

    let status_dot = if connected {
        Span::styled(" ● ", Style::default().fg(Color::Green).bold())
    } else {
        Span::styled(" ● ", Style::default().fg(Color::Red).bold())
    };

    let status_label = if connected {
        Span::styled("Live", Style::default().fg(Color::Green))
    } else {
        Span::styled("Disconnected", Style::default().fg(Color::Red))
    };

    let uptime = Span::styled(
        format!(" │ Up: {}", app.uptime_str()),
        Style::default().fg(Color::DarkGray),
    );

    let events = Span::styled(
        format!(" │ Events: {}", app.event_count()),
        Style::default().fg(Color::Cyan),
    );

    let title = Span::styled(
        " 🛡 DEDROOM ",
        Style::default().fg(Color::White).bold(),
    );

    // Right side: SSE activity indicator
    let sse = if app.sse_activity {
        Span::styled(" ◉ LIVE ", Style::default().fg(Color::Green).bold())
    } else {
        Span::styled("     ", Style::default())
    };

    // Key hints
    let hints = Span::styled(
        " │ [Tab] Cycle  [1-5] Jump  [R]efresh  [?] Help  [Q]uit",
        Style::default().fg(Color::DarkGray),
    );

    let content = Paragraph::new(Line::from(vec![
        title, status_dot, status_label, uptime, events, hints, sse,
    ]))
    .style(Style::default().bg(Color::Black));

    frame.render_widget(content, area);
}

fn render_tab_bar(frame: &mut Frame, area: Rect, app: &App) {
    // Build tab labels with active/inactive styling
    let tab_labels: Vec<Line> = Tab::ALL
        .iter()
        .map(|tab| {
            let is_active = *tab == app.current_tab;
            let label = format!(" {} {} ", tab.icon(), tab.label());

            if is_active {
                Line::from(Span::styled(
                    label,
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Cyan)
                        .bold(),
                ))
            } else {
                Line::from(Span::styled(
                    label,
                    Style::default().fg(Color::White).bg(Color::DarkGray),
                ))
            }
        })
        .collect();

    let tabs = Tabs::new(tab_labels)
        .block(
            Block::bordered()
                .style(Style::default().bg(Color::Black))
                .border_style(Style::default().fg(Color::DarkGray)),
        )
        .highlight_style(Style::default().fg(Color::Cyan));

    frame.render_widget(tabs, area);
}
