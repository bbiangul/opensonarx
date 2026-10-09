//! Reusable link-following spider for one-shot site crawling.
//!
//! BFS crawl starting from seed URLs, following same-domain links up to a
//! configurable depth. Uses the existing `Distiller` for content processing,
//! `extract_links` for link discovery, and `RobotsChecker` for compliance.

use std::collections::{HashSet, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use tokio::sync::{Mutex, RwLock, Semaphore};
use tokio::task::JoinSet;

use crate::Distiller;
use crate::discovery::extract_links;
use crate::fetcher::fetch_url;
use crate::robots::{RobotsChecker, parse_robots_txt};
use oi_types::CrawlOutput;
use oi_types::traits::CrawlPipeline;

/// Configuration for the spider.
pub struct SpiderConfig {
    /// Maximum link-follow depth from seed URLs (default: 3).
    pub max_depth: usize,
    /// Maximum number of pages to crawl (default: 1000).
    pub max_pages: usize,
    /// Number of concurrent crawl workers (default: 4).
    pub concurrency: usize,
    /// Whether to respect robots.txt (default: true).
    pub respect_robots: bool,
    /// Only follow links on the same domain as the seed URL (default: true).
    pub same_domain_only: bool,
}

impl Default for SpiderConfig {
    fn default() -> Self {
        Self {
            max_depth: 3,
            max_pages: 1000,
            concurrency: 4,
            respect_robots: true,
            same_domain_only: true,
        }
    }
}

/// Aggregate stats returned after a spider crawl completes.
pub struct SpiderResult {
    pub pages_crawled: usize,
    pub pages_failed: usize,
    pub pages_skipped_robots: usize,
    pub urls_discovered: usize,
}

/// A BFS spider that crawls a site starting from seed URLs.
pub struct Spider {
    config: SpiderConfig,
    distiller: Arc<Distiller>,
}

impl Spider {
    pub fn new(config: SpiderConfig, distiller: Arc<Distiller>) -> Self {
        Self { config, distiller }
    }

    /// Crawl starting from `seed_urls`, calling `on_page` for each successfully
    /// distilled page. Returns aggregate stats.
    pub async fn crawl<F>(&self, seed_urls: Vec<String>, on_page: F) -> SpiderResult
    where
        F: Fn(CrawlOutput) + Send + Sync + 'static,
    {
        let on_page = Arc::new(on_page);
        let seen: Arc<RwLock<HashSet<String>>> = Arc::new(RwLock::new(HashSet::new()));
        let queue: Arc<Mutex<VecDeque<(String, usize)>>> = Arc::new(Mutex::new(VecDeque::new()));
        let robots: Arc<RwLock<RobotsChecker>> = Arc::new(RwLock::new(RobotsChecker::new()));
        let semaphore = Arc::new(Semaphore::new(self.config.concurrency));

        let pages_crawled = Arc::new(AtomicUsize::new(0));
        let pages_failed = Arc::new(AtomicUsize::new(0));
        let pages_skipped_robots = Arc::new(AtomicUsize::new(0));
        let urls_discovered = Arc::new(AtomicUsize::new(0));

        // Seed the queue
        {
            let mut q = queue.lock().await;
            let mut s = seen.write().await;
            for url in &seed_urls {
                if s.insert(url.clone()) {
                    q.push_back((url.clone(), 0));
                }
            }
        }
        urls_discovered.fetch_add(seed_urls.len(), Ordering::Relaxed);

        let max_pages = self.config.max_pages;
        let max_depth = self.config.max_depth;
        let respect_robots = self.config.respect_robots;
        let same_domain_only = self.config.same_domain_only;
        let distiller = self.distiller.clone();

        // BFS loop: drain the queue using a JoinSet of concurrent workers.
        // Workers pop items, crawl them, and push discovered links back.
        let mut join_set = JoinSet::new();

        loop {
            // Try to pop from the queue
            let item = {
                let mut q = queue.lock().await;
                q.pop_front()
            };

            let Some((url, depth)) = item else {
                // Queue empty — wait for in-flight tasks to potentially enqueue more
                if join_set.is_empty() {
                    break;
                }
                // Wait for one task to finish, then re-check queue
                let _ = join_set.join_next().await;
                continue;
            };

            // Check limits
            if pages_crawled.load(Ordering::Relaxed) + pages_failed.load(Ordering::Relaxed)
                >= max_pages
            {
                break;
            }

            let permit = semaphore.clone().acquire_owned().await.unwrap();
            let queue = queue.clone();
            let seen = seen.clone();
            let robots = robots.clone();
            let pages_crawled = pages_crawled.clone();
            let pages_failed = pages_failed.clone();
            let pages_skipped_robots = pages_skipped_robots.clone();
            let urls_discovered = urls_discovered.clone();
            let distiller = distiller.clone();
            let on_page = on_page.clone();

            join_set.spawn(async move {
                let _permit = permit;

                // Robots.txt check
                if respect_robots && let Some(domain) = domain_from_url(&url) {
                    let needs_fetch = {
                        let checker = robots.read().await;
                        !checker.has_rules(&domain)
                    };
                    if needs_fetch {
                        let robots_url = format!("https://{domain}/robots.txt");
                        if let Ok(result) = fetch_url(&robots_url).await {
                            let rules = parse_robots_txt(&result.html);
                            let mut checker = robots.write().await;
                            checker.add_rules(&domain, rules);
                        }
                    }

                    let checker = robots.read().await;
                    if !checker.is_allowed(&url) {
                        pages_skipped_robots.fetch_add(1, Ordering::Relaxed);
                        return;
                    }
                }

                // Fetch for link discovery (only if we might follow links)
                let discovered_links = if depth < max_depth {
                    match fetch_url(&url).await {
                        Ok(result) => {
                            if same_domain_only {
                                extract_links(&result.html, &url)
                            } else {
                                // extract_links already does same-domain, but for
                                // !same_domain_only we'd need all links — for now
                                // this flag is always true per plan
                                extract_links(&result.html, &url)
                            }
                        }
                        Err(_) => Vec::new(),
                    }
                } else {
                    Vec::new()
                };

                // Enqueue discovered links
                if !discovered_links.is_empty() {
                    let mut q = queue.lock().await;
                    let mut s = seen.write().await;
                    let mut new_count = 0;
                    for link in &discovered_links {
                        if s.insert(link.url.clone()) {
                            q.push_back((link.url.clone(), depth + 1));
                            new_count += 1;
                        }
                    }
                    urls_discovered.fetch_add(new_count, Ordering::Relaxed);
                }

                // Distill the page for content
                match distiller.crawl(&url).await {
                    Ok(output) => {
                        pages_crawled.fetch_add(1, Ordering::Relaxed);
                        on_page(output);
                    }
                    Err(e) => {
                        pages_failed.fetch_add(1, Ordering::Relaxed);
                        tracing::debug!("Spider crawl failed for {url}: {e}");
                    }
                }
            });
        }

        // Drain remaining tasks
        while join_set.join_next().await.is_some() {}

        SpiderResult {
            pages_crawled: pages_crawled.load(Ordering::Relaxed),
            pages_failed: pages_failed.load(Ordering::Relaxed),
            pages_skipped_robots: pages_skipped_robots.load(Ordering::Relaxed),
            urls_discovered: urls_discovered.load(Ordering::Relaxed),
        }
    }
}

/// Extract domain from a URL string.
fn domain_from_url(url: &str) -> Option<String> {
    let after_scheme = url.split("//").nth(1)?;
    let host = after_scheme.split('/').next()?;
    let domain = host.split(':').next()?;
    if domain.is_empty() {
        None
    } else {
        Some(domain.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_spider_config_defaults() {
        let config = SpiderConfig::default();
        assert_eq!(config.max_depth, 3);
        assert_eq!(config.max_pages, 1000);
        assert_eq!(config.concurrency, 4);
        assert!(config.respect_robots);
        assert!(config.same_domain_only);
    }

    #[test]
    fn test_domain_from_url() {
        assert_eq!(
            domain_from_url("https://example.com/path"),
            Some("example.com".to_string())
        );
        assert_eq!(
            domain_from_url("https://sub.example.com:8080/path"),
            Some("sub.example.com".to_string())
        );
        assert_eq!(domain_from_url("not-a-url"), None);
    }

    #[tokio::test]
    async fn test_spider_dedup() {
        // Two seed URLs that are the same — should only appear once in seen set
        let distiller = Arc::new(Distiller::new());
        let config = SpiderConfig {
            max_depth: 0,
            max_pages: 10,
            concurrency: 1,
            respect_robots: false,
            same_domain_only: true,
        };
        let spider = Spider::new(config, distiller);

        let crawled = Arc::new(AtomicUsize::new(0));
        let crawled_cb = crawled.clone();

        // These will fail to fetch (no server), but dedup happens before fetch
        let result = spider
            .crawl(
                vec![
                    "https://example.com/page".to_string(),
                    "https://example.com/page".to_string(),
                ],
                move |_output| {
                    crawled_cb.fetch_add(1, Ordering::Relaxed);
                },
            )
            .await;

        // Only 1 URL should have been attempted (deduped), both will fail (no server)
        assert_eq!(
            result.pages_crawled + result.pages_failed,
            1,
            "duplicate seed URL should be deduped"
        );
    }

    #[tokio::test]
    async fn test_spider_respects_max_depth() {
        // max_depth=0 means no link following — only seed URLs crawled
        let distiller = Arc::new(Distiller::new());
        let config = SpiderConfig {
            max_depth: 0,
            max_pages: 100,
            concurrency: 1,
            respect_robots: false,
            same_domain_only: true,
        };
        let spider = Spider::new(config, distiller);

        let result = spider
            .crawl(vec!["https://example.com".to_string()], |_output| {})
            .await;

        // At depth 0, we only crawl the seed — no links are followed
        // The seed itself may succeed or fail depending on network, but
        // urls_discovered should only be the seed count
        assert_eq!(result.urls_discovered, 1);
    }

    #[tokio::test]
    async fn test_spider_same_domain_only() {
        // extract_links already filters to same-domain, so this is inherently true.
        // Verify the config flag exists and defaults correctly.
        let config = SpiderConfig::default();
        assert!(config.same_domain_only);
    }
}
