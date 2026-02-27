use oi_index::{FusionMethod, HybridConfig, HybridIndex};
use oi_types::traits::{EmbeddingPipeline, SearchIndex};
use oi_types::{ChunkMeta, ContentChunk, QueryFilters};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use crate::dataset::{BenchmarkDataset, Difficulty};
use crate::metrics;

/// Configuration for a parameter sweep.
pub struct SweepConfig {
    pub semantic_weights: Vec<f64>,
    pub ef_search_values: Vec<usize>,
    pub pool_multipliers: Vec<usize>,
    pub fusion_methods: Vec<FusionMethod>,
}

impl Default for SweepConfig {
    fn default() -> Self {
        Self {
            semantic_weights: vec![0.5, 0.6, 0.7, 0.8],
            ef_search_values: vec![50, 100],
            pool_multipliers: vec![2, 4],
            fusion_methods: vec![FusionMethod::WeightedSum, FusionMethod::Rrf],
        }
    }
}

/// Result of a single sweep configuration.
#[derive(Debug, Clone)]
pub struct SweepResult {
    pub config: HybridConfig,
    pub recall_1: f64,
    pub recall_5: f64,
    pub recall_10: f64,
    pub mrr: f64,
    pub ndcg_10: f64,
    pub avg_latency_ms: f64,
}

/// Pre-computed embeddings for all docs and queries in a dataset.
struct PrecomputedData {
    /// doc_id -> (embedding, content)
    doc_embeddings: HashMap<String, (Vec<f32>, String)>,
    /// case index -> query embedding
    query_embeddings: Vec<Option<Vec<f32>>>,
    /// content -> first indexed doc_id (for dedup)
    content_to_id: HashMap<String, String>,
}

fn precompute(
    dataset: &BenchmarkDataset,
    embedder: &dyn EmbeddingPipeline,
) -> PrecomputedData {
    let mut doc_embeddings = HashMap::new();
    let mut content_to_id = HashMap::new();

    // Collect unique docs for batched embedding
    let mut unique_docs: Vec<(String, String)> = Vec::new(); // (doc_id, content)
    for case in &dataset.cases {
        for doc in &case.relevant_docs {
            if content_to_id.contains_key(&doc.content) {
                continue;
            }
            content_to_id.insert(doc.content.clone(), doc.doc_id.clone());
            unique_docs.push((doc.doc_id.clone(), doc.content.clone()));
        }
    }

    // Batch-embed all docs (32 per batch)
    let doc_texts: Vec<&str> = unique_docs.iter().map(|(_, c)| c.as_str()).collect();
    if let Ok(embs) = oi_embed::batch::embed_in_batches(embedder, &doc_texts, 32) {
        for (i, emb) in embs.into_iter().enumerate() {
            let (ref id, ref content) = unique_docs[i];
            doc_embeddings.insert(id.clone(), (emb, content.clone()));
        }
    }

    // Batch-embed all queries (32 per batch) — queries need prefix, so use embed_query
    // We batch manually since embed_query isn't batched in the trait
    let query_embeddings: Vec<Option<Vec<f32>>> = dataset
        .cases
        .iter()
        .map(|case| embedder.embed_query(&case.query).ok())
        .collect();

    PrecomputedData {
        doc_embeddings,
        query_embeddings,
        content_to_id,
    }
}

/// Run a parameter sweep over hybrid search configurations.
pub fn run_sweep(
    dataset: &BenchmarkDataset,
    embedder: Arc<dyn EmbeddingPipeline>,
    sweep: &SweepConfig,
) -> Vec<SweepResult> {
    let dim = embedder.dimension();
    let precomputed = precompute(dataset, embedder.as_ref());

    let mut results = Vec::new();

    for &sw in &sweep.semantic_weights {
        for &ef in &sweep.ef_search_values {
            for &pool in &sweep.pool_multipliers {
                for &fusion in &sweep.fusion_methods {
                    let config = HybridConfig {
                        semantic_weight: sw,
                        bm25_weight: 1.0 - sw,
                        ef_search: ef,
                        pool_multiplier: pool,
                        fusion,
                        ..Default::default()
                    };

                    let result = evaluate_config(
                        dataset,
                        &precomputed,
                        dim,
                        config,
                    );
                    results.push(result);
                }
            }
        }
    }

    // Sort by recall@10 descending
    results.sort_by(|a, b| b.recall_10.partial_cmp(&a.recall_10).unwrap());
    results
}

fn evaluate_config(
    dataset: &BenchmarkDataset,
    precomputed: &PrecomputedData,
    dim: usize,
    config: HybridConfig,
) -> SweepResult {
    let index = HybridIndex::with_config(dim, config.clone());

    // Ingest all docs
    let chunks: Vec<ContentChunk> = precomputed
        .doc_embeddings
        .iter()
        .map(|(id, (emb, content))| ContentChunk {
            chunk_id: id.clone(),
            content: content.clone(),
            embedding: Some(emb.clone()),
            seq: 0,
            meta: Some(ChunkMeta {
                url: format!("https://bench.test/{id}"),
                title: id.clone(),
                domain: "bench.test".to_string(),
                content_hash: format!("hash_{id}"),
                crawled_at: Some(1700000000),
                content_type: Some("text/html".to_string()),
                entity: None,
                content_kind: None,
                ..Default::default()
            }),
        })
        .collect();

    index.ingest(&chunks).unwrap();

    let mut total = 0usize;
    let mut sum_recall_1 = 0.0;
    let mut sum_recall_5 = 0.0;
    let mut sum_recall_10 = 0.0;
    let mut sum_mrr = 0.0;
    let mut sum_ndcg = 0.0;
    let mut total_latency = 0.0;

    for (i, case) in dataset.cases.iter().enumerate() {
        if case.difficulty == Difficulty::Adversarial {
            continue;
        }

        let query_emb = match &precomputed.query_embeddings[i] {
            Some(e) => e,
            None => continue,
        };

        let start = Instant::now();
        let search_results = match index.query(query_emb, &case.query, &QueryFilters::default(), 10) {
            Ok(r) => r,
            Err(_) => continue,
        };
        total_latency += start.elapsed().as_secs_f64() * 1000.0;

        let retrieved_ids: Vec<&str> = search_results
            .iter()
            .map(|r| r.url.rsplit('/').next().unwrap_or(""))
            .collect();

        let relevant_ids: Vec<String> = case
            .relevant_docs
            .iter()
            .filter(|d| d.relevance > 0)
            .filter_map(|d| precomputed.content_to_id.get(&d.content).cloned())
            .collect();
        let relevant_refs: Vec<&str> = relevant_ids.iter().map(|s| s.as_str()).collect();

        let relevance_map: Vec<(String, u8)> = case
            .relevant_docs
            .iter()
            .filter_map(|d| {
                precomputed.content_to_id.get(&d.content).map(|id| (id.clone(), d.relevance))
            })
            .collect();
        let relevance_refs: Vec<(&str, u8)> =
            relevance_map.iter().map(|(s, r)| (s.as_str(), *r)).collect();

        sum_recall_1 += metrics::recall_at_k(&relevant_refs, &retrieved_ids, 1);
        sum_recall_5 += metrics::recall_at_k(&relevant_refs, &retrieved_ids, 5);
        sum_recall_10 += metrics::recall_at_k(&relevant_refs, &retrieved_ids, 10);
        sum_mrr += metrics::mrr(&relevant_refs, &retrieved_ids);
        sum_ndcg += metrics::ndcg_at_k(&relevance_refs, &retrieved_ids, 10);
        total += 1;
    }

    let n = total.max(1) as f64;
    SweepResult {
        config,
        recall_1: sum_recall_1 / n,
        recall_5: sum_recall_5 / n,
        recall_10: sum_recall_10 / n,
        mrr: sum_mrr / n,
        ndcg_10: sum_ndcg / n,
        avg_latency_ms: total_latency / n,
    }
}

impl std::fmt::Display for SweepResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let fusion = match self.config.fusion {
            FusionMethod::WeightedSum => "WSum",
            FusionMethod::Rrf => "RRF ",
        };
        write!(
            f,
            "sw={:.1} bw={:.1} ef={:<3} pool={} {} | R@1={:.3} R@5={:.3} R@10={:.3} MRR={:.3} nDCG={:.3} lat={:.1}ms",
            self.config.semantic_weight,
            self.config.bm25_weight,
            self.config.ef_search,
            self.config.pool_multiplier,
            fusion,
            self.recall_1, self.recall_5, self.recall_10,
            self.mrr, self.ndcg_10,
            self.avg_latency_ms,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generator::generate_text_dataset;
    use oi_embed::MockEmbedder;

    #[test]
    fn sweep_runs_without_panic() {
        let dataset = generate_text_dataset(20);
        let embedder: Arc<dyn EmbeddingPipeline> = Arc::new(MockEmbedder::default());
        let sweep = SweepConfig {
            semantic_weights: vec![0.5, 0.7],
            ef_search_values: vec![50],
            pool_multipliers: vec![2],
            fusion_methods: vec![FusionMethod::WeightedSum, FusionMethod::Rrf],
        };
        let results = run_sweep(&dataset, embedder, &sweep);
        assert_eq!(results.len(), 4); // 2 weights * 1 ef * 1 pool * 2 fusion
        for r in &results {
            assert!(r.recall_10 >= 0.0 && r.recall_10 <= 1.0);
        }
    }
}
