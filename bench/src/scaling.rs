use oi_index::{HybridIndex, QueryTimings};
use oi_types::traits::SearchIndex;
use oi_types::{ChunkMeta, ContentChunk, QueryFilters};
use std::time::Instant;

use crate::embed::text_to_embedding;
use crate::seed::{DOMAINS, TOPICS};

const DIM: usize = 384;
const NUM_QUERIES_DEFAULT: usize = 100;

/// Run scaling analysis across multiple corpus sizes.
pub fn run_scaling(sizes: &[usize], num_queries: usize, output_format: &str, vector_only: bool) {
    let queries = generate_queries(num_queries);

    if vector_only {
        eprintln!("Mode: vector-only (bm25_weight=0.0)\n");
    }

    let mut all_rows: Vec<SizeRow> = Vec::new();

    for &n in sizes {
        eprintln!("Building index with {n} documents...");
        let build_start = Instant::now();
        let index = build_bench_index(n, vector_only);
        let build_ms = build_start.elapsed().as_secs_f64() * 1000.0;
        eprintln!("  Built in {build_ms:.0}ms, running {num_queries} queries...");

        let mut timings_vec: Vec<QueryTimings> = Vec::with_capacity(num_queries);

        for (emb, text) in &queries {
            let filters = QueryFilters::default();
            let (_results, timings) = index
                .query_timed(emb, text, &filters, 10)
                .expect("query failed");
            timings_vec.push(timings);
        }

        let row = compute_size_row(n, &timings_vec);
        eprintln!(
            "  p50: total={:.2}ms hnsw={:.2}ms bm25={:.2}ms fusion={:.2}ms enrichment={:.2}ms",
            row.total_p50, row.hnsw_p50, row.bm25_p50, row.fusion_p50, row.enrichment_p50,
        );
        all_rows.push(row);
    }

    // Compute scaling exponents via log-log linear regression
    let exponents = compute_exponents(&all_rows);

    match output_format {
        "json" => print_json(&all_rows, &exponents),
        _ => print_table(&all_rows, &exponents),
    }
}

fn generate_queries(n: usize) -> Vec<(Vec<f32>, String)> {
    let mut queries = Vec::with_capacity(n);
    for i in 0..n {
        let topic = TOPICS[i % TOPICS.len()];
        let text = format!("{topic} query {i}");
        let emb = text_to_embedding(&text, DIM);
        queries.push((emb, text));
    }
    queries
}

fn build_bench_index(n: usize, _vector_only: bool) -> HybridIndex {
    let config = oi_index::HybridConfig {
        skip_dedup: true,
        semantic_weight: 1.0,
        bm25_weight: 0.0,
        ..Default::default()
    };
    let index = HybridIndex::with_config(DIM, config);
    let batch_size = 1000;

    for batch_start in (0..n).step_by(batch_size) {
        let batch_end = (batch_start + batch_size).min(n);
        let chunks: Vec<ContentChunk> = (batch_start..batch_end)
            .map(|i| {
                let cluster = i % TOPICS.len();
                let domain = DOMAINS[i % DOMAINS.len()];
                let topic = TOPICS[cluster];
                let text = format!("{topic} document {i} with content for scaling bench");

                let emb = text_to_embedding(&format!("{topic}_cluster_{cluster}_{i}"), DIM);

                ContentChunk {
                    chunk_id: format!("doc_{i}"),
                    content: text,
                    embedding: Some(emb),
                    seq: 0,
                    meta: Some(ChunkMeta {
                        url: format!("https://{domain}/doc/{i}"),
                        title: format!("Document {i}"),
                        domain: domain.to_string(),
                        content_hash: format!("hash_{i}"),
                        crawled_at: Some(1700000000 + i as u64 * 60),
                        content_type: Some("text/html".to_string()),
                        entity: None,
                        content_kind: None,
                        ..Default::default()
                    }),
                }
            })
            .collect();
        index.ingest(&chunks).unwrap();
    }
    index
}

struct SizeRow {
    n: usize,
    hnsw_p50: f64,
    hnsw_p95: f64,
    bm25_p50: f64,
    bm25_p95: f64,
    fusion_p50: f64,
    fusion_p95: f64,
    rerank_p50: f64,
    enrichment_p50: f64,
    enrichment_p95: f64,
    total_p50: f64,
    total_p95: f64,
}

fn compute_size_row(n: usize, timings: &[QueryTimings]) -> SizeRow {
    let hnsw: Vec<f64> = timings.iter().map(|t| t.hnsw_ms).collect();
    let bm25: Vec<f64> = timings.iter().map(|t| t.bm25_ms).collect();
    let fusion: Vec<f64> = timings.iter().map(|t| t.fusion_ms).collect();
    let rerank: Vec<f64> = timings.iter().map(|t| t.rerank_ms).collect();
    let enrichment: Vec<f64> = timings.iter().map(|t| t.enrichment_ms).collect();
    let total: Vec<f64> = timings.iter().map(|t| t.total_ms).collect();

    SizeRow {
        n,
        hnsw_p50: percentile(&hnsw, 50.0),
        hnsw_p95: percentile(&hnsw, 95.0),
        bm25_p50: percentile(&bm25, 50.0),
        bm25_p95: percentile(&bm25, 95.0),
        fusion_p50: percentile(&fusion, 50.0),
        fusion_p95: percentile(&fusion, 95.0),
        rerank_p50: percentile(&rerank, 50.0),
        enrichment_p50: percentile(&enrichment, 50.0),
        enrichment_p95: percentile(&enrichment, 95.0),
        total_p50: percentile(&total, 50.0),
        total_p95: percentile(&total, 95.0),
    }
}

fn percentile(data: &[f64], pct: f64) -> f64 {
    if data.is_empty() {
        return 0.0;
    }
    let mut sorted = data.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let idx = ((pct / 100.0) * (sorted.len() - 1) as f64) as usize;
    sorted[idx.min(sorted.len() - 1)]
}

struct ScalingExponents {
    hnsw: f64,
    bm25: f64,
    fusion: f64,
    enrichment: f64,
    total: f64,
}

/// Compute scaling exponent alpha via log-log linear regression.
/// latency = c * N^alpha  =>  log(latency) = log(c) + alpha * log(N)
fn compute_exponents(rows: &[SizeRow]) -> ScalingExponents {
    if rows.len() < 2 {
        return ScalingExponents {
            hnsw: 0.0,
            bm25: 0.0,
            fusion: 0.0,
            enrichment: 0.0,
            total: 0.0,
        };
    }

    let log_n: Vec<f64> = rows.iter().map(|r| (r.n as f64).ln()).collect();

    ScalingExponents {
        hnsw: log_log_slope(&log_n, &rows.iter().map(|r| r.hnsw_p50).collect::<Vec<_>>()),
        bm25: log_log_slope(&log_n, &rows.iter().map(|r| r.bm25_p50).collect::<Vec<_>>()),
        fusion: log_log_slope(&log_n, &rows.iter().map(|r| r.fusion_p50).collect::<Vec<_>>()),
        enrichment: log_log_slope(
            &log_n,
            &rows.iter().map(|r| r.enrichment_p50).collect::<Vec<_>>(),
        ),
        total: log_log_slope(&log_n, &rows.iter().map(|r| r.total_p50).collect::<Vec<_>>()),
    }
}

/// Simple linear regression slope on (log_n, log(latency)) pairs.
fn log_log_slope(log_n: &[f64], latency: &[f64]) -> f64 {
    let log_lat: Vec<f64> = latency.iter().map(|&v| if v > 0.0 { v.ln() } else { 0.0 }).collect();

    let n = log_n.len() as f64;
    let sum_x: f64 = log_n.iter().sum();
    let sum_y: f64 = log_lat.iter().sum();
    let sum_xy: f64 = log_n.iter().zip(&log_lat).map(|(x, y)| x * y).sum();
    let sum_x2: f64 = log_n.iter().map(|x| x * x).sum();

    let denom = n * sum_x2 - sum_x * sum_x;
    if denom.abs() < 1e-15 {
        return 0.0;
    }
    (n * sum_xy - sum_x * sum_y) / denom
}

fn classify_exponent(alpha: f64) -> &'static str {
    if alpha < 0.2 {
        "~constant"
    } else if alpha < 0.4 {
        "~logarithmic"
    } else if alpha < 0.7 {
        "sub-linear"
    } else if alpha < 1.3 {
        "~LINEAR"
    } else {
        "super-linear"
    }
}

fn print_table(rows: &[SizeRow], exp: &ScalingExponents) {
    println!("\n=== Scaling Analysis ===\n");
    println!(
        "{:>10} {:>10} {:>10} {:>10} {:>10} {:>10} {:>12}",
        "Corpus", "HNSW p50", "BM25 p50", "Fusion p50", "Enrich p50", "Total p50", "Total p95"
    );
    println!("{}", "-".repeat(76));

    for r in rows {
        println!(
            "{:>10} {:>9.2}ms {:>9.2}ms {:>9.2}ms {:>9.2}ms {:>9.2}ms {:>11.2}ms",
            format_count(r.n),
            r.hnsw_p50,
            r.bm25_p50,
            r.fusion_p50,
            r.enrichment_p50,
            r.total_p50,
            r.total_p95,
        );
    }

    println!("\n=== Scaling Exponents (alpha: latency ~ N^alpha) ===\n");
    println!(
        "  HNSW:       alpha={:.3}  ({})",
        exp.hnsw,
        classify_exponent(exp.hnsw)
    );
    println!(
        "  BM25:       alpha={:.3}  ({})",
        exp.bm25,
        classify_exponent(exp.bm25)
    );
    println!(
        "  Fusion:     alpha={:.3}  ({})",
        exp.fusion,
        classify_exponent(exp.fusion)
    );
    println!(
        "  Enrichment: alpha={:.3}  ({})",
        exp.enrichment,
        classify_exponent(exp.enrichment)
    );
    println!(
        "  Total:      alpha={:.3}  ({})",
        exp.total,
        classify_exponent(exp.total)
    );

    println!("\n  (alpha < 0.3 ~ logarithmic, ~1.0 ~ linear)\n");
}

fn print_json(rows: &[SizeRow], exp: &ScalingExponents) {
    let data: Vec<serde_json::Value> = rows
        .iter()
        .map(|r| {
            serde_json::json!({
                "corpus_size": r.n,
                "hnsw_p50_ms": r.hnsw_p50,
                "hnsw_p95_ms": r.hnsw_p95,
                "bm25_p50_ms": r.bm25_p50,
                "bm25_p95_ms": r.bm25_p95,
                "fusion_p50_ms": r.fusion_p50,
                "fusion_p95_ms": r.fusion_p95,
                "rerank_p50_ms": r.rerank_p50,
                "enrichment_p50_ms": r.enrichment_p50,
                "enrichment_p95_ms": r.enrichment_p95,
                "total_p50_ms": r.total_p50,
                "total_p95_ms": r.total_p95,
            })
        })
        .collect();

    let output = serde_json::json!({
        "data_points": data,
        "scaling_exponents": {
            "hnsw": { "alpha": exp.hnsw, "class": classify_exponent(exp.hnsw) },
            "bm25": { "alpha": exp.bm25, "class": classify_exponent(exp.bm25) },
            "fusion": { "alpha": exp.fusion, "class": classify_exponent(exp.fusion) },
            "enrichment": { "alpha": exp.enrichment, "class": classify_exponent(exp.enrichment) },
            "total": { "alpha": exp.total, "class": classify_exponent(exp.total) },
        },
    });

    println!("{}", serde_json::to_string_pretty(&output).unwrap());
}

fn format_count(n: usize) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{}K", n / 1_000)
    } else {
        n.to_string()
    }
}

/// Parse a comma-separated list of sizes, supporting K and M suffixes.
pub fn parse_sizes(s: &str) -> Vec<usize> {
    s.split(',')
        .filter_map(|tok| {
            let tok = tok.trim().to_lowercase();
            if tok.ends_with('m') {
                tok[..tok.len() - 1].parse::<f64>().ok().map(|v| (v * 1_000_000.0) as usize)
            } else if tok.ends_with('k') {
                tok[..tok.len() - 1].parse::<f64>().ok().map(|v| (v * 1_000.0) as usize)
            } else {
                tok.parse().ok()
            }
        })
        .collect()
}

pub fn default_sizes() -> Vec<usize> {
    vec![1_000, 5_000, 10_000, 50_000, 100_000, 500_000]
}

pub fn default_num_queries() -> usize {
    NUM_QUERIES_DEFAULT
}
