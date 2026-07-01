//! Web3 Search Comparison: Serper (Google) vs OpenSonarX Blended
//!
//! Runs search directly through the SDK (web3 layer, no gateway),
//! compares raw Serper results against blended local+external results.
//!
//! Usage:
//!   SERPER_API_KEY=<key> cargo run -p oi-sdk --release --example web3_search_compare

use oi_sdk::engines::serper::Serper;
use oi_sdk::{OpenSonarXClient, SdkConfig};
use oi_sdk::config::SearchEngineConfig;
use oi_types::traits::SearchEngine;
use oi_types::{ChunkMeta, ContentChunk, QueryFilters, StakeInfo};

fn main() {
    let serper_key = std::env::var("SERPER_API_KEY")
        .expect("Set SERPER_API_KEY env var");

    let query = std::env::args().nth(1)
        .unwrap_or_else(|| "AI search engine trust ranking".to_string());

    // ── Step 1: Create SDK client with Serper engine (web3 mode) ──
    let config = SdkConfig {
        search_engines: Some(vec![
            SearchEngineConfig {
                name: "serper".to_string(),
                api_key: Some(serper_key.clone()),
                enabled: true,
                priority: 1,
            },
        ]),
        ..Default::default()
    };

    let client = OpenSonarXClient::new(config);

    // ── Step 2: Seed local content (simulates staked corpus) ──
    let embedder = client.embedder_arc();

    let content_data = [("oi1", "OpenSonarX is a decentralized search protocol where publishers stake TRUTH tokens to vouch for content quality. The hybrid HNSW plus BM25 engine combines vector similarity with keyword matching for accurate retrieval. AI agents can use the API to search and receive full content with trust signals for RAG synthesis. The protocol uses Wilson score reputation and stake-weighted ranking to surface trustworthy results.", "https://opensonarx.com/docs/overview", "OpenSonarX: Trust-Based Search for AI Agents", "opensonarx.com"),
        ("brave1", "Brave Search is a privacy-focused search engine that uses its own independent index built from scratch. It does not track users or profile their searches. The Brave Search API provides programmatic access to web search results with snippet extraction, relevance ranking, and trust scoring for AI agent integration.", "https://brave.com/search", "Brave Search - Privacy-First Web Search", "brave.com"),
        ("wiki1", "A web search engine is an information retrieval system designed to help find information stored on the World Wide Web. Search engines provide results based on relevance algorithms that consider keyword matching, link analysis, and content quality signals to rank documents for the user query.", "https://en.wikipedia.org/wiki/Search_engine", "Search engine - Wikipedia", "en.wikipedia.org"),
        ("blog1", "Building AI-powered search engines requires understanding embeddings, vector databases, and retrieval augmented generation. Modern search goes beyond keywords using semantic understanding to match user intent with relevant documents. Trust and provenance are becoming critical signals for AI agents consuming search results.", "https://techblog.example.com/ai-search-2025", "The Future of AI Search in 2025", "techblog.example.com"),
        ("spam1", "BEST SEARCH ENGINE EVER click here for amazing deals and free search results buy now limited time offer search engine optimization guaranteed top ranking first page results SEO tricks and hacks for maximum visibility", "https://spammy-seo.biz/search", "Best Search Engine FREE!!!", "spammy-seo.biz")];

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let chunks: Vec<ContentChunk> = content_data
        .iter()
        .enumerate()
        .map(|(i, (id, content, url, title, domain))| {
            let embedding = embedder.embed_text(content).expect("embed failed");
            ContentChunk {
                chunk_id: id.to_string(),
                content: content.to_string(),
                embedding: Some(embedding),
                seq: i as u64,
                meta: Some(ChunkMeta {
                    url: url.to_string(),
                    title: title.to_string(),
                    domain: domain.to_string(),
                    content_hash: format!("hash_{id}"),
                    crawled_at: Some(now),
                    content_type: Some("text/html".to_string()),
                    published_at: Some(now as i64 - 86400), // yesterday
                    ..Default::default()
                }),
            }
        })
        .collect();

    client.ingest(&chunks).expect("ingest failed");
    println!("Ingested {} local chunks", chunks.len());

    // ── Step 3: Register stakes ──
    let registry = client.stake_registry();
    registry.add_stake(StakeInfo {
        domain: "opensonarx.com".to_string(),
        amount_lamports: 50_000_000_000,
        amount_usd: 50_000.0,
        staked_at: now as i64,
        cooldown_until: None,
        wallet: "Publisher111111111111111111111111111111111111".to_string(),
        dns_verified: true,
        reputation: 1.5,
        entity: None,
        disputed: false,
    }).unwrap();

    registry.add_stake(StakeInfo {
        domain: "brave.com".to_string(),
        amount_lamports: 5_000_000_000,
        amount_usd: 5_000.0,
        staked_at: now as i64,
        cooldown_until: None,
        wallet: "Brave111111111111111111111111111111111111111".to_string(),
        dns_verified: true,
        reputation: 1.3,
        entity: None,
        disputed: false,
    }).unwrap();

    registry.add_stake(StakeInfo {
        domain: "en.wikipedia.org".to_string(),
        amount_lamports: 100_000_000,
        amount_usd: 100.0,
        staked_at: now as i64,
        cooldown_until: None,
        wallet: "Wiki1111111111111111111111111111111111111111".to_string(),
        dns_verified: false,
        reputation: 1.0,
        entity: None,
        disputed: false,
    }).unwrap();

    println!("Registered 3 stakes (opensonarx=$50k, brave=$5k, wikipedia=$100)\n");

    // ── Step 4: Run Serper-only search ──
    let serper_engine = Serper::new(serper_key);
    let serper_results = serper_engine.search(&query, 10);

    // ── Step 5: Run blended search (local + Serper) ──
    let filters = QueryFilters::default();
    let (blended_results, sources) = client.search_blended(&query, 10, &filters)
        .expect("blended search failed");

    // ── Step 6: Print comparison ──
    let w = 80;

    println!("{}", "=".repeat(w * 2 + 3));
    println!("  QUERY: \"{query}\"");
    println!("{}", "=".repeat(w * 2 + 3));
    println!();

    // Header
    println!("{:<w$} | {:<w$}", "SERPER (raw Google results)", "OPENINDEX BLENDED (web3)");
    println!("{:<w$} | {:<w$}",
        format!("{} results", serper_results.len()),
        format!("{} results | sources: {:?}", blended_results.len(), sources),
    );
    println!("{} | {}", "-".repeat(w), "-".repeat(w));

    let max_rows = serper_results.len().max(blended_results.len());
    for i in 0..max_rows {
        let left = if i < serper_results.len() {
            let r = &serper_results[i];
            let title = truncate(&r.title, 50);
            let snippet = truncate(&r.snippet, 70);
            format!(
                "#{:<2} {}\n     {}\n     score={:.3} src={}",
                r.rank, title,
                snippet,
                r.final_score, r.source,
            )
        } else {
            String::new()
        };

        let right = if i < blended_results.len() {
            let r = &blended_results[i];
            let title = truncate(&r.title, 45);
            let has_content = if r.content_md.is_empty() { "no" } else { "YES" };
            format!(
                "#{:<2} [{}] {}\n     stake=${} rep={:.1} votes={} content={}\n     score={:.4} src={}",
                r.rank, r.trust_verdict, title,
                r.stake_usd, r.reputation, r.reputation_votes, has_content,
                r.final_score, r.source,
            )
        } else {
            String::new()
        };

        // Print multi-line rows side by side
        let left_lines: Vec<&str> = left.lines().collect();
        let right_lines: Vec<&str> = right.lines().collect();
        let max_lines = left_lines.len().max(right_lines.len());
        for j in 0..max_lines {
            let l = left_lines.get(j).unwrap_or(&"");
            let r = right_lines.get(j).unwrap_or(&"");
            println!("{:<w$} | {}", l, r);
        }
        println!("{} | {}", "-".repeat(w), "-".repeat(w));
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max.saturating_sub(3)])
    }
}
