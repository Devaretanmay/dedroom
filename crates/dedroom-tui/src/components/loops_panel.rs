//! Loops escaped panel — shows blocked call count, self-healing success rate,
//! and highlights of the most effective mutation strategies.

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Margin, Rect},
    style::{Color, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, BorderType, Gauge, Paragraph},
};

use crate::app::App;

/// Render the loops panel — shows blocked/escaped counts and healing highlights.
pub fn render_loops_panel(frame: &mut Frame, area: Rect, app: &App) {
    let vert = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(4), Constraint::Length(6), Constraint::Min(0)])
        .split(area.inner(Margin { horizontal: 1, vertical: 1 }));

    // Top: summary stats
    render_loop_summary(frame, vert[0], app);
    // Middle: healing success rate
    render_healing_rate(frame, vert[1], app);
    // Bottom: strategy breakdown (placeholder until proxy exposes this)
    render_loop_details(frame, vert[2], app);
}

pub fn render_loop_summary(frame: &mut Frame, area: Rect, app: &App) {
    let blocked = app.blocked_calls;
    let healed = app.self_healed_count as u64;
    let total_calls = app.total_calls_processed;

    let recovery_pct = if blocked > 0 {
        (healed as f64 / blocked as f64 * 100.0).clamp(0.0, 100.0)
    } else {
        0.0
    };

    let content = vec![
        Line::from(vec![
            Span::styled("Total Calls: ", Style::default().fg(Color::DarkGray)),
            Span::styled(format!("{total_calls}"), Style::default().fg(Color::White).bold()),
            Span::styled("  │  Blocked: ", Style::default().fg(Color::DarkGray)),
            Span::styled(format!("{blocked}"), Style::default().fg(Color::Yellow).bold()),
            Span::styled("  │  Healed: ", Style::default().fg(Color::DarkGray)),
            Span::styled(format!("{healed}"), Style::default().fg(Color::Green).bold()),
        ]),
        Line::from(Span::raw("")),
        Line::from(vec![
            Span::styled("Auto-Recovery Rate: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{:.0}%", recovery_pct),
                Style::default()
                    .fg(if recovery_pct > 50.0 { Color::Green } else { Color::Yellow })
                    .bold(),
            ),
        ]),
    ];

    let block = Block::bordered()
        .title(" Loop Summary ")
        .border_type(BorderType::Rounded)
        .border_style(Color::Yellow);
    let para = Paragraph::new(content).block(block);
    frame.render_widget(para, area);
}

fn render_healing_rate(frame: &mut Frame, area: Rect, app: &App) {
    let blocked = app.blocked_calls;
    let healed = app.self_healed_count as u64;
    let pct = if blocked > 0 {
        ((healed as f64 / blocked as f64) * 100.0) as u16
    } else {
        0
    };

    let block = Block::bordered()
        .title(" Healing Success Rate ")
        .border_type(BorderType::Rounded)
        .border_style(Color::Green);

    // Render a Gauge
    let gauge_area = {
        let inner = area.inner(Margin { horizontal: 1, vertical: 1 });
        // Center the gauge
        let top = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(30), Constraint::Length(3), Constraint::Percentage(30)])
            .split(inner);
        top[1]
    };

    frame.render_widget(block, area);

    let gauge_color = if pct >= 80 {
        Color::Green
    } else if pct >= 50 {
        Color::Yellow
    } else {
        Color::Red
    };

    let gauge = Gauge::default()
        .gauge_style(Style::default().fg(gauge_color).bg(Color::Black))
        .percent(pct)
        .label(Span::styled(
            format!("{healed} of {blocked} loops healed ({pct}%)"),
            Style::default().fg(Color::White).bold(),
        ));

    frame.render_widget(gauge, gauge_area);
}

fn render_loop_details(frame: &mut Frame, area: Rect, app: &App) {
    // Show most recent blocked events from the timeline
    let recent: Vec<&crate::api::ProxyEventItem> = app
        .events
        .iter()
        .rev()
        .filter(|e| e.verdict == "block")
        .take(10)
        .collect();

    let mut lines: Vec<Line> = vec![Line::from(
        Span::styled("Most Recent Blocks:", Style::default().fg(Color::DarkGray)),
    )];

    for event in &recent {
        let tool = &event.tool_name;
        let saved = event.original_tokens.unwrap_or(0);
        let saved_str = if saved >= 1000 {
            format!("{:.1}K tok saved", saved as f64 / 1000.0)
        } else {
            format!("{saved} tok saved")
        };

        lines.push(Line::from(vec![
            Span::styled("  🛑 ", Style::default().fg(Color::Red)),
            Span::styled(tool, Style::default().fg(Color::Yellow).bold()),
            Span::styled(format!(" — {saved_str}"), Style::default().fg(Color::Green)),
        ]));
    }

    if recent.is_empty() {
        lines.push(Line::from(Span::styled(
            "  No blocks recorded yet. Route an agent through the proxy to see data.",
            Style::default().fg(Color::DarkGray),
        )));
    }

    let block = Block::bordered()
        .title(" Recent Blocks ")
        .border_type(BorderType::Rounded)
        .border_style(Color::Red);

    let para = Paragraph::new(lines).block(block);
    frame.render_widget(para, area);
}
