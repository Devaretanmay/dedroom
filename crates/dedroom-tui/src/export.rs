//! Weekly summary export — generates markdown reports from dashboard data.
//!
//! TODO: Implement `export_weekly_summary(port, path)` that fetches
//! the attribution report and formats it as a beautiful markdown
//! document with savings highlights, top tools, and healing stats.

use anyhow::Result;

/// Export a weekly summary markdown file from the proxy data.
pub async fn export_weekly_summary(_port: u16, path: &str) -> Result<String> {
    Ok(format!(
        "# DedrooM Weekly Summary\n\n\
         Export not yet implemented. File would be written to: {path}\n"
    ))
}
