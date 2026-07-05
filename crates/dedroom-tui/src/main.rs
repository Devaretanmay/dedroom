//! DedrooM Event Dashboard — live TUI that tails the SSE event stream.
//!
//! Connects to `http://localhost:8080/admin/events/stream` (or a custom URL),
//! parses `ProxyEvent` NDJSON lines, and renders a real-time terminal UI
//! with color-coded verdicts and live statistics.
//!
//! Usage:
//!   dedroom-tui                               # connect to localhost:8080
//!   dedroom-tui http://10.0.0.5:9090           # custom proxy URL
//!   dedroom-tui --proxy-port 9090              # custom port on localhost
//!   dedroom-tui --proxy-port 9090 http://alt   # port + custom URL override

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::{
    layout::{Alignment, Constraint, Layout},
    style::{Color, Modifier, Style, Stylize},
    symbols::border,
    text::{Line, Span},
    widgets::{Block, Borders, Gauge, List, ListItem, Paragraph},
    Frame,
};
use serde::Deserialize;

// ── Event type (mirrors dedroom_core::telemetry::ProxyEvent) ───────────────

#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
struct ProxyEvent {
    timestamp: u64,
    session_id: Option<String>,
    agent_id: Option<String>,
    tool_name: String,
    args_hash: Option<String>,
    verdict: String,
    compression_ratio: Option<f64>,
    original_tokens: Option<u64>,
    compressed_tokens: Option<u64>,
    tilt_index: Option<f64>,
    latency_us: u64,
}

// ── App state ──────────────────────────────────────────────────────────────

struct App {
    events: VecDeque<ProxyEvent>,
    connected: bool,
    error: Option<String>,
    total: u64,
    allow_count: u64,
    block_count: u64,
    inject_count: u64,
    total_original_tokens: u64,
    total_compressed_tokens: u64,
    scroll_offset: usize,
    auto_scroll: bool,
    proxy_url: String,
}

impl App {
    fn new(proxy_url: String) -> Self {
        Self {
            events: VecDeque::with_capacity(101),
            connected: false,
            error: None,
            total: 0,
            allow_count: 0,
            block_count: 0,
            inject_count: 0,
            total_original_tokens: 0,
            total_compressed_tokens: 0,
            scroll_offset: 0,
            auto_scroll: true,
            proxy_url,
        }
    }

    fn push_event(&mut self, event: ProxyEvent) {
        self.total += 1;
        match event.verdict.as_str() {
            "allow" => self.allow_count += 1,
            "block" => self.block_count += 1,
            "inject" => self.inject_count += 1,
            _ => {}
        }

        if let Some(orig) = event.original_tokens {
            self.total_original_tokens += orig;
        }
        if let Some(comp) = event.compressed_tokens {
            self.total_compressed_tokens += comp;
        }

        if self.events.len() >= 100 {
            self.events.pop_front();
        }
        self.events.push_back(event);

        if self.auto_scroll {
            self.scroll_offset = 0;
        }
    }
}

// ── Helpers ────────────────────────────────────────────────────────────────

fn verdict_colors(verdict: &str) -> (Color, Color) {
    match verdict {
        "allow" => (Color::Green, Color::Black),
        "block" => (Color::Red, Color::White),
        "inject" => (Color::Yellow, Color::Black),
        _ => (Color::DarkGray, Color::White),
    }
}

fn fmt_timestamp(ts: u64) -> String {
    let secs = ts / 1000;
    format!("{:02}:{:02}:{:02}", (secs / 3600) % 24, (secs / 60) % 60, secs % 60)
}

fn fmt_latency(us: u64) -> String {
    if us < 1000 {
        format!("{us}µs")
    } else if us < 1_000_000 {
        format!("{:.1}ms", us as f64 / 1000.0)
    } else {
        format!("{:.2}s", us as f64 / 1_000_000.0)
    }
}

// ── SSE client ─────────────────────────────────────────────────────────────

fn start_sse_stream(
    base_url: &str,
    tx: tokio::sync::mpsc::UnboundedSender<ProxyEvent>,
) {
    let url = format!("{}/admin/events/stream", base_url.trim_end_matches('/'));
    let tx_clone = tx.clone();

    tokio::spawn(async move {
        let client = match reqwest::Client::builder()
            .timeout(Duration::from_secs(300))
            .build()
        {
            Ok(c) => c,
            Err(e) => {
                tracing::error!("[tui] failed to build client: {e}");
                return;
            }
        };

        loop {
            let response = match client.get(&url).send().await {
                Ok(resp) => resp,
                Err(e) => {
                    let err_msg = format!("Connection failed: {e}");
                    let _ = tx_clone.send(ProxyEvent {
                        timestamp: 0, session_id: None, agent_id: None,
                        tool_name: format!("__error__:{err_msg}"),
                        args_hash: None, verdict: "__error__".into(),
                        compression_ratio: None, original_tokens: None,
                        compressed_tokens: None, tilt_index: None, latency_us: 0,
                    });
                    tokio::time::sleep(Duration::from_secs(3)).await;
                    continue;
                }
            };

            let _ = tx_clone.send(ProxyEvent {
                timestamp: 0, session_id: None, agent_id: None,
                tool_name: "__connected__".into(), args_hash: None,
                verdict: "__connected__".into(), compression_ratio: None,
                original_tokens: None, compressed_tokens: None,
                tilt_index: None, latency_us: 0,
            });

            use futures::StreamExt;
            let mut stream = response.bytes_stream();
            let mut buffer = String::new();

            while let Some(chunk) = stream.next().await {
                match chunk {
                    Ok(bytes) => {
                        buffer.push_str(&String::from_utf8_lossy(&bytes));
                        while let Some(idx) = buffer.find("\n\n") {
                            let msg = buffer[..idx].to_string();
                            buffer = buffer[idx + 2..].to_string();
                            for line in msg.lines() {
                                if let Some(data) = line.strip_prefix("data: ")
                                    && let Ok(event) = serde_json::from_str::<ProxyEvent>(data)
                                {
                                    let _ = tx_clone.send(event);
                                }
                            }
                        }
                    }
                    Err(e) => {
                        let err_msg = format!("Stream error: {e}");
                        let _ = tx_clone.send(ProxyEvent {
                            timestamp: 0, session_id: None, agent_id: None,
                            tool_name: format!("__error__:{err_msg}"),
                            args_hash: None, verdict: "__error__".into(),
                            compression_ratio: None, original_tokens: None,
                            compressed_tokens: None, tilt_index: None, latency_us: 0,
                        });
                        break;
                    }
                }
            }

            tokio::time::sleep(Duration::from_secs(3)).await;
        }
    });
}

// ── UI rendering ───────────────────────────────────────────────────────────

fn render_ui(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    let vertical = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(3),
        Constraint::Min(1),
        Constraint::Length(1),
    ]);
    let [title_area, stats_area, list_area, status_area] = vertical.areas(area);

    // ── Title
    let title = Line::from(vec![
        Span::raw(" DedrooM ").bold().white().on_blue(),
        Span::raw(" Event Dashboard ").bold(),
        Span::raw(format!("— {}", app.proxy_url)).dim(),
    ])
    .alignment(Alignment::Left);
    frame.render_widget(title, title_area);

    // ── Stats
    let total = app.total.max(1);
    let allow_pct = (app.allow_count as f64 / total as f64 * 100.0) as u16;
    let inject_pct = (app.inject_count as f64 / total as f64 * 100.0) as u16;
    let block_pct = (app.block_count as f64 / total as f64 * 100.0) as u16;

    let stats_block = Block::bordered()
        .borders(Borders::TOP)
        .border_set(border::Set {
            vertical_left: "┃",
            vertical_right: "┃",
            horizontal_top: "━",
            horizontal_bottom: "━",
            top_left: "┏",
            top_right: "┓",
            bottom_left: "┗",
            bottom_right: "┛",
        });
    let inner_area = stats_block.inner(stats_area);
    frame.render_widget(stats_block, stats_area);

    let gauge_layout = Layout::horizontal([
        Constraint::Length(10),
        Constraint::Ratio(1, 3),
        Constraint::Ratio(1, 3),
        Constraint::Ratio(1, 3),
    ]);
    let [total_label_area, allow_area, inject_area, block_area] = gauge_layout.areas(inner_area);

    let total_label = Paragraph::new(Line::from(vec![
        Span::raw(format!(" {}", app.total)).bold(),
        Span::raw(" events").dim(),
    ]));
    frame.render_widget(total_label, total_label_area);

    let allow_gauge = Gauge::default()
        .gauge_style(Style::new().fg(Color::Green).bg(Color::DarkGray))
        .percent(allow_pct)
        .label(Span::styled(
            format!(" Allow {} ({allow_pct}%)", app.allow_count),
            Style::new().fg(Color::Green).add_modifier(Modifier::BOLD),
        ));
    frame.render_widget(allow_gauge, allow_area);

    let inject_gauge = Gauge::default()
        .gauge_style(Style::new().fg(Color::Yellow).bg(Color::DarkGray))
        .percent(inject_pct)
        .label(Span::styled(
            format!(" Inject {} ({inject_pct}%)", app.inject_count),
            Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        ));
    frame.render_widget(inject_gauge, inject_area);

    let block_gauge = Gauge::default()
        .gauge_style(Style::new().fg(Color::Red).bg(Color::DarkGray))
        .percent(block_pct)
        .label(Span::styled(
            format!(" Block {} ({block_pct}%)", app.block_count),
            Style::new().fg(Color::Red).add_modifier(Modifier::BOLD),
        ));
    frame.render_widget(block_gauge, block_area);

    // ── Event list
    let len = app.events.len();
    let list_height = list_area.height.saturating_sub(2) as usize;
    let max_visible = list_height.max(1);

    let skip = len.saturating_sub(max_visible + app.scroll_offset);

    let items: Vec<ListItem> = app
        .events
        .iter()
        .skip(skip)
        .take(max_visible)
        .enumerate()
        .map(|(i, ev)| {
            let (bg, fg) = verdict_colors(&ev.verdict);
            let index = skip + i + 1;
            let time = fmt_timestamp(ev.timestamp);
            let tool = &ev.tool_name;
            let tilt = ev.tilt_index.map(|t| format!("{:.2}", t)).unwrap_or("-".into());
            let tokens_val = ev.original_tokens.map(|t| t.to_string()).unwrap_or("-".into());
            let latency = fmt_latency(ev.latency_us);

            let verdict_tag = format!(" {} ", ev.verdict.to_uppercase());
            let line = Line::from(vec![
                Span::raw(format!(" {:>3} ", index)).dim(),
                Span::raw(" "),
                Span::raw(time).dim(),
                Span::raw("  "),
                Span::raw(format!("{:<12}", tool)).bold(),
                Span::raw("  "),
                Span::styled(verdict_tag.clone(), Style::new().bg(bg).fg(fg).add_modifier(Modifier::BOLD)),
                Span::raw(format!("  tilt:{tilt}  tok:{tokens_val}  {latency}")),
            ]);
            ListItem::new(line)
        })
        .collect();

    // Token savings summary line
    let savings_line = if app.total_original_tokens > 0 {
        let saved = app.total_original_tokens.saturating_sub(app.total_compressed_tokens);
        let ratio = saved as f64 / app.total_original_tokens as f64 * 100.0;
        let saved_str = if saved >= 1_000_000 {
            format!("{:.1}M", saved as f64 / 1_000_000.0)
        } else if saved >= 1_000 {
            format!("{:.1}K", saved as f64 / 1_000.0)
        } else {
            format!("{saved}")
        };
        let summary = format!(
            "   Tokens saved: {:>8} ({:.0}%)   Original: {}  →  Compressed: {}",
            saved_str,
            ratio,
            app.total_original_tokens,
            app.total_compressed_tokens,
        );
        ListItem::new(Line::from(Span::raw(summary)))
            .style(Style::new().bg(Color::DarkGray).fg(Color::White).add_modifier(Modifier::DIM))
    } else {
        ListItem::new(Line::from(Span::raw("   Waiting for data...").dim()))
    };

    let all_items: Vec<ListItem> = items
        .into_iter()
        .chain(std::iter::once(savings_line))
        .collect();

    let list = List::new(all_items)
        .block(Block::bordered().title(" Events ").borders(Borders::ALL));
    frame.render_widget(list, list_area);

    // ── Status bar
    let scroll_hint = if app.auto_scroll {
        Span::raw("Auto-scroll ON").dim()
    } else {
        Span::raw("Scrolled ↑ ↓ — END to auto-scroll").yellow()
    };

    let status = if let Some(ref err) = app.error {
        Line::from(vec![
            Span::raw(" ERROR ").red().bold(),
            Span::raw(format!(" {err} ")),
        ])
    } else if app.connected {
        Line::from(vec![
            Span::raw(" ● ").green(),
            Span::raw("Connected").green(),
            Span::raw(format!("  •  {} events  ", app.total)),
            scroll_hint,
            Span::raw("  •  q to quit"),
        ])
    } else {
        Line::from(vec![
            Span::raw(" ● ").yellow(),
            Span::raw("Connecting...").yellow(),
        ])
    };
    frame.render_widget(Paragraph::new(status), status_area);
}

// ── Main event loop

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let mut proxy_url = "http://localhost:8080".to_string();
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--proxy-port" => {
                i += 1;
                if i < args.len() {
                    proxy_url = format!("http://localhost:{}", args[i]);
                }
            }
            _ => {
                if !args[i].starts_with("--") {
                    proxy_url = args[i].clone();
                }
            }
        }
        i += 1;
    }

    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic| {
        let _ = crossterm::terminal::disable_raw_mode();
        let _ = crossterm::execute!(std::io::stdout(), crossterm::terminal::LeaveAlternateScreen);
        original_hook(panic);
    }));

    crossterm::terminal::enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    crossterm::execute!(stdout, crossterm::terminal::EnterAlternateScreen)?;
    let mut terminal = ratatui::Terminal::new(ratatui::backend::CrosstermBackend::new(stdout))?;
    terminal.clear()?;

    let mut app = App::new(proxy_url.clone());
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<ProxyEvent>();
    start_sse_stream(&proxy_url, tx);

    let mut last_tick = Instant::now();
    let tick_rate = Duration::from_millis(50);

    loop {
        let now = Instant::now();
        if now - last_tick >= tick_rate {
            terminal.draw(|f| render_ui(f, &mut app))?;
            last_tick = now;
        }

        while let Ok(event) = rx.try_recv() {
            match event.verdict.as_str() {
                "__connected__" => { app.connected = true; app.error = None; }
                "__error__" => {
                    app.connected = false;
                    app.error = Some(event.tool_name.strip_prefix("__error__:").unwrap_or("unknown").to_string());
                }
                _ => { app.connected = true; app.push_event(event); }
            }
        }

        if event::poll(Duration::from_millis(10))?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => break,
                        KeyCode::Up => {
                            app.auto_scroll = false;
                            app.scroll_offset = (app.scroll_offset + 1).min(99);
                        }
                        KeyCode::Down => {
                            if app.scroll_offset > 0 {
                                app.scroll_offset -= 1;
                            } else {
                                app.auto_scroll = true;
                            }
                        }
                        KeyCode::PageUp => {
                            app.auto_scroll = false;
                            app.scroll_offset = (app.scroll_offset + 10).min(99);
                        }
                        KeyCode::PageDown => {
                            app.scroll_offset = app.scroll_offset.saturating_sub(10);
                            if app.scroll_offset == 0 {
                                app.auto_scroll = true;
                            }
                        }
                        KeyCode::End => { app.scroll_offset = 0; app.auto_scroll = true; }
                        KeyCode::Home => { app.auto_scroll = false; app.scroll_offset = 99; }
                        _ => {}
                    }
        }
    }

    crossterm::terminal::disable_raw_mode()?;
    crossterm::execute!(terminal.backend_mut(), crossterm::terminal::LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}
