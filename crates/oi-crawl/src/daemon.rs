//! Sentinel crawl daemon — background loop that continuously discovers and
//! indexes web content from staked domains.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{mpsc, Mutex, RwLock};
use tracing::{debug, info, warn};

use crate::discovery::{self, DiscoveredUrl, DiscoverySource};
use crate::drift;
use crate::fetcher;
use crate::frontier::{Frontier, PriorityFactors};
use crate::quality;
use crate::robots::{self, RobotsChecker};
use oi_types::{ContentChunk, CrawlOutput};

/// Events emitted by the crawl daemon for the network layer.
#[derive(Debug, Clone)]
pub enum CrawlEvent {
    /// New content discovered and indexed.
    ContentDiscovered {
        url: String,
        domain: String,
        content_hash: String,
        chunk_count: usize,
        chunks: Vec<ContentChunk>,
    },
    /// Existing content updated (drift detected).
    ContentUpdated {
        url: String,
        domain: String,
        old_hash: String,
        new_hash: String,
        chunk_count: usize,
        chunks: Vec<ContentChunk>,
    },
}

/// Configuration for the crawl daemon.
#[derive(Debug, Clone)]
pub struct DaemonConfig {
    /// Max concurrent crawl tasks.
    pub concurrency: usize,
    /// How often to check feeds (seconds).
    pub feed_poll_interval: Duration,
    /// How often to re-crawl homepages for link discovery (seconds).
    pub homepage_poll_interval: Duration,
    /// Max crawl depth from homepage (hops).
    pub max_depth: usize,
    /// Minimum quality score to index content.
    pub min_quality: f64,
    /// Max entries in content hash cache before evicting oldest.
    pub max_content_hashes: usize,
    /// Max URLs to crawl per domain per epoch.
    pub max_urls_per_domain: usize,
    /// Max URLs to crawl per epoch (all domains).
    pub max_urls_per_epoch: usize,
    /// How often to reset crawl budget counters.
    pub epoch_reset_interval: Duration,
    /// Optional SQLite path for frontier persistence. None = in-memory only.
    pub db_path: Option<String>,
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            concurrency: 4,
            feed_poll_interval: Duration::from_secs(300),    // 5 min
            homepage_poll_interval: Duration::from_secs(1800), // 30 min
            min_quality: 0.3,
            max_depth: 2,
            max_content_hashes: 500_000,
            max_urls_per_domain: 1000,
            max_urls_per_epoch: 50_000,
            epoch_reset_interval: Duration::from_secs(86400), // 24h
            db_path: None,
        }
    }
}

/// A staked domain that the Sentinel should monitor.
#[derive(Debug, Clone)]
pub struct MonitoredDomain {
    pub domain: String,
    pub stake_amount: u64,
    /// Optional RSS/Atom feed URLs.
    pub feed_urls: Vec<String>,
    /// Optional hub pages to monitor (homepage + key pages).
    pub hub_urls: Vec<String>,
}

/// A staked entity (domain or sub-entity) that the Sentinel should monitor.
#[derive(Debug, Clone)]
pub struct MonitoredEntity {
    pub entity: oi_types::entity::EntityRef,
    pub stake_amount: u64,
    /// Optional RSS/Atom feed URLs.
    pub feed_urls: Vec<String>,
    /// Optional hub pages to monitor (homepage + key pages).
    pub hub_urls: Vec<String>,
}

/// Statistics from the crawl daemon.
#[derive(Debug, Clone, Default)]
pub struct DaemonStats {
    pub urls_crawled: u64,
    pub urls_indexed: u64,
    pub urls_rejected: u64,
    pub content_updates: u64,
    pub errors: u64,
    pub domains_monitored: usize,
}

/// The Sentinel crawl daemon.
pub struct CrawlDaemon {
    config: DaemonConfig,
    frontier: Arc<Mutex<Frontier>>,
    robots: Arc<RwLock<RobotsChecker>>,
    /// Domain → content_hash for drift detection.
    content_hashes: Arc<RwLock<HashMap<String, String>>>,
    /// Channel for emitting crawl events to the network layer.
    event_tx: mpsc::UnboundedSender<CrawlEvent>,
    stats: Arc<RwLock<DaemonStats>>,
}

impl CrawlDaemon {
    pub fn new(config: DaemonConfig) -> (Self, mpsc::UnboundedReceiver<CrawlEvent>) {
        let (event_tx, event_rx) = mpsc::unbounded_channel();

        let frontier = if let Some(ref path) = config.db_path {
            match Frontier::with_db(path) {
                Ok(f) => f,
                Err(e) => {
                    warn!("Failed to open frontier DB at {path}: {e}, using in-memory");
                    Frontier::new()
                }
            }
        } else {
            Frontier::new()
        }
        .with_max_urls_per_domain(config.max_urls_per_domain)
        .with_max_urls_per_epoch(config.max_urls_per_epoch);

        let daemon = Self {
            config,
            frontier: Arc::new(Mutex::new(frontier)),
            robots: Arc::new(RwLock::new(RobotsChecker::new())),
            content_hashes: Arc::new(RwLock::new(HashMap::new())),
            event_tx,
            stats: Arc::new(RwLock::new(DaemonStats::default())),
        };
        (daemon, event_rx)
    }

    /// Get current daemon statistics.
    pub async fn stats(&self) -> DaemonStats {
        self.stats.read().await.clone()
    }

    /// Register staked domains to monitor. Called when StakeEvent gossip arrives.
    pub async fn add_domains(&self, domains: Vec<MonitoredDomain>) {
        let mut frontier = self.frontier.lock().await;
        let mut stats = self.stats.write().await;

        for domain in &domains {
            // Add feed URLs with high priority
            for feed_url in &domain.feed_urls {
                frontier.push(
                    feed_url,
                    &domain.domain,
                    PriorityFactors {
                        domain_stake: domain.stake_amount,
                        source_bonus: 3,
                        staleness: u64::MAX, // new = highest staleness
                    },
                );
            }

            // Add hub/homepage URLs
            let hub_urls = if domain.hub_urls.is_empty() {
                vec![format!("https://{}", domain.domain)]
            } else {
                domain.hub_urls.clone()
            };
            for hub_url in &hub_urls {
                frontier.push(
                    hub_url,
                    &domain.domain,
                    PriorityFactors {
                        domain_stake: domain.stake_amount,
                        source_bonus: 2,
                        staleness: u64::MAX,
                    },
                );
            }

            // Try sitemap
            let sitemap_url = format!("https://{}/sitemap.xml", domain.domain);
            frontier.push(
                &sitemap_url,
                &domain.domain,
                PriorityFactors {
                    domain_stake: domain.stake_amount,
                    source_bonus: 2,
                    staleness: u64::MAX,
                },
            );
        }

        stats.domains_monitored = stats.domains_monitored.saturating_add(domains.len());
        info!("Added {} domains to monitor ({} total)", domains.len(), stats.domains_monitored);
    }

    /// Fetch and cache robots.txt for a domain (skips if already cached).
    async fn ensure_robots(&self, domain: &str) {
        // Check cache first to avoid refetching on every URL
        {
            let checker = self.robots.read().await;
            if checker.has_rules(domain) {
                return;
            }
        }

        let robots_url = format!("https://{}/robots.txt", domain);
        match fetcher::fetch_url(&robots_url).await {
            Ok(result) if result.status == 200 => {
                let rules = robots::parse_robots_txt(&result.html);
                if let Some(delay) = rules.crawl_delay {
                    let mut frontier = self.frontier.lock().await;
                    frontier.set_domain_delay(domain, Duration::from_secs(delay));
                }
                let mut checker = self.robots.write().await;
                checker.add_rules(domain, rules);
                debug!("Loaded robots.txt for {}", domain);
            }
            _ => {
                // Cache empty rules so we don't keep retrying
                let mut checker = self.robots.write().await;
                checker.add_rules(domain, robots::RobotsRules::default());
                debug!("No robots.txt for {} (allowing all)", domain);
            }
        }
    }

    /// Process a single URL: fetch, quality-check, index, discover links.
    async fn process_url(
        &self,
        url: &str,
        domain: &str,
        stake_amount: u64,
    ) -> Result<Option<CrawlOutput>, String> {
        // Check robots.txt
        {
            let checker = self.robots.read().await;
            if !checker.is_allowed(url) {
                debug!("Blocked by robots.txt: {}", url);
                return Ok(None);
            }
        }

        // Handle special URLs (feeds, sitemaps)
        if url.ends_with("/sitemap.xml") || (url.contains("sitemap") && url.ends_with(".xml")) {
            return self.process_sitemap(url, domain, stake_amount).await;
        }

        // Fetch
        let fetch_result = fetcher::fetch_url(url).await.map_err(|e| e.to_string())?;
        if fetch_result.status != 200 {
            return Err(format!("HTTP {}", fetch_result.status));
        }

        // Check if this is a feed (RSS/Atom)
        let content_type = fetch_result.content_type.to_lowercase();
        if content_type.contains("xml") || content_type.contains("rss") || content_type.contains("atom") {
            let urls = discovery::parse_feed(&fetch_result.html);
            if !urls.is_empty() {
                self.enqueue_discovered(urls, domain, stake_amount).await;
                return Ok(None);
            }
        }

        // Run through distiller pipeline
        let output = match crate::pipeline::process_html(url, &fetch_result.html, 512, 128) {
            Ok(o) => o,
            Err(e) => return Err(format!("Pipeline error: {}", e)),
        };

        // Quality gate
        let qc = quality::check_quality(&output.markdown);
        if !qc.passed || qc.quality_score < self.config.min_quality {
            debug!("Quality rejected {}: {:?}", url, qc.reason);
            return Err("quality_rejected".to_string());
        }

        // Drift detection — check if content changed
        let old_hash = {
            let hashes = self.content_hashes.read().await;
            hashes.get(url).cloned()
        };

        let is_update = if let Some(ref old) = old_hash {
            drift::has_drifted(old, &output.content_hash, None, None, 0.85)
        } else {
            false
        };

        // Store new hash (evict oldest if at capacity)
        {
            let mut hashes = self.content_hashes.write().await;
            if hashes.len() >= self.config.max_content_hashes {
                // Remove an arbitrary entry to make room
                if let Some(key) = hashes.keys().next().cloned() {
                    hashes.remove(&key);
                }
            }
            hashes.insert(url.to_string(), output.content_hash.clone());
        }

        // Emit event
        let event = if is_update {
            CrawlEvent::ContentUpdated {
                url: url.to_string(),
                domain: domain.to_string(),
                old_hash: old_hash.unwrap_or_default(),
                new_hash: output.content_hash.clone(),
                chunk_count: output.chunks.len(),
                chunks: output.chunks.clone(),
            }
        } else {
            CrawlEvent::ContentDiscovered {
                url: url.to_string(),
                domain: domain.to_string(),
                content_hash: output.content_hash.clone(),
                chunk_count: output.chunks.len(),
                chunks: output.chunks.clone(),
            }
        };
        let _ = self.event_tx.send(event);

        // Extract and enqueue internal links for further crawling
        let links = discovery::extract_links(&fetch_result.html, url);
        self.enqueue_discovered(links, domain, stake_amount).await;

        Ok(Some(output))
    }

    async fn process_sitemap(
        &self,
        url: &str,
        domain: &str,
        stake_amount: u64,
    ) -> Result<Option<CrawlOutput>, String> {
        let result = fetcher::fetch_url(url).await.map_err(|e| e.to_string())?;
        if result.status != 200 {
            return Err(format!("Sitemap HTTP {}", result.status));
        }
        let urls = discovery::parse_sitemap(&result.html);
        info!("Parsed sitemap {}: {} URLs", url, urls.len());
        self.enqueue_discovered(urls, domain, stake_amount).await;
        Ok(None)
    }

    async fn enqueue_discovered(&self, urls: Vec<DiscoveredUrl>, domain: &str, stake_amount: u64) {
        let mut frontier = self.frontier.lock().await;
        for discovered in urls {
            let bonus = match discovered.source {
                DiscoverySource::Feed => 3,
                DiscoverySource::Sitemap => 2,
                DiscoverySource::Link => 1,
            };
            frontier.push(
                &discovered.url,
                domain,
                PriorityFactors {
                    domain_stake: stake_amount,
                    source_bonus: bonus,
                    staleness: 0,
                },
            );
        }
    }

    /// Run the crawl daemon. Continuously pops URLs from the frontier and
    /// processes them with bounded concurrency.
    ///
    /// Stops when the `shutdown` receiver is triggered. All in-flight crawl
    /// tasks are awaited before returning (graceful shutdown).
    pub async fn run(self: Arc<Self>, mut shutdown: mpsc::Receiver<()>) {
        info!(
            "Sentinel daemon starting (concurrency={})",
            self.config.concurrency,
        );

        let semaphore = Arc::new(tokio::sync::Semaphore::new(self.config.concurrency));
        let mut tasks = tokio::task::JoinSet::new();
        let mut epoch_reset_interval =
            tokio::time::interval(self.config.epoch_reset_interval);
        let mut checkpoint_interval = tokio::time::interval(Duration::from_secs(60));
        // Consume the first immediate tick so it doesn't fire at t=0
        epoch_reset_interval.tick().await;
        checkpoint_interval.tick().await;

        loop {
            tokio::select! {
                _ = shutdown.recv() => {
                    info!("Sentinel daemon shutting down, waiting for {} in-flight tasks", tasks.len());
                    break;
                }
                // Reap completed tasks to avoid unbounded growth
                Some(result) = tasks.join_next(), if !tasks.is_empty() => {
                    if let Err(e) = result {
                        warn!("Crawl task panicked: {e}");
                    }
                }
                _ = epoch_reset_interval.tick() => {
                    info!("Epoch reset: clearing crawl budget counters");
                    let mut frontier = self.frontier.lock().await;
                    frontier.reset_epoch();
                }
                _ = checkpoint_interval.tick() => {
                    let frontier = self.frontier.lock().await;
                    if let Err(e) = frontier.checkpoint() {
                        warn!("Frontier checkpoint failed: {e}");
                    }
                }
                _ = tokio::time::sleep(Duration::from_millis(100)) => {
                    // Try to pop from frontier
                    let entry = {
                        let mut frontier = self.frontier.lock().await;
                        frontier.pop()
                    };

                    if let Some(entry) = entry {
                        let permit = match semaphore.clone().acquire_owned().await {
                            Ok(p) => p,
                            Err(e) => {
                                warn!("Semaphore closed, skipping URL {}: {e}", entry.url);
                                continue;
                            }
                        };
                        let daemon = self.clone();
                        let domain = entry.domain.clone();

                        tasks.spawn(async move {
                            daemon.ensure_robots(&domain).await;

                            match daemon.process_url(&entry.url, &entry.domain, entry.domain_stake).await {
                                Ok(Some(_output)) => {
                                    let mut stats = daemon.stats.write().await;
                                    stats.urls_crawled += 1;
                                    stats.urls_indexed += 1;
                                    info!("Indexed: {}", entry.url);
                                    drop(stats);
                                    daemon.frontier.lock().await.record_crawl(&entry.domain);
                                }
                                Ok(None) => {
                                    let mut stats = daemon.stats.write().await;
                                    stats.urls_crawled += 1;
                                    debug!("Processed (no content): {}", entry.url);
                                    drop(stats);
                                    daemon.frontier.lock().await.record_crawl(&entry.domain);
                                }
                                Err(ref e) if e == "quality_rejected" => {
                                    let mut stats = daemon.stats.write().await;
                                    stats.urls_crawled += 1;
                                    stats.urls_rejected += 1;
                                    debug!("Rejected: {}", entry.url);
                                }
                                Err(e) => {
                                    let mut stats = daemon.stats.write().await;
                                    stats.urls_crawled += 1;
                                    stats.errors += 1;
                                    warn!("Error crawling {}: {}", entry.url, e);
                                }
                            }

                            drop(permit);
                        });
                    }
                }
            }
        }

        // Graceful shutdown: checkpoint frontier + await all in-flight tasks
        {
            let frontier = self.frontier.lock().await;
            if let Err(e) = frontier.checkpoint() {
                warn!("Frontier checkpoint on shutdown failed: {e}");
            }
        }
        while let Some(result) = tasks.join_next().await {
            if let Err(e) = result {
                warn!("Crawl task panicked during shutdown: {e}");
            }
        }
        info!("Sentinel daemon stopped");
    }
}

/// Collect all staked domains from StakeEvent gossip messages.
/// This is the bridge between the network layer and the daemon.
pub fn domains_from_stake_events(
    events: &[(String, u64)], // (domain, stake_amount)
) -> Vec<MonitoredDomain> {
    let mut domain_map: HashMap<String, u64> = HashMap::new();
    for (domain, amount) in events {
        *domain_map.entry(domain.clone()).or_insert(0) += amount;
    }

    domain_map
        .into_iter()
        .map(|(domain, stake_amount)| MonitoredDomain {
            domain,
            stake_amount,
            feed_urls: Vec::new(),
            hub_urls: Vec::new(),
        })
        .collect()
}

/// Collect staked entities from StakeEvent gossip messages.
/// Parses entity refs and aggregates stake amounts.
pub fn entities_from_stake_events(
    events: &[(String, u64)], // (entity_ref_string, stake_amount)
) -> Vec<MonitoredEntity> {
    use oi_types::entity::EntityRef;
    let mut entity_map: HashMap<String, (EntityRef, u64)> = HashMap::new();
    for (raw, amount) in events {
        if let Ok(entity) = EntityRef::parse(raw) {
            let key = entity.canonical();
            entity_map
                .entry(key)
                .and_modify(|(_, total)| *total += amount)
                .or_insert_with(|| (entity, *amount));
        }
    }

    entity_map
        .into_values()
        .map(|(entity, stake_amount)| MonitoredEntity {
            entity,
            stake_amount,
            feed_urls: Vec::new(),
            hub_urls: Vec::new(),
        })
        .collect()
}

/// Discover feed URLs for a domain by checking common feed paths.
pub async fn discover_feeds(domain: &str) -> Vec<String> {
    let candidates = [
        format!("https://{}/feed", domain),
        format!("https://{}/rss", domain),
        format!("https://{}/feed.xml", domain),
        format!("https://{}/rss.xml", domain),
        format!("https://{}/atom.xml", domain),
        format!("https://{}/index.xml", domain),
    ];

    let mut feeds = Vec::new();
    for url in &candidates {
        match fetcher::fetch_url(url).await {
            Ok(result) if result.status == 200 => {
                let parsed = discovery::parse_feed(&result.html);
                if !parsed.is_empty() {
                    feeds.push(url.clone());
                }
            }
            _ => {}
        }
    }
    feeds
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_domains_from_stake_events() {
        let events = vec![
            ("example.com".to_string(), 10_000_000u64),
            ("example.com".to_string(), 5_000_000u64),
            ("other.org".to_string(), 20_000_000u64),
        ];
        let domains = domains_from_stake_events(&events);
        assert_eq!(domains.len(), 2);

        let example = domains.iter().find(|d| d.domain == "example.com").unwrap();
        assert_eq!(example.stake_amount, 15_000_000);
    }

    #[tokio::test]
    async fn test_daemon_creation() {
        let config = DaemonConfig::default();
        let (daemon, _rx) = CrawlDaemon::new(config);
        let stats = daemon.stats().await;
        assert_eq!(stats.urls_crawled, 0);
        assert_eq!(stats.domains_monitored, 0);
    }

    #[tokio::test]
    async fn test_add_domains() {
        let (daemon, _rx) = CrawlDaemon::new(DaemonConfig::default());
        daemon
            .add_domains(vec![MonitoredDomain {
                domain: "example.com".to_string(),
                stake_amount: 10_000_000,
                feed_urls: vec!["https://example.com/rss".to_string()],
                hub_urls: vec![],
            }])
            .await;

        let stats = daemon.stats().await;
        assert_eq!(stats.domains_monitored, 1);

        let frontier = daemon.frontier.lock().await;
        // Should have: feed URL + homepage + sitemap = 3 URLs
        assert!(frontier.len() >= 2);
    }

    #[test]
    fn test_entities_from_stake_events_aggregation() {
        let events = vec![
            ("youtube.com/c/3blue1brown".to_string(), 10_000u64),
            ("youtube.com/c/3blue1brown".to_string(), 5_000u64),
            ("reddit.com/r/rust".to_string(), 20_000u64),
        ];
        let entities = entities_from_stake_events(&events);
        assert_eq!(entities.len(), 2);

        let yt = entities
            .iter()
            .find(|e| e.entity.canonical() == "youtube.com/c/3blue1brown")
            .unwrap();
        assert_eq!(yt.stake_amount, 15_000);

        let reddit = entities
            .iter()
            .find(|e| e.entity.canonical() == "reddit.com/r/rust")
            .unwrap();
        assert_eq!(reddit.stake_amount, 20_000);
    }

    #[test]
    fn test_entities_from_stake_events_normalization() {
        // Different casing should merge into the same canonical key
        let events = vec![
            ("youtube.com/c/3Blue1Brown".to_string(), 10_000u64),
            ("youtube.com/c/3blue1brown".to_string(), 5_000u64),
        ];
        let entities = entities_from_stake_events(&events);
        assert_eq!(entities.len(), 1);
        assert_eq!(entities[0].stake_amount, 15_000);
        assert_eq!(entities[0].entity.canonical(), "youtube.com/c/3blue1brown");
    }

    #[test]
    fn test_entities_from_stake_events_skips_invalid() {
        let events = vec![
            ("youtube.com/c/test".to_string(), 10_000u64),
            ("".to_string(), 5_000u64),               // empty — invalid
            ("no-dot".to_string(), 5_000u64),          // no dot — invalid
        ];
        let entities = entities_from_stake_events(&events);
        assert_eq!(entities.len(), 1);
        assert_eq!(entities[0].entity.canonical(), "youtube.com/c/test");
    }

    #[test]
    fn test_entities_from_stake_events_twitter_alias() {
        // twitter.com → x.com normalization should merge
        let events = vec![
            ("twitter.com/elonmusk".to_string(), 10_000u64),
            ("x.com/elonmusk".to_string(), 5_000u64),
        ];
        let entities = entities_from_stake_events(&events);
        assert_eq!(entities.len(), 1);
        assert_eq!(entities[0].stake_amount, 15_000);
        assert_eq!(entities[0].entity.domain(), "x.com");
    }

    #[test]
    fn test_entities_from_stake_events_domain_only() {
        let events = vec![
            ("example.com".to_string(), 10_000u64),
        ];
        let entities = entities_from_stake_events(&events);
        assert_eq!(entities.len(), 1);
        assert!(entities[0].entity.is_domain_only());
        assert_eq!(entities[0].entity.domain(), "example.com");
    }
}
