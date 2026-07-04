//! Shared embedding pipeline.
//!
//! Serves both semantic loop detection and vector memory retrieval from
//! a single embedder backend. Avoids loading multiple models.

mod embedder;

pub use embedder::{
    Embedder, EmbedderError,
    EmbeddingBackend, EmbeddingConfig,
    cosine_similarity,
};
