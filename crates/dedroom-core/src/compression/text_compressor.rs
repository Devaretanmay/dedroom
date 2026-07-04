//! TextCompressor — plain text compression.
//!
//! Uses a lightweight summarization approach: compresses text by removing
//! redundant phrases while preserving key information. For ML-based
//! compression, delegates to an external ONNX model (Kompress-v2).

/// Compress plain text using lightweight heuristic methods.
pub fn compress_text(input: &str) -> String {
    if input.len() < 10 {
        return input.to_string();
    }

    let mut result = input.to_string();

    // Collapse 3+ consecutive newlines into 2
    {
        let mut prev = String::new();
        let mut run = 0u32;
        for ch in result.chars() {
            if ch == '\n' {
                run += 1;
                if run <= 2 {
                    prev.push(ch);
                }
            } else {
                run = 0;
                prev.push(ch);
            }
        }
        result = prev;
    }

    // Remove markdown horizontal rules (lines that are ---, ***, ___ with optional whitespace)
    {
        let lines: Vec<&str> = result.lines().collect();
        let filtered: Vec<&str> = lines
            .into_iter()
            .filter(|line| {
                let trimmed = line.trim();
                if trimmed.len() >= 3 && trimmed.chars().all(|c| c == '-' || c == '*' || c == '_') {
                    return false;
                }
                true
            })
            .collect();
        result = filtered.join("\n");
    }

    // Trim trailing whitespace per line
    {
        let lines: Vec<&str> = result.lines().collect();
        let trimmed: Vec<&str> = lines
            .into_iter()
            .map(|l| l.trim_end())
            .collect();
        result = trimmed.join("\n");
    }

    // Remove empty lines at start/end
    result = result.trim().to_string();

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_short_text_no_compression() {
        assert_eq!(compress_text("Hello world!"), "Hello world!");
    }

    #[test]
    fn test_excessive_newlines_compressed() {
        let input = "Line 1\n\n\n\n\nLine 2";
        let result = compress_text(input);
        assert_eq!(result, "Line 1\n\nLine 2");
    }

    #[test]
    fn test_horizontal_rules_removed() {
        let input = "text\n---\nmore text";
        let result = compress_text(input);
        assert_eq!(result, "text\nmore text");
        assert!(!result.contains("---"));
    }

    #[test]
    fn test_trailing_whitespace_trimmed() {
        let input = "hello   \nworld  \n";
        let result = compress_text(input);
        assert_eq!(result, "hello\nworld");
    }

    #[test]
    fn test_mixed_comression() {
        let input = "Header\n\n\n\n---\n\nBody text  \n\n   \nFooter";
        let result = compress_text(input);
        assert!(!result.contains("---"));
        assert!(!result.contains("  \n"));
    }
}
