use oi_embed::batch::embed_in_batches;
use oi_index::hybrid::HybridConfig;
use oi_index::HybridIndex;
use oi_types::traits::SearchIndex;
use oi_types::{ChunkMeta, ContentChunk, EmbeddingPipeline, QueryFilters};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use crate::metrics;

// ── Constants ──

const BASE_URL: &str = "https://msmarco.z22.web.core.windows.net/msmarcoranking";
const COLLECTION_TAR: &str = "collection.tar.gz";
const QUERIES_TAR: &str = "queries.tar.gz";
pub(crate) const QRELS_FILE: &str = "qrels.dev.small.tsv";
pub(crate) const QUERIES_TSV: &str = "queries.dev.tsv";

// ── Cache directory ──

fn cache_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("OI_MSMARCO_DIR") {
        return PathBuf::from(dir);
    }
    let home = std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."));
    home.join(".cache").join("opensonarx").join("msmarco")
}

// ── Download helpers ──

fn download_file(url: &str, dest: &Path) -> Result<(), String> {
    if dest.exists() {
        tracing::info!("Already cached: {}", dest.display());
        return Ok(());
    }

    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir: {e}"))?;
    }

    tracing::info!("Downloading {} ...", url);
    let resp = ureq::get(url)
        .call()
        .map_err(|e| format!("download failed: {e}"))?;

    let bytes = resp
        .into_body()
        .with_config()
        .limit(2 * 1024 * 1024 * 1024) // 2GB limit for collection.tar.gz
        .read_to_vec()
        .map_err(|e| format!("read body: {e}"))?;

    tracing::info!("Downloaded {} bytes -> {}", bytes.len(), dest.display());
    std::fs::write(dest, &bytes).map_err(|e| format!("write file: {e}"))?;
    Ok(())
}

fn extract_tar_gz(tar_gz_path: &Path, target_name: &str, dest: &Path) -> Result<(), String> {
    tracing::info!("Extracting {} from {} ...", target_name, tar_gz_path.display());
    let file = std::fs::File::open(tar_gz_path).map_err(|e| format!("open tar.gz: {e}"))?;
    let decoder = flate2::read::GzDecoder::new(file);
    let mut archive = tar::Archive::new(decoder);

    for entry in archive.entries().map_err(|e| format!("tar entries: {e}"))? {
        let mut entry = entry.map_err(|e| format!("tar entry: {e}"))?;
        let path = entry
            .path()
            .map_err(|e| format!("tar path: {e}"))?
            .into_owned();

        if path.file_name().and_then(|f| f.to_str()) == Some(target_name) {
            let mut out =
                std::fs::File::create(dest).map_err(|e| format!("create {target_name}: {e}"))?;
            std::io::copy(&mut entry, &mut out)
                .map_err(|e| format!("extract {target_name}: {e}"))?;
            tracing::info!("Extracted {}", target_name);
            return Ok(());
        }
    }

    Err(format!("{target_name} not found in archive"))
}

pub(crate) fn ensure_msmarco() -> Result<PathBuf, String> {
    let dir = cache_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("mkdir: {e}"))?;

    // Download and extract collection.tar.gz -> collection.tsv
    let collection_tsv = dir.join("collection.tsv");
    if !collection_tsv.exists() {
        let tar_gz_path = dir.join(COLLECTION_TAR);
        download_file(&format!("{BASE_URL}/{COLLECTION_TAR}"), &tar_gz_path)?;
        extract_tar_gz(&tar_gz_path, "collection.tsv", &collection_tsv)?;
    }

    // Download and extract queries.tar.gz -> queries.dev.tsv
    let queries_tsv = dir.join(QUERIES_TSV);
    if !queries_tsv.exists() {
        let tar_gz_path = dir.join(QUERIES_TAR);
        download_file(&format!("{BASE_URL}/{QUERIES_TAR}"), &tar_gz_path)?;
        extract_tar_gz(&tar_gz_path, QUERIES_TSV, &queries_tsv)?;
    }

    // Download qrels (small direct file)
    download_file(
        &format!("{BASE_URL}/{QRELS_FILE}"),
        &dir.join(QRELS_FILE),
    )?;

    Ok(dir)
}

// ── TSV parsing ──

/// Parse collection.tsv: `pid\tpassage_text` (no header)
pub(crate) fn parse_collection(path: &Path, limit: Option<usize>) -> Result<Vec<(String, String)>, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("open collection: {e}"))?;
    let reader = BufReader::with_capacity(1024 * 1024, file);
    let mut docs = Vec::new();
    let max = limit.unwrap_or(usize::MAX);

    for (i, line) in reader.lines().enumerate() {
        if docs.len() >= max {
            break;
        }
        let line = line.map_err(|e| format!("read line {i}: {e}"))?;
        if line.is_empty() {
            continue;
        }
        if let Some((pid, text)) = line.split_once('\t') {
            docs.push((pid.to_string(), text.to_string()));
        }
    }

    Ok(docs)
}

/// Parse queries TSV: `qid\tquery_text` (no header)
pub(crate) fn parse_queries(path: &Path) -> Result<Vec<(String, String)>, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("open queries: {e}"))?;
    let reader = BufReader::new(file);
    let mut queries = Vec::new();

    for (i, line) in reader.lines().enumerate() {
        let line = line.map_err(|e| format!("read line {i}: {e}"))?;
        if line.is_empty() {
            continue;
        }
        if let Some((qid, text)) = line.split_once('\t') {
            queries.push((qid.to_string(), text.to_string()));
        }
    }

    Ok(queries)
}

/// Parse qrels TSV: `qid\t0\tpid\t1` (TREC format, use cols 0 and 2)
pub(crate) fn parse_qrels(path: &Path) -> Result<HashMap<String, Vec<String>>, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("open qrels: {e}"))?;
    let reader = BufReader::new(file);
    let mut qrels: HashMap<String, Vec<String>> = HashMap::new();

    for (i, line) in reader.lines().enumerate() {
        let line = line.map_err(|e| format!("read line {i}: {e}"))?;
        let line = line.trim().to_string();
        if line.is_empty() {
            continue;
        }
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() < 3 {
            continue;
        }
        let qid = parts[0].to_string();
        let pid = parts[2].to_string();
        qrels.entry(qid).or_default().push(pid);
    }

    Ok(qrels)
}

// ── Embedding cache ──
//
// Single cache file per pipeline (not per limit). Resumable: if you embed
// 4M passages and crash, the next run picks up from 4M.
//
// Binary format: [u64 count] [u32 dim] [count * dim * f32 values]
// Sidecar file: `_ids.txt` with one passage ID per line.
// Checkpoint every CHECKPOINT_INTERVAL docs.

const CHECKPOINT_INTERVAL: usize = 50_000;

fn embedding_cache_path(pipeline: &str) -> (PathBuf, PathBuf) {
    let dir = cache_dir();
    (
        dir.join(format!("embeddings_{pipeline}.bin")),
        dir.join(format!("embeddings_{pipeline}_ids.txt")),
    )
}

fn save_embeddings(
    bin_path: &Path,
    ids_path: &Path,
    ids: &[String],
    embeddings: &[Vec<f32>],
    dim: usize,
) -> Result<(), String> {
    let count = embeddings.len() as u64;

    let file = std::fs::File::create(bin_path).map_err(|e| format!("create cache: {e}"))?;
    let mut w = BufWriter::with_capacity(1024 * 1024, file);
    w.write_all(&count.to_le_bytes())
        .map_err(|e| format!("write count: {e}"))?;
    w.write_all(&(dim as u32).to_le_bytes())
        .map_err(|e| format!("write dim: {e}"))?;
    for emb in embeddings {
        for &val in emb {
            w.write_all(&val.to_le_bytes())
                .map_err(|e| format!("write f32: {e}"))?;
        }
    }
    w.flush().map_err(|e| format!("flush: {e}"))?;

    // Write IDs sidecar
    let ids_content = ids.join("\n");
    std::fs::write(ids_path, ids_content).map_err(|e| format!("write ids: {e}"))?;

    Ok(())
}

fn load_embeddings(
    bin_path: &Path,
    ids_path: &Path,
) -> Result<(Vec<String>, Vec<Vec<f32>>, usize), String> {
    let file = std::fs::File::open(bin_path).map_err(|e| format!("open cache: {e}"))?;
    let mut r = BufReader::with_capacity(1024 * 1024, file);

    let mut buf8 = [0u8; 8];
    let mut buf4 = [0u8; 4];

    r.read_exact(&mut buf8)
        .map_err(|e| format!("read count: {e}"))?;
    let count = u64::from_le_bytes(buf8) as usize;

    r.read_exact(&mut buf4)
        .map_err(|e| format!("read dim: {e}"))?;
    let dim = u32::from_le_bytes(buf4) as usize;

    let mut embeddings = Vec::with_capacity(count);
    for _ in 0..count {
        let mut emb = Vec::with_capacity(dim);
        for _ in 0..dim {
            r.read_exact(&mut buf4)
                .map_err(|e| format!("read f32: {e}"))?;
            emb.push(f32::from_le_bytes(buf4));
        }
        embeddings.push(emb);
    }

    let ids_content =
        std::fs::read_to_string(ids_path).map_err(|e| format!("read ids: {e}"))?;
    let ids: Vec<String> = ids_content.lines().map(|s| s.to_string()).collect();

    if ids.len() != count {
        return Err(format!(
            "ID count mismatch: {} ids vs {} embeddings",
            ids.len(),
            count
        ));
    }

    Ok((ids, embeddings, dim))
}

// ── Parallel embedding (for a single chunk, used within checkpointed loop) ──

fn embed_parallel(
    pipeline: &Arc<dyn EmbeddingPipeline>,
    texts: &[String],
    global_offset: usize,
    global_total: usize,
    global_start: &Instant,
) -> Result<Vec<Vec<f32>>, String> {
    let n_cores = std::thread::available_parallelism()
        .map(|p| p.get())
        .unwrap_or(4);
    let n_workers = (n_cores / 4).max(1);
    let total = texts.len();

    if total == 0 {
        return Ok(Vec::new());
    }

    let chunk_size = (total + n_workers - 1) / n_workers;

    let results: Vec<Result<Vec<Vec<f32>>, String>> = std::thread::scope(|s| {
        let handles: Vec<_> = texts
            .chunks(chunk_size)
            .enumerate()
            .map(|(worker_id, slice)| {
                let pipeline = Arc::clone(pipeline);
                let global_start = global_start.clone();
                s.spawn(move || {
                    let refs: Vec<&str> = slice.iter().map(|s| s.as_str()).collect();
                    let mut worker_results = Vec::with_capacity(slice.len());
                    let batch_size = 32;

                    for (batch_idx, batch) in refs.chunks(batch_size).enumerate() {
                        let batch_embs = embed_in_batches(pipeline.as_ref(), batch, batch_size)
                            .map_err(|e| format!("worker {worker_id} batch {batch_idx}: {e}"))?;
                        worker_results.extend(batch_embs);

                        let local_done = (batch_idx + 1) * batch_size;
                        if local_done % 10_000 < batch_size && worker_id == 0 {
                            let done_global = global_offset + local_done.min(slice.len()) * n_workers;
                            let elapsed = global_start.elapsed().as_secs_f64();
                            let rate = (done_global - global_offset) as f64 / elapsed.max(0.1);
                            let remaining = (global_total - done_global) as f64 / rate;
                            let hrs = remaining / 3600.0;
                            if hrs >= 1.0 {
                                eprintln!(
                                    "[progress] {}/{} ({:.0} docs/sec, ETA {:.1}h)",
                                    done_global, global_total, rate, hrs
                                );
                            } else {
                                eprintln!(
                                    "[progress] {}/{} ({:.0} docs/sec, ETA {:.0}s)",
                                    done_global, global_total, rate, remaining
                                );
                            }
                        }
                    }

                    Ok(worker_results)
                })
            })
            .collect();

        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });

    let mut all = Vec::with_capacity(total);
    for result in results {
        all.extend(result?);
    }
    Ok(all)
}

// ── Resumable embedding with checkpoints ──

/// Embed corpus texts with periodic checkpointing. Resumes from existing cache.
/// Returns all embeddings for the requested corpus (first `corpus.len()` entries).
pub(crate) fn embed_with_checkpoints(
    pipeline: &Arc<dyn EmbeddingPipeline>,
    corpus: &[(String, String)],
    pipeline_name: &str,
    dim: usize,
) -> Vec<Vec<f32>> {
    let (bin_path, ids_path) = embedding_cache_path(pipeline_name);
    let total = corpus.len();

    // Try to load existing checkpoint
    let (mut cached_ids, mut cached_embs) = if bin_path.exists() && ids_path.exists() {
        match load_embeddings(&bin_path, &ids_path) {
            Ok((ids, embs, cached_dim)) => {
                if cached_dim != dim {
                    eprintln!(
                        "Cache dim mismatch ({} vs {}), starting fresh",
                        cached_dim, dim
                    );
                    (vec![], vec![])
                } else {
                    // Verify cached IDs are a prefix of our corpus
                    let prefix_ok = ids
                        .iter()
                        .zip(corpus.iter())
                        .all(|(cached_id, (pid, _))| cached_id == pid);
                    if prefix_ok {
                        (ids, embs)
                    } else {
                        eprintln!("Cache IDs don't match corpus order, starting fresh");
                        (vec![], vec![])
                    }
                }
            }
            Err(e) => {
                eprintln!("Cache load failed: {e}, starting fresh");
                (vec![], vec![])
            }
        }
    } else {
        (vec![], vec![])
    };

    let start_idx = cached_embs.len();

    if start_idx >= total {
        // Cache already has enough (or more). Truncate to exact size needed.
        tracing::info!(
            "Loaded all {} embeddings from cache (cache has {})",
            total,
            start_idx
        );
        cached_embs.truncate(total);
        return cached_embs;
    }

    let remaining = total - start_idx;
    let n_cores = std::thread::available_parallelism()
        .map(|p| p.get())
        .unwrap_or(4);
    let n_workers = (n_cores / 4).max(1);

    if start_idx > 0 {
        eprintln!(
            "Resuming from checkpoint: {}/{} already cached ({} remaining)",
            start_idx, total, remaining
        );
    }
    eprintln!(
        "Embedding {} docs with {} workers, checkpoint every {}",
        remaining, n_workers, CHECKPOINT_INTERVAL
    );

    let global_start = Instant::now();

    // Process in checkpoint-sized windows
    for window_start in (start_idx..total).step_by(CHECKPOINT_INTERVAL) {
        let window_end = (window_start + CHECKPOINT_INTERVAL).min(total);
        let window_texts: Vec<String> = corpus[window_start..window_end]
            .iter()
            .map(|(_, text)| text.clone())
            .collect();

        let window_embs =
            embed_parallel(pipeline, &window_texts, window_start, total, &global_start)
                .unwrap_or_else(|e| {
                    eprintln!("Embedding failed at offset {window_start}: {e}");
                    std::process::exit(1);
                });

        // Extend cached data
        for i in window_start..window_end {
            cached_ids.push(corpus[i].0.clone());
        }
        cached_embs.extend(window_embs);

        // Save checkpoint
        if let Err(e) = save_embeddings(&bin_path, &ids_path, &cached_ids, &cached_embs, dim) {
            eprintln!("Warning: checkpoint save failed: {e}");
        } else {
            let elapsed = global_start.elapsed().as_secs_f64();
            let done = cached_embs.len();
            let rate = (done - start_idx) as f64 / elapsed;
            eprintln!(
                "[checkpoint] Saved {}/{} embeddings ({:.0} docs/sec)",
                done, total, rate
            );
        }
    }

    let elapsed = global_start.elapsed().as_secs_f64();
    tracing::info!(
        "Embedded {} new docs in {:.1}s ({:.0} docs/sec), total cached: {}",
        remaining,
        elapsed,
        remaining as f64 / elapsed,
        cached_embs.len()
    );

    cached_embs
}

// ── Evaluation ──

pub fn run_msmarco(
    pipeline_name: &str,
    output_format: &str,
    limit: Option<usize>,
    semantic_weight: Option<f64>,
    rerank: bool,
) {
    let embedder = crate::get_embedder(pipeline_name);
    let dim = embedder.dimension();

    // 1. Download/extract
    let dir = match ensure_msmarco() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Failed to get MS MARCO data: {e}");
            std::process::exit(1);
        }
    };

    // 2. Parse
    let corpus = parse_collection(&dir.join("collection.tsv"), limit).unwrap_or_else(|e| {
        eprintln!("Failed to parse collection: {e}");
        std::process::exit(1);
    });
    let queries = parse_queries(&dir.join(QUERIES_TSV)).unwrap_or_else(|e| {
        eprintln!("Failed to parse queries: {e}");
        std::process::exit(1);
    });
    let qrels = parse_qrels(&dir.join(QRELS_FILE)).unwrap_or_else(|e| {
        eprintln!("Failed to parse qrels: {e}");
        std::process::exit(1);
    });

    // Memory warning for full corpus
    if limit.is_none() {
        let mem_gb = (corpus.len() as f64 * dim as f64 * 4.0) / (1024.0 * 1024.0 * 1024.0);
        eprintln!(
            "WARNING: Full corpus ({} passages) will use ~{:.1}GB for embeddings alone. \
             Use --limit for a smaller test.",
            corpus.len(),
            mem_gb
        );
    }

    let sw = semantic_weight.unwrap_or_else(|| HybridConfig::default().semantic_weight);

    let corpus_pids: Vec<String> = corpus.iter().map(|(pid, _)| pid.clone()).collect();
    let pid_set: std::collections::HashSet<&str> =
        corpus_pids.iter().map(|s| s.as_str()).collect();

    // Only evaluate queries whose relevant passages are in our (possibly limited) corpus
    let eval_query_ids: Vec<&str> = qrels
        .keys()
        .filter(|qid| {
            qrels[*qid].iter().any(|pid| pid_set.contains(pid.as_str()))
        })
        .map(|s| s.as_str())
        .collect();

    let query_map: HashMap<&str, &str> = queries
        .iter()
        .map(|(qid, text)| (qid.as_str(), text.as_str()))
        .collect();

    println!("\n=== MS MARCO Passage Ranking ===");
    println!(
        "Pipeline: {} ({}-dim) | Weights: semantic={:.2} bm25={:.2}",
        embedder.name(),
        dim,
        sw,
        1.0 - sw
    );
    println!(
        "Corpus: {} passages | Queries: {} (with relevant passages in corpus)\n",
        corpus.len(),
        eval_query_ids.len()
    );

    // 3. Load or compute embeddings (resumable with checkpoints)
    let embeddings = embed_with_checkpoints(&embedder, &corpus, pipeline_name, dim);

    // 4. Build index
    let ingest_start = Instant::now();
    let config = HybridConfig {
        semantic_weight: sw,
        bm25_weight: 1.0 - sw,
        skip_dedup: true,
        // Single-domain benchmark corpus: the SERP diversity cap would
        // truncate every query to ~top-10 (Recall@1000 == Recall@10).
        max_per_domain: 0,
        ..Default::default()
    };
    let index = HybridIndex::with_config(dim, config);

    // Attach reranker if requested
    #[cfg(feature = "reranker")]
    let index = if rerank {
        tracing::info!("Loading cross-encoder reranker...");
        let reranker = oi_embed::OnnxReranker::new().unwrap_or_else(|e| {
            eprintln!("Failed to load reranker: {e}");
            std::process::exit(1);
        });
        index.with_reranker(std::sync::Arc::new(reranker))
    } else {
        index
    };
    #[cfg(not(feature = "reranker"))]
    if rerank {
        eprintln!("Reranking requires --features reranker");
        std::process::exit(1);
    }

    let chunks: Vec<ContentChunk> = corpus
        .iter()
        .zip(embeddings.into_iter())
        .map(|((pid, text), emb)| ContentChunk {
            chunk_id: pid.clone(),
            content: text.clone(),
            embedding: Some(emb),
            seq: 0,
            meta: Some(ChunkMeta {
                url: format!("marco://{pid}"),
                title: String::new(),
                domain: "msmarco".to_string(),
                content_hash: pid.clone(),
                crawled_at: None,
                content_type: None,
                entity: None,
                content_kind: None,
                ..Default::default()
            }),
        })
        .collect();

    index.ingest(&chunks).unwrap_or_else(|e| {
        eprintln!("Ingest failed: {e}");
        std::process::exit(1);
    });
    let ingest_secs = ingest_start.elapsed().as_secs_f64();
    tracing::info!(
        "Indexed {} passages in {:.1}s ({:.0} docs/sec)",
        corpus.len(),
        ingest_secs,
        corpus.len() as f64 / ingest_secs
    );

    // 5. Evaluate
    let eval_start = Instant::now();
    let mut mrr_10_sum = 0.0;
    let mut recall_10_sum = 0.0;
    let mut recall_100_sum = 0.0;
    let mut recall_1000_sum = 0.0;
    let mut query_count = 0usize;
    let mut query_latencies = Vec::new();

    for (i, qid) in eval_query_ids.iter().enumerate() {
        let query_text = match query_map.get(qid) {
            Some(t) => *t,
            None => continue,
        };

        let start = Instant::now();
        let query_emb = match embedder.embed_query(query_text) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let results = match index.query(&query_emb, query_text, &QueryFilters::default(), 1000) {
            Ok(r) => r,
            Err(_) => continue,
        };
        let latency_ms = start.elapsed().as_secs_f64() * 1000.0;
        query_latencies.push(latency_ms);

        let retrieved_pids: Vec<&str> = results
            .iter()
            .map(|r| r.url.strip_prefix("marco://").unwrap_or(&r.url))
            .collect();

        let relevant_pids: Vec<&str> = qrels
            .get(*qid)
            .map(|pids| {
                pids.iter()
                    .filter(|pid| pid_set.contains(pid.as_str()))
                    .map(|s| s.as_str())
                    .collect()
            })
            .unwrap_or_default();

        if relevant_pids.is_empty() {
            continue;
        }

        // MRR@10
        let top_10: Vec<&str> = retrieved_pids.iter().take(10).copied().collect();
        mrr_10_sum += metrics::mrr(&relevant_pids, &top_10);

        // Recall@K
        recall_10_sum += metrics::recall_at_k(&relevant_pids, &retrieved_pids, 10);
        recall_100_sum += metrics::recall_at_k(&relevant_pids, &retrieved_pids, 100);
        recall_1000_sum += metrics::recall_at_k(&relevant_pids, &retrieved_pids, 1000);

        query_count += 1;

        if (i + 1) % 500 == 0 {
            let elapsed = eval_start.elapsed().as_secs_f64();
            let rate = (i + 1) as f64 / elapsed;
            eprintln!(
                "Evaluated {}/{} queries ({:.0} q/sec, running MRR@10={:.4})",
                i + 1,
                eval_query_ids.len(),
                rate,
                mrr_10_sum / query_count as f64
            );
        }
    }

    let n = query_count.max(1) as f64;
    let mrr_10 = mrr_10_sum / n;
    let recall_10 = recall_10_sum / n;
    let recall_100 = recall_100_sum / n;
    let recall_1000 = recall_1000_sum / n;

    query_latencies.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let p50 = percentile(&query_latencies, 50.0);
    let p95 = percentile(&query_latencies, 95.0);

    match output_format {
        "json" => {
            let output = serde_json::json!({
                "dataset": "msmarco-passage",
                "pipeline": pipeline_name,
                "semantic_weight": sw,
                "corpus_size": corpus.len(),
                "queries_evaluated": query_count,
                "metrics": {
                    "mrr_10": mrr_10,
                    "recall_10": recall_10,
                    "recall_100": recall_100,
                    "recall_1000": recall_1000,
                },
                "ingest_secs": ingest_secs,
                "query_latency_p50_ms": p50,
                "query_latency_p95_ms": p95,
            });
            println!("{}", serde_json::to_string_pretty(&output).unwrap());
        }
        _ => {
            println!(
                "{:<16} {:<10} {:<10}",
                "Metric", "Ours", "Published"
            );
            println!(
                "{:<16} {:<10} {:<10}",
                "------", "----", "---------"
            );
            println!("{:<16} {:<10.4} {:<10}", "MRR@10", mrr_10, "");
            println!("{:<16} {:<10.4} {:<10}", "Recall@10", recall_10, "-");
            println!("{:<16} {:<10.4} {:<10}", "Recall@100", recall_100, "-");
            println!("{:<16} {:<10.4} {:<10}", "Recall@1000", recall_1000, "-");

            println!("\n--- Published Baselines (MRR@10) ---");
            println!("{:<20} {:>8} {:>10}", "Model", "Params", "MRR@10");
            println!("{}", "-".repeat(40));
            println!("{:<20} {:>8} {:>10}", "BM25 (Anserini)", "-", "0.187");
            println!("{:<20} {:>8} {:>10}", "ANCE", "110M", "0.330");
            println!("{:<20} {:>8} {:>10}", "TAS-B", "66M", "0.344");
            println!("{:<20} {:>8} {:>10}", "ColBERTv2", "110M", "0.397");

            println!(
                "\nIngest: {:.1}s | Eval: {:.1}s | Query: p50={:.1}ms p95={:.1}ms\n",
                ingest_secs,
                eval_start.elapsed().as_secs_f64(),
                p50,
                p95
            );
        }
    }
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
    fn test_parse_collection_line() {
        let dir = std::env::temp_dir().join("oi_msmarco_test_collection");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("collection.tsv");
        std::fs::write(&path, "42\tsome passage text\n99\tanother passage\n").unwrap();

        let docs = parse_collection(&path, None).unwrap();
        assert_eq!(docs.len(), 2);
        assert_eq!(docs[0], ("42".to_string(), "some passage text".to_string()));
        assert_eq!(docs[1], ("99".to_string(), "another passage".to_string()));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_parse_collection_with_limit() {
        let dir = std::env::temp_dir().join("oi_msmarco_test_collection_limit");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("collection.tsv");
        std::fs::write(&path, "1\ta\n2\tb\n3\tc\n4\td\n5\te\n").unwrap();

        let docs = parse_collection(&path, Some(3)).unwrap();
        assert_eq!(docs.len(), 3);
        assert_eq!(docs[0].0, "1");
        assert_eq!(docs[2].0, "3");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_parse_queries_line() {
        let dir = std::env::temp_dir().join("oi_msmarco_test_queries");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("queries.tsv");
        std::fs::write(&path, "101\tdefine extreme\n202\twhat is a cat\n").unwrap();

        let queries = parse_queries(&path).unwrap();
        assert_eq!(queries.len(), 2);
        assert_eq!(queries[0], ("101".to_string(), "define extreme".to_string()));
        assert_eq!(
            queries[1],
            ("202".to_string(), "what is a cat".to_string())
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_parse_qrels_trec() {
        let dir = std::env::temp_dir().join("oi_msmarco_test_qrels");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("qrels.tsv");
        std::fs::write(&path, "101\t0\t5000\t1\n101\t0\t6000\t1\n202\t0\t7000\t1\n").unwrap();

        let qrels = parse_qrels(&path).unwrap();
        assert_eq!(qrels.len(), 2);
        assert_eq!(qrels["101"], vec!["5000".to_string(), "6000".to_string()]);
        assert_eq!(qrels["202"], vec!["7000".to_string()]);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_embedding_cache_roundtrip() {
        let dir = std::env::temp_dir().join("oi_msmarco_test_cache");
        std::fs::create_dir_all(&dir).unwrap();
        let bin_path = dir.join("test.bin");
        let ids_path = dir.join("test_ids.txt");

        let ids = vec!["p1".to_string(), "p2".to_string(), "p3".to_string()];
        let embeddings = vec![
            vec![1.0f32, 2.0, 3.0],
            vec![4.0, 5.0, 6.0],
            vec![7.0, 8.0, 9.0],
        ];

        save_embeddings(&bin_path, &ids_path, &ids, &embeddings, 3).unwrap();
        let (loaded_ids, loaded_embs, loaded_dim) = load_embeddings(&bin_path, &ids_path).unwrap();
        assert_eq!(loaded_dim, 3);

        assert_eq!(loaded_ids, ids);
        assert_eq!(loaded_embs, embeddings);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_embedding_cache_resume() {
        let dir = std::env::temp_dir().join("oi_msmarco_test_resume");
        std::fs::create_dir_all(&dir).unwrap();
        let bin_path = dir.join("test.bin");
        let ids_path = dir.join("test_ids.txt");

        // Save partial cache (2 of 4 docs)
        let ids_partial = vec!["p1".to_string(), "p2".to_string()];
        let embs_partial = vec![vec![1.0f32, 2.0], vec![3.0, 4.0]];
        save_embeddings(&bin_path, &ids_path, &ids_partial, &embs_partial, 2).unwrap();

        // Load and verify it's a valid prefix
        let (loaded_ids, loaded_embs, dim) = load_embeddings(&bin_path, &ids_path).unwrap();
        assert_eq!(loaded_ids.len(), 2);
        assert_eq!(loaded_embs.len(), 2);
        assert_eq!(dim, 2);

        // Simulate resume: extend and save full
        let ids_full = vec![
            "p1".to_string(),
            "p2".to_string(),
            "p3".to_string(),
            "p4".to_string(),
        ];
        let embs_full = vec![
            vec![1.0f32, 2.0],
            vec![3.0, 4.0],
            vec![5.0, 6.0],
            vec![7.0, 8.0],
        ];
        save_embeddings(&bin_path, &ids_path, &ids_full, &embs_full, 2).unwrap();

        let (final_ids, final_embs, _) = load_embeddings(&bin_path, &ids_path).unwrap();
        assert_eq!(final_ids.len(), 4);
        assert_eq!(final_embs[2], vec![5.0, 6.0]);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
