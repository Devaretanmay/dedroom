//! Loops escaped panel — shows blocked call count, self-healing success rate,
//! and highlights of the most effective mutation strategies.

use ratatui::{
    Frame, layout::Rect,
    style::{Color, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, BorderType, Paragraph},
};

use crate::app::App;

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


