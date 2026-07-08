//! Four big gauge cards showing the top-level savings metrics.
//!
//! Cards: tokens saved, dollars saved, loops escaped, compression ratio.
//! Each card has a label, a large value, a mini trend sparkline, and a Gauge bar.

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, BorderType, Paragraph},
};

use crate::app::App;
use crate::charts::sparkline::render_sparkline_f64;

/// Height of the overview cards area.
pub const OVERVIEW_HEIGHT: u16 = 8;

/// Render the four metric cards (tokens, dollars, loops, ratio).
pub fn render_overview_cards(frame: &mut Frame, area: Rect, app: &App) {
    // 2x2 grid
    let horiz = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    let left = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(horiz[0]);

    let right = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(horiz[1]);

    render_token_card(frame, left[0], app);
    render_dollar_card(frame, left[1], app);
    render_loops_card(frame, right[0], app);
    render_ratio_card(frame, right[1], app);
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

fn token_card_block() -> Block<'static> {
    Block::bordered()
        .title(" TOKENS SAVED ")
        .border_type(BorderType::Rounded)
        .border_style(Color::Cyan)
        .style(Color::White)
}

fn render_token_card(frame: &mut Frame, area: Rect, app: &App) {
    let tokens = app.total_tokens_saved;
    let savings_ratio = (app.savings_avg * 100.0).clamp(0.0, 100.0);

    // Sparkline from history
    let history: Vec<f64> = app.history.iter().map(|s| s.tokens_saved as f64).collect();
    let spark = render_sparkline_f64(&history, (area.width.saturating_sub(4)) as usize);

    let value = format_tokens(tokens);
    let pct = format!("{:.0}% savings ratio", savings_ratio);

    let content = vec![
        Line::from(Span::styled(value, Style::default().fg(Color::Cyan).bold())),
        Line::from(Span::styled(pct, Style::default().fg(Color::Green))),
        Line::from(Span::styled(spark, Style::default().fg(Color::Cyan))),
        Line::from(Span::raw("")),
    ];

    let para = Paragraph::new(content).block(token_card_block());
    frame.render_widget(para, area);
}

fn dollar_card_block() -> Block<'static> {
    Block::bordered()
        .title(" DOLLARS SAVED ")
        .border_type(BorderType::Rounded)
        .border_style(Color::Green)
}

fn render_dollar_card(frame: &mut Frame, area: Rect, app: &App) {
    let dollars = app.total_dollars_saved;
    let processed = app.attribution.as_ref().map(|a| a.estimated_cost_processed_usd).unwrap_or(0.0);
    let ratio = if processed > 0.0 { (dollars / processed * 100.0).clamp(0.0, 100.0) } else { 0.0 };

    let content = vec![
        Line::from(Span::styled(
            format!("${:.2}", dollars),
            Style::default().fg(Color::Green).bold(),
        )),
        Line::from(Span::styled(
            format!("${:.2} processed — {:.0}% savings rate", processed, ratio),
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(Span::styled(
            format!("At ${:.4}/1K tokens", 0.25),
            Style::default().fg(Color::DarkGray),
        )),
    ];

    let para = Paragraph::new(content).block(dollar_card_block());
    frame.render_widget(para, area);
}

fn loops_card_block() -> Block<'static> {
    Block::bordered()
        .title(" LOOPS ESCAPED ")
        .border_type(BorderType::Rounded)
        .border_style(Color::Yellow)
}

fn render_loops_card(frame: &mut Frame, area: Rect, app: &App) {
    let blocked = app.blocked_calls;
    let healed = app.self_healed_count;

    let content = vec![
        Line::from(vec![
            Span::styled(format!("{blocked}"), Style::default().fg(Color::Yellow).bold()),
            Span::styled(" blocked", Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled("✓ ", Style::default().fg(Color::Green)),
            Span::styled(format!("{healed} self-healed", healed = healed), Style::default().fg(Color::Green)),
        ]),
        Line::from(Span::styled(
            format!("{:.0}% auto-recovery", if blocked > 0 { healed as f64 / blocked as f64 * 100.0 } else { 0.0 }),
            Style::default().fg(Color::DarkGray),
        )),
    ];

    let para = Paragraph::new(content).block(loops_card_block());
    frame.render_widget(para, area);
}

fn ratio_card_block() -> Block<'static> {
    Block::bordered()
        .title(" COMPRESSION RATIO ")
        .border_type(BorderType::Rounded)
        .border_style(Color::Magenta)
}

fn render_ratio_card(frame: &mut Frame, area: Rect, app: &App) {
    let ratio = (app.compression_avg * 100.0).clamp(0.0, 100.0);

    // Sparkline of compression ratios
    let history: Vec<f64> = app.history.iter().map(|s| s.compression_ratio).collect();
    let spark = render_sparkline_f64(&history, (area.width.saturating_sub(4)) as usize);

    let content = vec![
        Line::from(Span::styled(
            format!("{:.1}%", ratio),
            Style::default().fg(Color::Magenta).bold(),
        )),
        Line::from(Span::styled(spark, Style::default().fg(Color::Magenta))),
    ];

    let para = Paragraph::new(content).block(ratio_card_block());
    frame.render_widget(para, area);
}
