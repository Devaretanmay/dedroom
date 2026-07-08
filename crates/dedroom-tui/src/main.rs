//! DedrooM TUI Dashboard — real-time savings, loop detection, and healing stats.
//!
//! Usage:
//!     dedroom dash                 # Launch TUI dashboard
//!     dedroom dash --port 9090     # Connect to proxy on custom port
//!     dedroom dash --export report.md  # Export weekly summary

use std::io;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::prelude::{CrosstermBackend, Terminal};
use tokio::sync::Mutex;

mod api;
mod app;
mod charts;
mod components;
mod events;
mod export;
mod ui;

pub struct DashArgs {
    pub port: u16,
    pub export: Option<String>,
}

fn parse_args() -> Result<DashArgs> {
    let args: Vec<String> = std::env::args().collect();
    let mut port = 8080u16;
    let mut export = None;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--port" => {
                i += 1;
                if i < args.len() {
                    port = args[i].parse().unwrap_or(8080);
                }
            }
            "--export" => {
                i += 1;
                if i < args.len() {
                    export = Some(args[i].clone());
                }
            }
            "--help" | "-h" => {
                eprintln!("Usage: dedroom dash [--port <N>] [--export <file>]");
                eprintln!();
                eprintln!("Options:");
                eprintln!("  --port <N>     Proxy port to connect to (default: 8080)");
                eprintln!("  --export <f>   Export weekly summary to file (markdown)");
                std::process::exit(0);
            }
            _ => {}
        }
        i += 1;
    }

    Ok(DashArgs { port, export })
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = parse_args()?;

    // Handle export mode (non-interactive)
    if let Some(path) = &args.export {
        export::export_weekly_summary(args.port, path).await?;
        eprintln!("Weekly summary exported to {}", path);
        return Ok(());
    }

    // Interactive TUI mode
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    // Application state (wrapped in Arc<Mutex> for SSE background task)
    let app = Arc::new(Mutex::new(app::App::new(args.port)));
    {
        let mut guard = app.lock().await;
        guard.refresh().await;
    }

    // Start SSE event listener in background
    events::start_event_listener(app.clone(), args.port).await;

    let tick_rate = Duration::from_millis(500);
    let mut show_help = false;

    loop {
        // Draw
        {
            let guard = app.lock().await;
            terminal.draw(|f| {
                if show_help {
                    ui::render_help_overlay(f, f.area());
                } else {
                    ui::render(f, &guard);
                }
            })?;
        }

        // Handle input
        if event::poll(tick_rate)? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    let mut guard = app.lock().await;

                    match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => {
                            if show_help {
                                show_help = false;
                            } else {
                                break;
                            }
                        }
                        KeyCode::Char('?') => {
                            show_help = !show_help;
                        }
                        KeyCode::Char('r') | KeyCode::F(5) => {
                            guard.refresh().await;
                        }
                        KeyCode::Tab => {
                            guard.next_tab();
                        }
                        KeyCode::BackTab => {
                            guard.prev_tab();
                        }
                        KeyCode::Char('z') | KeyCode::Char('Z') => {
                            drop(guard);
                            ui::cycle_zoom();
                        }
                        KeyCode::Char('e') | KeyCode::Char('E') => {
                            // Export: write to default path
                            let export_path = "dedroom-weekly-summary.md";
                            drop(guard);
                            match export::export_weekly_summary(args.port, export_path).await {
                                Ok(report) => {
                                    let mut g = app.lock().await;
                                    g.set_error(format!(
                                        "Exported to {export_path}\n{report}"
                                    ));
                                }
                                Err(e) => {
                                    let mut g = app.lock().await;
                                    g.set_error(format!("Export failed: {e}"));
                                }
                            }
                        }
                        KeyCode::Char(c @ '1'..='5') => {
                            let tab_idx = (c as u8 - b'1') as usize;
                            if let Some(tab) = app::Tab::ALL.get(tab_idx) {
                                guard.go_to_tab(*tab);
                            }
                        }
                        KeyCode::Char(' ') => {
                            // Space: force refresh
                            guard.refresh().await;
                        }
                        _ => {}
                    }
                }
            }
        }

        // Tick: refresh data if interval elapsed
        {
            let mut guard = app.lock().await;
            guard.tick().await;
        }
    }

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    Ok(())
}
