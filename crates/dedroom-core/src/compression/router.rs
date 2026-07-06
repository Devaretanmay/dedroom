//! Content router — detects content type and dispatches to compressors.

use super::ContentType;
use crate::config::ContentRouterConfig;

/// Routes content blocks to the appropriate compressor.
#[derive(Debug)]
pub struct ContentRouter {
    config: ContentRouterConfig,
}

impl ContentRouter {
    pub fn new(config: &ContentRouterConfig) -> Self {
        Self { config: config.clone() }
    }

    /// Detect the content type of a string.
    pub fn detect_type(&self, content: &str) -> ContentType {
        self.detect_type_with_value(content).0
    }

    /// Detect content type, returning the parsed JSON `Value` when the
    /// content is valid JSON so the caller can reuse it and avoid a
    /// second `serde_json::from_str`.
    pub fn detect_type_with_value(&self, content: &str) -> (ContentType, Option<serde_json::Value>) {
        // Try parsing as JSON — return the parsed value for reuse
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(content) {
            let ct = match &val {
                serde_json::Value::Array(_) => ContentType::JsonArray,
                _ => ContentType::JsonObject,
            };
            return (ct, Some(val));
        }

        // Check for code-like patterns
        if looks_like_code(content) {
            return (ContentType::Code, None);
        }

        // Check for log patterns
        if looks_like_logs(content) {
            return (ContentType::Log, None);
        }

        (ContentType::Text, None)
    }

    /// Maximum input tokens before truncation.
    pub fn max_input_tokens(&self) -> u64 {
        self.config.max_input_tokens
    }

    /// Whether to only compress the latest message + tool result.
    pub fn append_only(&self) -> bool {
        self.config.append_only
    }
}

/// Heuristic: looks like source code (has braces, semicolons, keywords).
fn looks_like_code(content: &str) -> bool {
    let code_indicators = [
        "fn ", "def ", "function ", "class ", "impl ", "import ",
        "pub ", "let ", "const ", "var ", "if ", "else ", "for ", "while ",
        "return ", "match ", "use ", "mod ", "struct ", "enum ", "trait ",
        "async ", "await ", "pub fn", "pub struct",
    ];
    let line_count = content.lines().count();
    if line_count < 2 {
        return false;
    }
    let first_lines: Vec<&str> = content.lines().take(5).collect();
    let first_joined = first_lines.join("\n");
    code_indicators.iter().any(|kw| first_joined.contains(kw))
}

/// Heuristic: looks like log/CLI output (timestamps, levels).
fn looks_like_logs(content: &str) -> bool {
    let log_indicators = [
        "[INFO]", "[WARN]", "[ERROR]", "[DEBUG]", "[TRACE]",
        "INFO:", "WARN:", "ERROR:", "DEBUG:",
        "202", "2024", "2025", "2026", // years in timestamps
    ];
    let lines: Vec<&str> = content.lines().filter(|l| !l.is_empty()).collect();
    if lines.len() < 2 {
        return false;
    }
    let sample: Vec<&str> = lines.iter().take(5).copied().collect();
    let sample_str = sample.join(" ");
    let has_timestamp = sample_str.contains("T") || sample_str.contains('-') && sample_str.contains(':');
    let has_indicator = log_indicators.iter().any(|kw| sample_str.contains(kw));
    has_timestamp || has_indicator
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_json_object() {
        let router = ContentRouter::new(&ContentRouterConfig::default());
        assert_eq!(router.detect_type(r#"{"key": "value"}"#), ContentType::JsonObject);
    }

    #[test]
    fn test_detect_json_array() {
        let router = ContentRouter::new(&ContentRouterConfig::default());
        assert_eq!(router.detect_type(r#"[{"id":1},{"id":2}]"#), ContentType::JsonArray);
    }

    #[test]
    fn test_detect_code_rust() {
        let router = ContentRouter::new(&ContentRouterConfig::default());
        assert_eq!(router.detect_type("fn main() {\n    println!(\"hello\");\n}"), ContentType::Code);
    }

    #[test]
    fn test_detect_code_python() {
        let router = ContentRouter::new(&ContentRouterConfig::default());
        assert_eq!(router.detect_type("def hello():\n    print('hi')"), ContentType::Code);
    }

    #[test]
    fn test_detect_log() {
        let router = ContentRouter::new(&ContentRouterConfig::default());
        assert_eq!(
            router.detect_type("[INFO] 2024-01-01T12:00:00 Starting service\n[ERROR] Connection refused"),
            ContentType::Log,
        );
    }

    #[test]
    fn test_detect_text() {
        let router = ContentRouter::new(&ContentRouterConfig::default());
        assert_eq!(router.detect_type("Just a plain sentence."), ContentType::Text);
    }


}
