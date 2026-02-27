use oi_types::{Reranker, Result};
use ort::session::Session;
use ort::value::Value;
use std::sync::Mutex;
use tokenizers::Tokenizer;

use crate::model_cache::{ModelCache, RERANKER_MODEL, RERANKER_TOKENIZER};

const MAX_SEQ_LEN: usize = 512;

/// ONNX cross-encoder reranker using `cross-encoder/ms-marco-MiniLM-L-6-v2`.
///
/// 22M params. Takes `[CLS] query [SEP] passage [SEP]` input, produces a single
/// relevance logit per pair. Higher score = more relevant.
pub struct OnnxReranker {
    session: Mutex<Session>,
    tokenizer: Tokenizer,
}

impl OnnxReranker {
    /// Load the reranker model, downloading if necessary.
    pub fn new() -> Result<Self> {
        let cache = ModelCache::new();
        Self::with_cache(&cache)
    }

    /// Load from a specific model cache directory.
    pub fn with_cache(cache: &ModelCache) -> Result<Self> {
        let model_path = cache.ensure_model(&RERANKER_MODEL)?;
        let tokenizer_path = cache.ensure_model(&RERANKER_TOKENIZER)?;

        let threads = crate::intra_threads();
        let session = Session::builder()
            .and_then(|b| b.with_intra_threads(threads))
            .and_then(|b| b.commit_from_file(&model_path))
            .map_err(|e| oi_types::Error::Internal(format!("Reranker ONNX session init: {e}")))?;

        let tokenizer = Tokenizer::from_file(&tokenizer_path)
            .map_err(|e| oi_types::Error::Internal(format!("Reranker tokenizer load: {e}")))?;

        Ok(Self {
            session: Mutex::new(session),
            tokenizer,
        })
    }

    /// Run inference on query-document pairs. Returns one logit per pair.
    fn run_inference(&self, pairs: &[(&str, &str)]) -> Result<Vec<f32>> {
        if pairs.is_empty() {
            return Ok(vec![]);
        }

        // Cross-encoders take "[CLS] query [SEP] passage [SEP]" as a single sequence.
        // The tokenizer handles this when given a pair of texts.
        let encodings: Vec<_> = pairs
            .iter()
            .map(|(query, doc)| {
                self.tokenizer
                    .encode((*query, *doc), true)
                    .map_err(|e| oi_types::Error::Internal(format!("Tokenization: {e}")))
            })
            .collect::<Result<Vec<_>>>()?;

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
        let attention_mask_val =
            Value::from_array((&shape as &[usize], attention_mask_flat.clone()))
                .map_err(|e| oi_types::Error::Internal(format!("Value: {e}")))?;
        let token_type_ids_val = Value::from_array((&shape as &[usize], token_type_ids_flat))
            .map_err(|e| oi_types::Error::Internal(format!("Value: {e}")))?;

        let inputs = vec![
            (
                std::borrow::Cow::from("input_ids"),
                ort::session::SessionInputValue::from(input_ids_val),
            ),
            (
                std::borrow::Cow::from("attention_mask"),
                ort::session::SessionInputValue::from(attention_mask_val),
            ),
            (
                std::borrow::Cow::from("token_type_ids"),
                ort::session::SessionInputValue::from(token_type_ids_val),
            ),
        ];

        let mut session = self
            .session
            .lock()
            .map_err(|e| oi_types::Error::Internal(format!("Session lock: {e}")))?;
        let outputs = session
            .run(inputs)
            .map_err(|e| oi_types::Error::Internal(format!("Reranker ONNX inference: {e}")))?;

        // Output shape: [batch_size, 1] — single logit per pair
        let tensor_ref = outputs[0]
            .try_extract_tensor::<f32>()
            .map_err(|e| oi_types::Error::Internal(format!("Output extract: {e}")))?;

        let raw = tensor_ref.1;
        // Output may be [batch_size] or [batch_size, 1]
        let scores: Vec<f32> = (0..batch_size).map(|i| raw[i]).collect();

        Ok(scores)
    }
}

impl Reranker for OnnxReranker {
    fn rerank(&self, query: &str, documents: &[&str]) -> Result<Vec<f32>> {
        if documents.is_empty() {
            return Ok(vec![]);
        }

        // Process in batches of 64 to avoid OOM on large candidate sets
        const BATCH_SIZE: usize = 64;
        let mut all_scores = Vec::with_capacity(documents.len());

        for chunk in documents.chunks(BATCH_SIZE) {
            let pairs: Vec<(&str, &str)> = chunk.iter().map(|doc| (query, *doc)).collect();
            let scores = self.run_inference(&pairs)?;
            all_scores.extend(scores);
        }

        Ok(all_scores)
    }

    fn name(&self) -> &str {
        "cross-encoder/ms-marco-MiniLM-L-6-v2"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reranker_basic() {
        let reranker = OnnxReranker::new().expect("Failed to load reranker model");

        let query = "what is the capital of France";
        let docs = &[
            "Paris is the capital and most populous city of France.",
            "Berlin is the capital of Germany.",
            "The quick brown fox jumps over the lazy dog.",
        ];

        let scores = reranker.rerank(query, docs).unwrap();
        assert_eq!(scores.len(), 3);

        // Paris doc should score highest
        assert!(
            scores[0] > scores[1],
            "Paris ({}) should score higher than Berlin ({})",
            scores[0],
            scores[1]
        );
        assert!(
            scores[0] > scores[2],
            "Paris ({}) should score higher than fox ({})",
            scores[0],
            scores[2]
        );
    }

    #[test]
    fn test_reranker_empty() {
        let reranker = OnnxReranker::new().expect("Failed to load reranker model");
        let scores = reranker.rerank("test query", &[]).unwrap();
        assert!(scores.is_empty());
    }

    #[test]
    fn test_reranker_name() {
        let reranker = OnnxReranker::new().expect("Failed to load reranker model");
        assert_eq!(reranker.name(), "cross-encoder/ms-marco-MiniLM-L-6-v2");
    }
}
