# SealQA (LongSeal) Benchmark Results

**Model:** Arctic Embed S (33M params, 384-dim)
**Dataset:** LongSeal — 254 questions, each with 1-3 gold docs hidden among ~20 hard-negative distractors
**Config:** Semantic-only (no BM25), no reranker

## Retrieval Recall

| Metric | Score |
|--------|-------|
| R@5 | 0.368 |
| R@10 | 0.581 |
| R@15 | 0.734 |
| R@20 | 0.839 |

## Gold Doc Rank Distribution

| Rank Bucket | Count | Pct |
|-------------|-------|-----|
| 1-5 | 102 | 40.2% |
| 6-10 | 53 | 20.9% |
| 11-15 | 39 | 15.4% |
| 16-20 | 27 | 10.6% |
| 21+ | 15 | 5.9% |
| Not found | 18 | 7.1% |

93% of gold docs are found somewhere in the ranking. The miss rate at R@10 (42%) is a ranking cutoff problem, not a retrieval problem.

## By Search Results Type

| Type | N | R@5 | R@10 | R@15 | R@20 |
|------|---|-----|------|------|------|
| conflicting | 144 | 0.306 | 0.484 | 0.684 | 0.809 |
| unhelpful | 110 | 0.450 | 0.709 | 0.800 | 0.877 |

## By Topic

| Topic | N | R@5 | R@10 | R@15 | R@20 |
|-------|---|-----|------|------|------|
| Entertainment | 55 | 0.373 | 0.655 | 0.827 | 0.873 |
| History & Geography | 21 | 0.310 | 0.548 | 0.643 | 0.857 |
| Others | 31 | 0.468 | 0.565 | 0.710 | 0.774 |
| Politics | 23 | 0.370 | 0.565 | 0.739 | 0.891 |
| Science & Technology | 68 | 0.331 | 0.529 | 0.654 | 0.787 |
| Sports | 56 | 0.375 | 0.601 | 0.786 | 0.875 |

## By Freshness

| Freshness | N | R@5 | R@10 | R@15 | R@20 |
|-----------|---|-----|------|------|------|
| fast-changing | 64 | 0.414 | 0.641 | 0.758 | 0.859 |
| never-changing | 79 | 0.317 | 0.519 | 0.707 | 0.802 |
| slow-changing | 111 | 0.378 | 0.592 | 0.740 | 0.853 |

## Latency (per question, arctic-s on CPU)

| Component | p50 | p95 |
|-----------|-----|-----|
| Embed (30 docs) | 835ms | 1,251ms |
| Search | 6.4ms | 9.0ms |

## Key Takeaway

For AI agents as clients, returning top-20 instead of top-10 pushes coverage from 58% to 84% — the single highest-impact change with zero infrastructure cost. The extra ~5K tokens per query costs ~$0.001 at current LLM prices.
