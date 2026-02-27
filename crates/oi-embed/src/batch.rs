use oi_types::{EmbeddingPipeline, Result};

/// Split texts into batches and embed each batch through the pipeline.
/// Returns all embeddings in order.
pub fn embed_in_batches(
    pipeline: &dyn EmbeddingPipeline,
    texts: &[&str],
    batch_size: usize,
) -> Result<Vec<Vec<f32>>> {
    let batch_size = batch_size.max(1);
    let mut all = Vec::with_capacity(texts.len());
    for chunk in texts.chunks(batch_size) {
        let batch_result = pipeline.embed_text_batch(chunk)?;
        all.extend(batch_result);
    }
    Ok(all)
}

/// Embed content strings (owned) through the pipeline with batching.
pub fn embed_strings(
    pipeline: &dyn EmbeddingPipeline,
    texts: &[String],
    batch_size: usize,
) -> Result<Vec<Vec<f32>>> {
    let refs: Vec<&str> = texts.iter().map(|s| s.as_str()).collect();
    embed_in_batches(pipeline, &refs, batch_size)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MockEmbedder;

    #[test]
    fn batch_preserves_order() {
        let e = MockEmbedder::default();
        let texts = &["alpha", "beta", "gamma", "delta", "epsilon"];
        let result = embed_in_batches(&e, texts, 2).unwrap();
        assert_eq!(result.len(), 5);
        for (i, t) in texts.iter().enumerate() {
            let single = e.embed_text(t).unwrap();
            assert_eq!(result[i], single);
        }
    }

    #[test]
    fn batch_size_one() {
        let e = MockEmbedder::default();
        let texts = &["a", "b", "c"];
        let result = embed_in_batches(&e, texts, 1).unwrap();
        assert_eq!(result.len(), 3);
    }

    #[test]
    fn batch_larger_than_input() {
        let e = MockEmbedder::default();
        let texts = &["x", "y"];
        let result = embed_in_batches(&e, texts, 100).unwrap();
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn empty_input() {
        let e = MockEmbedder::default();
        let result = embed_in_batches(&e, &[], 10).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn embed_owned_strings() {
        let e = MockEmbedder::default();
        let texts = vec!["hello".to_string(), "world".to_string()];
        let result = embed_strings(&e, &texts, 10).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0], e.embed_text("hello").unwrap());
    }
}
