//! UI rendering — dispatches tab rendering to component modules.
//!
//! The main entry point is [`render`], called by the terminal draw loop.
//! Each tab has a corresponding render function.

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Margin, Rect},
    style::{Color, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, BorderType, Clear, Paragraph},
};

use std::sync::atomic::{AtomicUsize, Ordering};

use crate::app::{App, Tab};
use crate::components::{
    header, healing_panel, loops_panel, savings_overview, summary_export, timeline, tool_breakdown,
    waste_panel,
};

/// Current zoom level for the timeline (index into ZOOM_OPTIONS).
static ZOOM_LEVEL: AtomicUsize = AtomicUsize::new(1); // default 5m

/// Cycle zoom level (for keyboard shortcuts).
pub fn cycle_zoom() {
    let next = (ZOOM_LEVEL.load(Ordering::Relaxed) + 1) % timeline::ZOOM_OPTIONS.len();
    ZOOM_LEVEL.store(next, Ordering::Relaxed);
}

/// Render the current tab's content into the terminal frame.
pub fn render(frame: &mut Frame, app: &App) {
    // Full screen area
    let area = frame.area();

    // ── Layout: header + content ────────────────────────────────────────
    let vert = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(header::HEADER_HEIGHT),
            Constraint::Min(0),
        ])
        .split(area);

    // Render the header (status bar + tab bar)
    header::render_header(frame, vert[0], app);

    // Render the current tab content
    let content_area = vert[1];
    match app.current_tab {
        Tab::Overview => render_overview_tab(frame, content_area, app),
        Tab::Tools => render_tools_tab(frame, content_area, app),
        Tab::Healing => render_healing_tab(frame, content_area, app),
        Tab::Waste => render_waste_tab(frame, content_area, app),
        Tab::Export => render_export_tab(frame, content_area, app),
    }

    // Overlay help if there's an error
    if let Some(ref error) = app.error {
        render_error_overlay(frame, area, error);
    }
}

// ── Tab renderers ──────────────────────────────────────────────────────────

fn render_overview_tab(frame: &mut Frame, area: Rect, app: &App) {
    let vert = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(savings_overview::OVERVIEW_HEIGHT),
            Constraint::Min(5),
            Constraint::Length(5),
        ])
        .split(area.inner(Margin { horizontal: 1, vertical: 0 }));

    // Top: 4 gauge cards
    savings_overview::render_overview_cards(frame, vert[0], app);

    // Middle: Top tools + Timeline
    let mid = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(vert[1]);

    render_top_tools(frame, mid[0], app);
    timeline::render_timeline(frame, mid[1], app, ZOOM_LEVEL.load(Ordering::Relaxed));

    // Bottom: Loops summary
    loops_panel::render_loop_summary(frame, vert[2], app);
}

fn render_tools_tab(frame: &mut Frame, area: Rect, app: &App) {
    tool_breakdown::render_tool_table(frame, area, app);
}

fn render_healing_tab(frame: &mut Frame, area: Rect, app: &App) {
    healing_panel::render_healing_panel(frame, area, app);
}

fn render_waste_tab(frame: &mut Frame, area: Rect, app: &App) {
    waste_panel::render_waste_panel(frame, area, app);
}

fn render_export_tab(frame: &mut Frame, area: Rect, app: &App) {
    summary_export::render_export_panel(frame, area, app);
}

// ── Helper: top tools panel (used in Overview tab) ─────────────────────────

fn render_top_tools(frame: &mut Frame, area: Rect, app: &App) {
    use ratatui::widgets::Table;

    let tools = app.top_tools(8);

    let widths = [
        Constraint::Length(20),
        Constraint::Length(12),
        Constraint::Length(10),
    ];

    let header = ratatui::widgets::Row::new(vec![
        ratatui::widgets::Cell::from(Span::styled(" Tool", Style::default().fg(Color::White).bold())),
        ratatui::widgets::Cell::from(Span::styled(" Saved", Style::default().fg(Color::White).bold())),
        ratatui::widgets::Cell::from(Span::styled(" Ratio", Style::default().fg(Color::White).bold())),
    ])
    .style(Style::default().bg(Color::DarkGray));

    let rows: Vec<ratatui::widgets::Row> = tools
        .iter()
        .map(|t| {
            let saved = if t.tokens_saved >= 1_000_000 {
                format!("{:.1}M", t.tokens_saved as f64 / 1_000_000.0)
            } else if t.tokens_saved >= 1_000 {
                format!("{:.1}K", t.tokens_saved as f64 / 1_000.0)
            } else {
                format!("{}", t.tokens_saved)
            };
            let ratio = t
                .compression_ratio
                .map(|r| format!("{:.0}%", r * 100.0))
                .unwrap_or_else(|| "--".to_string());

            ratatui::widgets::Row::new(vec![
                ratatui::widgets::Cell::from(Span::styled(format!(" {}", t.tool), Style::default().fg(Color::Cyan))),
                ratatui::widgets::Cell::from(Span::styled(saved, Style::default().fg(Color::Green).bold())),
                ratatui::widgets::Cell::from(Span::styled(ratio, Style::default().fg(Color::Magenta))),
            ])
            .height(1)
        })
        .collect();

    let table = Table::new(rows, widths)
        .header(header)
        .block(
            Block::bordered()
                .title(" Top Tools by Savings ")
                .border_type(BorderType::Rounded)
                .border_style(Color::Cyan),
        )
        .column_spacing(1);

    frame.render_widget(table, area);
}

// ── Help Overlay ───────────────────────────────────────────────────────────

/// Show a help overlay when the user presses `?`.
pub fn render_help_overlay(frame: &mut Frame, area: Rect) {
    let overlay = Block::bordered()
        .title(" Help ")
        .border_type(BorderType::Double)
        .border_style(Color::Cyan)
        .style(Style::default().bg(Color::Black));

    let help_text = vec![
        Line::from(Span::styled("Keyboard Shortcuts", Style::default().fg(Color::White).bold())),
        Line::from(Span::raw("")),
        Line::from(Span::styled("  Q / Esc    Quit dashboard", Style::default().fg(Color::White))),
        Line::from(Span::styled("  Tab        Next tab", Style::default().fg(Color::White))),
        Line::from(Span::styled("  Shift+Tab  Previous tab", Style::default().fg(Color::White))),
        Line::from(Span::styled("  1-5        Jump to tab", Style::default().fg(Color::White))),
        Line::from(Span::styled("  R          Force refresh data", Style::default().fg(Color::White))),
        Line::from(Span::styled("  ?          Toggle this help overlay", Style::default().fg(Color::White))),
        Line::from(Span::styled("  Z          Cycle timeline zoom", Style::default().fg(Color::White))),
        Line::from(Span::styled("  E          Export weekly summary", Style::default().fg(Color::White))),
        Line::from(Span::raw("")),
        Line::from(Span::styled("Tabs", Style::default().fg(Color::White).bold())),
        Line::from(Span::raw("")),
        Line::from(Span::styled("  1 Overview    — Top-level savings, loops, ratio", Style::default().fg(Color::DarkGray))),
        Line::from(Span::styled("  2 Tools       — Per-tool breakdown table", Style::default().fg(Color::DarkGray))),
        Line::from(Span::styled("  3 Healing     — Self-healing strategies & outcomes", Style::default().fg(Color::DarkGray))),
        Line::from(Span::styled("  4 Waste       — Token waste analysis", Style::default().fg(Color::DarkGray))),
        Line::from(Span::styled("  5 Export      — Export controls & summary preview", Style::default().fg(Color::DarkGray))),
        Line::from(Span::raw("")),
        Line::from(Span::styled("  Press any key to close", Style::default().fg(Color::DarkGray))),
    ];

    // Center the help overlay
    let help_area = centered_rect(60, 18, area);
    frame.render_widget(Clear, help_area);
    let para = Paragraph::new(help_text).block(overlay);
    frame.render_widget(para, help_area);
}

fn render_error_overlay(frame: &mut Frame, area: Rect, error: &str) {
    let overlay = Block::bordered()
        .title(" ⚠ Error ")
        .border_type(BorderType::Double)
        .border_style(Color::Red)
        .style(Style::default().bg(Color::Black));

    let text = vec![
        Line::from(Span::styled(error, Style::default().fg(Color::Red))),
        Line::from(Span::raw("")),
        Line::from(Span::styled("Check that the proxy is running:", Style::default().fg(Color::White))),
        Line::from(Span::styled("  dedroom proxy", Style::default().fg(Color::Cyan))),
    ];

    let err_area = centered_rect(50, 6, area);
    frame.render_widget(Clear, err_area);
    let para = Paragraph::new(text).block(overlay);
    frame.render_widget(para, err_area);
}

/// Create a centered rect within the given area.
fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let horiz = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Fill(1),
            Constraint::Length(width.min(area.width)),
            Constraint::Fill(1),
        ])
        .split(area);
    let vert = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Fill(1),
            Constraint::Length(height.min(area.height)),
            Constraint::Fill(1),
        ])
        .split(horiz[1]);
    vert[1]
}
