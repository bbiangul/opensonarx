use oi_types::{EmbeddingPipeline, Result};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

/// A caching wrapper around any `EmbeddingPipeline` that caches `embed_query()`
/// and `embed_text_batch()` results per input string.
///
/// Embeddings are deterministic for the same model and input. Batch caching
/// matters for long-lived processes: result enrichment re-embeds the same
/// document sentences on every query (~900ms for a top-5 of full articles),
/// which collapses to lookups once warm — only the misses within a batch go
/// to the inner model. Keys are namespaced ("q:"/"t:") because query and text
/// embeddings differ under asymmetric models (query prefix). `embed_text`
/// (single doc, typically ingested once) passes through uncached.
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

    fn cache_get(&self, key: &str) -> Option<Vec<f32>> {
        self.cache.read().ok()?.get(key).cloned()
    }

    /// Insert into the cache, evicting an arbitrary half when full.
    fn cache_put(&self, key: String, value: &[f32]) {
        if let Ok(mut cache) = self.cache.write() {
            if cache.len() >= self.max_entries {
                let to_remove: Vec<String> =
                    cache.keys().take(cache.len() / 2).cloned().collect();
                for k in to_remove {
                    cache.remove(&k);
                }
            }
            cache.insert(key, value.to_vec());
        }
    }
}

impl EmbeddingPipeline for CachedEmbedder {
    fn embed_text(&self, text: &str) -> Result<Vec<f32>> {
        self.inner.embed_text(text)
    }

    fn embed_query(&self, query: &str) -> Result<Vec<f32>> {
        let key = format!("q:{query}");
        if let Some(cached) = self.cache_get(&key) {
            return Ok(cached);
        }
        let embedding = self.inner.embed_query(query)?;
        self.cache_put(key, &embedding);
        Ok(embedding)
    }

    fn embed_text_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        // Serve hits from cache; batch only the misses through the model.
        let mut results: Vec<Option<Vec<f32>>> = Vec::with_capacity(texts.len());
        let mut miss_indices: Vec<usize> = Vec::new();
        for (i, text) in texts.iter().enumerate() {
            match self.cache_get(&format!("t:{text}")) {
                Some(hit) => results.push(Some(hit)),
                None => {
                    results.push(None);
                    miss_indices.push(i);
                }
            }
        }

        if !miss_indices.is_empty() {
            let miss_texts: Vec<&str> = miss_indices.iter().map(|&i| texts[i]).collect();
            let embeddings = self.inner.embed_text_batch(&miss_texts)?;
            for (&i, emb) in miss_indices.iter().zip(embeddings) {
                self.cache_put(format!("t:{}", texts[i]), &emb);
                results[i] = Some(emb);
            }
        }

        Ok(results.into_iter().map(|r| r.expect("all slots filled")).collect())
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
