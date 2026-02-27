use oi_index::hybrid::HybridConfig;
use oi_index::HybridIndex;
use oi_types::traits::SearchIndex;
use oi_types::{ChunkMeta, ContentChunk, QueryFilters};
use std::collections::HashMap;
use std::time::Instant;

use crate::metrics;
use crate::msmarco;

/// Run a distributed search simulation across N virtual nodes.
///
/// Loads the MS MARCO corpus, splits it across virtual nodes (uniform or Zipf),
/// queries with fanout, merges results, and compares MRR@10 against a single-node
/// baseline.
pub fn run_dist_sim(
    num_nodes: usize,
    fanout: usize,
    distribution: &str,
    pipeline_name: &str,
    limit: Option<usize>,
    semantic_weight: Option<f64>,
) {
    let embedder = crate::get_embedder(pipeline_name);
    let dim = embedder.dimension();
    let sw = semantic_weight.unwrap_or(1.0);

    // 1. Download/extract MS MARCO data
    let dir = match msmarco::ensure_msmarco() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Failed to get MS MARCO data: {e}");
            std::process::exit(1);
        }
    };

    let corpus = msmarco::parse_collection(&dir.join("collection.tsv"), limit).unwrap_or_else(|e| {
        eprintln!("Failed to parse collection: {e}");
        std::process::exit(1);
    });
    let queries = msmarco::parse_queries(&dir.join(msmarco::QUERIES_TSV)).unwrap_or_else(|e| {
        eprintln!("Failed to parse queries: {e}");
        std::process::exit(1);
    });
    let qrels = msmarco::parse_qrels(&dir.join(msmarco::QRELS_FILE)).unwrap_or_else(|e| {
        eprintln!("Failed to parse qrels: {e}");
        std::process::exit(1);
    });

    let corpus_pids: Vec<String> = corpus.iter().map(|(pid, _)| pid.clone()).collect();
    let pid_set: std::collections::HashSet<&str> =
        corpus_pids.iter().map(|s| s.as_str()).collect();

    // Only evaluate queries whose relevant passages are in our corpus
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

    println!("\n=== Distributed Search Simulation ===");
    println!(
        "Pipeline: {} ({}-dim) | Weights: semantic={:.2} bm25={:.2}",
        embedder.name(),
        dim,
        sw,
        1.0 - sw,
    );
    println!(
        "Corpus: {} passages | Nodes: {} | Fanout: {} | Distribution: {}",
        corpus.len(),
        num_nodes,
        fanout,
        distribution,
    );
    println!(
        "Queries: {} (with relevant passages in corpus)\n",
        eval_query_ids.len(),
    );

    // 2. Load or compute embeddings (resumable)
    let embeddings = msmarco::embed_with_checkpoints(&embedder, &corpus, pipeline_name, dim);

    // 3. Assign passages to nodes
    let assignments = assign_to_nodes(corpus.len(), num_nodes, distribution);

    // Print node distribution
    let mut node_sizes: Vec<usize> = vec![0; num_nodes];
    for &node_id in &assignments {
        node_sizes[node_id] += 1;
    }
    println!("Node distribution:");
    for (i, size) in node_sizes.iter().enumerate() {
        let pct = *size as f64 / corpus.len() as f64 * 100.0;
        println!("  Node {i}: {size} passages ({pct:.1}%)");
    }
    println!();

    // 4. Build per-node indices
    let ingest_start = Instant::now();
    let config = HybridConfig {
        semantic_weight: sw,
        bm25_weight: 1.0 - sw,
        skip_dedup: true,
        ..Default::default()
    };

    let node_indices: Vec<HybridIndex> = (0..num_nodes)
        .map(|_| HybridIndex::with_config(dim, config.clone()))
        .collect();

    // Group chunks by node, then ingest each batch
    let mut node_chunks: Vec<Vec<ContentChunk>> = vec![Vec::new(); num_nodes];
    for (i, ((pid, text), emb)) in corpus.iter().zip(embeddings.into_iter()).enumerate() {
        let node_id = assignments[i];
        node_chunks[node_id].push(ContentChunk {
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
        });
    }

    for (i, chunks) in node_chunks.into_iter().enumerate() {
        node_indices[i].ingest(&chunks).unwrap_or_else(|e| {
            eprintln!("Ingest failed for node {i}: {e}");
            std::process::exit(1);
        });
    }
    let ingest_secs = ingest_start.elapsed().as_secs_f64();
    tracing::info!(
        "Built {} node indices in {:.1}s ({:.0} docs/sec)",
        num_nodes,
        ingest_secs,
        corpus.len() as f64 / ingest_secs,
    );

    // 5. Also build a single-node baseline index for comparison
    println!("Building single-node baseline index...");
    let baseline_start = Instant::now();
    let baseline_index = HybridIndex::with_config(dim, config.clone());
    // Re-load embeddings for baseline (they were consumed above)
    let baseline_embeddings = msmarco::embed_with_checkpoints(&embedder, &corpus, pipeline_name, dim);
    let baseline_chunks: Vec<ContentChunk> = corpus
        .iter()
        .zip(baseline_embeddings.into_iter())
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
    baseline_index.ingest(&baseline_chunks).unwrap();
    let baseline_secs = baseline_start.elapsed().as_secs_f64();
    tracing::info!("Baseline index built in {:.1}s", baseline_secs);

    // 6. Evaluate: distributed vs single-node
    let eval_start = Instant::now();
    let mut dist_mrr_sum = 0.0;
    let mut dist_recall_10_sum = 0.0;
    let mut baseline_mrr_sum = 0.0;
    let mut baseline_recall_10_sum = 0.0;
    let mut query_count = 0usize;
    let mut dist_latencies = Vec::new();

    // Select which nodes to query: simple round-robin selection of `fanout` nodes.
    // In a real system this would use the routing table; here we simulate by
    // picking the nodes with the most documents for better coverage.
    let mut node_order: Vec<usize> = (0..num_nodes).collect();
    node_order.sort_by(|a, b| node_sizes[*b].cmp(&node_sizes[*a]));

    for (i, qid) in eval_query_ids.iter().enumerate() {
        let query_text = match query_map.get(qid) {
            Some(t) => *t,
            None => continue,
        };

        let query_emb = match embedder.embed_query(query_text) {
            Ok(v) => v,
            Err(_) => continue,
        };

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

        // Distributed query: fan out to `fanout` nodes, merge results
        let dist_start = Instant::now();
        let selected_nodes: Vec<usize> = node_order.iter().copied().take(fanout).collect();
        let mut all_results: Vec<(String, f64)> = Vec::new();

        for &node_id in &selected_nodes {
            if node_sizes[node_id] == 0 {
                continue;
            }
            if let Ok(results) = node_indices[node_id].query(
                &query_emb,
                query_text,
                &QueryFilters::default(),
                100,
            ) {
                for r in results {
                    let pid = r.url.strip_prefix("marco://").unwrap_or(&r.url).to_string();
                    all_results.push((pid, r.final_score));
                }
            }
        }

        // Merge: dedup by PID, keep highest score
        let mut merged: HashMap<String, f64> = HashMap::new();
        for (pid, score) in all_results {
            let entry = merged.entry(pid).or_insert(0.0);
            if score > *entry {
                *entry = score;
            }
        }
        let mut sorted_results: Vec<(String, f64)> = merged.into_iter().collect();
        sorted_results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        let dist_latency_ms = dist_start.elapsed().as_secs_f64() * 1000.0;
        dist_latencies.push(dist_latency_ms);

        let dist_pids: Vec<&str> = sorted_results.iter().map(|(pid, _)| pid.as_str()).collect();
        let dist_top_10: Vec<&str> = dist_pids.iter().take(10).copied().collect();
        dist_mrr_sum += metrics::mrr(&relevant_pids, &dist_top_10);
        dist_recall_10_sum += metrics::recall_at_k(&relevant_pids, &dist_pids, 10);

        // Baseline query: single node with all data
        if let Ok(results) =
            baseline_index.query(&query_emb, query_text, &QueryFilters::default(), 1000)
        {
            let baseline_pids: Vec<&str> = results
                .iter()
                .map(|r| r.url.strip_prefix("marco://").unwrap_or(&r.url))
                .collect();
            let baseline_top_10: Vec<&str> = baseline_pids.iter().take(10).copied().collect();
            baseline_mrr_sum += metrics::mrr(&relevant_pids, &baseline_top_10);
            baseline_recall_10_sum += metrics::recall_at_k(&relevant_pids, &baseline_pids, 10);
        }

        query_count += 1;

        if (i + 1) % 500 == 0 {
            let elapsed = eval_start.elapsed().as_secs_f64();
            let rate = (i + 1) as f64 / elapsed;
            eprintln!(
                "[eval] {}/{} queries ({:.0} q/sec, dist MRR@10={:.4}, baseline MRR@10={:.4})",
                i + 1,
                eval_query_ids.len(),
                rate,
                dist_mrr_sum / query_count as f64,
                baseline_mrr_sum / query_count as f64,
            );
        }
    }

    // 7. Report results
    let n = query_count.max(1) as f64;
    let dist_mrr = dist_mrr_sum / n;
    let dist_recall_10 = dist_recall_10_sum / n;
    let baseline_mrr = baseline_mrr_sum / n;
    let baseline_recall_10 = baseline_recall_10_sum / n;

    dist_latencies.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let p50 = percentile(&dist_latencies, 50.0);
    let p95 = percentile(&dist_latencies, 95.0);

    let mrr_delta = if baseline_mrr > 0.0 {
        (dist_mrr - baseline_mrr) / baseline_mrr * 100.0
    } else {
        0.0
    };

    println!("\n=== Results ({query_count} queries evaluated) ===\n");
    println!(
        "{:<20} {:>12} {:>12} {:>12}",
        "Metric", "Distributed", "Baseline", "Delta"
    );
    println!("{}", "-".repeat(58));
    println!(
        "{:<20} {:>12.4} {:>12.4} {:>+11.1}%",
        "MRR@10", dist_mrr, baseline_mrr, mrr_delta
    );
    let recall_delta = if baseline_recall_10 > 0.0 {
        (dist_recall_10 - baseline_recall_10) / baseline_recall_10 * 100.0
    } else {
        0.0
    };
    println!(
        "{:<20} {:>12.4} {:>12.4} {:>+11.1}%",
        "Recall@10", dist_recall_10, baseline_recall_10, recall_delta
    );
    println!(
        "\nDistributed query latency: p50={:.1}ms p95={:.1}ms",
        p50, p95
    );
    println!(
        "Configuration: {} nodes, fanout={}, distribution={}\n",
        num_nodes, fanout, distribution
    );
}

/// Assign each passage index to a node ID based on distribution mode.
fn assign_to_nodes(corpus_size: usize, num_nodes: usize, distribution: &str) -> Vec<usize> {
    match distribution {
        "zipf" => assign_zipf(corpus_size, num_nodes),
        _ => assign_uniform(corpus_size, num_nodes),
    }
}

/// Uniform round-robin assignment.
fn assign_uniform(corpus_size: usize, num_nodes: usize) -> Vec<usize> {
    (0..corpus_size).map(|i| i % num_nodes).collect()
}

/// Zipf (power-law) assignment: node 0 gets ~50% of docs, node 1 ~25%, etc.
/// Simulates real-world scenarios where some nodes have much more data.
fn assign_zipf(corpus_size: usize, num_nodes: usize) -> Vec<usize> {
    // Compute Zipf weights: w_i = 1/i for i=1..num_nodes
    let weights: Vec<f64> = (1..=num_nodes).map(|i| 1.0 / i as f64).collect();
    let total_weight: f64 = weights.iter().sum();

    // Compute cumulative thresholds
    let mut cumulative = Vec::with_capacity(num_nodes);
    let mut sum = 0.0;
    for w in &weights {
        sum += w / total_weight;
        cumulative.push(sum);
    }

    // Assign each document based on its fractional position
    (0..corpus_size)
        .map(|i| {
            let frac = i as f64 / corpus_size as f64;
            cumulative
                .iter()
                .position(|&threshold| frac < threshold)
                .unwrap_or(num_nodes - 1)
        })
        .collect()
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
    fn test_assign_uniform() {
        let assignments = assign_uniform(10, 3);
        assert_eq!(assignments, vec![0, 1, 2, 0, 1, 2, 0, 1, 2, 0]);
    }

    #[test]
    fn test_assign_zipf_distribution() {
        let assignments = assign_zipf(1000, 4);
        let mut counts = vec![0usize; 4];
        for &node in &assignments {
            counts[node] += 1;
        }
        // Node 0 should have the most docs, Node 3 the fewest
        assert!(counts[0] > counts[1]);
        assert!(counts[1] > counts[2]);
        assert!(counts[2] >= counts[3]);
        // Node 0 should have roughly ~48% (1/2.08 of total)
        assert!(counts[0] > 400);
    }

    #[test]
    fn test_assign_uniform_single_node() {
        let assignments = assign_uniform(5, 1);
        assert_eq!(assignments, vec![0, 0, 0, 0, 0]);
    }
}
