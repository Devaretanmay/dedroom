//! Unified YAML configuration for DedrooM.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DedrooMConfig {
    #[serde(default)]
    pub loop_detection: LoopDetectionConfig,
    #[serde(default)]
    pub compression: CompressionConfig,
    #[serde(default = "default_coupling")]
    pub loop_compression_coupling: LoopCompressionCoupling,
    #[serde(default = "default_security")]
    pub security: SecurityConfig,
    #[serde(default = "default_healing")]
    pub self_healing: SelfHealingConfig,
}

impl DedrooMConfig {
    pub fn from_yaml_str(yaml: &str) -> Result<Self, serde_yaml::Error> {
        serde_yaml::from_str(yaml)
    }
    pub fn from_yaml_path(path: impl AsRef<std::path::Path>) -> Result<Self, anyhow::Error> {
        let contents = std::fs::read_to_string(path.as_ref())?;
        Ok(serde_yaml::from_str(&contents)?)
    }
}

impl Default for DedrooMConfig {
    fn default() -> Self {
        Self {
            loop_detection: LoopDetectionConfig::default(),
            compression: CompressionConfig::default(),
            loop_compression_coupling: LoopCompressionCoupling {
                enabled: true,
                on_detected: LoopCouplingAction { compression_budget: CompressionBudget::Aggressive, inject_hint: false, hint_template: None },
                on_error_loop: LoopCouplingAction {
                    compression_budget: CompressionBudget::Maximum,
                    inject_hint: true,
                    hint_template: Some("You are looping on '{tool}'. Try a completely different approach.".into()),
                },
                on_recovery: RecoveryCouplingAction { compression_budget: CompressionBudget::Moderate, fresh_context_window: 3 },
            },
            security: SecurityConfig {
                redaction_enabled: true, context_detection: true,
                audit_log: true, custom_patterns: Vec::new(),
            },
            self_healing: default_healing(),
        }
    }
}

// ── Loop Detection ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoopDetectionConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_max_repeats")]
    pub max_repeats: u32,
    pub history_window: Option<u32>,
    #[serde(default)]
    pub strictness: Strictness,
    #[serde(default)]
    pub count_mode: CountMode,
    #[serde(default = "default_adaptive")]
    pub adaptive: AdaptiveConfig,
    #[serde(default = "default_volatile_fields")]
    pub volatile_fields: VolatileFieldConfig,
    #[serde(default)]
    pub tools: Vec<ToolOverride>,
    #[serde(default)]
    pub rules: Vec<RuleConfig>,
    #[serde(default = "default_history_backend")]
    pub history_backend: String,
    pub history_path: Option<String>,
}

impl Default for LoopDetectionConfig {
    fn default() -> Self {
        Self {
            enabled: true, max_repeats: 3, history_window: None,
            strictness: Strictness::Balanced, count_mode: CountMode::All,
            adaptive: default_adaptive(),
            volatile_fields: default_volatile_fields(),
            tools: Vec::new(), rules: Vec::new(),
            history_backend: "memory".into(), history_path: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Strictness { Lenient, #[default] Balanced, Strict }
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum CountMode { #[default] All, ErrorsOnly }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdaptiveConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_error_reduction")]
    pub error_reduction: u32,
    #[serde(default = "default_min_repeats")]
    pub min_repeats: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VolatileFieldConfig {
    #[serde(default)]
    pub configured: Vec<ConfiguredVolatileField>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfiguredVolatileField {
    pub tool: String,
    pub fields: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolOverride {
    pub name: String,
    pub max_repeats: Option<u32>,
    pub count_mode: Option<CountMode>,
    #[serde(default)]
    pub volatile_fields: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleConfig {
    pub tool: String,
    #[serde(flatten)]
    pub kind: RuleKind,
    #[serde(default)]
    pub on_match: RuleAction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum RuleKind {
    Regex { pattern: String },
    Exact { value: String },
    RequiredFields { fields: Vec<String> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum RuleAction { #[default] Allow, Warn, Block }

// ── Compression ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompressionConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub content_router: ContentRouterConfig,
    #[serde(default = "default_compressors")]
    pub compressors: CompressorsConfig,
    #[serde(default = "default_ccr_cfg")]
    pub ccr: CcrConfig,
}

impl Default for CompressionConfig {
    fn default() -> Self {
        Self { enabled: true, content_router: ContentRouterConfig::default(), compressors: default_compressors(), ccr: default_ccr_cfg() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentRouterConfig {
    #[serde(default = "default_max_input_tokens")]
    pub max_input_tokens: u64,
    #[serde(default = "default_true")]
    pub append_only: bool,
}

impl Default for ContentRouterConfig {
    fn default() -> Self {
        Self { max_input_tokens: 100_000, append_only: true }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompressorsConfig {
    #[serde(default = "default_true")]
    pub smart_crusher: bool,
    #[serde(default = "default_true")]
    pub code_compressor: bool,
    #[serde(default = "default_true")]
    pub log_compressor: bool,
    #[serde(default)]
    pub text_compressor: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CcrConfig {
    #[serde(default = "default_ccr_backend")]
    pub backend: String,
    #[serde(default = "default_ccr_ttl")]
    pub ttl_seconds: u64,
    pub path: Option<String>,
}

// ── Loop–Compression Coupling ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoopCompressionCoupling {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub on_detected: LoopCouplingAction,
    #[serde(default)]
    pub on_error_loop: LoopCouplingAction,
    #[serde(default)]
    pub on_recovery: RecoveryCouplingAction,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LoopCouplingAction {
    pub compression_budget: CompressionBudget,
    pub inject_hint: bool,
    pub hint_template: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RecoveryCouplingAction {
    pub compression_budget: CompressionBudget,
    pub fresh_context_window: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum CompressionBudget { #[default] Normal, Moderate, Aggressive, Maximum }

// ── Security ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityConfig {
    #[serde(default = "default_true")]
    pub redaction_enabled: bool,
    #[serde(default = "default_true")]
    pub context_detection: bool,
    #[serde(default = "default_true")]
    pub audit_log: bool,
    #[serde(default)]
    pub custom_patterns: Vec<String>,
}

// ── Self-Healing ────────────────────────────────────────────────────────────

/// Mode of operation for the self-healing engine.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
pub enum HealingMode {
    /// Only suggest mutations when very confident.
    Conservative,
    /// Suggest mutations when reasonably confident (default).
    #[default]
    Balanced,
    /// Always suggest a mutation on any loop detection.
    Aggressive,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelfHealingConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub mode: HealingMode,
    /// Backend for healing memory: "memory" (default) or "sqlite".
    #[serde(default = "default_memory_backend")]
    pub memory_backend: String,
    /// Path to SQLite database for healing memory (only used when `memory_backend` is "sqlite").
    pub memory_path: Option<String>,
}

// ── Default helpers ────────────────────────────────────────────────────────

fn default_true() -> bool { true }
fn default_max_repeats() -> u32 { 3 }
fn default_error_reduction() -> u32 { 1 }
fn default_min_repeats() -> u32 { 2 }
fn default_max_input_tokens() -> u64 { 100_000 }
fn default_history_backend() -> String { "memory".into() }
fn default_ccr_backend() -> String { "memory".into() }
fn default_ccr_ttl() -> u64 { 1800 }


fn default_adaptive() -> AdaptiveConfig {
    AdaptiveConfig { enabled: true, error_reduction: 1, min_repeats: 2 }
}
fn default_volatile_fields() -> VolatileFieldConfig {
    VolatileFieldConfig { configured: Vec::new() }
}
fn default_compressors() -> CompressorsConfig {
    CompressorsConfig { smart_crusher: true, code_compressor: true, log_compressor: true, text_compressor: false }
}
fn default_ccr_cfg() -> CcrConfig {
    CcrConfig { backend: "memory".into(), ttl_seconds: 1800, path: None }
}

fn default_coupling() -> LoopCompressionCoupling {
    LoopCompressionCoupling {
        enabled: true,
        on_detected: LoopCouplingAction { compression_budget: CompressionBudget::Aggressive, inject_hint: false, hint_template: None },
        on_error_loop: LoopCouplingAction {
            compression_budget: CompressionBudget::Maximum,
            inject_hint: true,
            hint_template: Some("You are looping on '{tool}'. Try a completely different approach.".into()),
        },
        on_recovery: RecoveryCouplingAction { compression_budget: CompressionBudget::Moderate, fresh_context_window: 3 },
    }
}
fn default_security() -> SecurityConfig {
    SecurityConfig { redaction_enabled: true, context_detection: true, audit_log: true, custom_patterns: Vec::new() }
}
fn default_memory_backend() -> String { "memory".into() }

fn default_healing() -> SelfHealingConfig {
    SelfHealingConfig { enabled: true, mode: HealingMode::Balanced, memory_backend: "memory".into(), memory_path: None }
}
