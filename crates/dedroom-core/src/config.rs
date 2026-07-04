//! Unified YAML configuration for DedrooM.
//!
//! A single config file defines both loop detection and compression behavior,
//! plus the coupling policy between them.

use serde::{Deserialize, Serialize};

// ── Top-level ──────────────────────────────────────────────────────────────

/// Root configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DedrooMConfig {
    /// Loop detection settings.
    #[serde(default)]
    pub loop_detection: LoopDetectionConfig,

    /// Context compression settings.
    #[serde(default)]
    pub compression: CompressionConfig,

    /// Policy linking loop state to compression behavior.
    #[serde(default)]
    pub loop_compression_coupling: LoopCompressionCoupling,

    /// Cross-agent memory.
    #[serde(default)]
    pub memory: MemoryConfig,
}

impl DedrooMConfig {
    /// Parse from a YAML string.
    pub fn from_yaml_str(yaml: &str) -> Result<Self, serde_yaml::Error> {
        serde_yaml::from_str(yaml)
    }

    /// Parse from a YAML file.
    pub fn from_yaml_path(path: impl AsRef<std::path::Path>) -> Result<Self, anyhow::Error> {
        let contents = std::fs::read_to_string(path.as_ref())?;
        Ok(serde_yaml::from_str(&contents)?)
    }
}

/// Sensible defaults for the full config.
impl Default for DedrooMConfig {
    fn default() -> Self {
        Self {
            loop_detection: LoopDetectionConfig::default(),
            compression: CompressionConfig::default(),
            loop_compression_coupling: LoopCompressionCoupling::default(),
            memory: MemoryConfig::default(),
        }
    }
}

// ── Loop Detection ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoopDetectionConfig {
    /// Master switch.
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// How many identical calls before blocking. Default 3.
    #[serde(default = "default_max_repeats")]
    pub max_repeats: u32,

    /// How far back to scan for repeats. Default = max_repeats × 2.
    pub history_window: Option<u32>,

    /// Behaviour strictness.
    #[serde(default)]
    pub strictness: Strictness,

    /// Count mode: all repetitions or only error-producing ones.
    #[serde(default)]
    pub count_mode: CountMode,

    /// Adaptive thresholding.
    #[serde(default)]
    pub adaptive: AdaptiveConfig,

    /// Volatile field configuration.
    #[serde(default)]
    pub volatile_fields: VolatileFieldConfig,

    /// Semantic (embedding-based) detection.
    #[serde(default)]
    pub semantic: SemanticConfig,

    /// Per-tool overrides.
    #[serde(default)]
    pub tools: Vec<ToolOverride>,

    /// Argument validation rules.
    #[serde(default)]
    pub rules: Vec<RuleConfig>,
}

impl Default for LoopDetectionConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_repeats: 3,
            history_window: None,
            strictness: Strictness::Balanced,
            count_mode: CountMode::All,
            adaptive: AdaptiveConfig::default(),
            volatile_fields: VolatileFieldConfig::default(),
            semantic: SemanticConfig::default(),
            tools: Vec::new(),
            rules: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Strictness {
    #[serde(rename = "lenient")]
    Lenient,
    #[serde(rename = "balanced")]
    #[default]
    Balanced,
    #[serde(rename = "strict")]
    Strict,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum CountMode {
    #[serde(rename = "all")]
    #[default]
    All,
    #[serde(rename = "errors_only")]
    ErrorsOnly,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdaptiveConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_error_reduction")]
    pub error_reduction: u32,
    #[serde(default = "default_min_repeats")]
    pub min_repeats: u32,
}

impl Default for AdaptiveConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            error_reduction: 1,
            min_repeats: 2,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VolatileFieldConfig {
    #[serde(default = "default_true")]
    pub auto_inference: bool,
    #[serde(default = "default_min_occurrences")]
    pub min_occurrences: u32,
    #[serde(default)]
    pub configured: Vec<ConfiguredVolatileField>,
}

impl Default for VolatileFieldConfig {
    fn default() -> Self {
        Self {
            auto_inference: true,
            min_occurrences: 2,
            configured: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfiguredVolatileField {
    pub tool: String,
    pub fields: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SemanticConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_similarity_threshold")]
    pub similarity_threshold: f32,
    #[serde(default = "default_semantic_window")]
    pub window: usize,
    #[serde(default = "default_embedder")]
    pub embedder: String,
}

impl Default for SemanticConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            similarity_threshold: 0.85,
            window: 5,
            embedder: "auto".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolOverride {
    pub name: String,
    pub max_repeats: Option<u32>,
    pub count_mode: Option<CountMode>,
    #[serde(default)]
    pub volatile_fields: Vec<String>,
    pub error_detection: Option<ErrorDetectionConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorDetectionConfig {
    pub json_path: Option<String>,
    pub status_field: Option<String>,
    pub status_not_in: Option<Vec<i32>>,
    pub regex: Option<String>,
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
    #[serde(rename = "regex")]
    Regex { pattern: String },
    #[serde(rename = "exact")]
    Exact { value: String },
    #[serde(rename = "json_schema")]
    JsonSchema { required: Vec<String>, type_name: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum RuleAction {
    #[serde(rename = "allow")]
    #[default]
    Allow,
    #[serde(rename = "warn")]
    Warn,
    #[serde(rename = "block")]
    Block,
}

// ── Compression ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompressionConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub content_router: ContentRouterConfig,
    #[serde(default)]
    pub compressors: CompressorsConfig,
    #[serde(default)]
    pub cache_aligner: CacheAlignerConfig,
    #[serde(default)]
    pub ccr: CcrConfig,
}

impl Default for CompressionConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            content_router: ContentRouterConfig::default(),
            compressors: CompressorsConfig::default(),
            cache_aligner: CacheAlignerConfig::default(),
            ccr: CcrConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentRouterConfig {
    #[serde(default = "default_detection_methods")]
    pub detection: Vec<String>,
    #[serde(default = "default_max_input_tokens")]
    pub max_input_tokens: u64,
    #[serde(default = "default_true")]
    pub append_only: bool,
}

impl Default for ContentRouterConfig {
    fn default() -> Self {
        Self {
            detection: vec!["magika".into(), "parser".into(), "regex".into()],
            max_input_tokens: 100_000,
            append_only: true,
        }
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

impl Default for CompressorsConfig {
    fn default() -> Self {
        Self {
            smart_crusher: true,
            code_compressor: true,
            log_compressor: true,
            text_compressor: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheAlignerConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
}

impl Default for CacheAlignerConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

// ── CCR (Compress-Cache-Retrieve) ──────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CcrConfig {
    #[serde(default = "default_ccr_backend")]
    pub backend: String,
    #[serde(default = "default_ccr_ttl")]
    pub ttl_seconds: u64,
    #[serde(default = "default_true")]
    pub shared_with_loop_detection: bool,
}

impl Default for CcrConfig {
    fn default() -> Self {
        Self {
            backend: "memory".into(),
            ttl_seconds: 1800,
            shared_with_loop_detection: true,
        }
    }
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

impl Default for LoopCompressionCoupling {
    fn default() -> Self {
        Self {
            enabled: true,
            on_detected: LoopCouplingAction {
                compression_budget: CompressionBudget::Aggressive,
                inject_hint: false,
                hint_template: None,
            },
            on_error_loop: LoopCouplingAction {
                compression_budget: CompressionBudget::Maximum,
                inject_hint: true,
                hint_template: Some(
                    "You are looping on '{tool}'. Try a completely different approach.".into(),
                ),
            },
            on_recovery: RecoveryCouplingAction {
                compression_budget: CompressionBudget::Moderate,
                fresh_context_window: 3,
            },
        }
    }
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
pub enum CompressionBudget {
    #[serde(rename = "normal")]
    #[default]
    Normal,
    #[serde(rename = "moderate")]
    Moderate,
    #[serde(rename = "aggressive")]
    Aggressive,
    #[serde(rename = "maximum")]
    Maximum,
}

// ── Memory ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_memory_backend")]
    pub backend: String,
    #[serde(default = "default_embedder")]
    pub embedder: String,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            backend: "sqlite-vec".into(),
            embedder: "fastembed".into(),
        }
    }
}

// ── Default helpers ────────────────────────────────────────────────────────

fn default_true() -> bool {
    true
}

fn default_max_repeats() -> u32 {
    3
}

fn default_error_reduction() -> u32 {
    1
}

fn default_min_repeats() -> u32 {
    2
}

fn default_min_occurrences() -> u32 {
    2
}

fn default_similarity_threshold() -> f32 {
    0.85
}

fn default_semantic_window() -> usize {
    5
}

fn default_embedder() -> String {
    "auto".into()
}

fn default_detection_methods() -> Vec<String> {
    vec!["magika".into(), "parser".into(), "regex".into()]
}

fn default_max_input_tokens() -> u64 {
    100_000
}

fn default_ccr_backend() -> String {
    "memory".into()
}

fn default_ccr_ttl() -> u64 {
    1800
}

fn default_memory_backend() -> String {
    "sqlite-vec".into()
}
