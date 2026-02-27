/// Anti-pay-to-win test: staked content should NOT rank high if irrelevant.
///
/// Usage:
///   SERPER_API_KEY=<key> cargo run -p oi-sdk --example compare_engines
use oi_sdk::{OpenSonarXClient, SdkConfig};
use oi_sdk::config::SearchEngineConfig;
use oi_types::{ChunkMeta, ContentChunk, QueryFilters};
use std::time::Instant;

fn main() {
    let serper_key = std::env::var("SERPER_API_KEY")
        .expect("Set SERPER_API_KEY env var");

    // ─── Build blended client ───────────────────────────────────────────
    let config = SdkConfig {
        search_engines: Some(vec![
            SearchEngineConfig {
                name: "serper".to_string(),
                api_key: Some(serper_key.clone()),
                enabled: true,
                priority: 1,
            },
            SearchEngineConfig {
                name: "duckduckgo".to_string(),
                api_key: None,
                enabled: true,
                priority: 10,
            },
        ]),
        ..SdkConfig::default()
    };
    let client = OpenSonarXClient::new(config);

    // ─── Ingest: mix of relevant and IRRELEVANT content ─────────────────
    let chunks = vec![
        // RELEVANT to "rust async runtime"
        chunk("tokio_tut", "Tokio is an asynchronous runtime for Rust. Spawning tasks, async IO, channels.",
              "https://tokio.rs/tokio/tutorial", "Tokio Tutorial", "tokio.rs", 0),
        chunk("tokio_async", "Understanding async/await in Rust with Tokio. Futures, tasks, scheduler.",
              "https://tokio.rs/tokio/tutorial/async", "Async in Depth — Tokio", "tokio.rs", 1),
        chunk("rust_book", "Rust async programming. Futures, async/await syntax, concurrency without GC.",
              "https://doc.rust-lang.org/book/ch17-00-async-await.html", "Async and Await — Rust Book", "doc.rust-lang.org", 2),

        // IRRELEVANT to "rust async runtime" — cooking website
        chunk("cook1", "Best chocolate cake recipe. Mix flour, sugar, cocoa powder, eggs, butter, and vanilla.",
              "https://bestcakes.example.com/chocolate-cake", "Ultimate Chocolate Cake", "bestcakes.example.com", 3),
        chunk("cook2", "How to make perfect sourdough bread. Starter, flour, water, salt, long fermentation.",
              "https://bestcakes.example.com/sourdough", "Sourdough Bread Guide", "bestcakes.example.com", 4),
        chunk("cook3", "Top 10 pasta recipes for weeknight dinners. Quick and easy Italian cooking.",
              "https://bestcakes.example.com/pasta", "10 Easy Pasta Recipes", "bestcakes.example.com", 5),

        // RELEVANT to "machine learning python"
        chunk("sklearn", "Scikit-learn Python ML library. Classification, regression, clustering.",
              "https://scikit-learn.org/stable/", "scikit-learn", "scikit-learn.org", 6),
        chunk("pytorch", "PyTorch deep learning. Tensors, autograd, neural networks, GPU.",
              "https://pytorch.org/tutorials/", "PyTorch Tutorials", "pytorch.org", 7),
    ];
    client.ingest(&chunks).unwrap();
    println!("Ingested {} chunks\n", chunks.len());

    // ─── Stakes: bestcakes stakes MORE than tokio ───────────────────────
    client.stake("bestcakes.example.com", 50_000_000).unwrap();  // 50 $TRUTH — whale
    client.stake("tokio.rs", 5_000_000).unwrap();                // 5 $TRUTH
    client.stake("doc.rust-lang.org", 3_000_000).unwrap();       // 3 $TRUTH
    client.stake("scikit-learn.org", 2_000_000).unwrap();        // 2 $TRUTH

    println!("Stakes:");
    println!("  bestcakes.example.com: 50M lamports (WHALE — but cooking content)");
    println!("  tokio.rs:              5M lamports");
    println!("  doc.rust-lang.org:     3M lamports");
    println!("  scikit-learn.org:      2M lamports\n");

    // ─── Test queries ───────────────────────────────────────────────────
    let test_cases = vec![
        ("rust async runtime",      "Should bestcakes rank high? NO — irrelevant content"),
        ("chocolate cake recipe",   "Should bestcakes rank high? YES — relevant content"),
        ("machine learning python", "bestcakes is irrelevant here too"),
    ];

    for (query, description) in &test_cases {
        println!("\n{}", "=".repeat(130));
        println!("  QUERY: \"{query}\"");
        println!("  TEST:  {description}");
        println!("{}\n", "=".repeat(130));

        let filters = QueryFilters {
            locale: Some("en".to_string()),
            gl: Some("us".to_string()),
            ..Default::default()
        };

        let start = Instant::now();
        let (results, sources) = client
            .search_blended(query, 10, &filters)
            .unwrap_or_default();
        let ms = start.elapsed().as_millis();

        println!(
            "{:<4} {:<7} {:<7} {:<7} {:<55} {:<20} {}",
            "#", "FINAL", "SEMANT", "BM25", "URL", "SOURCE", "STAKE"
        );
        println!("{}", "-".repeat(130));

        for r in &results {
            let stake_label = if r.stake_truth > 0 {
                format!("{}M $TRUTH", r.stake_truth / 1_000_000)
            } else {
                "-".to_string()
            };
            let source_label = if r.source == "cached" {
                format!("ext+staked")
            } else {
                r.source.clone()
            };
            println!(
                "{:<4} {:<7.4} {:<7.4} {:<7.4} {:<55} {:<20} {}",
                r.rank, r.final_score, r.semantic_score, r.bm25_score,
                truncate(&r.url, 53), source_label, stake_label,
            );
        }

        let staked_count = results.iter().filter(|r| r.stake_truth > 0).count();
        let cake_in_top3 = results.iter().take(3).any(|r| r.url.contains("bestcakes"));
        println!("\n  {ms}ms | sources: {sources:?} | {staked_count} staked in top {} | bestcakes in top 3: {cake_in_top3}",
            results.len());
    }
}

fn chunk(id: &str, content: &str, url: &str, title: &str, domain: &str, seq: u64) -> ContentChunk {
    ContentChunk {
        chunk_id: id.into(),
        content: content.into(),
        embedding: None,
        seq,
        meta: Some(ChunkMeta {
            url: url.into(),
            title: title.into(),
            domain: domain.into(),
            content_hash: String::new(),
            crawled_at: None,
            content_type: None,
            entity: None,
            content_kind: None,
            ..Default::default()
        }),
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max.saturating_sub(3)])
    }
}
