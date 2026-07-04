//! Compress-Cache-Retrieve (CCR).
//!
//! Stores originals of compressed content keyed by BLAKE3 hash. The LLM
//! can retrieve originals on demand via an injected tool or MCP server.
//! Also serves as shared storage for loop detection.

mod store;

#[cfg(feature = "sqlite")]
pub use store::SqliteStore;
pub use store::{CcrStore, CcrBackend, InMemoryStore, CcrEntry, hash_tool_call};
