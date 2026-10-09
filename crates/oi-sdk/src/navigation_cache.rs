//! Persistent metadata cache for canonical navigational destinations.
//!
//! Some first-party homepages are JavaScript shells and yield no crawlable
//! body. They still need to survive process restarts after an external engine
//! has resolved the destination. These records are deliberately kept outside
//! the content index so they can never become factual evidence.

use oi_types::SearchResultItem;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct NavigationRecord {
    url: String,
    title: String,
    snippet: String,
    #[serde(default = "default_destination_kind")]
    destination_kind: String,
    discovered_at: u64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct NavigationFile {
    #[serde(default)]
    destinations: HashMap<String, NavigationRecord>,
    #[serde(default)]
    route_misses: HashMap<String, u64>,
}

#[derive(Debug, Default)]
pub(crate) struct NavigationCache {
    path: Option<PathBuf>,
    destinations: HashMap<String, NavigationRecord>,
    route_misses: HashMap<String, u64>,
}

impl NavigationCache {
    pub(crate) fn load(index_path: Option<&str>) -> Self {
        let path = index_path.map(|path| Path::new(path).join("navigation.json"));
        let file = path
            .as_ref()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .and_then(|json| serde_json::from_str::<NavigationFile>(&json).ok())
            .unwrap_or_default();
        Self {
            path,
            destinations: file.destinations,
            route_misses: file.route_misses,
        }
    }

    pub(crate) fn get(&self, key: &str) -> Option<SearchResultItem> {
        let record = self.destinations.get(key)?;
        Some(SearchResultItem {
            result_id: format!("navigation:{key}"),
            rank: 0,
            url: record.url.clone(),
            title: record.title.clone(),
            snippet: record.snippet.clone(),
            content_md: String::new(),
            semantic_score: 0.0,
            bm25_score: 0.0,
            final_score: 0.0,
            stake_truth: 0,
            stake_usd: 0,
            stake_type: "none".to_string(),
            reputation: 0.0,
            content_hash: String::new(),
            extraction_quality: record.destination_kind.clone(),
            source: "navigation_cache".to_string(),
            entity: None,
            content_kind: None,
            crawled_at: None,
            published_at: None,
            trust_verdict: "external".to_string(),
            reputation_votes: 0,
            disputed: false,
            facts: None,
            content_relevant: None,
            domain_first_seen_at: Some(record.discovered_at as i64),
            domain_url_count: None,
            domain_reputation_score: None,
            domain_total_votes: None,
            decision: None,
        })
    }

    pub(crate) fn remember(
        &mut self,
        key: String,
        result: &SearchResultItem,
        canonical_url: String,
    ) {
        let record = NavigationRecord {
            url: canonical_url,
            title: result.title.clone(),
            snippet: result.snippet.clone(),
            destination_kind: if result.extraction_quality == "verified_transaction_route" {
                result.extraction_quality.clone()
            } else {
                default_destination_kind()
            },
            discovered_at: unix_now(),
        };
        if self
            .destinations
            .get(&key)
            .is_some_and(|existing| existing.url == record.url)
        {
            return;
        }
        self.route_misses.remove(&key);
        self.destinations.insert(key, record);
        if let Err(error) = self.persist() {
            tracing::warn!("failed to persist navigation discovery cache: {error}");
        }
    }

    /// Avoid repeatedly probing a route that every candidate origin answered
    /// with a definitive not-found response. Transport failures are never
    /// recorded as misses by the caller. The short TTL lets newly launched
    /// application/login routes appear without a process restart.
    pub(crate) fn route_probe_due(&self, key: &str) -> bool {
        const MISS_TTL_SECS: u64 = 300;
        !self
            .route_misses
            .get(key)
            .is_some_and(|checked_at| unix_now().saturating_sub(*checked_at) < MISS_TTL_SECS)
    }

    pub(crate) fn remember_route_miss(&mut self, key: String) {
        self.route_misses.insert(key, unix_now());
        if let Err(error) = self.persist() {
            tracing::warn!("failed to persist transaction-route miss: {error}");
        }
    }

    fn persist(&self) -> std::io::Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_vec_pretty(&NavigationFile {
            destinations: self.destinations.clone(),
            route_misses: self.route_misses.clone(),
        })
        .map_err(std::io::Error::other)?;
        std::fs::write(path, json)
    }
}

fn default_destination_kind() -> String {
    "discovery_metadata".to_string()
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn destination(url: &str) -> SearchResultItem {
        SearchResultItem {
            result_id: url.to_string(),
            rank: 1,
            url: url.to_string(),
            title: "Perplexity".to_string(),
            snippet: "AI search".to_string(),
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
            source: "test".to_string(),
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
        }
    }

    #[test]
    fn metadata_destination_survives_reload_without_becoming_content() {
        let dir = std::env::temp_dir().join(format!(
            "opensonarx-navigation-cache-{}",
            std::process::id()
        ));
        let index_path = dir.to_string_lossy().to_string();
        let mut cache = NavigationCache::load(Some(&index_path));
        cache.remember(
            "perplexity".to_string(),
            &destination("https://www.perplexity.ai/search/example"),
            "https://www.perplexity.ai/".to_string(),
        );

        let restored = NavigationCache::load(Some(&index_path))
            .get("perplexity")
            .expect("persisted destination");
        assert_eq!(restored.url, "https://www.perplexity.ai/");
        assert_eq!(restored.source, "navigation_cache");
        assert!(restored.content_md.is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn verified_transaction_kind_and_definitive_miss_survive_reload() {
        let dir = std::env::temp_dir().join(format!(
            "opensonarx-transaction-cache-{}",
            std::process::id()
        ));
        let index_path = dir.to_string_lossy().to_string();
        let mut verified = destination("https://claude.ai/login");
        verified.extraction_quality = "verified_transaction_route".to_string();

        let mut cache = NavigationCache::load(Some(&index_path));
        cache.remember(
            "transaction:claude:login".to_string(),
            &verified,
            verified.url.clone(),
        );
        cache.remember_route_miss("transaction:missing:login".to_string());

        let restored = NavigationCache::load(Some(&index_path));
        let route = restored
            .get("transaction:claude:login")
            .expect("persisted verified route");
        assert_eq!(route.url, "https://claude.ai/login");
        assert_eq!(route.extraction_quality, "verified_transaction_route");
        assert!(!restored.route_probe_due("transaction:missing:login"));
        let _ = std::fs::remove_dir_all(dir);
    }
}
