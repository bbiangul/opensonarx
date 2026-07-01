/// E2E Facts Integration Pipeline Test
///
/// Tests the full stack: crawl real URLs → extract facts → index chunks → search
/// with performance tracing at each stage. Uses real MiniLM ONNX embedder for
/// semantic search quality. MockFactExtractor for entity/fact extraction.
use oi_crawl::{chunker, extractor, fetcher, quality};
use oi_embed::MiniLmEmbedder;
use oi_facts::MockFactExtractor;
use oi_index::{HybridConfig, HybridIndex};
use oi_types::traits::{EmbeddingPipeline, FactExtractionPipeline, SearchIndex};
use oi_types::{ChunkMeta, ContentChunk, QueryFilters};
use sha2::{Digest, Sha256};
use std::time::Instant;

// ── Performance trace types ─────────────────────────────────────────────

struct CrawlTrace {
    url: String,
    lang: &'static str,
    fetch_ms: f64,
    html_to_md_ms: f64,
    quality_ms: f64,
    chunk_ms: f64,
    embed_ms: f64,
    fact_extraction_ms: f64,
    total_crawl_ms: f64,
    markdown_len: usize,
    chunk_count: usize,
    entity_count: usize,
    fact_count: usize,
}

#[allow(dead_code)]
struct QueryTrace {
    query: String,
    hnsw_ms: f64,
    bm25_ms: f64,
    fusion_ms: f64,
    total_ms: f64,
    num_results: usize,
    num_entities: usize,
    num_facts: usize,
    top_entities: Vec<String>,
}

// ── Test URLs ───────────────────────────────────────────────────────────

const TEST_URLS: &[(&str, &str)] = &[
    ("https://en.wikipedia.org/wiki/Microsoft", "en"),
    ("https://pt.wikipedia.org/wiki/Petrobras", "pt"),
    ("https://es.wikipedia.org/wiki/Real_Madrid_Club_de_F%C3%BAtbol", "es"),
    ("https://de.wikipedia.org/wiki/Volkswagen", "de"),
    ("https://fr.wikipedia.org/wiki/Tour_Eiffel", "fr"),
];

const TEST_QUERIES: &[(&str, &[&str])] = &[
    (
        "Microsoft founders Bill Gates",
        &["person", "organization"],
    ),
    (
        "Petrobras oil company Brazil",
        &["organization", "location"],
    ),
    (
        "Real Madrid Champions League",
        &["organization", "event"],
    ),
    (
        "Volkswagen automobile Germany",
        &["organization", "location"],
    ),
    (
        "Eiffel Tower Paris France",
        &["location"],
    ),
];

// ── Helper: run the crawl pipeline for a single URL ─────────────────────

async fn crawl_url_with_tracing(
    url: &str,
    lang: &'static str,
    embedder: &dyn EmbeddingPipeline,
    factifier: &dyn FactExtractionPipeline,
) -> Option<(CrawlTrace, Vec<ContentChunk>)> {
    let total_start = Instant::now();

    // 1. Fetch
    let t = Instant::now();
    let fetch_result = match fetcher::fetch_url(url).await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("  SKIP {url}: fetch failed: {e}");
            return None;
        }
    };
    let fetch_ms = t.elapsed().as_secs_f64() * 1000.0;

    if fetch_result.status >= 400 {
        eprintln!("  SKIP {url}: HTTP {}", fetch_result.status);
        return None;
    }

    // 2. HTML → Markdown
    let t = Instant::now();
    let extracted = extractor::html_to_markdown(&fetch_result.html);
    let html_to_md_ms = t.elapsed().as_secs_f64() * 1000.0;
    let markdown = extracted.markdown;

    // 3. Quality gate
    let t = Instant::now();
    let quality_check = quality::check_quality(&markdown);
    let quality_ms = t.elapsed().as_secs_f64() * 1000.0;

    if !quality_check.passed {
        eprintln!(
            "  SKIP {url}: quality gate rejected: {}",
            quality_check.reason.as_deref().unwrap_or("unknown"),
        );
        return None;
    }

    // 4. Content hash
    let mut hasher = Sha256::new();
    hasher.update(markdown.as_bytes());
    let content_hash = format!("{:x}", hasher.finalize());

    // 5. Chunking
    let t = Instant::now();
    let text_chunks = chunker::chunk_text(&markdown, 512, 128);
    let chunk_ms = t.elapsed().as_secs_f64() * 1000.0;

    // 6. Embedding
    let t = Instant::now();
    let texts: Vec<&str> = text_chunks.iter().map(|s| s.as_str()).collect();
    let embeddings = embedder.embed_text_batch(&texts).unwrap();
    let embed_ms = t.elapsed().as_secs_f64() * 1000.0;

    // 7. Fact extraction (per-chunk for speed and granularity)
    let t = Instant::now();
    let chunk_facts: Vec<_> = texts
        .iter()
        .map(|chunk_text| factifier.extract(chunk_text).unwrap())
        .collect();
    let fact_extraction_ms = t.elapsed().as_secs_f64() * 1000.0;

    let total_crawl_ms = total_start.elapsed().as_secs_f64() * 1000.0;

    let domain = url
        .split("//")
        .nth(1)
        .and_then(|s| s.split('/').next())
        .unwrap_or("")
        .to_string();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    // Build chunks with per-chunk content hash, embeddings, and facts
    let chunks: Vec<ContentChunk> = text_chunks
        .into_iter()
        .zip(embeddings)
        .zip(chunk_facts.into_iter())
        .enumerate()
        .map(|(i, ((text, emb), facts))| {
            let mut chunk_hasher = Sha256::new();
            chunk_hasher.update(text.as_bytes());
            let chunk_hash = format!("{:x}", chunk_hasher.finalize());

            ContentChunk {
                chunk_id: format!("{content_hash}_{i}"),
                content: text,
                embedding: Some(emb),
                seq: i as u64,
                meta: Some(ChunkMeta {
                    url: url.to_string(),
                    title: extracted.title.clone().unwrap_or_default(),
                    domain: domain.clone(),
                    content_hash: chunk_hash,
                    crawled_at: Some(now),
                    content_type: Some(fetch_result.content_type.clone()),
                    facts: Some(facts),
                    ..Default::default()
                }),
            }
        })
        .collect();

    // Aggregate entity/fact counts across all chunks
    let total_entity_count: usize = chunks
        .iter()
        .filter_map(|c| c.meta.as_ref()?.facts.as_ref())
        .map(|f| f.entities.len())
        .sum();
    let total_fact_count: usize = chunks
        .iter()
        .filter_map(|c| c.meta.as_ref()?.facts.as_ref())
        .map(|f| f.facts.len())
        .sum();

    let trace = CrawlTrace {
        url: url.to_string(),
        lang,
        fetch_ms,
        html_to_md_ms,
        quality_ms,
        chunk_ms,
        embed_ms,
        fact_extraction_ms,
        total_crawl_ms,
        markdown_len: markdown.len(),
        chunk_count: chunks.len(),
        entity_count: total_entity_count,
        fact_count: total_fact_count,
    };

    Some((trace, chunks))
}

// ── Main E2E test ───────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_facts_e2e_pipeline() {
    let embedder = MiniLmEmbedder::new().expect("Failed to load MiniLM model");
    let factifier = MockFactExtractor::new();

    println!("\n=== E2E Facts Pipeline: Crawl → Extract → Index → Search ===\n");

    // ── Phase 1: Crawl all URLs with tracing ────────────────────────────
    let mut crawl_traces: Vec<CrawlTrace> = Vec::new();
    let mut all_chunks_with_facts: Vec<ContentChunk> = Vec::new();
    let mut all_chunks_without_facts: Vec<ContentChunk> = Vec::new();

    for &(url, lang) in TEST_URLS {
        println!("Crawling [{lang}] {url}...");

        if let Some((trace, chunks)) = crawl_url_with_tracing(url, lang, &embedder, &factifier).await {
            println!(
                "  OK: {} chunks, {} entities, {} facts ({:.0}ms total)",
                trace.chunk_count, trace.entity_count, trace.fact_count, trace.total_crawl_ms,
            );

            // Build "without facts" copy (same chunks but no facts in meta)
            for chunk in &chunks {
                let mut no_facts_chunk = chunk.clone();
                if let Some(ref mut meta) = no_facts_chunk.meta {
                    meta.facts = None;
                }
                all_chunks_without_facts.push(no_facts_chunk);
            }

            all_chunks_with_facts.extend(chunks);
            crawl_traces.push(trace);
        }
    }

    // Must have at least some successfully crawled URLs
    assert!(
        !crawl_traces.is_empty(),
        "At least one URL should be crawlable"
    );
    assert!(
        !all_chunks_with_facts.is_empty(),
        "Should have indexed chunks"
    );

    println!(
        "\nCrawled {}/{} URLs, {} total chunks",
        crawl_traces.len(),
        TEST_URLS.len(),
        all_chunks_with_facts.len(),
    );

    // ── Phase 2: Index chunks ───────────────────────────────────────────
    let dim = embedder.dimension();
    let config = HybridConfig {
        semantic_weight: 1.0,
        bm25_weight: 0.0,
        ..Default::default()
    };

    // Index WITH facts
    let index_with_facts = HybridIndex::with_config(dim, config.clone());
    let t = Instant::now();
    index_with_facts.ingest(&all_chunks_with_facts).unwrap();
    let ingest_with_facts_ms = t.elapsed().as_secs_f64() * 1000.0;

    // Index WITHOUT facts
    let index_without_facts = HybridIndex::with_config(dim, config);
    let t = Instant::now();
    index_without_facts.ingest(&all_chunks_without_facts).unwrap();
    let ingest_without_facts_ms = t.elapsed().as_secs_f64() * 1000.0;

    println!(
        "\nIndex ingest: {:.1}ms WITH facts ({} docs), {:.1}ms WITHOUT facts ({} docs)",
        ingest_with_facts_ms,
        index_with_facts.doc_count(),
        ingest_without_facts_ms,
        index_without_facts.doc_count(),
    );

    // Both indexes should have the same doc count (facts don't affect dedup)
    assert_eq!(
        index_with_facts.doc_count(),
        index_without_facts.doc_count(),
        "Doc count should be identical with/without facts",
    );

    // ── Phase 3: Search queries ─────────────────────────────────────────
    let filters = QueryFilters::default();
    let mut query_traces: Vec<QueryTrace> = Vec::new();

    println!("\n=== Search Results ===\n");

    for &(query_text, expected_types) in TEST_QUERIES {
        let query_emb = embedder.embed_text(query_text).unwrap();

        // Query WITH facts
        let (results_with, timings) = index_with_facts
            .query_timed(&query_emb, query_text, &filters, 5)
            .unwrap();

        // Query WITHOUT facts (for comparison)
        let (results_without, _) = index_without_facts
            .query_timed(&query_emb, query_text, &filters, 5)
            .unwrap();

        // Count entities and facts in results
        let mut total_entities = 0usize;
        let mut total_facts = 0usize;
        let mut top_entities: Vec<String> = Vec::new();

        for result in &results_with {
            if let Some(ref facts) = result.facts {
                total_entities += facts.entities.len();
                total_facts += facts.facts.len();
                for entity in facts.entities.iter().take(3) {
                    top_entities.push(format!(
                        "[{}] {}",
                        entity.entity_type, entity.mention
                    ));
                }
            }
        }

        // Deduplicate top entities list
        top_entities.sort();
        top_entities.dedup();
        top_entities.truncate(6);

        let trace = QueryTrace {
            query: query_text.to_string(),
            hnsw_ms: timings.hnsw_ms,
            bm25_ms: timings.bm25_ms,
            fusion_ms: timings.fusion_ms,
            total_ms: timings.total_ms,
            num_results: results_with.len(),
            num_entities: total_entities,
            num_facts: total_facts,
            top_entities: top_entities.clone(),
        };

        println!("Query: \"{}\"", query_text);
        println!(
            "  HNSW: {:.1}ms | BM25: {:.1}ms | Fusion: {:.1}ms | Total: {:.1}ms",
            timings.hnsw_ms, timings.bm25_ms, timings.fusion_ms, timings.total_ms,
        );
        println!(
            "  Results: {} | Entities: {} | Facts: {}",
            results_with.len(),
            total_entities,
            total_facts,
        );
        if !top_entities.is_empty() {
            println!("  Top entities: {}", top_entities.join(", "));
        }

        // Verify facts are present in results (with facts index)
        let has_facts = results_with.iter().any(|r| r.facts.is_some());
        assert!(
            has_facts,
            "Query '{}': at least one result should have facts",
            query_text,
        );

        // Verify no facts in results from the no-facts index
        let no_facts_has_facts = results_without.iter().any(|r| r.facts.is_some());
        assert!(
            !no_facts_has_facts,
            "Query '{}': no-facts index should not return facts",
            query_text,
        );

        // Verify scores are identical (facts don't affect ranking)
        let with_scores: Vec<f64> = results_with.iter().map(|r| r.final_score).collect();
        let without_scores: Vec<f64> = results_without.iter().map(|r| r.final_score).collect();
        assert_eq!(
            with_scores.len(),
            without_scores.len(),
            "Query '{}': result count should match with/without facts",
            query_text,
        );
        for (i, (ws, nos)) in with_scores.iter().zip(without_scores.iter()).enumerate() {
            assert!(
                (ws - nos).abs() < 1e-10,
                "Query '{}' result[{}]: scores differ with ({}) vs without ({}) facts",
                query_text,
                i,
                ws,
                nos,
            );
        }

        // Verify at least some expected entity types are found
        if !top_entities.is_empty() {
            let found_expected = expected_types.iter().any(|t| {
                top_entities.iter().any(|e| e.starts_with(&format!("[{t}]")))
            });
            // This is a soft check — MockFactExtractor assigns types by hash, not semantics
            if found_expected {
                println!("  ✓ Found expected entity types: {expected_types:?}");
            } else {
                println!(
                    "  (mock extractor assigned different types — expected {expected_types:?})"
                );
            }
        }
        println!();

        query_traces.push(trace);
    }

    // ── Phase 4: Performance trace table ────────────────────────────────
    println!("=== E2E Pipeline Performance Trace ===\n");

    // Header
    let langs: Vec<&str> = crawl_traces.iter().map(|t| t.lang).collect();
    print!("{:<20}", "Stage");
    for lang in &langs {
        print!("{:>10}", format!("{lang} (ms)"));
    }
    println!();
    println!("{}", "─".repeat(20 + langs.len() * 10));

    // Rows
    type StageGetter = fn(&CrawlTrace) -> f64;
    let stages: &[(&str, StageGetter)] = &[
        ("Fetch", |t| t.fetch_ms),
        ("HTML → Markdown", |t| t.html_to_md_ms),
        ("Quality gate", |t| t.quality_ms),
        ("Chunking", |t| t.chunk_ms),
        ("Embedding", |t| t.embed_ms),
        ("Fact extraction", |t| t.fact_extraction_ms),
    ];

    for &(name, getter) in stages {
        print!("{:<20}", name);
        for trace in &crawl_traces {
            print!("{:>10}", format!("{:.1}", getter(trace)));
        }
        println!();
    }

    println!("{}", "─".repeat(20 + langs.len() * 10));
    print!("{:<20}", "Total crawl");
    for trace in &crawl_traces {
        print!("{:>10}", format!("{:.1}", trace.total_crawl_ms));
    }
    println!();

    println!(
        "\nIndex ingest: {:.1}ms ({} chunks total, deduped to {} docs)",
        ingest_with_facts_ms,
        all_chunks_with_facts.len(),
        index_with_facts.doc_count(),
    );

    // ── Phase 5: Noise reduction table ──────────────────────────────────
    println!("\n=== Noise Reduction ===\n");
    println!(
        "{:<45} {:>15} {:>15} {:>10}",
        "URL", "Markdown chars", "Facts tokens", "Reduction",
    );
    println!("{}", "─".repeat(85));

    for trace in &crawl_traces {
        // Estimate "facts tokens" as count of entity mentions + fact triplets
        let facts_tokens = trace.entity_count * 3 + trace.fact_count * 5;
        let reduction = if trace.markdown_len > 0 {
            (1.0 - facts_tokens as f64 / trace.markdown_len as f64) * 100.0
        } else {
            0.0
        };
        let short_url = trace
            .url
            .strip_prefix("https://")
            .unwrap_or(&trace.url);
        println!(
            "{:<45} {:>15} {:>15} {:>9.1}%",
            short_url, trace.markdown_len, facts_tokens, reduction,
        );
    }

    // ── Phase 6: Summary assertions ─────────────────────────────────────
    println!("\n=== Summary ===\n");

    let total_entities: usize = crawl_traces.iter().map(|t| t.entity_count).sum();
    let total_facts: usize = crawl_traces.iter().map(|t| t.fact_count).sum();
    println!("Total entities extracted: {total_entities}");
    println!("Total facts extracted: {total_facts}");
    println!("Queries executed: {}", query_traces.len());

    assert!(
        total_entities > 0,
        "Should extract at least some entities from Wikipedia pages",
    );
    assert!(
        total_facts > 0,
        "Should extract at least some facts from Wikipedia pages",
    );

    // All queries should return results
    for qt in &query_traces {
        assert!(
            qt.num_results > 0,
            "Query '{}' should return at least one result",
            qt.query,
        );
    }

    println!("\n✓ E2E pipeline test passed!");
}
