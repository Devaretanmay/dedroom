//! Embedder trait and backend implementations.

use std::sync::Arc;

/// Errors from embedding operations.
#[derive(Debug, thiserror::Error)]
pub enum EmbedderError {
    #[error("embedding backend not available: {0}")]
    NotAvailable(String),
    #[error("embedding computation failed: {0}")]
    ComputationFailed(String),
    #[error("dimension mismatch: expected {expected}, got {actual}")]
    DimensionMismatch { expected: usize, actual: usize },
}

/// Configuration for an embedding backend.
#[derive(Debug, Clone)]
pub struct EmbeddingConfig {
    pub backend: String,
    pub model: String,
    pub dimensions: usize,
}

impl Default for EmbeddingConfig {
    fn default() -> Self {
        Self {
            backend: "fastembed".into(),
            model: "BAAI/bge-small-en-v1.5".into(),
            dimensions: 384,
        }
    }
}

/// A single embedding vector.
pub type Embedding = Vec<f32>;

/// Trait for embedding backends.
#[async_trait::async_trait]
pub trait Embedder: Send + Sync + std::fmt::Debug {
    /// Embed one or more text strings.
    async fn embed(&self, texts: &[&str]) -> Result<Vec<Embedding>, EmbedderError>;
    /// The dimensionality of produced embeddings.
    fn dimensions(&self) -> usize;
    /// Whether this backend is available on the current system.
    fn is_available(&self) -> bool;
}

/// Compute cosine similarity between two embeddings.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }
    (dot / (norm_a * norm_b)).clamp(-1.0, 1.0)
}

/// No-op embedder for when no backend is available.
#[derive(Debug)]
#[allow(dead_code)]
pub struct NoopEmbedder;

#[async_trait::async_trait]
impl Embedder for NoopEmbedder {
    async fn embed(&self, _texts: &[&str]) -> Result<Vec<Embedding>, EmbedderError> {
        Err(EmbedderError::NotAvailable("no embedding backend configured".into()))
    }

    fn dimensions(&self) -> usize {
        0
    }

    fn is_available(&self) -> bool {
        false
    }
}

/// Thread-safe reference to an embedder.
pub type EmbeddingBackend = Arc<dyn Embedder>;

/// Embedding cache using LRU.
use lru::LruCache;

#[derive(Debug)]
#[allow(dead_code)]
pub struct EmbeddingCache {
    cache: LruCache<u64, Embedding>,
    embedder: EmbeddingBackend,
}

#[allow(dead_code)]
impl EmbeddingCache {
    pub fn new(embedder: EmbeddingBackend, capacity: usize) -> Self {
        Self {
            cache: LruCache::new(std::num::NonZeroUsize::new(capacity).unwrap()),
            embedder,
        }
    }

    /// Get embedding for text, using cache if available.
    pub async fn get(&mut self, text: &str) -> Result<Embedding, EmbedderError> {
        let key = {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            text.hash(&mut hasher);
            hasher.finish()
        };

        if let Some(emb) = self.cache.get(&key) {
            return Ok(emb.clone());
        }

        let emb = self.embedder.embed(&[text]).await?
            .into_iter()
            .next()
            .ok_or_else(|| EmbedderError::ComputationFailed("empty result".into()))?;

        self.cache.put(key, emb.clone());
        Ok(emb)
    }

    /// The inner embedder.
    pub fn embedder(&self) -> &EmbeddingBackend {
        &self.embedder
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cosine_similarity_identical() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![1.0, 0.0, 0.0];
        assert!((cosine_similarity(&a, &b) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_cosine_similarity_orthogonal() {
        let a = vec![1.0, 0.0];
        let b = vec![0.0, 1.0];
        assert!((cosine_similarity(&a, &b) - 0.0).abs() < 1e-6);
    }

    #[test]
    fn test_cosine_similarity_opposite() {
        let a = vec![1.0, 0.0];
        let b = vec![-1.0, 0.0];
        assert!((cosine_similarity(&a, &b) - (-1.0)).abs() < 1e-6);
    }

    #[test]
    fn test_cosine_similarity_zero_vector() {
        let a = vec![0.0, 0.0];
        let b = vec![1.0, 0.0];
        assert!((cosine_similarity(&a, &b) - 0.0).abs() < 1e-6);
    }

    #[test]
    fn test_noop_embedder_not_available() {
        let embedder = NoopEmbedder;
        assert!(!embedder.is_available());
        assert_eq!(embedder.dimensions(), 0);
    }
}
