use oi_types::{EmbeddingPipeline, Result};
use serde_json::json;

/// Default Ollama API endpoint.
const DEFAULT_BASE_URL: &str = "http://localhost:11434";

/// Batch size for embedding requests (avoid overwhelming Ollama with huge payloads).
const BATCH_SIZE: usize = 32;

/// Embedding pipeline backed by a local Ollama server.
/// Calls `POST /api/embed` with the configured model.
pub struct OllamaEmbedder {
    model: String,
    base_url: String,
    dim: usize,
}

impl OllamaEmbedder {
    /// Create a new Ollama embedder, probing the model for its output dimension.
    pub fn new(model: &str) -> Result<Self> {
        Self::with_base_url(model, DEFAULT_BASE_URL)
    }

    /// Create with a custom Ollama base URL.
    pub fn with_base_url(model: &str, base_url: &str) -> Result<Self> {
        // Probe dimension by embedding a single token
        let url = format!("{base_url}/api/embed");
        let body = json!({
            "model": model,
            "input": "dim_probe"
        });
        let body_bytes = body.to_string();

        let resp = ureq::post(&url)
            .header("content-type", "application/json")
            .send(body_bytes.as_bytes())
            .map_err(|e| oi_types::Error::Internal(format!("Ollama probe request: {e}")))?;

        let resp_bytes = resp
            .into_body()
            .read_to_vec()
            .map_err(|e| oi_types::Error::Internal(format!("Ollama probe read: {e}")))?;

        let parsed: serde_json::Value = serde_json::from_slice(&resp_bytes)
            .map_err(|e| oi_types::Error::Internal(format!("Ollama probe JSON: {e}")))?;

        let dim = parsed["embeddings"][0]
            .as_array()
            .map(|a| a.len())
            .ok_or_else(|| {
                oi_types::Error::Internal(format!(
                    "Ollama probe: unexpected response shape: {}",
                    String::from_utf8_lossy(&resp_bytes).chars().take(200).collect::<String>()
                ))
            })?;

        tracing::info!("Ollama embedder ready: model={model}, dim={dim}");

        Ok(Self {
            model: model.to_string(),
            base_url: base_url.to_string(),
            dim,
        })
    }

    fn embed_batch_raw(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        let url = format!("{}/api/embed", self.base_url);
        let input: Vec<&str> = texts.to_vec();
        let body = json!({
            "model": self.model,
            "input": input
        });
        let body_bytes = body.to_string();

        let resp = ureq::post(&url)
            .header("content-type", "application/json")
            .send(body_bytes.as_bytes())
            .map_err(|e| oi_types::Error::Internal(format!("Ollama embed request: {e}")))?;

        let resp_bytes = resp
            .into_body()
            .read_to_vec()
            .map_err(|e| oi_types::Error::Internal(format!("Ollama embed read: {e}")))?;

        let parsed: serde_json::Value = serde_json::from_slice(&resp_bytes)
            .map_err(|e| oi_types::Error::Internal(format!("Ollama embed JSON: {e}")))?;

        let embeddings = parsed["embeddings"]
            .as_array()
            .ok_or_else(|| {
                oi_types::Error::Internal("Ollama: missing embeddings array".into())
            })?;

        let mut results = Vec::with_capacity(embeddings.len());
        for emb in embeddings {
            let vec: Vec<f32> = emb
                .as_array()
                .ok_or_else(|| oi_types::Error::Internal("Ollama: embedding not array".into()))?
                .iter()
                .map(|v| v.as_f64().unwrap_or(0.0) as f32)
                .collect();
            results.push(vec);
        }

        Ok(results)
    }
}

impl EmbeddingPipeline for OllamaEmbedder {
    fn embed_text(&self, text: &str) -> Result<Vec<f32>> {
        let batch = self.embed_batch_raw(&[text])?;
        batch
            .into_iter()
            .next()
            .ok_or_else(|| oi_types::Error::Internal("Ollama: empty response".into()))
    }

    fn embed_query(&self, query: &str) -> Result<Vec<f32>> {
        // Qwen3-Embedding supports instruction-aware queries
        let prefixed = format!("Instruct: Given a web search query, retrieve relevant passages that answer the query\nQuery: {query}");
        self.embed_text(&prefixed)
    }

    fn embed_text_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(vec![]);
        }

        // Batch in chunks to avoid huge payloads
        let mut all_results = Vec::with_capacity(texts.len());
        for chunk in texts.chunks(BATCH_SIZE) {
            let batch = self.embed_batch_raw(chunk)?;
            all_results.extend(batch);
        }
        Ok(all_results)
    }

    fn dimension(&self) -> usize {
        self.dim
    }

    fn name(&self) -> &str {
        "ollama"
    }
}
