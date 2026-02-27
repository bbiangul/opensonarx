use oi_types::embed::text_to_embedding;
use oi_types::{EmbeddingPipeline, Result};

/// Mock embedding pipeline using SHA-256 PRNG vectors.
/// Always available (no feature flags). Deterministic but semantically meaningless.
pub struct MockEmbedder {
    dim: usize,
}

impl MockEmbedder {
    pub fn new(dim: usize) -> Self {
        Self { dim }
    }
}

impl Default for MockEmbedder {
    fn default() -> Self {
        Self::new(384)
    }
}

impl EmbeddingPipeline for MockEmbedder {
    fn embed_text(&self, text: &str) -> Result<Vec<f32>> {
        Ok(text_to_embedding(text, self.dim))
    }

    fn dimension(&self) -> usize {
        self.dim
    }

    fn name(&self) -> &str {
        "mock"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic() {
        let e = MockEmbedder::default();
        let a = e.embed_text("rust programming").unwrap();
        let b = e.embed_text("rust programming").unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn correct_dimension() {
        let e = MockEmbedder::new(128);
        let v = e.embed_text("hello").unwrap();
        assert_eq!(v.len(), 128);
        assert_eq!(e.dimension(), 128);
    }

    #[test]
    fn default_384() {
        let e = MockEmbedder::default();
        assert_eq!(e.dimension(), 384);
        assert_eq!(e.name(), "mock");
    }

    #[test]
    fn normalized() {
        let e = MockEmbedder::default();
        let v = e.embed_text("test query").unwrap();
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 0.01);
    }

    #[test]
    fn different_texts_differ() {
        let e = MockEmbedder::default();
        let a = e.embed_text("rust").unwrap();
        let b = e.embed_text("python").unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn no_image_support() {
        let e = MockEmbedder::default();
        assert!(!e.supports_images());
        assert!(e.embed_image(&[1, 2, 3]).is_err());
    }

    #[test]
    fn batch_consistency() {
        let e = MockEmbedder::default();
        let texts = &["hello", "world", "rust"];
        let batch = e.embed_text_batch(texts).unwrap();
        assert_eq!(batch.len(), 3);
        for (i, t) in texts.iter().enumerate() {
            let single = e.embed_text(t).unwrap();
            assert_eq!(batch[i], single);
        }
    }
}
