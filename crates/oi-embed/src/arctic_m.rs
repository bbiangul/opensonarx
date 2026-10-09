use oi_types::{EmbeddingPipeline, Result};
use ort::session::Session;
use ort::value::Value;
use std::sync::Mutex;
use tokenizers::Tokenizer;

use crate::model_cache::{ARCTIC_M_MODEL, ARCTIC_M_TOKENIZER, ModelCache};

const DIM: usize = 768;
const MAX_SEQ_LEN: usize = 512;
const QUERY_PREFIX: &str = "Represent this sentence for searching relevant passages: ";

/// Snowflake Arctic Embed M v2.0 via ONNX Runtime.
/// 110M params, 768-dim, CLS pooling + L2 normalize.
pub struct ArcticMEmbedder {
    session: Mutex<Session>,
    tokenizer: Tokenizer,
}

impl ArcticMEmbedder {
    pub fn new() -> Result<Self> {
        let cache = ModelCache::new();
        Self::with_cache(&cache)
    }

    pub fn with_cache(cache: &ModelCache) -> Result<Self> {
        let model_path = cache.ensure_model(&ARCTIC_M_MODEL)?;
        let tokenizer_path = cache.ensure_model(&ARCTIC_M_TOKENIZER)?;

        let threads = crate::intra_threads();
        let session = Session::builder()
            .and_then(|b| b.with_intra_threads(threads))
            .and_then(|b| b.commit_from_file(&model_path))
            .map_err(|e| oi_types::Error::Internal(format!("ONNX session init: {e}")))?;

        let tokenizer = Tokenizer::from_file(&tokenizer_path)
            .map_err(|e| oi_types::Error::Internal(format!("Tokenizer load: {e}")))?;

        Ok(Self {
            session: Mutex::new(session),
            tokenizer,
        })
    }

    fn run_inference(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        let encodings = self
            .tokenizer
            .encode_batch(texts.to_vec(), true)
            .map_err(|e| oi_types::Error::Internal(format!("Tokenization: {e}")))?;

        let batch_size = encodings.len();
        let max_len = encodings
            .iter()
            .map(|e| e.get_ids().len().min(MAX_SEQ_LEN))
            .max()
            .unwrap_or(0);

        let mut input_ids_flat = vec![0i64; batch_size * max_len];
        let mut attention_mask_flat = vec![0i64; batch_size * max_len];

        for (i, enc) in encodings.iter().enumerate() {
            let ids = enc.get_ids();
            let mask = enc.get_attention_mask();
            let len = ids.len().min(max_len);

            for j in 0..len {
                input_ids_flat[i * max_len + j] = ids[j] as i64;
                attention_mask_flat[i * max_len + j] = mask[j] as i64;
            }
        }

        let shape = [batch_size, max_len];

        let input_ids_val = Value::from_array((&shape as &[usize], input_ids_flat.clone()))
            .map_err(|e| oi_types::Error::Internal(format!("Value: {e}")))?;
        let attention_mask_val =
            Value::from_array((&shape as &[usize], attention_mask_flat.clone()))
                .map_err(|e| oi_types::Error::Internal(format!("Value: {e}")))?;

        // Arctic Embed M v2.0 (XLM-R based) only takes input_ids + attention_mask.
        let inputs = vec![
            (
                std::borrow::Cow::from("input_ids"),
                ort::session::SessionInputValue::from(input_ids_val),
            ),
            (
                std::borrow::Cow::from("attention_mask"),
                ort::session::SessionInputValue::from(attention_mask_val),
            ),
        ];

        let mut session = self
            .session
            .lock()
            .map_err(|e| oi_types::Error::Internal(format!("Session lock: {e}")))?;

        let outputs = session
            .run(inputs)
            .map_err(|e| oi_types::Error::Internal(format!("ONNX inference: {e}")))?;

        let tensor_ref = outputs[0]
            .try_extract_tensor::<f32>()
            .map_err(|e| oi_types::Error::Internal(format!("Output extract: {e}")))?;

        let tensor_shape = tensor_ref.0;
        let hidden_states = tensor_ref.1;
        let seq_len = if tensor_shape.len() >= 2 {
            tensor_shape[1] as usize
        } else {
            max_len
        };

        Ok(Self::cls_pool(hidden_states, batch_size, seq_len))
    }

    fn cls_pool(hidden_states: &[f32], batch: usize, seq_len: usize) -> Vec<Vec<f32>> {
        let mut results = Vec::with_capacity(batch);

        for b in 0..batch {
            let offset = b * seq_len * DIM;
            let mut pooled: Vec<f32> = hidden_states[offset..offset + DIM].to_vec();

            let norm: f32 = pooled.iter().map(|x| x * x).sum::<f32>().sqrt();
            if norm > 1e-10 {
                for x in &mut pooled {
                    *x /= norm;
                }
            }

            results.push(pooled);
        }

        results
    }
}

impl EmbeddingPipeline for ArcticMEmbedder {
    fn embed_text(&self, text: &str) -> Result<Vec<f32>> {
        let batch = self.embed_text_batch(&[text])?;
        Ok(batch.into_iter().next().unwrap())
    }

    fn embed_query(&self, query: &str) -> Result<Vec<f32>> {
        let prefixed = format!("{QUERY_PREFIX}{query}");
        self.embed_text(&prefixed)
    }

    fn embed_text_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(vec![]);
        }
        self.run_inference(texts)
    }

    fn dimension(&self) -> usize {
        DIM
    }

    fn name(&self) -> &str {
        "arctic-embed-m-v2.0"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_arctic_m_dimension() {
        let embedder = ArcticMEmbedder::new().expect("Failed to load Arctic M model");
        assert_eq!(embedder.dimension(), 768);
    }

    #[test]
    fn test_arctic_m_single_embed() {
        let embedder = ArcticMEmbedder::new().expect("Failed to load Arctic M model");
        let v = embedder.embed_text("hello world").unwrap();
        assert_eq!(v.len(), 768);
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 0.01, "not normalized: {norm}");
    }

    #[test]
    fn test_arctic_m_empty_batch() {
        let embedder = ArcticMEmbedder::new().expect("Failed to load Arctic M model");
        let result = embedder.embed_text_batch(&[]).unwrap();
        assert!(result.is_empty());
    }
}
