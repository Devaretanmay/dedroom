//! Context compression pipeline.
//!
//! Type-aware compressors for different content types (JSON, code, logs,
//! text). The [`ContentRouter`] detects content type and dispatches to the
//! appropriate compressor.

pub mod router;
pub mod smart_crusher;
pub mod code_compressor;
pub mod log_compressor;
pub mod text_compressor;
pub mod policy;

pub use router::ContentRouter;
pub use smart_crusher::{compress_json_array, estimate_tokens};
pub use code_compressor::compress_code;
pub use log_compressor::compress_logs;
pub use text_compressor::compress_text;
pub use policy::CompressionPolicy;

/// Result of compressing a content block.
#[derive(Debug, Clone)]
pub struct CompressionResult {
    pub original_tokens: u64,
    pub compressed_tokens: u64,
    pub content: String,
    pub content_type: ContentType,
}

/// Type of content detected by the router.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentType {
    JsonArray,
    JsonObject,
    Code,
    Log,
    Text,
    Diff,
    SearchResults,
    Tabular,
    Html,
    Unknown,
}

impl ContentType {
    pub fn name(&self) -> &'static str {
        match self {
            Self::JsonArray => "json_array",
            Self::JsonObject => "json_object",
            Self::Code => "code",
            Self::Log => "log",
            Self::Text => "text",
            Self::Diff => "diff",
            Self::SearchResults => "search_results",
            Self::Tabular => "tabular",
            Self::Html => "html",
            Self::Unknown => "unknown",
        }
    }
}
