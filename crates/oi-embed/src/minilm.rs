use oi_types::{EmbeddingPipeline, Result};
use ort::session::Session;
use ort::value::Value;
use std::sync::Mutex;
use tokenizers::Tokenizer;

use crate::model_cache::{ModelCache, MINILM_MODEL, MINILM_TOKENIZER};

const DIM: usize = 384;
const MAX_SEQ_LEN: usize = 256;
const QUERY_PREFIX: &str = "Represent this sentence for searching relevant passages: ";

/// Snowflake Arctic Embed S (INT8 quantized) via ONNX Runtime.
/// 33M params, 384-dim, CLS pooling + L2 normalize. nDCG@10 = 51.98 on BEIR.
pub struct MiniLmEmbedder {
    session: Mutex<Session>,
    tokenizer: Tokenizer,
}

impl MiniLmEmbedder {
    /// Load the model, downloading if necessary.
    pub fn new() -> Result<Self> {
        let cache = ModelCache::new();
        Self::with_cache(&cache)
    }

    /// Load from a specific model cache directory.
    pub fn with_cache(cache: &ModelCache) -> Result<Self> {
        let model_path = cache.ensure_model(&MINILM_MODEL)?;
        let tokenizer_path = cache.ensure_model(&MINILM_TOKENIZER)?;

        let threads = crate::intra_threads();
        let session = Session::builder()
            .and_then(|b| b.with_intra_threads(threads))
            .and_then(|b| b.commit_from_file(&model_path))
            .map_err(|e| oi_types::Error::Internal(format!("ONNX session init: {e}")))?;

        let tokenizer = Tokenizer::from_file(&tokenizer_path)
            .map_err(|e| oi_types::Error::Internal(format!("Tokenizer load: {e}")))?;

        Ok(Self { session: Mutex::new(session), tokenizer })
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
        let mut token_type_ids_flat = vec![0i64; batch_size * max_len];

        for (i, enc) in encodings.iter().enumerate() {
            let ids = enc.get_ids();
            let mask = enc.get_attention_mask();
            let types = enc.get_type_ids();
            let len = ids.len().min(max_len);

            for j in 0..len {
                input_ids_flat[i * max_len + j] = ids[j] as i64;
                attention_mask_flat[i * max_len + j] = mask[j] as i64;
                token_type_ids_flat[i * max_len + j] = types[j] as i64;
            }
        }

        let shape = [batch_size, max_len];

        let input_ids_val = Value::from_array((&shape as &[usize], input_ids_flat.clone()))
            .map_err(|e| oi_types::Error::Internal(format!("Value: {e}")))?;
        let attention_mask_val = Value::from_array((&shape as &[usize], attention_mask_flat.clone()))
            .map_err(|e| oi_types::Error::Internal(format!("Value: {e}")))?;
        let token_type_ids_val = Value::from_array((&shape as &[usize], token_type_ids_flat))
            .map_err(|e| oi_types::Error::Internal(format!("Value: {e}")))?;

        let inputs = vec![
            (std::borrow::Cow::from("input_ids"), ort::session::SessionInputValue::from(input_ids_val)),
            (std::borrow::Cow::from("attention_mask"), ort::session::SessionInputValue::from(attention_mask_val)),
            (std::borrow::Cow::from("token_type_ids"), ort::session::SessionInputValue::from(token_type_ids_val)),
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
        let seq_len = if tensor_shape.len() >= 2 { tensor_shape[1] as usize } else { max_len };

        Ok(Self::cls_pool(hidden_states, batch_size, seq_len))
    }

    /// CLS pooling: take the [CLS] token (index 0) output and L2 normalize.
    /// Arctic Embed models use CLS pooling, not mean pooling.
    fn cls_pool(hidden_states: &[f32], batch: usize, seq_len: usize) -> Vec<Vec<f32>> {
        let mut results = Vec::with_capacity(batch);

        for b in 0..batch {
            let offset = b * seq_len * DIM; // token 0 = [CLS]
            let mut pooled: Vec<f32> = hidden_states[offset..offset + DIM].to_vec();

            // L2 normalize
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

impl EmbeddingPipeline for MiniLmEmbedder {
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
        "arctic-embed-s"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_minilm_single_embed() {
        let embedder = MiniLmEmbedder::new().expect("Failed to load MiniLM model");
        let v = embedder.embed_text("hello world").unwrap();
        assert_eq!(v.len(), 384);
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 0.01, "not normalized: {norm}");
    }

    #[test]
    fn test_minilm_batch_embed() {
        let embedder = MiniLmEmbedder::new().expect("Failed to load MiniLM model");
        let texts = &["hello world", "rust programming language", "machine learning"];
        let batch = embedder.embed_text_batch(texts).unwrap();
        assert_eq!(batch.len(), 3);
        for v in &batch {
            assert_eq!(v.len(), 384);
        }
    }

    #[test]
    fn test_minilm_deterministic() {
        let embedder = MiniLmEmbedder::new().expect("Failed to load MiniLM model");
        let a = embedder.embed_text("test query").unwrap();
        let b = embedder.embed_text("test query").unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn test_minilm_similar_texts_closer() {
        let embedder = MiniLmEmbedder::new().expect("Failed to load MiniLM model");
        let rust = embedder.embed_text("rust programming language systems").unwrap();
        let cpp = embedder.embed_text("c++ programming language systems").unwrap();
        let cooking = embedder.embed_text("best chocolate cake recipe baking").unwrap();

        let sim_rust_cpp: f32 = rust.iter().zip(&cpp).map(|(a, b)| a * b).sum();
        let sim_rust_cooking: f32 = rust.iter().zip(&cooking).map(|(a, b)| a * b).sum();

        assert!(
            sim_rust_cpp > sim_rust_cooking,
            "rust-cpp ({sim_rust_cpp:.3}) should be more similar than rust-cooking ({sim_rust_cooking:.3})"
        );
    }

    #[test]
    fn test_minilm_empty_batch() {
        let embedder = MiniLmEmbedder::new().expect("Failed to load MiniLM model");
        let result = embedder.embed_text_batch(&[]).unwrap();
        assert!(result.is_empty());
    }
}
