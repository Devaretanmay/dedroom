//! PII and secret redaction engine.
//!
//! Detects and redacts sensitive information from tool call payloads using:
//!
//! - **Regex patterns** — known secret formats (AWS keys, GitHub tokens, JWTs, etc.)
//! - **Entropy analysis** — high-entropy strings that look like secrets
//! - **Context-aware detection** — field names like `password`, `token`, `secret`
//!
//! Run this **before** compression so secrets are never stored in the CCR cache
//! or sent to the LLM.

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

// ── Configuration ──────────────────────────────────────────────────────────

/// Settings for the PII/secret redaction engine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RedactionConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_entropy_threshold")]
    pub entropy_threshold: f64,
    #[serde(default = "default_true")]
    pub entropy_detection: bool,
    #[serde(default = "default_true")]
    pub context_detection: bool,
    #[serde(default = "default_true")]
    pub audit_log: bool,
    #[serde(default)]
    pub custom_patterns: Vec<CustomPattern>,
    #[serde(default)]
    pub redact_strings: Vec<String>,
    #[serde(default)]
    pub sensitive_fields: Option<Vec<String>>,
}

impl Default for RedactionConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            entropy_threshold: 4.5,
            entropy_detection: true,
            context_detection: true,
            audit_log: true,
            custom_patterns: Vec::new(),
            redact_strings: Vec::new(),
            sensitive_fields: None,
        }
    }
}

/// A user-defined redaction pattern.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomPattern {
    pub name: String,
    pub regex: String,
}

fn default_true() -> bool { true }
fn default_entropy_threshold() -> f64 { 4.5 }

// ── Report ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RedactionReport {
    pub total_redacted: usize,
    pub pattern_matches: usize,
    pub entropy_matches: usize,
    pub context_matches: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<RedactedItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RedactedItem {
    pub method: RedactionMethod,
    pub label: String,
    pub length: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RedactionMethod {
    Pattern,
    Entropy,
    Context,
    Custom,
}

// ── Redaction Engine ───────────────────────────────────────────────────────

#[derive(Debug)]
pub struct RedactionEngine {
    config: RedactionConfig,
    builtin_patterns: Vec<(String, Regex)>,
    custom_patterns: Vec<(String, Regex)>,
    sensitive_fields: HashSet<String>,
}

impl RedactionEngine {
    pub fn new(config: RedactionConfig) -> Self {
        let builtin_patterns = compile_builtin_patterns();
        let custom_patterns = compile_custom_patterns(&config.custom_patterns);
        let sensitive_fields = config
            .sensitive_fields
            .as_ref()
            .map(|fields| fields.iter().cloned().collect())
            .unwrap_or_else(default_sensitive_fields);
        Self { config, builtin_patterns, custom_patterns, sensitive_fields }
    }

    pub fn default_enabled() -> Self {
        Self::new(RedactionConfig::default())
    }

    pub fn disabled() -> Self {
        Self::new(RedactionConfig {
            enabled: false,
            ..Default::default()
        })
    }

    pub fn redact(&self, input: &str) -> (String, RedactionReport) {
        if !self.config.enabled {
            return (input.to_string(), RedactionReport::default());
        }

        let mut output = input.to_string();
        let mut report = RedactionReport::default();

        // 1. Pattern-based redaction (built-in + custom)
        for (label, pattern) in &self.builtin_patterns {
            let matches: Vec<_> = pattern.find_iter(&output).collect();
            if !matches.is_empty() {
                report.pattern_matches += matches.len();
                for m in &matches {
                    report.items.push(RedactedItem {
                        method: RedactionMethod::Pattern,
                        label: label.clone(),
                        length: m.len(),
                    });
                }
                output = pattern.replace_all(&output, "[REDACTED]").to_string();
            }
        }
        for (label, pattern) in &self.custom_patterns {
            let matches: Vec<_> = pattern.find_iter(&output).collect();
            if !matches.is_empty() {
                report.pattern_matches += matches.len();
                for m in &matches {
                    report.items.push(RedactedItem {
                        method: RedactionMethod::Custom,
                        label: label.clone(),
                        length: m.len(),
                    });
                }
                output = pattern.replace_all(&output, "[REDACTED]").to_string();
            }
        }

        // 2. Context-aware redaction
        if self.config.context_detection {
            let (ctx_out, ctx_report) = self.redact_context_aware(&output);
            output = ctx_out;
            report.context_matches = ctx_report.context_matches;
            report.items.extend(ctx_report.items);
        }

        // 3. Entropy-based redaction
        if self.config.entropy_detection {
            let (ent_out, ent_report) = self.redact_high_entropy(&output);
            output = ent_out;
            report.entropy_matches = ent_report.entropy_matches;
            report.items.extend(ent_report.items);
        }

        report.total_redacted = report.items.len();
        if !self.config.audit_log {
            report.items.clear();
        }

        (output, report)
    }

    fn redact_context_aware(&self, input: &str) -> (String, RedactionReport) {
        let mut output = input.to_string();
        let mut report = RedactionReport::default();

        let fields: Vec<&str> = self.sensitive_fields.iter().map(|s| s.as_str()).collect();
        let field_pattern = fields.join("|");

        // Match: "<field>": "value" (JSON style) or <field> = "value"
        // (assignment style like env vars)
        let context_re = match Regex::new(&format!(
            r#"(?i)"?({})"?\s*[:=]\s*"([^"]+)""#,
            field_pattern,
        )) {
            Ok(re) => re,
            Err(_) => return (output, report),
        };

        let mut replacements: Vec<(usize, usize, String)> = Vec::new();
        for cap in context_re.captures_iter(&output) {
            let full_match = cap.get(0).unwrap();
            let field_name = cap.get(1).unwrap();
            let value = cap.get(2).unwrap();

            report.context_matches += 1;
            report.items.push(RedactedItem {
                method: RedactionMethod::Context,
                label: format!("field: {:?}", value.as_str()),
                length: value.len(),
            });

            let replacement = format!("{}[REDACTED]", &output[field_name.start()..value.start()]);
            replacements.push((full_match.start(), value.end(), replacement));
        }

        // Apply right-to-left (reverse order by start position)
        replacements.sort_unstable_by_key(|k| std::cmp::Reverse(k.0));
        for (start, end, replacement) in &replacements {
            output.replace_range(*start..*end, replacement);
        }

        (output, report)
    }

    fn redact_high_entropy(&self, input: &str) -> (String, RedactionReport) {
        let mut report = RedactionReport::default();

        let output = match serde_json::from_str::<serde_json::Value>(input) {
            Ok(value) => {
                let redacted = self.redact_high_entropy_value(&value, &mut report);
                serde_json::to_string(&redacted).unwrap_or_else(|_| input.to_string())
            }
            Err(_) => self.scan_entropy_tokens(input, &mut report),
        };

        (output, report)
    }

    fn redact_high_entropy_value(
        &self,
        value: &serde_json::Value,
        report: &mut RedactionReport,
    ) -> serde_json::Value {
        match value {
            serde_json::Value::String(s) => {
                if self.is_high_entropy(s) && s.len() >= 16 {
                    report.entropy_matches += 1;
                    report.items.push(RedactedItem {
                        method: RedactionMethod::Entropy,
                        label: format!("high-entropy string ({} chars)", s.len()),
                        length: s.len(),
                    });
                    serde_json::Value::String("[REDACTED]".to_string())
                } else {
                    serde_json::Value::String(s.clone())
                }
            }
            serde_json::Value::Object(map) => {
                let mut new_map = serde_json::Map::new();
                for (k, v) in map {
                    if self.sensitive_fields.contains(k) && let serde_json::Value::String(s) = v {
                        report.context_matches += 1;
                        report.items.push(RedactedItem {
                            method: RedactionMethod::Context,
                            label: format!("field: {:?}", k),
                            length: s.len(),
                        });
                        new_map.insert(k.clone(), serde_json::Value::String("[REDACTED]".into()));
                        continue;
                    }
                    new_map.insert(k.clone(), self.redact_high_entropy_value(v, report));
                }
                serde_json::Value::Object(new_map)
            }
            serde_json::Value::Array(arr) => {
                let new_arr: Vec<_> = arr.iter().map(|v| self.redact_high_entropy_value(v, report)).collect();
                serde_json::Value::Array(new_arr)
            }
            other => other.clone(),
        }
    }

    fn scan_entropy_tokens(&self, input: &str, report: &mut RedactionReport) -> String {
        let mut output = input.to_string();

        let tokens: Vec<String> = input
            .split_whitespace()
            .map(|t| t.trim_matches(|c: char| c.is_ascii_punctuation()))
            .filter(|t| t.len() >= 16)
            .map(|t| t.to_string())
            .collect();

        let mut seen = HashSet::new();
        for token in &tokens {
            if seen.contains(token) || !self.is_high_entropy(token) {
                continue;
            }
            seen.insert(token.clone());
            report.entropy_matches += 1;
            report.items.push(RedactedItem {
                method: RedactionMethod::Entropy,
                label: format!("high-entropy token ({} chars)", token.len()),
                length: token.len(),
            });
            output = output.replace(token, "[REDACTED]");
        }

        output
    }

    fn calculate_entropy(&self, s: &str) -> f64 {
        if s.is_empty() {
            return 0.0;
        }
        let bytes = s.as_bytes();
        let len = bytes.len() as f64;

        let mut freq = [0u64; 256];
        for &b in bytes {
            freq[b as usize] += 1;
        }

        let mut entropy = 0.0_f64;
        for &count in freq.iter() {
            if count > 0 {
                let p = count as f64 / len;
                entropy -= p * p.log2();
            }
        }

        entropy
    }

    fn is_high_entropy(&self, s: &str) -> bool {
        s.len() >= 16 && self.calculate_entropy(s) >= self.config.entropy_threshold
    }
}

// ── Built-in patterns ──────────────────────────────────────────────────────

fn compile_builtin_patterns() -> Vec<(String, Regex)> {
    let mut patterns: Vec<(&str, &str)> = Vec::new();

    // AWS Access Key ID
    patterns.push(("AWS Access Key ID", "AKIA[0-9A-Z]{16}"));
    // AWS Secret Access Key
    patterns.push(("AWS Secret Key",
        r#"(?i)aws[_-]?secret[_-]?access[_-]?key\s*[:=]\s*['"]?[a-zA-Z0-9/+=]{40}['"]?"#));
    // GitHub PATs
    patterns.push(("GitHub PAT", "ghp_[a-zA-Z0-9]{36}"));
    patterns.push(("GitHub Fine-Grained PAT", "github_pat_[a-zA-Z0-9]{82}"));
    patterns.push(("GitHub OAuth Token", "gho_[a-zA-Z0-9]{36}"));
    patterns.push(("GitHub Refresh Token", "ghr_[a-zA-Z0-9]{76}"));
    // JWT -- uses double-quote chars in regex matching, but the pattern itself
    // does not contain literal double quotes, only escapes
    patterns.push(("JWT Token",
        "eyJ[a-zA-Z0-9_-]+\\.[a-zA-Z0-9_-]+\\.[a-zA-Z0-9_-]+"));
    // PEM private key
    patterns.push(("Private Key",
        "-----BEGIN\\s+(RSA\\s+)?(EC\\s+)?PRIVATE\\s+KEY-----"));
    // API keys
    patterns.push(("OpenAI API Key", "sk-[a-zA-Z0-9]{20,}"));
    patterns.push(("Anthropic API Key", "sk-ant-[a-zA-Z0-9]{20,}"));
    patterns.push(("Google API Key", "AIza[0-9A-Za-z\\-_]{35}"));
    // Slack
    patterns.push(("Slack Bot Token",
        "xoxb-[0-9]{10,13}-[0-9]{10,13}-[a-zA-Z0-9]{24}"));
    patterns.push(("Slack Webhook URL",
        "https://hooks\\.slack\\.com/services/T[a-zA-Z0-9]{8,10}/B[a-zA-Z0-9]{8,10}/[a-zA-Z0-9]{24}"));
    // Heroku
    patterns.push(("Heroku API Key",
        r#"(?i)(heroku.*api.*key|heroku.*auth)\s*[:=]\s*['"]?[a-zA-Z0-9-]{20,}['"]?"#));
    // Generic API key
    patterns.push(("Generic API Key",
        r#"(?i)(api[_-]?key|apikey|api_secret)\s*[:=]\s*['"]?[a-zA-Z0-9]{20,}['"]?"#));

    patterns
        .into_iter()
        .filter_map(|(label, pattern)| {
            Regex::new(pattern).ok().map(|re| (label.to_string(), re))
        })
        .collect()
}

fn compile_custom_patterns(custom: &[CustomPattern]) -> Vec<(String, Regex)> {
    custom
        .iter()
        .filter_map(|cp| Regex::new(&cp.regex).ok().map(|re| (cp.name.clone(), re)))
        .collect()
}

fn default_sensitive_fields() -> HashSet<String> {
    HashSet::from([
        String::from("password"), String::from("passwd"),
        String::from("secret"), String::from("token"),
        String::from("api_key"), String::from("apikey"),
        String::from("api_secret"), String::from("api_key_secret"),
        String::from("access_token"), String::from("access_token_secret"),
        String::from("refresh_token"), String::from("auth_token"),
        String::from("private_key"), String::from("privatekey"),
        String::from("client_secret"), String::from("client_secret_value"),
        String::from("session_key"), String::from("session_secret"),
        String::from("encryption_key"), String::from("encryption_key_id"),
        String::from("connection_string"), String::from("conn_string"),
        String::from("db_password"), String::from("db_passwd"),
        String::from("ssh_key"), String::from("ssh_private_key"),
        String::from("pem"), String::from("certificate_private_key"),
        String::from("jwt"), String::from("jwt_token"),
        String::from("authorization"), String::from("authorization_token"),
        String::from("bearer"), String::from("bearer_token"),
        String::from("oauth_token"), String::from("oauth_secret"),
        String::from("consumer_key"), String::from("consumer_secret"),
    ])
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_entropy_calculation() {
        let engine = RedactionEngine::default_enabled();
        let low = engine.calculate_entropy("aaaaaaaaaaaaaaaa");
        assert!(low < 1.0, "low entropy expected, got {low}");
        let high = engine.calculate_entropy("aB3$xK9#mP2&qR7@vW5");
        assert!(high > 4.0, "high entropy expected, got {high}");
    }

    #[test]
    fn test_redact_aws_key() {
        let engine = RedactionEngine::default_enabled();
        let (out, report) = engine.redact("Use key AKIAIOSFODNN7EXAMPLE3 to connect.");
        assert!(!out.contains("AKIAIOSFODNN7EXAMPLE3"));
        assert!(out.contains("[REDACTED]"));
        assert_eq!(report.pattern_matches, 1);
    }

    #[test]
    fn test_redact_jwt() {
        let engine = RedactionEngine::default_enabled();
        let jwt = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNqPZNo1YgTgPqHQdE4nTYo";
        let (out, report) = engine.redact(&format!("Bearer {jwt}"));
        assert!(!out.contains("eyJhbGci"), "JWT should be redacted");
        assert_eq!(report.pattern_matches, 1);
    }

    #[test]
    fn test_redact_openai_key() {
        let engine = RedactionEngine::default_enabled();
        let (out, report) = engine.redact("OPENAI_API_KEY=sk-abcdefghijklmnopqrstuvwxyz123456");
        assert!(!out.contains("sk-abcdefghij"));
        assert_eq!(report.pattern_matches, 1);
    }

    #[test]
    fn test_redact_private_key_header() {
        let engine = RedactionEngine::default_enabled();
        let (out, _) = engine.redact("-----BEGIN RSA PRIVATE KEY-----\nMIIEpAIBAAKCAQEA...");
        assert!(!out.contains("BEGIN RSA PRIVATE KEY"));
        assert!(out.contains("[REDACTED]"));
    }

    #[test]
    fn test_context_aware_redaction() {
        let engine = RedactionEngine::default_enabled();
        let input = r#"{"password": "hunter2", "username": "admin"}"#;
        let (out, report) = engine.redact(input);
        assert!(!out.contains("hunter2"), "password should be redacted");
        assert!(out.contains("admin"), "username should remain");
        assert!(report.context_matches >= 1);
    }

    #[test]
    fn test_context_no_false_positive() {
        let engine = RedactionEngine::default_enabled();
        let (out, _) = engine.redact(r#"{"description": "password is common"}"#);
        assert!(!out.contains("[REDACTED]"), "should not redact non-sensitive values");
    }

    #[test]
    fn test_entropy_based_redaction_disabling_patterns() {
        let engine = RedactionEngine {
            config: RedactionConfig {
                entropy_threshold: 3.5,
                context_detection: false,
                ..Default::default()
            },
            builtin_patterns: vec![], custom_patterns: vec![],
            sensitive_fields: HashSet::new(),
        };
        let input = r#"{"key": "aB3$xK9#mP2&qR7@vW5!nL8*yF4"}"#;
        let (out, report) = engine.redact(input);
        assert!(!out.contains("aB3$xK9#mP2&qR7@vW5!nL8*yF4"), "high-entropy should be redacted");
        assert_eq!(report.entropy_matches, 1);
    }

    #[test]
    fn test_disabled_engine() {
        let engine = RedactionEngine::disabled();
        let input = "AKIAIOSFODNN7EXAMPLE3";
        let (out, report) = engine.redact(input);
        assert_eq!(out, input);
        assert_eq!(report.total_redacted, 0);
    }

    #[test]
    fn test_custom_pattern() {
        let config = RedactionConfig {
            enabled: true,
            custom_patterns: vec![CustomPattern {
                name: "Custom".into(),
                regex: r"MY_SECRET_[A-Z0-9]{16}".into(),
            }],
            ..Default::default()
        };
        let engine = RedactionEngine::new(config);
        let (out, report) = engine.redact("MY_SECRET_ABCDEF1234567890");
        assert!(!out.contains("MY_SECRET_ABCDEF1234567890"));
        assert_eq!(report.pattern_matches, 1);
    }

    #[test]
    fn test_no_false_positives_on_clean_text() {
        let engine = RedactionEngine::default_enabled();
        let (out, report) = engine.redact("Hello, this is normal text.");
        assert_eq!(out, "Hello, this is normal text.");
        assert_eq!(report.total_redacted, 0);
    }

    #[test]
    fn test_multiple_redaction_types() {
        let engine = RedactionEngine::default_enabled();
        let jwt = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNqPZNo1YgTgPqHQdE4nTYo";
        let input = format!(r#"{{"api_key":"AKIAIOSFODNN7EXAMPLE3","token":"{}"}}"#, jwt);
        let (_, report) = engine.redact(&input);
        assert!(report.pattern_matches >= 2, "pattern matches: {}", report.pattern_matches);
        assert!(report.context_matches >= 2, "context matches: {}", report.context_matches);
    }

    #[test]
    fn test_audit_log_disabled() {
        let config = RedactionConfig { enabled: true, audit_log: false, ..Default::default() };
        let engine = RedactionEngine::new(config);
        let (_, report) = engine.redact("AKIAIOSFODNN7EXAMPLE3 key");
        assert_eq!(report.total_redacted, 1);
        assert!(report.items.is_empty());
    }

    #[test]
    fn test_nested_json_redacts_sensitive_fields() {
        let engine = RedactionEngine::default_enabled();
        let input = r#"{"credentials": {"password": "secret!","username": "admin"},"config": {"api_key": "sk-abc123def456ghi789jkl012mno345pqr","endpoint": "https://api.example.com"}}"#;
        let (out, report) = engine.redact(input);
        assert!(!out.contains("secret!"), "password redacted");
        assert!(!out.contains("sk-abc123def456ghi789jkl012mno345pqr"), "API key redacted");
        assert!(out.contains("admin"), "username remains");
        assert!(report.pattern_matches >= 1);
        assert!(report.context_matches >= 1);
    }

    #[test]
    fn test_short_string_not_flagged() {
        let engine = RedactionEngine::default_enabled();
        let (_, report) = engine.redact(r#"{"k": "abc"}"#);
        assert_eq!(report.entropy_matches, 0);
    }

    #[test]
    fn test_context_detection_assignment_style() {
        let engine = RedactionEngine::default_enabled();
        let (out, report) = engine.redact(r#"password = "hunter2""#);
        assert!(!out.contains("hunter2"));
        assert_eq!(report.context_matches, 1);
    }

    #[test]
    fn test_redact_anthropic_key() {
        let engine = RedactionEngine::default_enabled();
        let (out, report) = engine.redact("sk-ant-abcdefghijklmnopqrstuvwxyz1234567890");
        assert!(!out.contains("sk-ant-"));
        assert_eq!(report.pattern_matches, 1);
    }

    #[test]
    fn test_each_pattern_compiles() {
        let engine = RedactionEngine::default_enabled();
        // The engine should have compiled all built-in patterns
        assert!(!engine.builtin_patterns.is_empty(),
            "should have compiled built-in patterns, got {}",
            engine.builtin_patterns.len());
        // Verify specific known patterns are present
        let labels: Vec<_> = engine.builtin_patterns.iter().map(|(l, _)| l.as_str()).collect();
        assert!(labels.contains(&"AWS Access Key ID"));
        assert!(labels.contains(&"JWT Token"));
        assert!(labels.contains(&"OpenAI API Key"));
        assert!(labels.contains(&"Anthropic API Key"));
    }
}
