use oi_embed::batch::embed_in_batches;
use oi_index::hybrid::HybridConfig;
use oi_index::HybridIndex;
use oi_types::traits::SearchIndex;
use oi_types::{ChunkMeta, ContentChunk, QueryFilters};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;

use crate::metrics;

// ── Data structures ──

#[derive(Debug, Clone)]
struct SealDoc {
    #[allow(dead_code)]
    date: Option<String>,
    text: String,
    title: String,
    url: String,
}

#[derive(Debug, Clone)]
struct SealQuestion {
    question: String,
    #[allow(dead_code)]
    answer: String,
    golds: Vec<SealDoc>,
    docs_12: Vec<SealDoc>,
    docs_20: Vec<SealDoc>,
    docs_30: Vec<SealDoc>,
    freshness: String,
    topic: String,
    search_results: String,
    #[allow(dead_code)]
    effective_year: String,
}

// ── Cache / download ──

fn cache_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("OI_SEALQA_DIR") {
        return PathBuf::from(dir);
    }
    let home = std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."));
    home.join(".cache").join("opensonarx").join("sealqa")
}

fn ensure_parquet() -> Result<PathBuf, String> {
    let dir = cache_dir();
    let path = dir.join("longseal.parquet");
    if path.exists() {
        tracing::info!("SealQA parquet cached at {}", path.display());
        return Ok(path);
    }

    // Step 1: Resolve the parquet URL via the HuggingFace JSON API
    let api_url =
        "https://huggingface.co/api/datasets/vtllms/sealqa/parquet/longseal/test";
    tracing::info!("Resolving parquet URL from {}", api_url);

    let resp = ureq::get(api_url)
        .call()
        .map_err(|e| format!("HF API request failed: {e}"))?;
    let body = resp
        .into_body()
        .with_config()
        .limit(1024 * 1024)
        .read_to_vec()
        .map_err(|e| format!("read API body: {e}"))?;
    let urls: Vec<String> =
        serde_json::from_slice(&body).map_err(|e| format!("parse API JSON: {e}"))?;
    let parquet_url = urls
        .first()
        .ok_or_else(|| "no parquet URLs returned by HF API".to_string())?;

    // Step 2: Download the actual parquet file
    tracing::info!("Downloading SealQA parquet from {}", parquet_url);
    let resp = ureq::get(parquet_url)
        .call()
        .map_err(|e| format!("download failed: {e}"))?;
    let bytes = resp
        .into_body()
        .with_config()
        .limit(200 * 1024 * 1024)
        .read_to_vec()
        .map_err(|e| format!("read parquet body: {e}"))?;

    tracing::info!("Downloaded {} bytes", bytes.len());
    std::fs::create_dir_all(&dir).map_err(|e| format!("mkdir: {e}"))?;
    std::fs::write(&path, &bytes).map_err(|e| format!("write parquet: {e}"))?;
    tracing::info!("Cached to {}", path.display());
    Ok(path)
}

// ── Parquet parsing ──

/// Extract a list of SealDoc from a struct-array column (golds, 12_docs, etc.)
fn extract_doc_list(
    list_array: &arrow::array::ListArray,
    row: usize,
) -> Vec<SealDoc> {
    use arrow::array::Array;

    if list_array.is_null(row) {
        return Vec::new();
    }

    let offsets = list_array.offsets();
    let start = offsets[row] as usize;
    let end = offsets[row + 1] as usize;
    if start == end {
        return Vec::new();
    }

    let values = list_array.values();
    extract_docs_from_struct(values, start, end)
}

fn parse_parquet(path: &std::path::Path) -> Result<Vec<SealQuestion>, String> {
    use arrow::array::Array;
    use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

    let file = std::fs::File::open(path).map_err(|e| format!("open parquet: {e}"))?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)
        .map_err(|e| format!("parquet reader: {e}"))?;
    let reader = builder.build().map_err(|e| format!("build reader: {e}"))?;

    let mut questions = Vec::new();

    for batch_result in reader {
        let batch = batch_result.map_err(|e| format!("read batch: {e}"))?;
        let num_rows = batch.num_rows();
        let schema = batch.schema();

        // Helper to get a string column by name
        let get_string_col = |name: &str| -> Option<&arrow::array::StringArray> {
            schema
                .index_of(name)
                .ok()
                .and_then(|i| batch.column(i).as_any().downcast_ref())
        };

        // Helper to get a list column by name (for doc arrays)
        let get_list_col = |name: &str| -> Option<&arrow::array::ListArray> {
            schema
                .index_of(name)
                .ok()
                .and_then(|i| batch.column(i).as_any().downcast_ref())
        };

        // Try large_list if regular list fails (HuggingFace sometimes uses LargeList)
        let get_large_list_col =
            |name: &str| -> Option<&arrow::array::LargeListArray> {
                schema
                    .index_of(name)
                    .ok()
                    .and_then(|i| batch.column(i).as_any().downcast_ref())
            };

        let question_col = get_string_col("question");
        let answer_col = get_string_col("answer");
        let freshness_col = get_string_col("freshness");
        let topic_col = get_string_col("topic");
        let search_results_col = get_string_col("search_results");
        let effective_year_col = get_string_col("effective_year");

        let golds_list = get_list_col("golds");
        let docs12_list = get_list_col("12_docs");
        let docs20_list = get_list_col("20_docs");
        let docs30_list = get_list_col("30_docs");

        // Also try large list variants
        let golds_large = get_large_list_col("golds");
        let docs12_large = get_large_list_col("12_docs");
        let docs20_large = get_large_list_col("20_docs");
        let docs30_large = get_large_list_col("30_docs");

        for row in 0..num_rows {
            let question = question_col
                .map(|c| c.value(row).to_string())
                .unwrap_or_default();
            let answer = answer_col
                .map(|c| c.value(row).to_string())
                .unwrap_or_default();
            let freshness = freshness_col
                .map(|c| c.value(row).to_string())
                .unwrap_or_default();
            let topic = topic_col
                .map(|c| c.value(row).to_string())
                .unwrap_or_default();
            let search_results = search_results_col
                .map(|c| c.value(row).to_string())
                .unwrap_or_default();
            let effective_year = effective_year_col
                .map(|c| c.value(row).to_string())
                .unwrap_or_default();

            let golds = if let Some(list) = golds_list {
                extract_doc_list(list, row)
            } else if let Some(large) = golds_large {
                extract_doc_list_large(large, row)
            } else {
                Vec::new()
            };

            let docs_12 = if let Some(list) = docs12_list {
                extract_doc_list(list, row)
            } else if let Some(large) = docs12_large {
                extract_doc_list_large(large, row)
            } else {
                Vec::new()
            };

            let docs_20 = if let Some(list) = docs20_list {
                extract_doc_list(list, row)
            } else if let Some(large) = docs20_large {
                extract_doc_list_large(large, row)
            } else {
                Vec::new()
            };

            let docs_30 = if let Some(list) = docs30_list {
                extract_doc_list(list, row)
            } else if let Some(large) = docs30_large {
                extract_doc_list_large(large, row)
            } else {
                Vec::new()
            };

            questions.push(SealQuestion {
                question,
                answer,
                golds,
                docs_12,
                docs_20,
                docs_30,
                freshness,
                topic,
                search_results,
                effective_year,
            });
        }
    }

    Ok(questions)
}

/// Extract docs from a LargeListArray (HuggingFace sometimes uses this type)
fn extract_doc_list_large(
    list_array: &arrow::array::LargeListArray,
    row: usize,
) -> Vec<SealDoc> {
    use arrow::array::Array;

    if list_array.is_null(row) {
        return Vec::new();
    }

    let offsets = list_array.offsets();
    let start = offsets[row] as usize;
    let end = offsets[row + 1] as usize;
    if start == end {
        return Vec::new();
    }

    let values = list_array.values();
    extract_docs_from_struct(values, start, end)
}

/// Shared extraction logic: given an ArrayRef pointing at a StructArray of docs,
/// extract SealDoc entries from index range [start..end).
fn extract_docs_from_struct(
    values: &arrow::array::ArrayRef,
    start: usize,
    end: usize,
) -> Vec<SealDoc> {
    use arrow::array::Array;

    let struct_arr = values
        .as_any()
        .downcast_ref::<arrow::array::StructArray>()
        .expect("doc list values should be StructArray");

    // Try StringArray first, then LargeStringArray
    let get_string = |name: &str, i: usize| -> Option<String> {
        struct_arr.column_by_name(name).and_then(|c| {
            if c.is_null(i) {
                return None;
            }
            c.as_any()
                .downcast_ref::<arrow::array::StringArray>()
                .map(|arr| arr.value(i).to_string())
                .or_else(|| {
                    c.as_any()
                        .downcast_ref::<arrow::array::LargeStringArray>()
                        .map(|arr| arr.value(i).to_string())
                })
        })
    };

    let mut docs = Vec::with_capacity(end - start);
    for i in start..end {
        docs.push(SealDoc {
            date: get_string("date", i),
            text: get_string("text", i).unwrap_or_default(),
            title: get_string("title", i).unwrap_or_default(),
            url: get_string("url", i).unwrap_or_default(),
        });
    }
    docs
}

// ── Evaluation ──

fn percentile(sorted: &[f64], pct: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = ((pct / 100.0) * (sorted.len() - 1) as f64) as usize;
    sorted[idx.min(sorted.len() - 1)]
}

/// Per-question result for aggregation
struct QuestionResult {
    mrr_10: f64,
    recall_1: f64,
    recall_3: f64,
    recall_5: f64,
    recall_10: f64,
    recall_15: f64,
    recall_20: f64,
    gold_rank: Option<usize>,
    latency_ms: f64,
    topic: String,
    freshness: String,
    search_results: String,
}

pub fn run_sealqa(
    pipeline_name: &str,
    output_format: &str,
    semantic_weight: Option<f64>,
    docs_size: usize,
    rerank: bool,
) {
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

    // Validate docs size
    if docs_size != 12 && docs_size != 20 && docs_size != 30 {
        eprintln!("Invalid --docs value: {docs_size}. Must be 12, 20, or 30.");
        std::process::exit(1);
    }

    // Download / cache parquet
    let parquet_path = match ensure_parquet() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Failed to get SealQA dataset: {e}");
            std::process::exit(1);
        }
    };

    // Parse
    let questions = parse_parquet(&parquet_path).unwrap_or_else(|e| {
        eprintln!("Failed to parse SealQA parquet: {e}");
        std::process::exit(1);
    });

    let sw = semantic_weight.unwrap_or_else(|| HybridConfig::default().semantic_weight);

    println!("\n=== SealQA Evaluation (LongSeal) ===");
    println!(
        "Pipeline: {} ({}-dim) | Doc set: {} | Weights: semantic={:.2} bm25={:.2} | Rerank: {}",
        embedder.name(),
        dim,
        docs_size,
        sw,
        1.0 - sw,
        if reranker.is_some() { "yes" } else { "no" }
    );
    println!("Questions: {}\n", questions.len());

    let mut results: Vec<QuestionResult> = Vec::new();
    let mut embed_latencies = Vec::new();
    let mut search_latencies = Vec::new();

    for (qi, q) in questions.iter().enumerate() {
        // Pick distractor set, then merge gold docs in (they are separate in the dataset)
        let distractors = match docs_size {
            12 => &q.docs_12,
            20 => &q.docs_20,
            _ => &q.docs_30,
        };

        if distractors.is_empty() && q.golds.is_empty() {
            tracing::warn!("Question {qi} has no docs for size {docs_size}, skipping");
            continue;
        }

        // Merge: golds + distractors (dedup by URL in case of overlap)
        let mut seen_urls = std::collections::HashSet::new();
        let mut merged_docs: Vec<&SealDoc> = Vec::new();
        for doc in &q.golds {
            if seen_urls.insert(&doc.url) {
                merged_docs.push(doc);
            }
        }
        for doc in distractors {
            if seen_urls.insert(&doc.url) {
                merged_docs.push(doc);
            }
        }

        let total_start = Instant::now();

        // Batch-embed all docs (title + text)
        let embed_start = Instant::now();
        let doc_texts: Vec<String> = merged_docs
            .iter()
            .map(|d| format!("{} {}", d.title, d.text))
            .collect();
        let doc_refs: Vec<&str> = doc_texts.iter().map(|s| s.as_str()).collect();

        let embeddings = match embed_in_batches(embedder.as_ref(), &doc_refs, 32) {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!("Embedding failed for question {qi}: {e}");
                continue;
            }
        };
        let embed_ms = embed_start.elapsed().as_secs_f64() * 1000.0;
        embed_latencies.push(embed_ms);

        // Build fresh index for this question
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

        let chunks: Vec<ContentChunk> = merged_docs
            .iter()
            .zip(embeddings)
            .enumerate()
            .map(|(i, (doc, emb))| ContentChunk {
                chunk_id: format!("seal-{qi}-{i}"),
                content: format!("{} {}", doc.title, doc.text),
                embedding: Some(emb),
                seq: 0,
                meta: Some(ChunkMeta {
                    url: doc.url.clone(),
                    title: doc.title.clone(),
                    domain: "sealqa".to_string(),
                    content_hash: format!("seal-{qi}-{i}"),
                    crawled_at: None,
                    content_type: None,
                    entity: None,
                    content_kind: None,
                    ..Default::default()
                }),
            })
            .collect();

        if let Err(e) = index.ingest(&chunks) {
            tracing::warn!("Ingest failed for question {qi}: {e}");
            continue;
        }

        // Query
        let search_start = Instant::now();
        let query_emb = match embedder.embed_query(&q.question) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("Query embedding failed for question {qi}: {e}");
                continue;
            }
        };

        // Fetch ALL docs to see where gold actually ranks
        let full_k = merged_docs.len();
        let search_results =
            match index.query(&query_emb, &q.question, &QueryFilters::default(), full_k) {
                Ok(r) => r,
                Err(e) => {
                    tracing::warn!("Search failed for question {qi}: {e}");
                    continue;
                }
            };
        let search_ms = search_start.elapsed().as_secs_f64() * 1000.0;
        search_latencies.push(search_ms);

        let total_ms = total_start.elapsed().as_secs_f64() * 1000.0;

        // Compute metrics using top-10 slice
        let gold_urls: Vec<&str> = q.golds.iter().map(|g| g.url.as_str()).collect();
        let all_retrieved_urls: Vec<&str> =
            search_results.iter().map(|r| r.url.as_str()).collect();

        let mrr_10 = metrics::mrr(&gold_urls, &all_retrieved_urls.iter().take(10).copied().collect::<Vec<_>>());
        let recall_1 = metrics::recall_at_k(&gold_urls, &all_retrieved_urls, 1);
        let recall_3 = metrics::recall_at_k(&gold_urls, &all_retrieved_urls, 3);
        let recall_5 = metrics::recall_at_k(&gold_urls, &all_retrieved_urls, 5);
        let recall_10 = metrics::recall_at_k(&gold_urls, &all_retrieved_urls, 10);
        let recall_15 = metrics::recall_at_k(&gold_urls, &all_retrieved_urls, 15);
        let recall_20 = metrics::recall_at_k(&gold_urls, &all_retrieved_urls, 20);

        // Find the true rank of the first gold doc in the full ranking
        let gold_rank = all_retrieved_urls
            .iter()
            .position(|url| gold_urls.contains(url))
            .map(|pos| pos + 1); // 1-indexed

        results.push(QuestionResult {
            mrr_10,
            recall_1,
            recall_3,
            recall_5,
            recall_10,
            recall_15,
            recall_20,
            gold_rank,
            latency_ms: total_ms,
            topic: q.topic.clone(),
            freshness: q.freshness.clone(),
            search_results: q.search_results.clone(),
        });

        // Progress indicator every 50 questions
        if (qi + 1) % 50 == 0 || qi + 1 == questions.len() {
            tracing::info!(
                "Progress: {}/{} questions evaluated",
                qi + 1,
                questions.len()
            );
        }
    }

    if results.is_empty() {
        eprintln!("No questions were successfully evaluated.");
        std::process::exit(1);
    }

    // Aggregate metrics
    let n = results.len() as f64;
    let avg_mrr = results.iter().map(|r| r.mrr_10).sum::<f64>() / n;
    let avg_r1 = results.iter().map(|r| r.recall_1).sum::<f64>() / n;
    let avg_r3 = results.iter().map(|r| r.recall_3).sum::<f64>() / n;
    let avg_r5 = results.iter().map(|r| r.recall_5).sum::<f64>() / n;
    let avg_r10 = results.iter().map(|r| r.recall_10).sum::<f64>() / n;
    let avg_r15 = results.iter().map(|r| r.recall_15).sum::<f64>() / n;
    let avg_r20 = results.iter().map(|r| r.recall_20).sum::<f64>() / n;

    // Gold rank distribution analysis
    let mut rank_buckets = HashMap::new(); // "1-5", "6-10", "11-15", "16-20", "21+", "not_found"
    for r in &results {
        let bucket = match r.gold_rank {
            Some(rank) if rank <= 5 => "1-5",
            Some(rank) if rank <= 10 => "6-10",
            Some(rank) if rank <= 15 => "11-15",
            Some(rank) if rank <= 20 => "16-20",
            Some(_) => "21+",
            None => "not_found",
        };
        *rank_buckets.entry(bucket).or_insert(0usize) += 1;
    }
    let total_with_gold = results.iter().filter(|r| r.gold_rank.is_some()).count();
    let total_missing = results.len() - total_with_gold;

    // Latency
    let mut latencies: Vec<f64> = results.iter().map(|r| r.latency_ms).collect();
    latencies.sort_by(|a, b| a.partial_cmp(b).unwrap());
    embed_latencies.sort_by(|a, b| a.partial_cmp(b).unwrap());
    search_latencies.sort_by(|a, b| a.partial_cmp(b).unwrap());

    let lat_p50 = percentile(&latencies, 50.0);
    let lat_p95 = percentile(&latencies, 95.0);
    let emb_p50 = percentile(&embed_latencies, 50.0);
    let emb_p95 = percentile(&embed_latencies, 95.0);
    let srch_p50 = percentile(&search_latencies, 50.0);
    let srch_p95 = percentile(&search_latencies, 95.0);

    // Breakdowns
    let topic_breakdown = breakdown_by(&results, |r| r.topic.clone());
    let freshness_breakdown = breakdown_by(&results, |r| r.freshness.clone());
    let search_type_breakdown = breakdown_by(&results, |r| r.search_results.clone());

    match output_format {
        "json" => {
            let output = serde_json::json!({
                "dataset": "longseal",
                "pipeline": pipeline_name,
                "doc_set_size": docs_size,
                "semantic_weight": sw,
                "questions_evaluated": results.len(),
                "metrics": {
                    "mrr_10": avg_mrr,
                    "recall_1": avg_r1,
                    "recall_3": avg_r3,
                    "recall_5": avg_r5,
                    "recall_10": avg_r10,
                    "recall_15": avg_r15,
                    "recall_20": avg_r20,
                },
                "gold_rank_distribution": {
                    "rank_1_5": rank_buckets.get("1-5").copied().unwrap_or(0),
                    "rank_6_10": rank_buckets.get("6-10").copied().unwrap_or(0),
                    "rank_11_15": rank_buckets.get("11-15").copied().unwrap_or(0),
                    "rank_16_20": rank_buckets.get("16-20").copied().unwrap_or(0),
                    "rank_21_plus": rank_buckets.get("21+").copied().unwrap_or(0),
                    "not_found": total_missing,
                },
                "breakdown_by_topic": topic_breakdown,
                "breakdown_by_freshness": freshness_breakdown,
                "breakdown_by_search_type": search_type_breakdown,
                "latency": {
                    "total_p50_ms": lat_p50,
                    "total_p95_ms": lat_p95,
                    "embed_p50_ms": emb_p50,
                    "embed_p95_ms": emb_p95,
                    "search_p50_ms": srch_p50,
                    "search_p95_ms": srch_p95,
                },
            });
            println!("{}", serde_json::to_string_pretty(&output).unwrap());
        }
        _ => {
            // Overall metrics
            println!(
                "{:<16} {:>10}",
                "Metric", "Score"
            );
            println!("{}", "-".repeat(28));
            println!("{:<16} {:>10.4}", "MRR@10", avg_mrr);
            println!("{:<16} {:>10.4}", "Recall@1", avg_r1);
            println!("{:<16} {:>10.4}", "Recall@3", avg_r3);
            println!("{:<16} {:>10.4}", "Recall@5", avg_r5);
            println!("{:<16} {:>10.4}", "Recall@10", avg_r10);
            println!("{:<16} {:>10.4}", "Recall@15", avg_r15);
            println!("{:<16} {:>10.4}", "Recall@20", avg_r20);

            // Gold rank distribution
            println!("\n--- Gold Doc Rank Distribution ---");
            println!("{:<16} {:>6} {:>8}", "Rank Bucket", "Count", "Pct");
            println!("{}", "-".repeat(32));
            for bucket in &["1-5", "6-10", "11-15", "16-20", "21+", "not_found"] {
                let count = rank_buckets.get(bucket).copied().unwrap_or(0);
                let pct = count as f64 / results.len() as f64 * 100.0;
                println!("{:<16} {:>6} {:>7.1}%", bucket, count, pct);
            }
            println!("{:<16} {:>6} {:>7.1}%", "TOTAL found", total_with_gold,
                total_with_gold as f64 / results.len() as f64 * 100.0);
            println!("{:<16} {:>6} {:>7.1}%", "TOTAL missing", total_missing,
                total_missing as f64 / results.len() as f64 * 100.0);

            // Topic breakdown
            println!("\n--- By Topic ---");
            print_breakdown_table(&topic_breakdown);

            // Freshness breakdown
            println!("\n--- By Freshness ---");
            print_breakdown_table(&freshness_breakdown);

            // Search results type breakdown
            println!("\n--- By Search Results Type ---");
            print_breakdown_table(&search_type_breakdown);

            // Latency
            println!(
                "\nLatency (end-to-end): p50={:.1}ms  p95={:.1}ms",
                lat_p50, lat_p95
            );
            println!(
                "  Embed:  p50={:.1}ms  p95={:.1}ms",
                emb_p50, emb_p95
            );
            println!(
                "  Search: p50={:.1}ms  p95={:.1}ms\n",
                srch_p50, srch_p95
            );
        }
    }
}

fn breakdown_by<F>(results: &[QuestionResult], key_fn: F) -> serde_json::Value
where
    F: Fn(&QuestionResult) -> String,
{
    let mut groups: HashMap<String, Vec<&QuestionResult>> = HashMap::new();
    for r in results {
        groups.entry(key_fn(r)).or_default().push(r);
    }

    let mut entries: Vec<serde_json::Value> = groups
        .iter()
        .map(|(k, items)| {
            let n = items.len() as f64;
            serde_json::json!({
                "key": k,
                "count": items.len(),
                "mrr_10": items.iter().map(|r| r.mrr_10).sum::<f64>() / n,
                "recall_1": items.iter().map(|r| r.recall_1).sum::<f64>() / n,
                "recall_3": items.iter().map(|r| r.recall_3).sum::<f64>() / n,
                "recall_5": items.iter().map(|r| r.recall_5).sum::<f64>() / n,
                "recall_10": items.iter().map(|r| r.recall_10).sum::<f64>() / n,
                "recall_15": items.iter().map(|r| r.recall_15).sum::<f64>() / n,
                "recall_20": items.iter().map(|r| r.recall_20).sum::<f64>() / n,
            })
        })
        .collect();
    entries.sort_by(|a, b| {
        a["key"]
            .as_str()
            .unwrap_or("")
            .cmp(b["key"].as_str().unwrap_or(""))
    });
    serde_json::Value::Array(entries)
}

fn print_breakdown_table(breakdown: &serde_json::Value) {
    println!(
        "{:<25} {:>5} {:>8} {:>8} {:>8} {:>8} {:>8} {:>8}",
        "Category", "N", "R@5", "R@10", "R@15", "R@20", "MRR@10", "R@1"
    );
    println!("{}", "-".repeat(84));
    if let serde_json::Value::Array(items) = breakdown {
        for item in items {
            println!(
                "{:<25} {:>5} {:>8.4} {:>8.4} {:>8.4} {:>8.4} {:>8.4} {:>8.4}",
                item["key"].as_str().unwrap_or("?"),
                item["count"].as_u64().unwrap_or(0),
                item["recall_5"].as_f64().unwrap_or(0.0),
                item["recall_10"].as_f64().unwrap_or(0.0),
                item["recall_15"].as_f64().unwrap_or(0.0),
                item["recall_20"].as_f64().unwrap_or(0.0),
                item["mrr_10"].as_f64().unwrap_or(0.0),
                item["recall_1"].as_f64().unwrap_or(0.0),
            );
        }
    }
}
