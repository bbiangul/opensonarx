//! TTL-based response cache for search engines.
//!
//! Wraps any `SearchEngine` and caches results by query hash for a configurable
//! duration. Prevents redundant external HTTP calls when the same query is
//! issued multiple times within the TTL window (e.g. crawl-on-query re-search).

use oi_types::{SearchEngine, SearchParams, SearchResultItem};
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

struct CacheEntry {
    results: Vec<SearchResultItem>,
    expires_at: Instant,
}

/// A caching wrapper around any `SearchEngine`.
///
/// Caches search results keyed by `(query, limit)` hash with a configurable TTL.
/// Cache hits bypass the inner engine entirely.
pub struct CachedEngine {
    inner: Arc<dyn SearchEngine>,
    cache: RwLock<HashMap<u64, CacheEntry>>,
    ttl: Duration,
}

impl CachedEngine {
    /// Wrap an engine with a 5-minute TTL cache (default).
    pub fn new(inner: Arc<dyn SearchEngine>) -> Self {
        Self {
            inner,
            cache: RwLock::new(HashMap::new()),
            ttl: Duration::from_secs(300),
        }
    }

    /// Wrap an engine with a custom TTL.
    pub fn with_ttl(inner: Arc<dyn SearchEngine>, ttl: Duration) -> Self {
        Self {
            inner,
            cache: RwLock::new(HashMap::new()),
            ttl,
        }
    }

    fn cache_key(query: &str, limit: usize) -> u64 {
        let mut hasher = DefaultHasher::new();
        query.hash(&mut hasher);
        limit.hash(&mut hasher);
        hasher.finish()
    }
}

impl SearchEngine for CachedEngine {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn search(&self, query: &str, limit: usize) -> Vec<SearchResultItem> {
        let key = Self::cache_key(query, limit);

        // Check cache
        {
            let cache = self.cache.read().unwrap();
            if let Some(entry) = cache.get(&key)
                && entry.expires_at > Instant::now()
            {
                tracing::debug!(engine = %self.inner.name(), query, "engine cache hit");
                return entry.results.clone();
            }
        }

        // Cache miss — query the inner engine
        let results = self.inner.search(query, limit);

        // Store in cache
        {
            let mut cache = self.cache.write().unwrap();
            cache.insert(key, CacheEntry {
                results: results.clone(),
                expires_at: Instant::now() + self.ttl,
            });

            // Lazy eviction: if cache grows large, clean up
            if cache.len() > 1000 {
                let now = Instant::now();
                cache.retain(|_, entry| entry.expires_at > now);
            }
        }

        results
    }

    fn search_with_params(&self, query: &str, limit: usize, params: &SearchParams) -> Vec<SearchResultItem> {
        // For parameterized searches, include params in cache key
        let mut hasher = DefaultHasher::new();
        query.hash(&mut hasher);
        limit.hash(&mut hasher);
        params.locale.hash(&mut hasher);
        params.gl.hash(&mut hasher);
        params.site.hash(&mut hasher);
        let key = hasher.finish();

        // Check cache
        {
            let cache = self.cache.read().unwrap();
            if let Some(entry) = cache.get(&key)
                && entry.expires_at > Instant::now()
            {
                tracing::debug!(engine = %self.inner.name(), query, "engine cache hit (params)");
                return entry.results.clone();
            }
        }

        let results = self.inner.search_with_params(query, limit, params);

        {
            let mut cache = self.cache.write().unwrap();
            cache.insert(key, CacheEntry {
                results: results.clone(),
                expires_at: Instant::now() + self.ttl,
            });

            if cache.len() > 1000 {
                let now = Instant::now();
                cache.retain(|_, entry| entry.expires_at > now);
            }
        }

        results
    }

    fn requires_api_key(&self) -> bool {
        self.inner.requires_api_key()
    }

    fn is_available(&self) -> bool {
        self.inner.is_available()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct CountingEngine {
        call_count: AtomicUsize,
    }

    impl CountingEngine {
        fn new() -> Self {
            Self { call_count: AtomicUsize::new(0) }
        }
    }

    impl SearchEngine for CountingEngine {
        fn name(&self) -> &str { "counting" }

        fn search(&self, _query: &str, _limit: usize) -> Vec<SearchResultItem> {
            self.call_count.fetch_add(1, Ordering::SeqCst);
            vec![SearchResultItem {
                result_id: "r1".to_string(),
                rank: 1,
                url: "https://example.com".to_string(),
                title: "Example".to_string(),
                snippet: "A snippet".to_string(),
                content_md: String::new(),
                semantic_score: 0.0,
                bm25_score: 0.0,
                final_score: 0.0,
                stake_truth: 0,
                stake_usd: 0,
                stake_type: "none".to_string(),
                reputation: 0.0,
                content_hash: String::new(),
                extraction_quality: "external".to_string(),
                source: "counting".to_string(),
                entity: None,
                content_kind: None,
                crawled_at: None,
                published_at: None,
                trust_verdict: "external".to_string(),
                reputation_votes: 0,
                disputed: false,
                facts: None,
                content_relevant: None,
                domain_first_seen_at: None,
                domain_url_count: None,
                domain_reputation_score: None,
                domain_total_votes: None,
            }]
        }
    }

    #[test]
    fn test_cache_hit_avoids_inner_call() {
        let inner = Arc::new(CountingEngine::new());
        let cached = CachedEngine::new(inner.clone());

        // First call — miss
        let r1 = cached.search("test query", 10);
        assert_eq!(r1.len(), 1);
        assert_eq!(inner.call_count.load(Ordering::SeqCst), 1);

        // Second call — should hit cache
        let r2 = cached.search("test query", 10);
        assert_eq!(r2.len(), 1);
        assert_eq!(inner.call_count.load(Ordering::SeqCst), 1); // still 1

        // Different query — miss
        let r3 = cached.search("different query", 10);
        assert_eq!(r3.len(), 1);
        assert_eq!(inner.call_count.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn test_cache_ttl_expiry() {
        let inner = Arc::new(CountingEngine::new());
        let cached = CachedEngine::with_ttl(inner.clone(), Duration::from_millis(0));

        // First call
        cached.search("test", 10);
        assert_eq!(inner.call_count.load(Ordering::SeqCst), 1);

        // TTL=0 means always expired — should call again
        cached.search("test", 10);
        assert_eq!(inner.call_count.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn test_cache_different_limits_are_separate() {
        let inner = Arc::new(CountingEngine::new());
        let cached = CachedEngine::new(inner.clone());

        cached.search("test", 5);
        assert_eq!(inner.call_count.load(Ordering::SeqCst), 1);

        // Same query, different limit → separate cache entry
        cached.search("test", 10);
        assert_eq!(inner.call_count.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn test_cache_name_delegates() {
        let inner = Arc::new(CountingEngine::new());
        let cached = CachedEngine::new(inner);
        assert_eq!(cached.name(), "counting");
    }
}
