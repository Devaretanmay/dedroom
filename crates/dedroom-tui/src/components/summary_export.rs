//! Export panel — weekly summary export, copy stats, and data overview.

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Margin, Rect},
    style::{Color, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, BorderType, Paragraph},
};

use crate::app::App;

/// Render the export panel.
pub fn render_export_panel(frame: &mut Frame, area: Rect, app: &App) {
    let vert = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(8), Constraint::Min(0)])
        .split(area.inner(Margin { horizontal: 1, vertical: 1 }));

    // Export actions
    render_export_actions(frame, vert[0], app);
    // Summary preview
    render_summary_preview(frame, vert[1], app);
}

fn render_export_actions(frame: &mut Frame, area: Rect, app: &App) {
    let mut lines = vec![
        Line::from(Span::styled(
            "Export Dashboard Data",
            Style::default().fg(Color::White).bold(),
        )),
        Line::from(Span::raw("")),
        Line::from(vec![
            Span::styled("  [E] ", Style::default().fg(Color::Cyan).bold()),
            Span::styled("Export weekly summary", Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled("  [C] ", Style::default().fg(Color::Cyan).bold()),
            Span::styled("Copy stats to clipboard", Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled("  [F5] ", Style::default().fg(Color::Cyan).bold()),
            Span::styled("Force refresh all data", Style::default().fg(Color::White)),
        ]),
        Line::from(Span::raw("")),
    ];

    // If we have data, show export path
    if app.attribution.is_some() {
        lines.push(Line::from(vec![
            Span::styled("  Data available for export", Style::default().fg(Color::Green)),
            Span::styled(
                " — run with --export <file> for non-interactive export",
                Style::default().fg(Color::DarkGray),
            ),
        ]));
    } else {
        lines.push(Line::from(Span::styled(
            "  No data yet. Route an agent through the proxy to collect metrics.",
            Style::default().fg(Color::DarkGray),
        )));
    }

    let block = Block::bordered()
        .title(" Export Controls ")
        .border_type(BorderType::Rounded)
        .border_style(Color::Green);
    let para = Paragraph::new(lines).block(block);
    frame.render_widget(para, area);
}

fn render_summary_preview(frame: &mut Frame, area: Rect, app: &App) {
    let mut lines = vec![
        Line::from(Span::styled(
            "Weekly Summary Preview",
            Style::default().fg(Color::White).bold(),
        )),
        Line::from(Span::raw("")),
    ];

    if let Some(ref att) = app.attribution {
        // Build a summary preview
        lines.push(Line::from(vec![
            Span::styled("# DedrooM Weekly Report", Style::default().fg(Color::White).bold()),
        ]));
        lines.push(Line::from(Span::raw("")));
        lines.push(Line::from(vec![
            Span::styled("**Uptime**: ", Style::default().fg(Color::DarkGray)),
            Span::styled(app.uptime_str(), Style::default().fg(Color::White)),
        ]));
        lines.push(Line::from(vec![
            Span::styled("**Total Calls**: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{}", att.total_calls),
                Style::default().fg(Color::White),
            ),
        ]));
        lines.push(Line::from(vec![
            Span::styled("**Tokens Saved**: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format_tokens(att.total_tokens_saved),
                Style::default().fg(Color::Green).bold(),
            ),
            Span::styled(
                format!(" ({:.0}%)", att.savings_ratio * 100.0),
                Style::default().fg(Color::Green),
            ),
        ]));
        lines.push(Line::from(vec![
            Span::styled("**Cost Saved**: $", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{:.2}", att.estimated_cost_saved_usd),
                Style::default().fg(Color::Green).bold(),
            ),
        ]));
        lines.push(Line::from(vec![
            Span::styled("**Compression Ratio**: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{:.1}%", att.compression_ratio * 100.0),
                Style::default().fg(Color::Magenta).bold(),
            ),
        ]));
        lines.push(Line::from(vec![
            Span::styled("**Loops Blocked**: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{}", att.blocked_calls),
                Style::default().fg(Color::Yellow).bold(),
            ),
            Span::styled(
                format!(" ({} self-healed)", app.self_healed_count),
                Style::default().fg(Color::Green),
            ),
        ]));

        // Top tools
        if !att.per_tool.is_empty() {
            lines.push(Line::from(Span::raw("")));
            lines.push(Line::from(Span::styled(
                "### Top Tools by Savings",
                Style::default().fg(Color::White).bold(),
            )));
            for tool in att.per_tool.iter().take(5) {
                let cr = tool
                    .compression_ratio
                    .map(|r| format!("{:.0}%", r * 100.0))
                    .unwrap_or_else(|| "--".to_string());
                lines.push(Line::from(vec![
                    Span::styled(
                        format!(
                            "  - {}: {} saved ({} calls, {} compression)",
                            tool.tool,
                            format_tokens(tool.tokens_saved),
                            tool.call_count,
                            cr,
                        ),
                        Style::default().fg(Color::Cyan),
                    ),
                ]));
            }
        }
    } else {
        lines.push(Line::from(Span::styled(
            "No data to preview yet.",
            Style::default().fg(Color::DarkGray),
        )));
    }

    let block = Block::bordered()
        .title(" Summary Preview ")
        .border_type(BorderType::Rounded)
        .border_style(Color::Cyan);
    let para = Paragraph::new(lines).block(block);
    frame.render_widget(para, area);
}

fn format_tokens(v: u64) -> String {
    if v >= 1_000_000 {
        format!("{:.1}M", v as f64 / 1_000_000.0)
    } else if v >= 1_000 {
        format!("{:.1}K", v as f64 / 1_000.0)
    } else {
        format!("{v}")
    }
}
