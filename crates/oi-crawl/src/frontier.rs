//! URL frontier — priority queue for crawl scheduling.
//!
//! Orders URLs by priority (higher = crawl sooner) and enforces per-domain
//! rate limits. Uses in-memory storage with a binary heap.
//! Optionally backed by SQLite for persistence across restarts.

use std::collections::{BinaryHeap, HashMap, HashSet};
use std::time::Duration;

use crate::crawl_db::CrawlDb;

/// Current Unix timestamp in seconds.
fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

/// An entry in the URL frontier with priority metadata.
#[derive(Debug, Clone)]
pub struct FrontierEntry {
    pub url: String,
    pub domain: String,
    pub priority: u64,
    /// Original domain stake amount (for passing to child URL discovery).
    pub domain_stake: u64,
    /// Unix timestamp (seconds) when this entry was added.
    pub added_at: i64,
}

impl PartialEq for FrontierEntry {
    fn eq(&self, other: &Self) -> bool {
        self.url == other.url
    }
}

impl Eq for FrontierEntry {}

impl PartialOrd for FrontierEntry {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for FrontierEntry {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Higher priority first; break ties by earliest added_at
        self.priority
            .cmp(&other.priority)
            .then_with(|| other.added_at.cmp(&self.added_at))
    }
}

/// Priority factors for computing a URL's crawl priority.
#[derive(Debug, Clone)]
pub struct PriorityFactors {
    /// Stake amount for the domain (higher stake = higher priority).
    pub domain_stake: u64,
    /// Discovery source bonus: Feed=3, Sitemap=2, Link=1.
    pub source_bonus: u64,
    /// Time since last crawl in seconds (longer = higher priority).
    pub staleness: u64,
}

impl PriorityFactors {
    pub fn compute(&self) -> u64 {
        // Weighted sum: stake is most important, then staleness, then source
        let stake_score = (self.domain_stake / 1_000_000).min(100);
        let stale_score = (self.staleness / 3600).min(48); // cap at 48h
        let source_score = self.source_bonus;
        stake_score * 10 + stale_score * 5 + source_score * 3
    }
}

/// The URL frontier with per-domain rate limiting and crawl budgets.
pub struct Frontier {
    queue: BinaryHeap<FrontierEntry>,
    seen: HashSet<String>,
    /// Per-domain last-crawl timestamps (Unix seconds) for rate limiting.
    domain_last_crawl: HashMap<String, i64>,
    /// Per-domain crawl delays (from robots.txt).
    domain_delays: HashMap<String, Duration>,
    /// Default delay between crawls to the same domain.
    default_delay: Duration,
    /// Maximum queue size.
    max_size: usize,
    // -- Crawl budget fields --
    /// Per-domain crawl counts for budget enforcement.
    domain_crawl_counts: HashMap<String, usize>,
    /// Maximum URLs to crawl per domain per epoch.
    max_urls_per_domain: usize,
    /// Total URLs crawled this epoch (across all domains).
    total_crawled_this_epoch: usize,
    /// Maximum URLs to crawl per epoch.
    max_urls_per_epoch: usize,
    // -- Persistence --
    db: Option<CrawlDb>,
}

impl Frontier {
    pub fn new() -> Self {
        Self {
            queue: BinaryHeap::new(),
            seen: HashSet::new(),
            domain_last_crawl: HashMap::new(),
            domain_delays: HashMap::new(),
            default_delay: Duration::from_secs(2),
            max_size: 100_000,
            domain_crawl_counts: HashMap::new(),
            max_urls_per_domain: 1000,
            total_crawled_this_epoch: 0,
            max_urls_per_epoch: 50_000,
            db: None,
        }
    }

    /// Open a frontier backed by a SQLite database, restoring saved state.
    pub fn with_db(path: &str) -> Result<Self, String> {
        let db = CrawlDb::open(path)?;
        let (entries, seen, domain_last_crawl, domain_delays, domain_crawl_counts) =
            db.load_frontier()?;

        let total_crawled: usize = domain_crawl_counts.values().sum();

        let mut queue = BinaryHeap::new();
        for entry in entries {
            queue.push(entry);
        }

        Ok(Self {
            queue,
            seen,
            domain_last_crawl,
            domain_delays,
            default_delay: Duration::from_secs(2),
            max_size: 100_000,
            domain_crawl_counts,
            max_urls_per_domain: 1000,
            total_crawled_this_epoch: total_crawled,
            max_urls_per_epoch: 50_000,
            db: Some(db),
        })
    }

    pub fn with_default_delay(mut self, delay: Duration) -> Self {
        self.default_delay = delay;
        self
    }

    pub fn with_max_size(mut self, max_size: usize) -> Self {
        self.max_size = max_size;
        self
    }

    pub fn with_max_urls_per_domain(mut self, max: usize) -> Self {
        self.max_urls_per_domain = max;
        self
    }

    pub fn with_max_urls_per_epoch(mut self, max: usize) -> Self {
        self.max_urls_per_epoch = max;
        self
    }

    /// Set the crawl delay for a specific domain (from robots.txt).
    pub fn set_domain_delay(&mut self, domain: &str, delay: Duration) {
        self.domain_delays.insert(domain.to_string(), delay);
    }

    /// Add a URL to the frontier. Returns false if already seen, queue full, or budget exceeded.
    pub fn push(&mut self, url: &str, domain: &str, priority: PriorityFactors) -> bool {
        // Budget checks
        let domain_count = self.domain_crawl_counts.get(domain).copied().unwrap_or(0);
        if domain_count >= self.max_urls_per_domain {
            return false;
        }
        if self.total_crawled_this_epoch >= self.max_urls_per_epoch {
            return false;
        }

        if self.seen.contains(url) || self.queue.len() >= self.max_size {
            return false;
        }
        self.seen.insert(url.to_string());
        self.queue.push(FrontierEntry {
            url: url.to_string(),
            domain: domain.to_string(),
            priority: priority.compute(),
            domain_stake: priority.domain_stake,
            added_at: unix_now(),
        });
        true
    }

    /// Pop the highest-priority URL that respects rate limits.
    /// Returns None if the queue is empty or all URLs are rate-limited.
    pub fn pop(&mut self) -> Option<FrontierEntry> {
        let now = unix_now();
        let mut deferred = Vec::new();

        while let Some(entry) = self.queue.pop() {
            let delay = self
                .domain_delays
                .get(&entry.domain)
                .copied()
                .unwrap_or(self.default_delay);

            if let Some(&last) = self.domain_last_crawl.get(&entry.domain) {
                let elapsed_secs = (now - last).max(0) as u64;
                if elapsed_secs < delay.as_secs() {
                    deferred.push(entry);
                    continue;
                }
            }

            // This domain is OK to crawl — re-add deferred items
            for d in deferred {
                self.queue.push(d);
            }
            self.domain_last_crawl.insert(entry.domain.clone(), now);
            return Some(entry);
        }

        // All items were rate-limited — put them back
        for d in deferred {
            self.queue.push(d);
        }
        None
    }

    /// Record that a URL was crawled (for re-crawl support — removes from seen set).
    pub fn mark_for_recrawl(&mut self, url: &str) {
        self.seen.remove(url);
    }

    /// Record a successful crawl for budget tracking.
    pub fn record_crawl(&mut self, domain: &str) {
        *self.domain_crawl_counts.entry(domain.to_string()).or_insert(0) += 1;
        self.total_crawled_this_epoch += 1;
    }

    /// Reset all per-epoch crawl counts (call at epoch boundaries).
    pub fn reset_epoch(&mut self) {
        self.domain_crawl_counts.clear();
        self.total_crawled_this_epoch = 0;
    }

    /// Get the number of URLs crawled for a specific domain this epoch.
    pub fn domain_crawl_count(&self, domain: &str) -> usize {
        self.domain_crawl_counts.get(domain).copied().unwrap_or(0)
    }

    /// Get the total URLs crawled this epoch.
    pub fn total_crawled(&self) -> usize {
        self.total_crawled_this_epoch
    }

    /// Number of URLs in the queue.
    pub fn len(&self) -> usize {
        self.queue.len()
    }

    /// Whether the queue is empty.
    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    /// Total number of unique URLs ever seen.
    pub fn seen_count(&self) -> usize {
        self.seen.len()
    }

    /// Extract the domain from a URL string.
    pub fn domain_of(url: &str) -> Option<String> {
        url::Url::parse(url).ok().and_then(|u| u.domain().map(|d| d.to_lowercase()))
    }

    /// Flush current state to the SQLite database (no-op if no DB attached).
    pub fn checkpoint(&self) -> Result<(), String> {
        if let Some(ref db) = self.db {
            let entries: Vec<FrontierEntry> = self.queue.iter().cloned().collect();
            db.save_frontier(
                &entries,
                &self.seen,
                &self.domain_last_crawl,
                &self.domain_delays,
                &self.domain_crawl_counts,
            )?;
        }
        Ok(())
    }
}

impl Default for Frontier {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_push_and_pop() {
        let mut frontier = Frontier::new().with_default_delay(Duration::from_millis(0));

        frontier.push(
            "https://example.com/page1",
            "example.com",
            PriorityFactors { domain_stake: 10_000_000, source_bonus: 3, staleness: 7200 },
        );
        frontier.push(
            "https://example.com/page2",
            "example.com",
            PriorityFactors { domain_stake: 50_000_000, source_bonus: 2, staleness: 3600 },
        );

        assert_eq!(frontier.len(), 2);

        // Higher priority (higher stake) should come first
        let first = frontier.pop().unwrap();
        assert_eq!(first.url, "https://example.com/page2");
    }

    #[test]
    fn test_dedup() {
        let mut frontier = Frontier::new();

        let added1 = frontier.push(
            "https://example.com/page1",
            "example.com",
            PriorityFactors { domain_stake: 0, source_bonus: 1, staleness: 0 },
        );
        let added2 = frontier.push(
            "https://example.com/page1",
            "example.com",
            PriorityFactors { domain_stake: 0, source_bonus: 1, staleness: 0 },
        );

        assert!(added1);
        assert!(!added2);
        assert_eq!(frontier.len(), 1);
    }

    #[test]
    fn test_rate_limiting() {
        let mut frontier = Frontier::new().with_default_delay(Duration::from_secs(60));

        frontier.push(
            "https://example.com/page1",
            "example.com",
            PriorityFactors { domain_stake: 0, source_bonus: 1, staleness: 0 },
        );
        frontier.push(
            "https://example.com/page2",
            "example.com",
            PriorityFactors { domain_stake: 0, source_bonus: 1, staleness: 0 },
        );
        frontier.push(
            "https://other.com/page1",
            "other.com",
            PriorityFactors { domain_stake: 0, source_bonus: 1, staleness: 0 },
        );

        // First pop should succeed for one domain
        let first = frontier.pop().unwrap();
        // Second pop should get the other domain (same domain rate-limited)
        let second = frontier.pop();
        if let Some(ref s) = second {
            assert_ne!(s.domain, first.domain);
        }
    }

    #[test]
    fn test_mark_for_recrawl() {
        let mut frontier = Frontier::new().with_default_delay(Duration::from_millis(0));

        frontier.push(
            "https://example.com/page1",
            "example.com",
            PriorityFactors { domain_stake: 0, source_bonus: 1, staleness: 0 },
        );
        assert_eq!(frontier.seen_count(), 1);

        let _ = frontier.pop();
        frontier.mark_for_recrawl("https://example.com/page1");
        assert_eq!(frontier.seen_count(), 0);

        // Can re-add after marking for recrawl
        let added = frontier.push(
            "https://example.com/page1",
            "example.com",
            PriorityFactors { domain_stake: 0, source_bonus: 1, staleness: 0 },
        );
        assert!(added);
    }

    #[test]
    fn test_priority_computation() {
        let high = PriorityFactors { domain_stake: 50_000_000, source_bonus: 3, staleness: 86400 };
        let low = PriorityFactors { domain_stake: 1_000_000, source_bonus: 1, staleness: 3600 };
        assert!(high.compute() > low.compute());
    }

    #[test]
    fn test_domain_of() {
        assert_eq!(Frontier::domain_of("https://www.example.com/path"), Some("www.example.com".to_string()));
        assert_eq!(Frontier::domain_of("not-a-url"), None);
    }

    // -- Budget tests --

    #[test]
    fn test_domain_budget_rejects_over_limit() {
        let mut frontier = Frontier::new().with_max_urls_per_domain(2);

        // Simulate 2 crawled URLs for this domain
        frontier.record_crawl("example.com");
        frontier.record_crawl("example.com");

        let added = frontier.push(
            "https://example.com/page3",
            "example.com",
            PriorityFactors { domain_stake: 0, source_bonus: 1, staleness: 0 },
        );
        assert!(!added);

        // Different domain should still work
        let added2 = frontier.push(
            "https://other.com/page1",
            "other.com",
            PriorityFactors { domain_stake: 0, source_bonus: 1, staleness: 0 },
        );
        assert!(added2);
    }

    #[test]
    fn test_epoch_budget_rejects_over_limit() {
        let mut frontier = Frontier::new().with_max_urls_per_epoch(2);

        frontier.record_crawl("a.com");
        frontier.record_crawl("b.com");

        let added = frontier.push(
            "https://c.com/page1",
            "c.com",
            PriorityFactors { domain_stake: 0, source_bonus: 1, staleness: 0 },
        );
        assert!(!added);
    }

    #[test]
    fn test_epoch_reset_clears_counts() {
        let mut frontier = Frontier::new().with_max_urls_per_epoch(2);

        frontier.record_crawl("a.com");
        frontier.record_crawl("b.com");
        assert_eq!(frontier.total_crawled(), 2);

        frontier.reset_epoch();
        assert_eq!(frontier.total_crawled(), 0);
        assert_eq!(frontier.domain_crawl_count("a.com"), 0);

        // Should be able to push again
        let added = frontier.push(
            "https://c.com/page1",
            "c.com",
            PriorityFactors { domain_stake: 0, source_bonus: 1, staleness: 0 },
        );
        assert!(added);
    }

    #[test]
    fn test_record_crawl_increments() {
        let mut frontier = Frontier::new();

        assert_eq!(frontier.domain_crawl_count("example.com"), 0);
        assert_eq!(frontier.total_crawled(), 0);

        frontier.record_crawl("example.com");
        assert_eq!(frontier.domain_crawl_count("example.com"), 1);
        assert_eq!(frontier.total_crawled(), 1);

        frontier.record_crawl("example.com");
        assert_eq!(frontier.domain_crawl_count("example.com"), 2);
        assert_eq!(frontier.total_crawled(), 2);
    }

    // -- Persistence tests --

    #[test]
    fn test_checkpoint_and_restore_via_frontier() {
        use crate::crawl_db::CrawlDb;

        // Use an in-memory DB by attaching it directly
        let db = CrawlDb::open_in_memory();

        let mut frontier = Frontier::new();
        frontier.db = Some(db);
        frontier.push(
            "https://a.com/1",
            "a.com",
            PriorityFactors { domain_stake: 5_000_000, source_bonus: 2, staleness: 0 },
        );
        frontier.push(
            "https://b.com/1",
            "b.com",
            PriorityFactors { domain_stake: 1_000_000, source_bonus: 1, staleness: 0 },
        );
        frontier.record_crawl("a.com");

        frontier.checkpoint().unwrap();

        // Load into a new frontier from the same DB
        let db2 = frontier.db.take().unwrap();
        let (entries, seen, _, _, counts) = db2.load_frontier().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(seen.len(), 2);
        assert_eq!(*counts.get("a.com").unwrap(), 1);
    }
}
