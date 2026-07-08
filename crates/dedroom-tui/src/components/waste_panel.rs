//! Waste analysis panel — shows where tokens went.
//!
//! Three categories: error waste (tokens spent on failed calls),
//! blocked savings (tokens saved by blocking loops), and
//! uncompressible waste (content that couldn't be compressed).

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Margin, Rect},
    style::{Color, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, BorderType, Gauge, Paragraph},
};

use crate::app::App;

/// Render the waste analysis panel.
pub fn render_waste_panel(frame: &mut Frame, area: Rect, app: &App) {
    if let Some(ref att) = app.attribution {
        let vert = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Length(6),
                Constraint::Length(6),
                Constraint::Min(0),
            ])
            .split(area.inner(Margin { horizontal: 1, vertical: 1 }));

        render_waste_header(frame, vert[0], att);
        render_waste_gauges(frame, vert[1], att);
        render_waste_breakdown(frame, vert[2], att);
        render_waste_opportunities(frame, vert[3], att);
    } else {
        let block = Block::bordered()
            .title(" Waste Analysis ")
            .border_type(BorderType::Rounded)
            .border_style(Color::Red);
        let para = Paragraph::new(Line::from(Span::styled(
            "No attribution data available. Route an agent through the proxy to collect metrics.",
            Style::default().fg(Color::DarkGray),
        )))
        .block(block);
        frame.render_widget(para, area);
    }
}

fn render_waste_header(frame: &mut Frame, area: Rect, att: &crate::api::AttributionResponse) {
    let total_cost = att.estimated_cost_processed_usd;
    let waste_pct = if att.total_tokens_processed > 0 {
        (att.waste.error_waste_tokens + att.waste.uncompressible_waste_tokens) as f64
            / att.total_tokens_processed as f64
            * 100.0
    } else {
        0.0
    };

    let content = vec![
        Line::from(vec![
            Span::styled("Total Processed: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format_tokens(att.total_tokens_processed),
                Style::default().fg(Color::White).bold(),
            ),
            Span::styled("  │  Cost: ", Style::default().fg(Color::DarkGray)),
            Span::styled(format!("${:.2}", total_cost), Style::default().fg(Color::Yellow).bold()),
            Span::styled("  │  Waste Rate: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{waste_pct:.1}%"),
                Style::default()
                    .fg(if waste_pct > 30.0 { Color::Red } else if waste_pct > 15.0 { Color::Yellow } else { Color::Green })
                    .bold(),
            ),
        ]),
    ];

    let block = Block::bordered()
        .title(" Waste Analysis ")
        .border_type(BorderType::Rounded)
        .border_style(Color::Red);
    let para = Paragraph::new(content).block(block);
    frame.render_widget(para, area);
}

fn render_waste_gauges(frame: &mut Frame, area: Rect, att: &crate::api::AttributionResponse) {
    let total = att.total_tokens_processed.max(1);

    let error_pct = (att.waste.error_waste_tokens as f64 / total as f64 * 100.0) as u16;
    let blocked_pct = (att.waste.blocked_saved_tokens as f64 / total as f64 * 100.0) as u16;
    let uncompress_pct = (att.waste.uncompressible_waste_tokens as f64 / total as f64 * 100.0) as u16;

    let horiz = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(33), Constraint::Percentage(33), Constraint::Percentage(33)])
        .split(area);

    // Error waste gauge
    let error_block = Block::bordered()
        .title(" ❌ Error Waste ")
        .border_style(Color::Red);
    let error_gauge = Gauge::default()
        .block(error_block)
        .gauge_style(Style::default().fg(Color::Red).bg(Color::Black))
        .percent(error_pct)
        .label(Span::styled(
            format!(
                "{} tok ({error_pct}%)",
                format_tokens(att.waste.error_waste_tokens)
            ),
            Style::default().fg(Color::White).bold(),
        ));
    frame.render_widget(error_gauge, horiz[0]);

    // Blocked savings gauge
    let blocked_block = Block::bordered()
        .title(" ✅ Blocked Savings ")
        .border_style(Color::Green);
    let blocked_gauge = Gauge::default()
        .block(blocked_block)
        .gauge_style(Style::default().fg(Color::Green).bg(Color::Black))
        .percent(blocked_pct)
        .label(Span::styled(
            format!(
                "{} tok ({blocked_pct}%)",
                format_tokens(att.waste.blocked_saved_tokens)
            ),
            Style::default().fg(Color::White).bold(),
        ));
    frame.render_widget(blocked_gauge, horiz[1]);

    // Uncompressible waste gauge
    let uncompress_block = Block::bordered()
        .title(" ░ Uncompressible ")
        .border_style(Color::Yellow);
    let uncompress_gauge = Gauge::default()
        .block(uncompress_block)
        .gauge_style(Style::default().fg(Color::Yellow).bg(Color::Black))
        .percent(uncompress_pct)
        .label(Span::styled(
            format!(
                "{} tok ({uncompress_pct}%)",
                format_tokens(att.waste.uncompressible_waste_tokens)
            ),
            Style::default().fg(Color::White).bold(),
        ));
    frame.render_widget(uncompress_gauge, horiz[2]);
}

fn render_waste_breakdown(frame: &mut Frame, area: Rect, att: &crate::api::AttributionResponse) {
    let lines = vec![
        Line::from(Span::styled("Call Count Breakdown:", Style::default().fg(Color::White).bold())),
        Line::from(Span::raw("")),
        Line::from(vec![
            Span::styled("  Error Calls:    ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{}", att.waste.error_call_count),
                Style::default().fg(Color::Red).bold(),
            ),
            Span::styled(
                format!(" ({} tok wasted)", format_tokens(att.waste.error_waste_tokens)),
                Style::default().fg(Color::DarkGray),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Blocked Calls:  ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{}", att.waste.blocked_call_count),
                Style::default().fg(Color::Green).bold(),
            ),
            Span::styled(
                format!(" ({} tok saved)", format_tokens(att.waste.blocked_saved_tokens)),
                Style::default().fg(Color::DarkGray),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Uncompressible: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{}", att.waste.uncompressible_call_count),
                Style::default().fg(Color::Yellow).bold(),
            ),
            Span::styled(
                format!(" ({} tok)", format_tokens(att.waste.uncompressible_waste_tokens)),
                Style::default().fg(Color::DarkGray),
            ),
        ]),
    ];

    let block = Block::bordered()
        .title(" Call Breakdown ")
        .border_type(BorderType::Rounded)
        .border_style(Color::DarkGray);
    let para = Paragraph::new(lines).block(block);
    frame.render_widget(para, area);
}

fn render_waste_opportunities(frame: &mut Frame, area: Rect, att: &crate::api::AttributionResponse) {
    let mut items = Vec::new();

    if att.waste.error_waste_tokens > 0 {
        items.push((
            "High error waste".to_string(),
            format!(
                "{} tokens lost to errors. Consider adding error handling or retry logic.",
                format_tokens(att.waste.error_waste_tokens)
            ),
            Color::Red,
        ));
    }

    if att.waste.uncompressible_waste_tokens > 0 {
        items.push((
            "Uncompressible content".to_string(),
            format!(
                "{} tokens could not be compressed. Enable text_compressor or use more structured output.",
                format_tokens(att.waste.uncompressible_waste_tokens)
            ),
            Color::Yellow,
        ));
    }

    // Check per-tool for opportunities
    for tool in &att.per_tool {
        if tool.tokens_processed > 10_000 && tool.compression_ratio.unwrap_or(0.0) < 0.1 {
            items.push((
                format!("Low compression: {}", tool.tool),
                format!(
                    "{} processed with {:.0}% compression. Consider custom compressors for this tool.",
                    format_tokens(tool.tokens_processed),
                    tool.compression_ratio.unwrap_or(0.0) * 100.0
                ),
                Color::Yellow,
            ));
        }
    }

    if items.is_empty() {
        items.push((
            "No major waste detected".to_string(),
            "Your pipeline is running efficiently. Keep up the good work!".to_string(),
            Color::Green,
        ));
    }

    let mut lines = vec![
        Line::from(Span::styled("Optimization Opportunities:", Style::default().fg(Color::White).bold())),
        Line::from(Span::raw("")),
    ];

    for (title, desc, color) in &items {
        lines.push(Line::from(vec![
            Span::styled("  💡 ", Style::default().fg(*color)),
            Span::styled(title.as_str(), Style::default().fg(*color).bold()),
        ]));
        lines.push(Line::from(vec![
            Span::styled(format!("     {desc}"), Style::default().fg(Color::DarkGray)),
        ]));
    }

    let block = Block::bordered()
        .title(" Opportunities ")
        .border_type(BorderType::Rounded)
        .border_style(Color::Yellow);
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
