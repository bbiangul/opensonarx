pub mod batch;
pub mod cache;
pub mod mock;

#[cfg(feature = "onnx")]
pub mod model_cache;

#[cfg(feature = "onnx")]
pub mod minilm;

#[cfg(feature = "reranker")]
pub mod reranker;

#[cfg(feature = "arctic-m")]
pub mod arctic_m;

#[cfg(feature = "arctic-l")]
pub mod arctic_l;

#[cfg(feature = "ollama")]
pub mod ollama;

#[cfg(feature = "clip")]
pub mod clip;

#[cfg(feature = "clip")]
pub mod corpus;

#[cfg(feature = "clip")]
pub mod projection;

// Re-exports
pub use cache::CachedEmbedder;
pub use mock::MockEmbedder;
pub use oi_types::EmbeddingPipeline;

#[cfg(feature = "onnx")]
pub use minilm::MiniLmEmbedder;

#[cfg(feature = "reranker")]
pub use reranker::OnnxReranker;

#[cfg(feature = "arctic-m")]
pub use arctic_m::ArcticMEmbedder;

#[cfg(feature = "arctic-l")]
pub use arctic_l::ArcticLEmbedder;

#[cfg(feature = "ollama")]
pub use ollama::OllamaEmbedder;

#[cfg(feature = "clip")]
pub use clip::ClipEmbedder;

/// Choose ONNX intra-op thread count.
/// Respects OI_THREADS env var, otherwise uses min(4, available_cores).
/// 4 threads is the sweet spot for transformer models — enough parallelism
/// for matrix multiplies, low enough overhead for single-text inference.
#[cfg(feature = "onnx")]
pub(crate) fn intra_threads() -> usize {
    if let Ok(v) = std::env::var("OI_THREADS")
        && let Ok(n) = v.parse::<usize>()
    {
        return n.max(1);
    }
    let available = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    available.min(4)
}
