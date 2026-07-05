use std::path::Path;
use anyhow::Result;

pub fn remove_rtk_instructions(path: &Path) -> Result<bool> {
    if !path.exists() {
        return Ok(false);
    }
    let content = std::fs::read_to_string(path)?;
    let start_marker = "<!-- dedroom:rtk-instructions -->";
    let end_marker = "<!-- /dedroom:rtk-instructions -->";

    if let Some(start_idx) = content.find(start_marker) {
        if let Some(end_offset) = content[start_idx..].find(end_marker) {
            let end_idx = start_idx + end_offset + end_marker.len();
            
            // Reconstruct without the block
            let before = &content[..start_idx];
            let after = &content[end_idx..];
            
            let mut new_content = String::new();
            new_content.push_str(before);
            new_content.push_str(after);
            
            let new_content = new_content.trim();
            if new_content.is_empty() {
                std::fs::remove_file(path)?;
            } else {
                std::fs::write(path, new_content)?;
            }
            return Ok(true);
        }
    }
    
    Ok(false)
}
