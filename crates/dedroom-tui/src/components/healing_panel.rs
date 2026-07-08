//! Healing panel — top mutations that worked and strategy effectiveness.
//!
//! Shows the best strategies for each tool, success rates, and a
//! summary of self-healing activity.

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Margin, Rect},
    style::{Color, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, BorderType, Paragraph},
};

use crate::app::App;

/// Render the self-healing panel.
pub fn render_healing_panel(frame: &mut Frame, area: Rect, app: &App) {
    let vert = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0)])
        .split(area.inner(Margin { horizontal: 1, vertical: 1 }));

    render_healing_summary(frame, vert[0], app);
    render_healing_strategies(frame, vert[1], app);
}

fn render_healing_summary(frame: &mut Frame, area: Rect, app: &App) {
    let healed = app.self_healed_count;
    let blocked = app.blocked_calls;
    let rate = if blocked > 0 {
        (healed as f64 / blocked as f64 * 100.0).clamp(0.0, 100.0)
    } else {
        0.0
    };

    let content = vec![
        Line::from(vec![
            Span::styled("Total Self-Healings: ", Style::default().fg(Color::DarkGray)),
            Span::styled(format!("{healed}"), Style::default().fg(Color::Green).bold()),
            Span::styled("  │  Recovery Rate: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{rate:.0}%"),
                Style::default()
                    .fg(if rate >= 80.0 { Color::Green } else if rate >= 50.0 { Color::Yellow } else { Color::Red })
                    .bold(),
            ),
            Span::styled(
                "  │  (from healing memory)",
                Style::default().fg(Color::DarkGray),
            ),
        ]),
        Line::from(Span::styled(
            "When loop detection blocks a tool call, the self-healing engine generates alternative strategies \
             (parameter tweaks, tool substitutions, decomposition) and learns which work best over time.",
            Style::default().fg(Color::DarkGray),
        )),
    ];

    let block = Block::bordered()
        .title(" ❤️ Self-Healing Engine ")
        .border_type(BorderType::Rounded)
        .border_style(Color::Green);
    let para = Paragraph::new(content).block(block);
    frame.render_widget(para, area);
}

fn render_healing_strategies(frame: &mut Frame, area: Rect, app: &App) {
    // Show known mutation strategies and which tools they've helped
    let strategies = [
        ("Parameter Tweak", "Reducing batch sizes, limits, and pagination counts to narrow scope", Color::Cyan),
        ("Tool Substitution", "Swapping tools for alternatives (e.g., read_file → grep + head)", Color::Green),
        ("Decomposition", "Breaking batch operations into item-by-item processing", Color::Magenta),
        ("Rephrase", "Suggesting a fundamentally different approach after repeated failures", Color::Yellow),
    ];

    let mut lines = vec![
        Line::from(Span::styled("Known Mutation Strategies:", Style::default().fg(Color::White).bold())),
        Line::from(Span::raw("")),
    ];

    for (name, desc, color) in &strategies {
        lines.push(Line::from(vec![
            Span::styled(format!("  ▸ {name}"), Style::default().fg(*color).bold()),
        ]));
        lines.push(Line::from(vec![
            Span::styled(format!("    {desc}"), Style::default().fg(Color::DarkGray)),
        ]));
    }

    // Show tools that have been healed (from event log)
    let healed_tools: Vec<&crate::api::ProxyEventItem> = app
        .events
        .iter()
        .rev()
        .filter(|e| e.verdict == "inject")
        .take(5)
        .collect();

    if !healed_tools.is_empty() {
        lines.push(Line::from(Span::raw("")));
        lines.push(Line::from(Span::styled(
            "Recent Healing Injections:",
            Style::default().fg(Color::White).bold(),
        )));
        for event in &healed_tools {
            lines.push(Line::from(vec![
                Span::styled("  ✓ ", Style::default().fg(Color::Green)),
                Span::styled(&event.tool_name, Style::default().fg(Color::Cyan).bold()),
                Span::styled(
                    format!(
                        " — tilt: {:.1}",
                        event.tilt_index.unwrap_or(0.0)
                    ),
                    Style::default().fg(Color::DarkGray),
                ),
            ]));
        }
    } else {
        lines.push(Line::from(Span::raw("")));
        lines.push(Line::from(Span::styled(
            "  No healing injections recorded yet. Self-healing activates when loop detection finds repeated failures.",
            Style::default().fg(Color::DarkGray),
        )));
    }

    let block = Block::bordered()
        .title(" Mutation Strategies ")
        .border_type(BorderType::Rounded)
        .border_style(Color::Cyan);
    let para = Paragraph::new(lines).block(block);
    frame.render_widget(para, area);
}
