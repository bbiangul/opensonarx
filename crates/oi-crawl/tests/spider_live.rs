//! Quick spider smoke test against a real domain.
//! Run: cargo test -p oi-crawl --test spider_live -- --nocapture --ignored

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use oi_crawl::Distiller;
use oi_crawl::spider::{Spider, SpiderConfig};

#[tokio::test]
#[ignore] // requires network
async fn spider_live_crawl() {
    let distiller = Arc::new(Distiller::new());
    let config = SpiderConfig {
        max_depth: 1,
        max_pages: 10,
        concurrency: 3,
        respect_robots: true,
        same_domain_only: true,
    };
    let spider = Spider::new(config, distiller);

    let pages = Arc::new(AtomicUsize::new(0));
    let chunks_total = Arc::new(AtomicUsize::new(0));
    let p = pages.clone();
    let c = chunks_total.clone();

    let result = spider
        .crawl(
            vec!["https://docs.rs/tokio/latest/tokio/".to_string()],
            move |output| {
                let n = output.chunks.len();
                p.fetch_add(1, Ordering::Relaxed);
                c.fetch_add(n, Ordering::Relaxed);
                println!(
                    "  [page] {} — {} chunks, hash={}",
                    output.url,
                    n,
                    &output.content_hash[..12]
                );
            },
        )
        .await;

    println!("\n=== Spider Result ===");
    println!("  pages_crawled:        {}", result.pages_crawled);
    println!("  pages_failed:         {}", result.pages_failed);
    println!("  pages_skipped_robots: {}", result.pages_skipped_robots);
    println!("  urls_discovered:      {}", result.urls_discovered);
    println!("  callback invocations: {}", pages.load(Ordering::Relaxed));
    println!(
        "  total chunks:         {}",
        chunks_total.load(Ordering::Relaxed)
    );

    // Should have crawled at least the seed
    assert!(
        result.pages_crawled + result.pages_failed >= 1,
        "spider should have attempted at least the seed URL"
    );
    // At depth=1 we should discover some links beyond just the seed
    assert!(
        result.urls_discovered >= 1,
        "spider should discover at least the seed URL"
    );
}
