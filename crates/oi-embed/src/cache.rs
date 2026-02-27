use oi_types::{EmbeddingPipeline, Result};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

/// A caching wrapper around any `EmbeddingPipeline` that caches `embed_query()` results.
///
/// Document embeddings (`embed_text` / `embed_text_batch`) pass through uncached
/// since documents are typically embedded once. Query embeddings are deterministic
/// for the same model and input, so caching avoids redundant ~3ms embed calls
/// for repeated searches.
pub struct CachedEmbedder {
    inner: Arc<dyn EmbeddingPipeline>,
    cache: RwLock<HashMap<String, Vec<f32>>>,
    max_entries: usize,
}

impl CachedEmbedder {
    pub fn new(inner: Arc<dyn EmbeddingPipeline>, max_entries: usize) -> Self {
        Self {
            inner,
            cache: RwLock::new(HashMap::new()),
            max_entries,
        }
    }
}

impl EmbeddingPipeline for CachedEmbedder {
    fn embed_text(&self, text: &str) -> Result<Vec<f32>> {
        self.inner.embed_text(text)
    }

    fn embed_query(&self, query: &str) -> Result<Vec<f32>> {
        // Fast path: check read lock
        if let Ok(cache) = self.cache.read()
            && let Some(cached) = cache.get(query)
        {
            return Ok(cached.clone());
        }

        // Cache miss: compute embedding
        let embedding = self.inner.embed_query(query)?;

        // Store in cache
        if let Ok(mut cache) = self.cache.write() {
            // Evict oldest half when full
            if cache.len() >= self.max_entries {
                let to_remove: Vec<String> = cache
                    .keys()
                    .take(cache.len() / 2)
                    .cloned()
                    .collect();
                for key in to_remove {
                    cache.remove(&key);
                }
            }
            cache.insert(query.to_string(), embedding.clone());
        }

        Ok(embedding)
    }

    fn embed_text_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        self.inner.embed_text_batch(texts)
    }

    fn dimension(&self) -> usize {
        self.inner.dimension()
    }

    fn name(&self) -> &str {
        self.inner.name()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MockEmbedder;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct CountingEmbedder {
        inner: MockEmbedder,
        query_count: AtomicUsize,
    }

    impl CountingEmbedder {
        fn new(dim: usize) -> Self {
            Self {
                inner: MockEmbedder::new(dim),
                query_count: AtomicUsize::new(0),
            }
        }
    }

    impl EmbeddingPipeline for CountingEmbedder {
        fn embed_text(&self, text: &str) -> Result<Vec<f32>> {
            self.inner.embed_text(text)
        }

        fn embed_query(&self, query: &str) -> Result<Vec<f32>> {
            self.query_count.fetch_add(1, Ordering::SeqCst);
            self.inner.embed_query(query)
        }

        fn dimension(&self) -> usize {
            self.inner.dimension()
        }

        fn name(&self) -> &str {
            "counting"
        }
    }

    #[test]
    fn test_cache_hit_avoids_recompute() {
        let counting = Arc::new(CountingEmbedder::new(4));
        let cached = CachedEmbedder::new(counting.clone(), 100);

        let _r1 = cached.embed_query("hello world").unwrap();
        let _r2 = cached.embed_query("hello world").unwrap();
        let _r3 = cached.embed_query("hello world").unwrap();

        assert_eq!(counting.query_count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn test_different_queries_are_separate() {
        let counting = Arc::new(CountingEmbedder::new(4));
        let cached = CachedEmbedder::new(counting.clone(), 100);

        let _r1 = cached.embed_query("hello").unwrap();
        let _r2 = cached.embed_query("world").unwrap();

        assert_eq!(counting.query_count.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn test_eviction_when_full() {
        let counting = Arc::new(CountingEmbedder::new(4));
        let cached = CachedEmbedder::new(counting.clone(), 4);

        // Fill the cache
        for i in 0..4 {
            cached.embed_query(&format!("query_{i}")).unwrap();
        }
        assert_eq!(counting.query_count.load(Ordering::SeqCst), 4);

        // This should trigger eviction (removes half = 2)
        cached.embed_query("query_new").unwrap();
        assert_eq!(counting.query_count.load(Ordering::SeqCst), 5);

        // Cache should still work for non-evicted entries
        let cache = cached.cache.read().unwrap();
        assert!(cache.len() <= 4);
    }

    #[test]
    fn test_embed_text_passes_through() {
        let counting = Arc::new(CountingEmbedder::new(4));
        let cached = CachedEmbedder::new(counting.clone(), 100);

        // embed_text should not increment query_count
        let _r = cached.embed_text("hello").unwrap();
        assert_eq!(counting.query_count.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn test_dimension_and_name_passthrough() {
        let inner = Arc::new(MockEmbedder::new(384));
        let cached = CachedEmbedder::new(inner, 100);

        assert_eq!(cached.dimension(), 384);
        assert_eq!(cached.name(), "mock");
    }
}
