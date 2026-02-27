use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use oi_index::HybridIndex;
use oi_types::traits::SearchIndex;
use oi_types::{ChunkMeta, ContentChunk, QueryFilters};
use std::time::Duration;

/// Generate a 384-dim clustered embedding. Documents in the same cluster
/// share a base direction with small random perturbation.
fn make_embedding(cluster: usize, doc_idx: usize, dim: usize) -> Vec<f32> {
    let mut v = vec![0.0f32; dim];
    // Deterministic "random" using xorshift seeded by (cluster, doc_idx)
    let mut seed = (cluster as u64 * 100003 + doc_idx as u64 * 999983 + 7) | 1;
    let next = |s: &mut u64| -> f64 {
        *s ^= *s << 13;
        *s ^= *s >> 7;
        *s ^= *s << 17;
        (*s as f64) / (u64::MAX as f64)
    };

    // Base direction from cluster id
    let base_idx = cluster % dim;
    v[base_idx] = 1.0;

    // Small perturbation
    for d in 0..dim {
        v[d] += (next(&mut seed) as f32 - 0.5) * 0.1;
    }

    // Normalize
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 1e-10 {
        for x in &mut v {
            *x /= norm;
        }
    }
    v
}

fn make_query_embedding(dim: usize) -> Vec<f32> {
    make_embedding(0, 999999, dim)
}

static DOMAINS: &[&str] = &[
    "docs.rust-lang.org", "stackoverflow.com", "arxiv.org", "github.com",
    "en.wikipedia.org", "developer.mozilla.org", "news.ycombinator.com",
    "medium.com", "dev.to", "crates.io", "reddit.com", "python.org",
    "golang.org", "nodejs.org", "reactjs.org", "kubernetes.io",
    "docker.com", "aws.amazon.com", "cloud.google.com", "azure.microsoft.com",
];

static TOPICS: &[&str] = &[
    "rust programming language systems safety",
    "python machine learning data science numpy",
    "javascript web development react frontend",
    "kubernetes container orchestration deployment",
    "database sql postgres optimization queries",
    "cryptography blockchain decentralized protocol",
    "neural network deep learning transformer attention",
    "distributed systems consensus replication",
    "linux kernel operating system memory management",
    "compiler optimization abstract syntax tree parsing",
    "networking tcp http protocol websocket",
    "security vulnerability exploit mitigation defense",
    "cloud computing serverless microservices api",
    "testing continuous integration deployment pipeline",
    "functional programming monads category theory",
    "embedded systems iot sensor real time",
    "graphics rendering shader gpu compute",
    "natural language processing tokenization embeddings",
    "game engine physics simulation collision detection",
    "robotics control systems path planning autonomous",
];

fn build_index(n: usize) -> HybridIndex {
    let dim = 384;
    let index = HybridIndex::new(dim);
    let num_clusters = 20;

    // Build in batches of 1000
    let batch_size = 1000;
    for batch_start in (0..n).step_by(batch_size) {
        let batch_end = (batch_start + batch_size).min(n);
        let chunks: Vec<ContentChunk> = (batch_start..batch_end)
            .map(|i| {
                let cluster = i % num_clusters;
                let domain = DOMAINS[i % DOMAINS.len()];
                let topic = TOPICS[cluster];

                ContentChunk {
                    chunk_id: format!("doc_{i}"),
                    content: format!("{topic} document number {i}"),
                    embedding: Some(make_embedding(cluster, i, dim)),
                    seq: 0,
                    meta: Some(ChunkMeta {
                        url: format!("https://{domain}/doc/{i}"),
                        title: format!("Document {i}: {topic}"),
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

fn bench_hnsw(c: &mut Criterion) {
    let dim = 384;
    let mut group = c.benchmark_group("hnsw_search");

    for &n in &[1_000, 10_000, 50_000, 100_000] {
        let index = build_index(n);
        let query = make_query_embedding(dim);
        let filters = QueryFilters::default();

        if n >= 50_000 {
            group.measurement_time(Duration::from_secs(30));
            group.sample_size(20);
        }

        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, _| {
            b.iter(|| {
                index.query(&query, "rust programming", &filters, 10).unwrap();
            });
        });
    }
    group.finish();
}

fn bench_hybrid(c: &mut Criterion) {
    let dim = 384;
    let mut group = c.benchmark_group("hybrid_query");

    for &n in &[1_000, 10_000, 50_000, 100_000] {
        let index = build_index(n);
        let query = make_query_embedding(dim);
        let filters = QueryFilters::default();

        if n >= 50_000 {
            group.measurement_time(Duration::from_secs(30));
            group.sample_size(20);
        }

        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, _| {
            b.iter(|| {
                index.query(&query, "rust programming language", &filters, 10).unwrap();
            });
        });
    }
    group.finish();
}

fn bench_query_timed(c: &mut Criterion) {
    let dim = 384;
    let index = build_index(10_000);
    let query = make_query_embedding(dim);
    let filters = QueryFilters::default();

    let mut group = c.benchmark_group("query_timed_overhead");

    group.bench_function("query", |b| {
        b.iter(|| {
            index.query(&query, "rust programming", &filters, 10).unwrap();
        });
    });

    group.bench_function("query_timed", |b| {
        b.iter(|| {
            index.query_timed(&query, "rust programming", &filters, 10).unwrap();
        });
    });

    group.finish();
}

fn bench_hybrid_filtered(c: &mut Criterion) {
    let dim = 384;
    let mut group = c.benchmark_group("hybrid_query_filtered");

    let index = build_index(10_000);
    let query = make_query_embedding(dim);

    let filters_fresh = QueryFilters {
        freshness: Some("7d".to_string()),
        ..Default::default()
    };
    group.bench_function("freshness_7d", |b| {
        b.iter(|| {
            index.query(&query, "rust programming", &filters_fresh, 10).unwrap();
        });
    });

    let filters_type = QueryFilters {
        types: vec!["html".to_string()],
        ..Default::default()
    };
    group.bench_function("type_html", |b| {
        b.iter(|| {
            index.query(&query, "rust programming", &filters_type, 10).unwrap();
        });
    });

    group.finish();
}

fn bench_ingest(c: &mut Criterion) {
    let dim = 384;
    let num_clusters = 20;

    let chunks: Vec<ContentChunk> = (0..1_000)
        .map(|i| {
            let cluster = i % num_clusters;
            ContentChunk {
                chunk_id: format!("doc_{i}"),
                content: format!("{} document {i}", TOPICS[cluster]),
                embedding: Some(make_embedding(cluster, i, dim)),
                seq: 0,
                meta: None,
            }
        })
        .collect();

    c.bench_function("ingest_batch_1000", |b| {
        b.iter(|| {
            let index = HybridIndex::new(dim);
            index.ingest(&chunks).unwrap();
        });
    });
}

criterion_group!(benches, bench_hnsw, bench_hybrid, bench_hybrid_filtered, bench_ingest, bench_query_timed);
criterion_main!(benches);
