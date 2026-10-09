//! Spider + Search E2E: spider a real domain, ingest into a local index,
//! then verify the content is findable via search queries.
//!
//! Run: cargo test -p oi-integration-tests --test spider_search_e2e -- --nocapture --ignored

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use oi_crawl::spider::{Spider, SpiderConfig};
use oi_crawl::Distiller;
use oi_sdk::OpenSonarXClient;
use oi_sdk::SdkConfig;
use oi_types::QueryFilters;

#[tokio::test]
#[ignore] // requires network access
async fn spider_ingest_then_search() {
    // 1. Build an OpenSonarXClient with an in-memory index (no persistence)
    let config = SdkConfig::default();
    let sdk = Arc::new(OpenSonarXClient::new(config));

    // Verify we got the real embedder
    let dim = sdk.embedder_arc().dimension();
    println!("\n  Embedder dimension: {dim}");
    assert_eq!(dim, 384, "Expected MiniLM 384-dim embedder, got {dim}-dim (likely MockEmbedder). Build with onnx feature.");

    // 2. Build a Spider with the same embedder as the SDK (so embeddings match)
    let distiller = Arc::new(Distiller::new().with_embedder(sdk.embedder_arc()));
    let spider_config = SpiderConfig {
        max_depth: 1,
        max_pages: 10,
        concurrency: 3,
        respect_robots: true,
        same_domain_only: true,
    };
    let spider = Spider::new(spider_config, distiller);

    // 3. Spider docs.rs/tokio and ingest each page
    let pages_ingested = Arc::new(AtomicUsize::new(0));
    let chunks_ingested = Arc::new(AtomicUsize::new(0));
    let p = pages_ingested.clone();
    let c = chunks_ingested.clone();
    let sdk_cb = sdk.clone();

    let result = spider
        .crawl(
            vec!["https://docs.rs/tokio/latest/tokio/".to_string()],
            move |output| {
                if output.chunks.is_empty() {
                    return;
                }
                let n = output.chunks.len();
                println!("  [ingest] {} — {} chunks", output.url, n);
                match sdk_cb.ingest(&output.chunks) {
                    Ok(()) => {
                        p.fetch_add(1, Ordering::Relaxed);
                        c.fetch_add(n, Ordering::Relaxed);
                    }
                    Err(e) => {
                        eprintln!("  [ingest FAILED] {}: {e}", output.url);
                    }
                }
            },
        )
        .await;

    let total_pages = pages_ingested.load(Ordering::Relaxed);
    let total_chunks = chunks_ingested.load(Ordering::Relaxed);
    println!("\n=== Spider Stats ===");
    println!("  pages_crawled:   {}", result.pages_crawled);
    println!("  pages_failed:    {}", result.pages_failed);
    println!("  urls_discovered: {}", result.urls_discovered);
    println!("  pages_ingested:  {total_pages}");
    println!("  chunks_ingested: {total_chunks}");
    println!("  index doc count: {}", sdk.index().doc_count());

    assert!(total_pages > 0, "should have ingested at least one page");
    assert!(total_chunks > 0, "should have ingested at least one chunk");

    // 4. Search the index with diverse queries
    let filters = QueryFilters::default();
    let queries = [
        "tokio async runtime for Rust",
        "how to spawn a background task",
        "TCP server with tokio",
        "what is mio",
        "parking_lot mutex",
    ];

    println!("\n{}", "=".repeat(80));
    println!("=== SEARCH EVALUATION (MiniLM 384-dim ONNX) ===");
    println!("{}", "=".repeat(80));

    for query in &queries {
        let results = sdk
            .search(query, 5, &filters)
            .expect("search should succeed");
        println!("\n┌──────────────────────────────────────────────────────────────");
        println!("│ Query: \"{query}\"");
        println!("│ Results: {}", results.len());
        println!("├──────────────────────────────────────────────────────────────");
        if results.is_empty() {
            println!("│  (no results)");
        } else {
            for (i, r) in results.iter().enumerate() {
                println!("│");
                println!("│  #{} — {}", i + 1, r.title);
                println!("│  URL:       {}", r.url);
                println!(
                    "│  Scores:    final={:.4}  semantic={:.4}  bm25={:.4}",
                    r.final_score, r.semantic_score, r.bm25_score
                );
                // Snippet: first 300 chars
                let snippet: String = r.snippet.chars().take(300).collect();
                if !snippet.is_empty() {
                    println!("│  Snippet:   {}", snippet.replace('\n', " "));
                }
                // Content relevant: first 300 chars
                if let Some(ref relevant) = r.content_relevant {
                    if !relevant.is_empty() {
                        let preview: String = relevant.chars().take(300).collect();
                        println!("│  Relevant:  {}", preview.replace('\n', " "));
                    }
                }
                println!("│");
            }
        }
        println!("└──────────────────────────────────────────────────────────────");
    }
}
