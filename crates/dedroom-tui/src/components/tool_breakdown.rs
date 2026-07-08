//! Tool breakdown table — per-tool metrics with sortable columns.
//!
//! Shows each tool's call count, tokens saved, compression ratio,
//! blocked calls, and error count in a sortable table format.

use ratatui::{
    Frame,
    layout::{Constraint, Rect},
    style::{Color, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, BorderType, Cell, Paragraph, Row, Table},
};

use crate::app::App;

/// Render the per-tool table.
pub fn render_tool_table(frame: &mut Frame, area: Rect, app: &App) {
    let tools = app.top_tools(100); // Get all tools sorted by savings

    if tools.is_empty() {
        let block = Block::bordered()
            .title(" Tool Breakdown ")
            .border_type(BorderType::Rounded)
            .border_style(Color::Cyan);
        let para = Paragraph::new(Line::from(Span::styled(
            "No tool data yet. Route an agent through the proxy to collect metrics.",
            Style::default().fg(Color::DarkGray),
        )))
        .block(block);
        frame.render_widget(para, area);
        return;
    }

    // Column widths
    let widths = [
        Constraint::Length(22),  // Tool name
        Constraint::Length(10),  // Calls
        Constraint::Length(14),  // Tokens saved
        Constraint::Length(12),  // Tokens processed
        Constraint::Length(10),  // Compression
        Constraint::Length(10),  // Blocked
        Constraint::Length(10),  // Errors
    ];

    // Header row
    let header = Row::new(vec![
        Cell::from(Span::styled(" Tool", Style::default().fg(Color::White).bold())),
        Cell::from(Span::styled("Calls", Style::default().fg(Color::White).bold())),
        Cell::from(Span::styled("Tokens Saved", Style::default().fg(Color::White).bold())),
        Cell::from(Span::styled("Processed", Style::default().fg(Color::White).bold())),
        Cell::from(Span::styled("Compress", Style::default().fg(Color::White).bold())),
        Cell::from(Span::styled("Blocked", Style::default().fg(Color::White).bold())),
        Cell::from(Span::styled("Errors", Style::default().fg(Color::White).bold())),
    ])
    .style(Style::default().bg(Color::DarkGray));

    // Data rows
    let rows: Vec<Row> = tools
        .iter()
        .map(|t| {
            let tool_style = Style::default().fg(Color::Cyan);
            let calls = format!("{}", t.call_count);
            let saved = format_tokens_short(t.tokens_saved);
            let processed = format_tokens_short(t.tokens_processed);
            let cr = t
                .compression_ratio
                .map(|r| format!("{:.0}%", r * 100.0))
                .unwrap_or_else(|| "--".to_string());
            let blocked = format!("{}", t.blocked_count);
            let errors = format!("{}", t.error_count);

            let savings_pct = if t.tokens_processed > 0 {
                (t.tokens_saved as f64 / t.tokens_processed as f64 * 100.0) as u64
            } else {
                0
            };

            // Color code: green if high savings, yellow if medium, red if low
            let savings_color = if savings_pct > 60 {
                Color::Green
            } else if savings_pct > 30 {
                Color::Yellow
            } else {
                Color::Red
            };

            Row::new(vec![
                Cell::from(Span::styled(format!(" {}", t.tool), tool_style)),
                Cell::from(Span::styled(calls, Style::default().fg(Color::White))),
                Cell::from(Span::styled(saved, Style::default().fg(savings_color).bold())),
                Cell::from(Span::styled(processed, Style::default().fg(Color::DarkGray))),
                Cell::from(Span::styled(cr, Style::default().fg(Color::Magenta))),
                Cell::from(Span::styled(blocked, Style::default().fg(Color::Yellow))),
                Cell::from(Span::styled(errors, Style::default().fg(Color::Red))),
            ])
            .height(1)
        })
        .collect();

    let table = Table::new(rows, widths)
        .header(header)
        .block(
            Block::bordered()
                .title(" Tool Breakdown ")
                .border_type(BorderType::Rounded)
                .border_style(Color::Cyan),
        )
        .column_spacing(1);

    frame.render_widget(table, area);
}

/// Format token counts for display in the table.
fn format_tokens_short(v: u64) -> String {
    if v >= 1_000_000 {
        format!("{:.1}M", v as f64 / 1_000_000.0)
    } else if v >= 1_000 {
        format!("{:.1}K", v as f64 / 1_000.0)
    } else {
        format!("{v}")
    }
}
