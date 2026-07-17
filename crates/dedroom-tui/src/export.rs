//! Weekly summary export — generates markdown reports from dashboard data.
//!
//! `export_weekly_summary(port, path)` fetches the live stats, attribution,
//! and learning reports from the proxy and writes a real markdown summary.

use anyhow::Result;
use std::fmt::Write as _;

use crate::api::DashboardApi;

/// Export a weekly summary markdown file from the proxy data.
pub async fn export_weekly_summary(port: u16, path: &str) -> Result<String> {
    let client = DashboardApi::new(port);
    let stats = client.fetch_stats().await.ok();
    let attribution = client.fetch_attribution().await.ok();
    let learning = client.fetch_learning().await.ok();

    let mut md = String::new();
    writeln!(md, "# DedrooM Summary\n")?;
    writeln!(md, "_Generated from proxy on port {port}_\n")?;

    if let Some(s) = &stats {
        writeln!(md, "## Savings")?;
        writeln!(
            md,
            "- Compression savings: {} tokens",
            s.savings.total_compression_savings_tokens
        )?;
        writeln!(
            md,
            "- Loop-block savings: {} tokens",
            s.savings.total_loop_savings_tokens
        )?;
        writeln!(md, "- Calls blocked: {}", s.savings.total_calls_blocked)?;
        writeln!(md, "- Total calls tracked: {}", s.loop_state.total_calls)?;
        writeln!(md, "- max_repeats: {}", s.config.max_repeats)?;
        writeln!(md)?;
    }

    if let Some(a) = &attribution {
        writeln!(md, "## Attribution")?;
        writeln!(md, "- Tokens processed: {}", a.total_tokens_processed)?;
        writeln!(md, "- Tokens saved: {}", a.total_tokens_saved)?;
        writeln!(md, "- Savings ratio: {:.1}%", a.savings_ratio * 100.0)?;
        writeln!(md, "- Estimated cost saved: ${:.4}", a.estimated_cost_saved_usd)?;
        writeln!(md, "- Uptime: {}s", a.uptime_seconds)?;
        writeln!(md)?;
        if !a.per_tool.is_empty() {
            writeln!(md, "### Per-tool")?;
            writeln!(md, "| Tool | Calls | Tokens Saved | Blocked | Errors |")?;
            writeln!(md, "| --- | --- | --- | --- | --- |")?;
            for t in &a.per_tool {
                writeln!(
                    md,
                    "| {} | {} | {} | {} | {} |",
                    t.tool, t.call_count, t.tokens_saved, t.blocked_count, t.error_count
                )?;
            }
            writeln!(md)?;
        }
    }

    if let Some(l) = &learning {
        writeln!(md, "## Healing / Learning")?;
        if let Some(stats) = &l.stats {
            writeln!(md, "- Total records: {}", stats.total_records)?;
            writeln!(md, "- Success rate: {:.1}%", stats.overall_success_rate * 100.0)?;
        }
        writeln!(md)?;
    }

    if stats.is_none() && attribution.is_none() {
        writeln!(
            md,
            "> No data available — is the proxy running on port {port}?"
        )?;
    }

    std::fs::write(path, &md)
        .map_err(|e| anyhow::anyhow!("failed to write export to {path}: {e}"))?;

    Ok(md)
}
