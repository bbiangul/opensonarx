pub mod mock;
pub mod normalizer;

#[cfg(feature = "onnx")]
pub mod model_cache;

#[cfg(feature = "onnx")]
pub mod gliner;

// Re-exports
pub use mock::MockFactExtractor;
pub use oi_types::FactExtractionPipeline;

#[cfg(feature = "onnx")]
pub use gliner::GlinerFactExtractor;

/// Choose ONNX intra-op thread count.
/// Respects OI_THREADS env var, otherwise uses min(4, available_cores).
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
