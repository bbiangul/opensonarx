# OpenSonarX Benchmark Results

## MS MARCO Passage Ranking (Dev Small)

**Dataset**: 8,841,823 passages | 6,980 queries | Binary relevance
**Date**: 2026-02-17

### Configuration

| Parameter | Value |
|-----------|-------|
| Embedding model | Snowflake Arctic Embed S |
| Model params | 33M |
| Embedding dim | 384 |
| Pooling | CLS + L2 normalized |
| Index | HNSW + Tantivy (hybrid) |
| Semantic weight | 0.80 |
| BM25 weight | 0.20 |
| Fusion | Reciprocal Rank Fusion |
| ef_search | 50 |

### Results

| Metric | Score |
|--------|-------|
| **MRR@10** | **0.2393** |
| Recall@10 | 0.4435 |
| Recall@100 | 0.6558 |
| Recall@1000 | 0.7834 |

### Published Baselines (MRR@10)

| Model | Params | MRR@10 | vs OpenSonarX |
|-------|--------|--------|--------------|
| BM25 (Anserini) | - | 0.187 | -22% |
| **OpenSonarX (Arctic-S)** | **33M** | **0.239** | **--** |
| ANCE | 110M | 0.330 | +38% |
| TAS-B | 66M | 0.344 | +44% |
| ColBERTv2 | 110M | 0.397 | +66% |

### Performance

| Metric | Value |
|--------|-------|
| Ingest time | 15,964s (4h26m) |
| Ingest throughput | 554 docs/sec |
| Eval time | 4,629s (77min) |
| Query latency p50 | 581.5ms |
| Query latency p95 | 1,525.1ms |
| Peak memory | ~20GB |

### Scaling Behavior

| Corpus Size | MRR@10 | Ingest Rate | Query p50 |
|-------------|--------|-------------|-----------|
| 10,000 | - | ~600 docs/sec | - |
| 500,000 | 0.2913 | 751 docs/sec | 34.2ms |
| 8,841,823 | 0.2393 | 554 docs/sec | 581.5ms |

MRR@10 drops ~18% from 500K to full corpus as the search space grows 17.7x.

---

## BEIR Benchmarks

**Date**: 2026-02-15

### SciFact (nDCG@10)

| Sem.W | BM25.W | nDCG@10 | R@10 | R@100 | MRR@10 |
|-------|--------|---------|------|-------|--------|
| 1.00 | 0.00 | 0.6822 | 0.9000 | 0.9733 | 0.6556 |
| 0.90 | 0.10 | 0.6960 | 0.9200 | 0.9800 | 0.6667 |
| **0.80** | **0.20** | **0.7094** | **0.9267** | **0.9800** | **0.6778** |
| 0.70 | 0.30 | 0.7000 | 0.9200 | 0.9800 | 0.6722 |
| 0.50 | 0.50 | 0.6766 | 0.9067 | 0.9767 | 0.6500 |
| 0.00 | 1.00 | 0.6430 | 0.8400 | 0.9567 | 0.6111 |

Best: **nDCG@10 = 0.7094** at semantic=0.80, bm25=0.20

### NFCorpus (nDCG@10)

| Sem.W | BM25.W | nDCG@10 | R@10 | R@100 | MRR@10 |
|-------|--------|---------|------|-------|--------|
| 1.00 | 0.00 | 0.3161 | 0.2643 | 0.2643 | 0.4598 |
| 0.90 | 0.10 | 0.3286 | 0.2799 | 0.2799 | 0.4629 |
| **0.80** | **0.20** | **0.3351** | **0.2863** | **0.2863** | **0.4753** |
| 0.70 | 0.30 | 0.3298 | 0.2832 | 0.2832 | 0.4659 |
| 0.50 | 0.50 | 0.3148 | 0.2738 | 0.2738 | 0.4473 |
| 0.00 | 1.00 | 0.2919 | 0.2488 | 0.2488 | 0.3978 |

Best: **nDCG@10 = 0.3351** at semantic=0.80, bm25=0.20

---

## Notes

- All benchmarks run on CPU (Apple M-series, 48GB RAM)
- Embedding model: Snowflake Arctic Embed S via ONNX Runtime
- Query prefix: `"Represent this sentence for searching relevant passages: "` (asymmetric)
- Embeddings cached to disk for reproducibility (~13GB for full MS MARCO)
