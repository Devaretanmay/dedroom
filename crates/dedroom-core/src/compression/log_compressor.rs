//! LogCompressor — structured log/CLI output compression.
//!
//! Deduplicates repeated log lines, preserves error/warning lines, and
//! compresses repetitive patterns.

use std::collections::HashSet;

/// Compress log/CLI output by deduplicating and summarizing.
pub fn compress_logs(input: &str) -> String {
    let lines: Vec<&str> = input.lines().collect();
    if lines.len() < 3 {
        return input.to_string();
    }

    let mut seen_lines: HashSet<&str> = HashSet::new();
    let mut unique_lines: Vec<&str> = Vec::new();
    let mut skipped_count = 0usize;

    for line in &lines {
        let trimmed = line.trim();

        // Always keep errors, warnings, and important markers
        if is_important(trimmed) {
            unique_lines.push(line);
            continue;
        }

        // Deduplicate repeated lines
        if seen_lines.contains(trimmed) {
            skipped_count += 1;
        } else {
            seen_lines.insert(trimmed);
            unique_lines.push(line);
        }
    }

    let mut result = unique_lines.join("\n");

    if skipped_count > 0 {
        result.push_str(&format!("\n[... {} duplicate lines suppressed ...]", skipped_count));
    }

    result
}

/// Check if a log line is important (error, warning, failure).
fn is_important(line: &str) -> bool {
    let upper = line.to_uppercase();
    upper.contains("ERROR")
        || upper.contains("FATAL")
        || upper.contains("PANIC")
        || upper.contains("WARN")
        || upper.contains("EXCEPTION")
        || upper.contains("TRACE")
        // Coding tool patterns (often lowercase or bracketed)
        || upper.contains("FAIL") // catches FAIL, FAILED, FAILURE, FAIL:
        || line.contains("error[")
        || line.contains("warning[")
        || line.contains("panic!")
        || line.contains("panicked")
        || line.starts_with(">>>")
        || line.starts_with("===")
        || line.starts_with("---")
        || line.contains("exit code")
        || line.contains("Exit code")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_short_log_preserved() {
        assert_eq!(compress_logs("line 1\nline 2"), "line 1\nline 2");
    }

    #[test]
    fn test_deduplication() {
        let input = "INFO: starting\nINFO: processing\nINFO: processing\nINFO: processing\nINFO: done";
        let result = compress_logs(input);
        assert!(result.contains("[... 2 duplicate lines suppressed ...]"));
    }

    #[test]
    fn test_errors_preserved() {
        let input = "INFO: step 1\nERROR: something failed\nINFO: step 1\nINFO: step 1";
        let result = compress_logs(input);
        assert!(result.contains("ERROR: something failed"));
        // Duplicate INFO lines should be compressed
        assert!(result.contains("suppressed"));
    }

    #[test]
    fn test_all_unique_lines_no_suppression() {
        let input = "a\nb\nc\nd\ne";
        let result = compress_logs(input);
        assert!(!result.contains("suppressed"));
    }
}
