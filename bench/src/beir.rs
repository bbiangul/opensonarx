use oi_embed::batch::embed_in_batches;
use oi_index::hybrid::HybridConfig;
use oi_index::HybridIndex;
use oi_types::traits::SearchIndex;
use oi_types::{ChunkMeta, ContentChunk, QueryFilters};
use serde::Deserialize;
use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::metrics;

// ── Data structures ──

#[derive(Debug, Deserialize)]
struct BeirDoc {
    #[serde(rename = "_id")]
    id: String,
    title: String,
    text: String,
}

#[derive(Debug, Deserialize)]
struct BeirQuery {
    #[serde(rename = "_id")]
    id: String,
    text: String,
}

/// Parsed qrels: query_id -> [(doc_id, relevance)]
type Qrels = HashMap<String, Vec<(String, u8)>>;

/// Metrics from a single BEIR evaluation run.
struct BeirMetrics {
    ndcg_10: f64,
    recall_10: f64,
    recall_100: f64,
    mrr_10: f64,
}

// ── Published baselines (Arctic Embed S, nDCG@10) ──

fn published_ndcg10(dataset: &str) -> Option<f64> {
    match dataset {
        "scifact" => Some(0.6974),
        "nfcorpus" => Some(0.3426),
        _ => None,
    }
}

// ── Cache / download ──

fn cache_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("OI_BEIR_DIR") {
        return PathBuf::from(dir);
    }
    let home = std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."));
    home.join(".cache").join("opensonarx").join("beir")
}

fn dataset_dir(name: &str) -> PathBuf {
    cache_dir().join(name)
}

/// Download and extract a BEIR dataset ZIP if not already cached.
fn ensure_dataset(name: &str) -> Result<PathBuf, String> {
    let dir = dataset_dir(name);
    if dir.join("corpus.jsonl").exists() {
        tracing::info!("BEIR dataset '{}' already cached at {}", name, dir.display());
        return Ok(dir);
    }

    let url = format!(
        "https://public.ukp.informatik.tu-darmstadt.de/thakur/BEIR/datasets/{}.zip",
        name
    );
    tracing::info!("Downloading BEIR dataset '{}' from {}", name, url);

    let resp = ureq::get(&url)
        .call()
        .map_err(|e| format!("download failed: {e}"))?;

    let bytes = resp
        .into_body()
        .with_config()
        .limit(500 * 1024 * 1024)
        .read_to_vec()
        .map_err(|e| format!("read body: {e}"))?;

    tracing::info!("Downloaded {} bytes, extracting...", bytes.len());

    let reader = std::io::Cursor::new(&bytes);
    let mut archive =
        zip::ZipArchive::new(reader).map_err(|e| format!("zip open failed: {e}"))?;

    let base = cache_dir();
    std::fs::create_dir_all(&base).map_err(|e| format!("mkdir: {e}"))?;

    for i in 0..archive.len() {
        let mut file = archive
            .by_index(i)
            .map_err(|e| format!("zip entry {i}: {e}"))?;
        let outpath = base.join(file.name());

        if file.is_dir() {
            std::fs::create_dir_all(&outpath).map_err(|e| format!("mkdir: {e}"))?;
        } else {
            if let Some(parent) = outpath.parent() {
                std::fs::create_dir_all(parent).map_err(|e| format!("mkdir: {e}"))?;
            }
            let mut outfile =
                std::fs::File::create(&outpath).map_err(|e| format!("create file: {e}"))?;
            std::io::copy(&mut file, &mut outfile).map_err(|e| format!("extract: {e}"))?;
        }
    }

    if !dir.join("corpus.jsonl").exists() {
        return Err(format!(
            "extracted archive but corpus.jsonl not found at {}",
            dir.display()
        ));
    }

    tracing::info!("Extracted to {}", dir.display());
    Ok(dir)
}

// ── Parsing ──

fn parse_corpus(path: &Path) -> Result<Vec<BeirDoc>, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("open corpus: {e}"))?;
    let reader = BufReader::new(file);
    let mut docs = Vec::new();
    for (i, line) in reader.lines().enumerate() {
        let line = line.map_err(|e| format!("read line {i}: {e}"))?;
        if line.trim().is_empty() {
            continue;
        }
        let doc: BeirDoc =
            serde_json::from_str(&line).map_err(|e| format!("parse corpus line {i}: {e}"))?;
        docs.push(doc);
    }
    Ok(docs)
}

fn parse_queries(path: &Path) -> Result<Vec<BeirQuery>, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("open queries: {e}"))?;
    let reader = BufReader::new(file);
    let mut queries = Vec::new();
    for (i, line) in reader.lines().enumerate() {
        let line = line.map_err(|e| format!("read line {i}: {e}"))?;
        if line.trim().is_empty() {
            continue;
        }
        let q: BeirQuery =
            serde_json::from_str(&line).map_err(|e| format!("parse query line {i}: {e}"))?;
        queries.push(q);
    }
    Ok(queries)
}

fn parse_qrels(path: &Path) -> Result<Qrels, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("open qrels: {e}"))?;
    let reader = BufReader::new(file);
    let mut qrels: Qrels = HashMap::new();
    for (i, line) in reader.lines().enumerate() {
        let line = line.map_err(|e| format!("read line {i}: {e}"))?;
        let line = line.trim().to_string();
        if line.is_empty() {
            continue;
        }
        // Skip header line
        if i == 0 && line.starts_with("query-id") {
            continue;
        }
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() < 3 {
            continue;
        }
        let query_id = parts[0].to_string();
        let doc_id = parts[1].to_string();
        let score: u8 = parts[2]
            .parse()
            .map_err(|e| format!("parse qrel score line {i}: {e}"))?;
        qrels.entry(query_id).or_default().push((doc_id, score));
    }
    Ok(qrels)
}

// ── Evaluation core (reusable for sweeps) ──

/// Evaluate queries against an already-built index. Returns averaged metrics.
fn evaluate_queries(
    index: &HybridIndex,
    embedder: &dyn oi_types::EmbeddingPipeline,
    query_map: &HashMap<&str, &str>,
    eval_query_ids: &[&str],
    qrels: &Qrels,
) -> (BeirMetrics, Vec<f64>) {
    let mut ndcg_10_sum = 0.0;
    let mut recall_10_sum = 0.0;
    let mut recall_100_sum = 0.0;
    let mut mrr_10_sum = 0.0;
    let mut query_count = 0usize;
    let mut query_latencies = Vec::new();

    for qid in eval_query_ids {
        let query_text = match query_map.get(qid) {
            Some(t) => *t,
            None => continue,
        };

        let start = Instant::now();
        let query_emb = match embedder.embed_query(query_text) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let results = match index.query(&query_emb, query_text, &QueryFilters::default(), 100) {
            Ok(r) => r,
            Err(_) => continue,
        };
        let latency_ms = start.elapsed().as_secs_f64() * 1000.0;
        query_latencies.push(latency_ms);

        let retrieved_ids: Vec<&str> = results
            .iter()
            .map(|r| r.url.strip_prefix("beir://").unwrap_or(&r.url))
            .collect();

        let doc_qrels = match qrels.get(*qid) {
            Some(q) => q,
            None => continue,
        };

        let relevant_ids: Vec<&str> = doc_qrels
            .iter()
            .filter(|(_, s)| *s > 0)
            .map(|(id, _)| id.as_str())
            .collect();

        let relevance_refs: Vec<(&str, u8)> = doc_qrels
            .iter()
            .map(|(id, s)| (id.as_str(), *s))
            .collect();

        ndcg_10_sum += metrics::ndcg_at_k(&relevance_refs, &retrieved_ids, 10);
        recall_10_sum += metrics::recall_at_k(&relevant_ids, &retrieved_ids, 10);
        recall_100_sum += metrics::recall_at_k(&relevant_ids, &retrieved_ids, 100);

        let top_10: Vec<&str> = retrieved_ids.iter().take(10).copied().collect();
        mrr_10_sum += metrics::mrr(&relevant_ids, &top_10);

        query_count += 1;
    }

    let n = query_count.max(1) as f64;
    (
        BeirMetrics {
            ndcg_10: ndcg_10_sum / n,
            recall_10: recall_10_sum / n,
            recall_100: recall_100_sum / n,
            mrr_10: mrr_10_sum / n,
        },
        query_latencies,
    )
}

/// Build chunks from corpus + embeddings (reusable for sweeps).
fn build_chunks(corpus: &[BeirDoc], embeddings: Vec<Vec<f32>>) -> Vec<ContentChunk> {
    corpus
        .iter()
        .zip(embeddings)
        .map(|(doc, emb)| ContentChunk {
            chunk_id: doc.id.clone(),
            content: format!("{} {}", doc.title, doc.text),
            embedding: Some(emb),
            seq: 0,
            meta: Some(ChunkMeta {
                url: format!("beir://{}", doc.id),
                title: doc.title.clone(),
                domain: format!("beir-{}", doc.id),
                content_hash: doc.id.clone(),
                crawled_at: None,
                content_type: None,
                entity: None,
                content_kind: None,
                ..Default::default()
            }),
        })
        .collect()
}

// ── Public entry points ──

pub fn run_beir(dataset_name: &str, pipeline_name: &str, output_format: &str, semantic_weight: Option<f64>, rerank: bool) {
    let embedder = crate::get_embedder(pipeline_name);
    let dim = embedder.dimension();

    #[cfg(feature = "reranker")]
    let reranker: Option<std::sync::Arc<dyn oi_types::Reranker>> = if rerank {
        tracing::info!("Loading cross-encoder reranker...");
        Some(std::sync::Arc::new(
            oi_embed::OnnxReranker::new().expect("Failed to load reranker"),
        ))
    } else {
        None
    };
    #[cfg(not(feature = "reranker"))]
    let reranker: Option<std::sync::Arc<dyn oi_types::Reranker>> = {
        if rerank {
            eprintln!("Reranker requires --features reranker");
            std::process::exit(1);
        }
        None
    };

    // 1. Download/extract
    let dir = match ensure_dataset(dataset_name) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Failed to get BEIR dataset '{}': {}", dataset_name, e);
            std::process::exit(1);
        }
    };

    // 2. Parse corpus, queries, qrels
    let corpus = parse_corpus(&dir.join("corpus.jsonl")).unwrap_or_else(|e| {
        eprintln!("Failed to parse corpus: {e}");
        std::process::exit(1);
    });
    let queries = parse_queries(&dir.join("queries.jsonl")).unwrap_or_else(|e| {
        eprintln!("Failed to parse queries: {e}");
        std::process::exit(1);
    });
    let qrels = parse_qrels(&dir.join("qrels").join("test.tsv")).unwrap_or_else(|e| {
        eprintln!("Failed to parse qrels: {e}");
        std::process::exit(1);
    });

    let query_map: HashMap<&str, &str> = queries.iter().map(|q| (q.id.as_str(), q.text.as_str())).collect();
    let eval_query_ids: Vec<&str> = qrels.keys().map(|s| s.as_str()).collect();

    // Default to the production HybridConfig weight so an unflagged run
    // measures the shipped configuration, not vector-only.
    let sw = semantic_weight.unwrap_or_else(|| HybridConfig::default().semantic_weight);

    println!("\n=== BEIR Evaluation: {} ===", dataset_name);
    println!(
        "Pipeline: {} ({}-dim) | Weights: semantic={:.2} bm25={:.2} | Rerank: {}",
        embedder.name(), dim, sw, 1.0 - sw,
        if reranker.is_some() { "yes" } else { "no" }
    );
    println!(
        "Corpus: {} docs | Queries: {} (with qrels)\n",
        corpus.len(),
        eval_query_ids.len()
    );

    // 3. Batch-embed corpus
    let ingest_start = Instant::now();
    let doc_texts: Vec<String> = corpus
        .iter()
        .map(|d| format!("{} {}", d.title, d.text))
        .collect();
    let doc_refs: Vec<&str> = doc_texts.iter().map(|s| s.as_str()).collect();

    let embeddings = embed_in_batches(embedder.as_ref(), &doc_refs, 32).unwrap_or_else(|e| {
        eprintln!("Embedding failed: {e}");
        std::process::exit(1);
    });

    // 4. Build index with specified weights
    let config = HybridConfig {
        semantic_weight: sw,
        bm25_weight: 1.0 - sw,
        skip_dedup: true,
        ..Default::default()
    };
    let mut index = HybridIndex::with_config(dim, config);
    if let Some(ref rr) = reranker {
        index = index.with_reranker(rr.clone());
    }
    let chunks = build_chunks(&corpus, embeddings);

    index.ingest(&chunks).unwrap_or_else(|e| {
        eprintln!("Ingest failed: {e}");
        std::process::exit(1);
    });
    let ingest_secs = ingest_start.elapsed().as_secs_f64();
    let docs_per_sec = corpus.len() as f64 / ingest_secs;

    // 5. Evaluate
    let (m, mut query_latencies) = evaluate_queries(&index, embedder.as_ref(), &query_map, &eval_query_ids, &qrels);

    query_latencies.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let p50 = percentile(&query_latencies, 50.0);
    let p95 = percentile(&query_latencies, 95.0);
    let published = published_ndcg10(dataset_name);

    match output_format {
        "json" => {
            let output = serde_json::json!({
                "dataset": dataset_name,
                "pipeline": pipeline_name,
                "semantic_weight": sw,
                "corpus_size": corpus.len(),
                "queries_evaluated": eval_query_ids.len(),
                "metrics": {
                    "ndcg_10": m.ndcg_10,
                    "recall_10": m.recall_10,
                    "recall_100": m.recall_100,
                    "mrr_10": m.mrr_10,
                },
                "published_ndcg_10": published,
                "ingest_secs": ingest_secs,
                "docs_per_sec": docs_per_sec,
                "query_latency_p50_ms": p50,
                "query_latency_p95_ms": p95,
            });
            println!("{}", serde_json::to_string_pretty(&output).unwrap());
        }
        _ => {
            let published_str = published
                .map(|v| format!("{:.4}", v))
                .unwrap_or_else(|| "-".to_string());

            println!(
                "{:<16} {:<10} {:<10}",
                "Metric", "Ours", "Published"
            );
            println!(
                "{:<16} {:<10} {:<10}",
                "------", "----", "---------"
            );
            println!("{:<16} {:<10.4} {:<10}", "nDCG@10", m.ndcg_10, published_str);
            println!("{:<16} {:<10.4} {:<10}", "Recall@10", m.recall_10, "-");
            println!("{:<16} {:<10.4} {:<10}", "Recall@100", m.recall_100, "-");
            println!("{:<16} {:<10.4} {:<10}", "MRR@10", m.mrr_10, "-");
            println!(
                "\nIngest: {:.1}s ({:.0} docs/sec) | Query: p50={:.1}ms p95={:.1}ms\n",
                ingest_secs, docs_per_sec, p50, p95
            );
        }
    }
}

/// Sweep semantic weights on a dataset. Embeds once, evaluates multiple configs.
pub fn sweep_beir(dataset_name: &str, pipeline_name: &str) {
    let embedder = crate::get_embedder(pipeline_name);
    let dim = embedder.dimension();

    let dir = match ensure_dataset(dataset_name) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Failed to get BEIR dataset '{}': {}", dataset_name, e);
            std::process::exit(1);
        }
    };

    let corpus = parse_corpus(&dir.join("corpus.jsonl")).unwrap_or_else(|e| {
        eprintln!("Failed to parse corpus: {e}");
        std::process::exit(1);
    });
    let queries = parse_queries(&dir.join("queries.jsonl")).unwrap_or_else(|e| {
        eprintln!("Failed to parse queries: {e}");
        std::process::exit(1);
    });
    let qrels = parse_qrels(&dir.join("qrels").join("test.tsv")).unwrap_or_else(|e| {
        eprintln!("Failed to parse qrels: {e}");
        std::process::exit(1);
    });

    let query_map: HashMap<&str, &str> = queries.iter().map(|q| (q.id.as_str(), q.text.as_str())).collect();
    let eval_query_ids: Vec<&str> = qrels.keys().map(|s| s.as_str()).collect();

    println!("\n=== BEIR Weight Sweep: {} ===", dataset_name);
    println!("Pipeline: {} ({}-dim)", embedder.name(), dim);
    println!("Corpus: {} docs | Queries: {} (with qrels)", corpus.len(), eval_query_ids.len());

    // Embed once
    let embed_start = Instant::now();
    let doc_texts: Vec<String> = corpus.iter().map(|d| format!("{} {}", d.title, d.text)).collect();
    let doc_refs: Vec<&str> = doc_texts.iter().map(|s| s.as_str()).collect();
    let embeddings = embed_in_batches(embedder.as_ref(), &doc_refs, 32).unwrap_or_else(|e| {
        eprintln!("Embedding failed: {e}");
        std::process::exit(1);
    });
    let embed_secs = embed_start.elapsed().as_secs_f64();
    println!("Embedding: {:.1}s\n", embed_secs);

    let weights: &[f64] = &[0.50, 0.60, 0.70, 0.75, 0.80, 0.85, 0.90, 0.95, 1.00];
    let published = published_ndcg10(dataset_name);

    println!(
        "{:>8} {:>8} {:>10} {:>10} {:>10} {:>8}",
        "Sem.W", "BM25.W", "nDCG@10", "R@10", "R@100", "MRR@10"
    );
    println!("{}", "-".repeat(60));

    for &sw in weights {
        let config = HybridConfig {
            semantic_weight: sw,
            bm25_weight: 1.0 - sw,
            skip_dedup: true,
            ..Default::default()
        };
        let index = HybridIndex::with_config(dim, config);
        let chunks = build_chunks(&corpus, embeddings.clone());
        index.ingest(&chunks).unwrap();

        let (m, _) = evaluate_queries(&index, embedder.as_ref(), &query_map, &eval_query_ids, &qrels);

        let marker = if let Some(pub_val) = published {
            if m.ndcg_10 >= pub_val { " *" } else { "" }
        } else {
            ""
        };

        println!(
            "{:>8.2} {:>8.2} {:>10.4} {:>10.4} {:>10.4} {:>8.4}{}",
            sw, 1.0 - sw, m.ndcg_10, m.recall_10, m.recall_100, m.mrr_10, marker
        );
    }

    if let Some(pub_val) = published {
        println!("\n* = beats published baseline ({:.4})", pub_val);
    }
    println!();
}

fn percentile(sorted: &[f64], pct: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = ((pct / 100.0) * (sorted.len() - 1) as f64) as usize;
    sorted[idx.min(sorted.len() - 1)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_corpus_line() {
        let line = r#"{"_id": "42", "title": "Test Title", "text": "Some body text"}"#;
        let doc: BeirDoc = serde_json::from_str(line).unwrap();
        assert_eq!(doc.id, "42");
        assert_eq!(doc.title, "Test Title");
        assert_eq!(doc.text, "Some body text");
    }

    #[test]
    fn test_parse_query_line() {
        let line = r#"{"_id": "q1", "text": "What is the effect of X on Y?"}"#;
        let q: BeirQuery = serde_json::from_str(line).unwrap();
        assert_eq!(q.id, "q1");
        assert_eq!(q.text, "What is the effect of X on Y?");
    }

    #[test]
    fn test_parse_qrels_tsv() {
        let tsv = "query-id\tcorpus-id\tscore\nq1\td1\t2\nq1\td2\t1\nq2\td3\t1\n";
        let tmp = std::env::temp_dir().join("oi_beir_qrels_test.tsv");
        std::fs::write(&tmp, tsv).unwrap();

        let qrels = parse_qrels(&tmp).unwrap();
        assert_eq!(qrels.len(), 2);
        assert_eq!(qrels["q1"].len(), 2);
        assert_eq!(qrels["q1"][0], ("d1".to_string(), 2));
        assert_eq!(qrels["q1"][1], ("d2".to_string(), 1));
        assert_eq!(qrels["q2"].len(), 1);
        assert_eq!(qrels["q2"][0], ("d3".to_string(), 1));

        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn test_doc_content_concat() {
        let doc = BeirDoc {
            id: "1".to_string(),
            title: "Climate Change".to_string(),
            text: "Global temperatures are rising.".to_string(),
        };
        let content = format!("{} {}", doc.title, doc.text);
        assert_eq!(content, "Climate Change Global temperatures are rising.");
    }
}
