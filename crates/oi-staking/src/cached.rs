//! Caching decorator for StakeRegistry — adds TTL-based caching to any registry.

use oi_types::traits::StakeRegistry;
use oi_types::{Result, StakeInfo};
use std::collections::HashMap;
use std::sync::RwLock;
use std::time::{Duration, Instant};

struct CacheEntry {
    info: Option<StakeInfo>,
    cached_at: Instant,
}

/// A caching wrapper around any `StakeRegistry`.
///
/// Caches `get_stake()` lookups with a configurable TTL. Batch lookups
/// check the cache per-domain and only fetch misses from the inner registry.
/// `list_active_stakes()` and `is_staked()` delegate directly (no caching benefit).
pub struct CachedStakeRegistry<T: StakeRegistry> {
    inner: T,
    cache: RwLock<HashMap<String, CacheEntry>>,
    ttl: Duration,
}

impl<T: StakeRegistry> CachedStakeRegistry<T> {
    pub fn new(inner: T) -> Self {
        Self {
            inner,
            cache: RwLock::new(HashMap::new()),
            ttl: Duration::from_secs(60),
        }
    }

    pub fn with_ttl(mut self, ttl: Duration) -> Self {
        self.ttl = ttl;
        self
    }

    /// Remove a cached entry for a specific domain.
    pub fn invalidate(&self, domain: &str) {
        let mut cache = self.cache.write().unwrap();
        cache.remove(domain);
    }

    /// Clear the entire cache.
    pub fn clear(&self) {
        let mut cache = self.cache.write().unwrap();
        cache.clear();
    }
}

impl<T: StakeRegistry> Default for CachedStakeRegistry<T>
where
    T: Default,
{
    fn default() -> Self {
        Self::new(T::default())
    }
}

impl<T: StakeRegistry> StakeRegistry for CachedStakeRegistry<T> {
    fn get_stake(&self, domain: &str) -> Result<Option<StakeInfo>> {
        // Check cache (read lock)
        {
            let cache = self.cache.read().unwrap();
            if let Some(entry) = cache.get(domain)
                && entry.cached_at.elapsed() < self.ttl
            {
                return Ok(entry.info.clone());
            }
        }

        // Cache miss or expired — fetch from inner
        let result = self.inner.get_stake(domain)?;

        // Write to cache
        {
            let mut cache = self.cache.write().unwrap();
            cache.insert(
                domain.to_string(),
                CacheEntry {
                    info: result.clone(),
                    cached_at: Instant::now(),
                },
            );
        }

        Ok(result)
    }

    fn list_active_stakes(&self) -> Result<Vec<StakeInfo>> {
        self.inner.list_active_stakes()
    }

    fn is_staked(&self, domain: &str) -> Result<bool> {
        self.inner.is_staked(domain)
    }

    fn get_stakes_batch(&self, domains: &[&str]) -> Result<HashMap<String, StakeInfo>> {
        let now = Instant::now();
        let mut result = HashMap::new();
        let mut misses = Vec::new();

        // Check cache for each domain
        {
            let cache = self.cache.read().unwrap();
            for &domain in domains {
                if let Some(entry) = cache.get(domain)
                    && now.duration_since(entry.cached_at) < self.ttl
                {
                    if let Some(ref info) = entry.info {
                        result.insert(domain.to_string(), info.clone());
                    }
                    continue;
                }
                misses.push(domain);
            }
        }

        if misses.is_empty() {
            return Ok(result);
        }

        // Fetch misses from inner
        let fetched = self.inner.get_stakes_batch(&misses)?;

        // Update cache and result
        {
            let mut cache = self.cache.write().unwrap();
            for domain in &misses {
                let info = fetched.get(*domain).cloned();
                if let Some(ref info) = info {
                    result.insert(domain.to_string(), info.clone());
                }
                cache.insert(
                    domain.to_string(),
                    CacheEntry {
                        info,
                        cached_at: Instant::now(),
                    },
                );
            }
        }

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::InMemoryStakeRegistry;

    fn make_stake(domain: &str, amount: u64) -> StakeInfo {
        StakeInfo {
            domain: domain.to_string(),
            amount_lamports: amount,
            amount_usd: amount as f64 / 1_000_000.0,
            staked_at: 1700000000,
            cooldown_until: None,
            wallet: "11111111111111111111111111111111".to_string(),
            dns_verified: true,
            reputation: 1.0,
            entity: None,
            disputed: false,
        }
    }

    #[test]
    fn test_cache_hit() {
        let inner = InMemoryStakeRegistry::new();
        inner
            .add_stake(make_stake("example.com", 1_000_000))
            .unwrap();

        let cached = CachedStakeRegistry::new(inner);

        // First call populates cache
        let r1 = cached.get_stake("example.com").unwrap();
        assert!(r1.is_some());

        // Remove from inner — cache should still return it
        cached.inner.remove_stake("example.com").unwrap();
        let r2 = cached.get_stake("example.com").unwrap();
        assert!(r2.is_some());
        assert_eq!(r2.unwrap().amount_lamports, 1_000_000);
    }

    #[test]
    fn test_cache_expiry() {
        let inner = InMemoryStakeRegistry::new();
        inner
            .add_stake(make_stake("example.com", 1_000_000))
            .unwrap();

        let cached = CachedStakeRegistry::new(inner).with_ttl(Duration::from_millis(0));

        // First lookup
        let r1 = cached.get_stake("example.com").unwrap();
        assert!(r1.is_some());

        // Remove from inner — with TTL=0 the cache is always expired
        cached.inner.remove_stake("example.com").unwrap();
        let r2 = cached.get_stake("example.com").unwrap();
        assert!(r2.is_none());
    }

    #[test]
    fn test_invalidate() {
        let inner = InMemoryStakeRegistry::new();
        inner
            .add_stake(make_stake("example.com", 1_000_000))
            .unwrap();

        let cached = CachedStakeRegistry::new(inner);

        // Populate cache
        cached.get_stake("example.com").unwrap();

        // Remove from inner and invalidate cache
        cached.inner.remove_stake("example.com").unwrap();
        cached.invalidate("example.com");

        // Should now return None (fetches from inner)
        let r = cached.get_stake("example.com").unwrap();
        assert!(r.is_none());
    }

    #[test]
    fn test_batch_partial_hit() {
        let inner = InMemoryStakeRegistry::new();
        inner.add_stake(make_stake("a.com", 100)).unwrap();
        inner.add_stake(make_stake("b.com", 200)).unwrap();

        let cached = CachedStakeRegistry::new(inner);

        // Pre-populate cache for a.com only
        cached.get_stake("a.com").unwrap();

        // Batch lookup for both — b.com should be fetched from inner
        let batch = cached.get_stakes_batch(&["a.com", "b.com"]).unwrap();
        assert_eq!(batch.len(), 2);
        assert_eq!(batch["a.com"].amount_lamports, 100);
        assert_eq!(batch["b.com"].amount_lamports, 200);
    }
}
