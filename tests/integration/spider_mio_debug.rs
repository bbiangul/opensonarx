//! Debug test: why does "what is mio" not surface the mio crate page?
//!
//! Run: cargo test -p oi-integration-tests --test spider_mio_debug -- --nocapture --ignored

use std::sync::Arc;
use std::sync::Mutex;

use oi_crawl::spider::{Spider, SpiderConfig};
use oi_crawl::Distiller;
use oi_sdk::OpenSonarXClient;
use oi_sdk::SdkConfig;
use oi_types::{CrawlOutput, QueryFilters};

#[tokio::test]
#[ignore]
async fn debug_mio_ranking() {
    // 1. Build SDK with real ONNX embedder
    let config = SdkConfig::default();
    let sdk = Arc::new(OpenSonarXClient::new(config));
    assert_eq!(
        sdk.embedder_arc().dimension(),
        384,
        "Need MiniLM ONNX embedder"
    );

    // 2. Spider docs.rs/tokio — same as main test
    let distiller = Arc::new(Distiller::new().with_embedder(sdk.embedder_arc()));
    let spider = Spider::new(
        SpiderConfig {
            max_depth: 1,
            max_pages: 10,
            concurrency: 3,
            respect_robots: true,
            same_domain_only: true,
        },
        distiller,
    );

    // Collect all CrawlOutputs for inspection
    let outputs: Arc<Mutex<Vec<CrawlOutput>>> = Arc::new(Mutex::new(Vec::new()));
    let outputs_cb = outputs.clone();
    let sdk_cb = sdk.clone();

    spider
        .crawl(
            vec!["https://docs.rs/tokio/latest/tokio/".to_string()],
            move |output| {
                if !output.chunks.is_empty() {
                    sdk_cb.ingest(&output.chunks).ok();
                }
                outputs_cb.lock().unwrap().push(output);
            },
        )
        .await;

    let all_outputs = outputs.lock().unwrap();

    // 3. Find the mio page output
    println!("\n=== ALL CRAWLED PAGES ===");
    for o in all_outputs.iter() {
        let md_len = o.markdown.len();
        let chunk_count = o.chunks.len();
        println!(
            "  {} — {} chunks, markdown={} chars, hash={}",
            o.url,
            chunk_count,
            md_len,
            &o.content_hash[..12]
        );
    }

    // Find the mio page specifically
    let mio_output = all_outputs.iter().find(|o| o.url.contains("mio"));
    println!("\n=== MIO PAGE DETAILS ===");
    if let Some(mio) = mio_output {
        println!("  URL:      {}", mio.url);
        println!("  Chunks:   {}", mio.chunks.len());
        println!("  Markdown: {} chars", mio.markdown.len());
        println!("  Quality:  {:.2}", mio.extraction_quality);
        println!("\n  --- Full markdown content ---");
        println!("{}", mio.markdown);
        println!("  --- End markdown ---\n");

        // Show each chunk's text
        for (i, chunk) in mio.chunks.iter().enumerate() {
            println!("  Chunk {i}: {} chars", chunk.content.len());
            println!("  Text: {}", &chunk.content[..chunk.content.len().min(500)]);
            if let Some(ref emb) = chunk.embedding {
                println!(
                    "  Embedding: {}-dim, first 5 values: {:?}",
                    emb.len(),
                    &emb[..5.min(emb.len())]
                );
            } else {
                println!("  Embedding: NONE");
            }
            println!();
        }
    } else {
        println!("  !! mio page was NOT crawled !!");
    }

    // 4. Embed the query and check cosine similarity against all chunks
    let query = "what is mio";
    let query_emb = sdk.embedder_arc().embed_query(query).expect("embed query");
    println!("=== COSINE SIMILARITY: \"{query}\" vs all chunks ===");

    // Collect (url, chunk_idx, similarity, text_preview) and sort by similarity
    let mut sims: Vec<(String, usize, f32, String)> = Vec::new();
    for o in all_outputs.iter() {
        for (i, chunk) in o.chunks.iter().enumerate() {
            if let Some(ref emb) = chunk.embedding {
                let sim = cosine_similarity(&query_emb, emb);
                let preview: String = chunk.content.chars().take(100).collect();
                sims.push((o.url.clone(), i, sim, preview));
            }
        }
    }
    sims.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap());

    println!("  Top 10 chunks by cosine similarity to \"{query}\":\n");
    for (rank, (url, idx, sim, preview)) in sims.iter().take(10).enumerate() {
        let is_mio = if url.contains("mio") { " <-- MIO" } else { "" };
        println!(
            "  #{}: sim={:.4} [{url} chunk {idx}]{is_mio}",
            rank + 1,
            sim
        );
        println!("      {preview}...");
        println!();
    }

    // Also show where mio chunks rank
    println!("=== MIO CHUNKS RANKING ===");
    for (rank, (url, idx, sim, _)) in sims.iter().enumerate() {
        if url.contains("mio") {
            println!("  mio chunk {idx} ranked #{} with sim={:.4}", rank + 1, sim);
        }
    }

    // 5. Also run the full search to see final ranking
    let filters = QueryFilters::default();
    let results = sdk.search(query, 10, &filters).expect("search");
    println!("\n=== FULL SEARCH RESULTS for \"{query}\" (top 10) ===");
    for (i, r) in results.iter().enumerate() {
        let is_mio = if r.url.contains("mio") {
            " <-- MIO"
        } else {
            ""
        };
        println!(
            "  #{}: final={:.4} sem={:.4} bm25={:.4} — {} [{}]{is_mio}",
            i + 1,
            r.final_score,
            r.semantic_score,
            r.bm25_score,
            r.title,
            r.url
        );
    }
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }
    dot / (norm_a * norm_b)
}
