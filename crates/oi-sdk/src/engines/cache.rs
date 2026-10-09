//! TTL-based response cache for search engines.
//!
//! Wraps any `SearchEngine` and caches results by query hash for a configurable
//! duration. Prevents redundant external HTTP calls when the same query is
//! issued multiple times within the TTL window (e.g. crawl-on-query re-search).

use oi_types::{SearchEngine, SearchParams, SearchResultItem};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, RwLock};
use std::time::{Duration, Instant};

struct CacheEntry {
    results: Vec<SearchResultItem>,
    expires_at: Instant,
    expires_at_unix_ms: u64,
}

#[derive(Serialize, Deserialize)]
struct DiskCache {
    schema_version: u32,
    entries: Vec<DiskCacheEntry>,
}

#[derive(Serialize, Deserialize)]
struct DiskCacheEntry {
    key: u64,
    expires_at_unix_ms: u64,
    results: Vec<SearchResultItem>,
}

struct InFlight {
    result: Mutex<Option<Vec<SearchResultItem>>>,
    ready: Condvar,
}

/// A caching wrapper around any `SearchEngine`.
///
/// Caches search results keyed by `(query, limit)` hash with a configurable TTL.
/// Cache hits bypass the inner engine entirely.
pub struct CachedEngine {
    inner: Arc<dyn SearchEngine>,
    cache: RwLock<HashMap<u64, CacheEntry>>,
    in_flight: Mutex<HashMap<u64, Arc<InFlight>>>,
    ttl: Duration,
    persistent_path: Option<PathBuf>,
}

impl CachedEngine {
    /// Wrap an engine with a 5-minute TTL cache (default).
    pub fn new(inner: Arc<dyn SearchEngine>) -> Self {
        Self {
            inner,
            cache: RwLock::new(HashMap::new()),
            in_flight: Mutex::new(HashMap::new()),
            ttl: Duration::from_secs(300),
            persistent_path: None,
        }
    }

    /// Wrap an engine with a custom TTL.
    pub fn with_ttl(inner: Arc<dyn SearchEngine>, ttl: Duration) -> Self {
        Self {
            inner,
            cache: RwLock::new(HashMap::new()),
            in_flight: Mutex::new(HashMap::new()),
            ttl,
            persistent_path: None,
        }
    }

    /// Wrap an engine with a short-lived cache that survives process restarts.
    /// Persisted entries remain discovery metadata: serving still applies the
    /// semantic batch gate, and claim-bearing answers still require fetched,
    /// retained evidence.
    pub fn with_persistence(
        inner: Arc<dyn SearchEngine>,
        ttl: Duration,
        persistent_path: PathBuf,
    ) -> Self {
        let cache = load_persistent_cache(&persistent_path);
        Self {
            inner,
            cache: RwLock::new(cache),
            in_flight: Mutex::new(HashMap::new()),
            ttl,
            persistent_path: Some(persistent_path),
        }
    }

    fn cache_key(query: &str, limit: usize) -> u64 {
        let mut hasher = DefaultHasher::new();
        query.hash(&mut hasher);
        limit.hash(&mut hasher);
        hasher.finish()
    }

    fn cached(&self, key: u64) -> Option<Vec<SearchResultItem>> {
        let cache = self.cache.read().ok()?;
        let mut results = cache
            .get(&key)
            .filter(|entry| entry.expires_at > Instant::now())
            .map(|entry| entry.results.clone())?;
        for result in &mut results {
            result.extraction_quality = "external_cache".to_string();
        }
        Some(results)
    }

    fn has_cached(&self, key: u64) -> bool {
        self.cache.read().is_ok_and(|cache| {
            cache
                .get(&key)
                .is_some_and(|entry| entry.expires_at > Instant::now())
        })
    }

    fn persist_locked(&self, cache: &HashMap<u64, CacheEntry>) {
        let Some(path) = &self.persistent_path else {
            return;
        };
        if let Err(error) = persist_cache(path, cache) {
            tracing::warn!(
                engine = %self.inner.name(),
                path = %path.display(),
                error,
                "failed to persist short-lived discovery cache"
            );
        }
    }

    fn execute_coalesced(
        &self,
        key: u64,
        query: impl FnOnce() -> Vec<SearchResultItem>,
    ) -> Vec<SearchResultItem> {
        if let Some(results) = self.cached(key) {
            return results;
        }

        let (flight, leader) = {
            let mut flights = self.in_flight.lock().unwrap();
            if let Some(flight) = flights.get(&key) {
                (Arc::clone(flight), false)
            } else if let Some(results) = self.cached(key) {
                // The previous leader may have populated the cache between
                // our optimistic read above and acquiring the flight map.
                // Recheck here so a late member of the same burst does not
                // become a redundant second provider request.
                return results;
            } else {
                let flight = Arc::new(InFlight {
                    result: Mutex::new(None),
                    ready: Condvar::new(),
                });
                flights.insert(key, Arc::clone(&flight));
                (flight, true)
            }
        };

        if !leader {
            let mut result = flight.result.lock().unwrap();
            while result.is_none() {
                result = flight.ready.wait(result).unwrap();
            }
            return result.clone().unwrap_or_default();
        }

        let results = query();
        // Empty failures are shared only with callers already waiting on this
        // exact request. They are not retained in the TTL cache, so a later
        // bounded retry can still recover from a transient bot wall.
        if !results.is_empty() {
            let mut cache = self.cache.write().unwrap();
            let expires_at_unix_ms =
                unix_time_ms().saturating_add(self.ttl.as_millis().min(u64::MAX as u128) as u64);
            cache.insert(
                key,
                CacheEntry {
                    results: results.clone(),
                    expires_at: Instant::now() + self.ttl,
                    expires_at_unix_ms,
                },
            );
            if cache.len() > 1000 {
                let now = Instant::now();
                cache.retain(|_, entry| entry.expires_at > now);
            }
            self.persist_locked(&cache);
        }
        {
            let mut result = flight.result.lock().unwrap();
            *result = Some(results.clone());
            flight.ready.notify_all();
        }
        self.in_flight.lock().unwrap().remove(&key);
        results
    }
}

impl SearchEngine for CachedEngine {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn search(&self, query: &str, limit: usize) -> Vec<SearchResultItem> {
        let key = Self::cache_key(query, limit);
        if self.has_cached(key) {
            tracing::debug!(engine = %self.inner.name(), query, "engine cache hit");
        }
        self.execute_coalesced(key, || self.inner.search(query, limit))
    }

    fn search_with_params(
        &self,
        query: &str,
        limit: usize,
        params: &SearchParams,
    ) -> Vec<SearchResultItem> {
        // For parameterized searches, include params in cache key
        let mut hasher = DefaultHasher::new();
        query.hash(&mut hasher);
        limit.hash(&mut hasher);
        params.locale.hash(&mut hasher);
        params.gl.hash(&mut hasher);
        params.site.hash(&mut hasher);
        params.freshness.hash(&mut hasher);
        let key = hasher.finish();
        if self.has_cached(key) {
            tracing::debug!(engine = %self.inner.name(), query, "engine cache hit (params)");
        }
        self.execute_coalesced(key, || self.inner.search_with_params(query, limit, params))
    }

    fn requires_api_key(&self) -> bool {
        self.inner.requires_api_key()
    }

    fn is_available(&self) -> bool {
        self.inner.is_available()
    }
}

fn load_persistent_cache(path: &Path) -> HashMap<u64, CacheEntry> {
    const MAX_ENTRIES: usize = 1_000;
    const MAX_RESULTS_PER_ENTRY: usize = 100;
    let Ok(bytes) = std::fs::read(path) else {
        return HashMap::new();
    };
    let Ok(disk) = serde_json::from_slice::<DiskCache>(&bytes) else {
        tracing::warn!(path = %path.display(), "ignoring malformed discovery cache");
        return HashMap::new();
    };
    if disk.schema_version != 1 {
        tracing::warn!(
            path = %path.display(),
            schema_version = disk.schema_version,
            "ignoring unsupported discovery cache schema"
        );
        return HashMap::new();
    }
    let now_unix_ms = unix_time_ms();
    let now = Instant::now();
    disk.entries
        .into_iter()
        .take(MAX_ENTRIES)
        .filter(|entry| entry.expires_at_unix_ms > now_unix_ms)
        .filter(|entry| {
            !entry.results.is_empty()
                && entry.results.len() <= MAX_RESULTS_PER_ENTRY
                && entry.results.iter().all(|result| {
                    result.url.starts_with("https://") || result.url.starts_with("http://")
                })
        })
        .map(|entry| {
            let remaining = Duration::from_millis(entry.expires_at_unix_ms - now_unix_ms);
            (
                entry.key,
                CacheEntry {
                    results: entry.results,
                    expires_at: now + remaining,
                    expires_at_unix_ms: entry.expires_at_unix_ms,
                },
            )
        })
        .collect()
}

fn persist_cache(path: &Path, cache: &HashMap<u64, CacheEntry>) -> Result<(), String> {
    let mut entries = cache
        .iter()
        .filter(|(_, entry)| entry.expires_at > Instant::now())
        .map(|(key, entry)| DiskCacheEntry {
            key: *key,
            expires_at_unix_ms: entry.expires_at_unix_ms,
            results: entry.results.clone(),
        })
        .collect::<Vec<_>>();
    entries.sort_by_key(|entry| entry.key);
    let serialized = serde_json::to_vec(&DiskCache {
        schema_version: 1,
        entries,
    })
    .map_err(|error| error.to_string())?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let temporary = path.with_extension(format!("tmp-{}", std::process::id()));
    std::fs::write(&temporary, serialized).map_err(|error| error.to_string())?;
    std::fs::rename(&temporary, path).map_err(|error| error.to_string())
}

fn unix_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct CountingEngine {
        call_count: AtomicUsize,
    }

    struct EmptyEngine {
        call_count: AtomicUsize,
    }

    impl CountingEngine {
        fn new() -> Self {
            Self {
                call_count: AtomicUsize::new(0),
            }
        }
    }

    impl SearchEngine for CountingEngine {
        fn name(&self) -> &str {
            "counting"
        }

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
                decision: None,
            }]
        }
    }

    impl SearchEngine for EmptyEngine {
        fn name(&self) -> &str {
            "empty"
        }

        fn search(&self, _query: &str, _limit: usize) -> Vec<SearchResultItem> {
            self.call_count.fetch_add(1, Ordering::SeqCst);
            Vec::new()
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
    fn test_empty_provider_response_is_not_cached() {
        let inner = Arc::new(EmptyEngine {
            call_count: AtomicUsize::new(0),
        });
        let cached = CachedEngine::new(inner.clone());

        assert!(cached.search("test", 10).is_empty());
        assert!(cached.search("test", 10).is_empty());
        assert_eq!(inner.call_count.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn test_concurrent_identical_misses_share_one_provider_request() {
        let inner = Arc::new(CountingEngine::new());
        let cached = Arc::new(CachedEngine::new(inner.clone()));
        let barrier = Arc::new(std::sync::Barrier::new(8));
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let cached = Arc::clone(&cached);
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    cached.search("same burst", 10)
                })
            })
            .collect();
        for handle in handles {
            assert_eq!(handle.join().unwrap().len(), 1);
        }
        assert_eq!(inner.call_count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn short_lived_discovery_cache_survives_a_process_style_restart() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("provider.json");
        let first_inner = Arc::new(CountingEngine::new());
        let first = CachedEngine::with_persistence(
            first_inner.clone(),
            Duration::from_secs(300),
            path.clone(),
        );
        let first_results = first.search("same after restart", 10);
        assert_eq!(first_results.len(), 1);
        assert_eq!(first_inner.call_count.load(Ordering::SeqCst), 1);
        assert!(path.is_file());
        drop(first);

        let second_inner = Arc::new(CountingEngine::new());
        let second =
            CachedEngine::with_persistence(second_inner.clone(), Duration::from_secs(300), path);
        let cached_results = second.search("same after restart", 10);
        assert_eq!(cached_results.len(), 1);
        assert_eq!(cached_results[0].extraction_quality, "external_cache");
        assert_eq!(second_inner.call_count.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn test_cache_name_delegates() {
        let inner = Arc::new(CountingEngine::new());
        let cached = CachedEngine::new(inner);
        assert_eq!(cached.name(), "counting");
    }
}
