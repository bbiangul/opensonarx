# OpenSonarX

**Free, open-source, self-building search engine for AI agents.** Answers from a local index in ~1ms when it knows, reaches for the live web when it doesn't — and permanently learns from every miss.

## The Problem

When an AI agent needs the web today, it:

1. Calls a search API → 10 links with thin snippets (~250 tokens)
2. Fetches 3 pages → parses HTML, strips boilerplate (~6,500 tokens)
3. Sends it all to the LLM → expensive, slow, full of noise

**~$20,000/month per 1M queries** in input tokens alone — mostly navigation menus, cookie banners, and ads.

## The Solution

OpenSonarX keeps a local hybrid index (HNSW vectors + BM25) of clean, distilled content and blends in live web results only when local coverage is weak. External results the blend surfaces get crawled, distilled, and indexed in the background — **every query makes the next one faster, cheaper, and more local**.

| Approach | Median tokens | Cost/1M queries (Sonnet) | Warm latency |
|----------|:------------:|:------------------------:|:------------:|
| Search API + fetch 3 pages | 6,448 | $19,978 | 300–800ms |
| **OpenSonarX (covered query)** | **835** | **$2,480** | **~1–10ms** |

> 50-query token benchmark: [COMPARISON.md](COMPARISON.md) · IR benchmarks (BEIR, MS MARCO, scale tests): [bench/RESULTS.md](bench/RESULTS.md)

## Quick Start (5 minutes)

```bash
# Build (ONNX embeddings are on by default)
git clone https://github.com/bbiangul/opensonarx.git
cd opensonarx
cargo build --release --bin oi

# 1. Bootstrap the index with ~80 curated pages (reference + dev docs)
./target/release/oi seed starter

# 2. Search — local results blend with live web (Bing, Brave public HTML,
#    and DuckDuckGo need no key; production should add an authenticated API)
./target/release/oi search "how does the rust borrow checker work"
#    Watch the "(learning: indexing N new sources...)" line — that's the
#    index growing from your query. Run the same query again: it's local now.

# 3. Run a resident node — this is where search gets fast (~1ms warm)
./target/release/oi network node --http-port 8001

# 4. Query it over HTTP (what your agent does)
curl -s -X POST localhost:8001/search \
  -H 'content-type: application/json' \
  -d '{"query":"rust borrow checker","limit":5}'
```

Or with Docker:

```bash
docker compose up   # builds, seeds the starter corpus, serves on :8001
```

## What makes it different

**Self-building.** Misses become coverage: external results are crawled, distilled to clean Markdown, quality-gated (spam/slop scoring), deduplicated (exact + SimHash), and indexed — automatically, after your results are already shown. A sentinel daemon (`oi sentinel start --feeds <rss>`) keeps subscribed feeds fresh continuously.

**Fast where it counts.** Resident node answers covered queries in ~1–10ms (100× faster than commercial search APIs). Blended live-web queries take ~2s. Cold CLI runs pay ~1s of model load.

**Transparent for agents.** Every result carries comparable absolute scores (semantic + BM25 + final), full Markdown content, query-relevant extracted spans, and a Stage-2 decision trace. `oi search "<query>" --answer --explain` exposes intent, evidence status, citations, search passes, hydration count, uncertainty mode, and latency.

**Trust is claim-specific, never purchased rank.** Domain stake, reputation, and DNS verification never move retrieval scores. Experimental OpenUncertainty separates deterministic results, numerical estimates and evidential-only support/conflict/ignorance. Ten reports of one registered poll are not ten measurements. Observed descriptive mass is bounded so page volume cannot erase a fixed missing requirement; a generic `critical_missing` flag blocks complete reliance and probabilities. Full search checklist integration remains pending. The former marginal cash update is disabled because it can double count source competence/claim selection. Cash has zero probability effect until a joint/conditional model validates incremental information over the same no-cash evidence baseline. Legacy `trust_verdict` / `--min-trust` remains compatibility metadata, not calibrated confidence. See the [MVP-1 protocol](docs/OPENUNCERTAINTY-MVP1.md), [scoped pilot and stop gates](docs/experiments/PILOT-GATES-2026-10-08.md), [historical poll experiment](docs/experiments/BRAZIL-ELECTION-POLLS-2026-08-21.md), and [security economics](docs/SECURITY-ECONOMICS.md).

**Decentralized corpus.** Nodes announce shards over gossipsub, fan queries out to peers, and sync same-shard content via verified incremental replication. A new node with one bootstrap address inherits the corpus:

```bash
oi network node --port 4002 --http-port 8002 --bootstrap /ip4/<peer>/tcp/4001
# → catch-up replication pulls the peer's index, then serves it locally
```

## How a query flows

```
query ──► spell-correct (index vocabulary, guarded)
      ──► embed (Arctic Embed S, 384-dim ONNX)
      ──► local hybrid retrieval (HNSW + BM25, quality/freshness weighting)
      ──► strong local coverage? ──► answer in ~1ms
              │ no
              ▼
          configured external engines (parallel, cached, circuit-broken)
      ──► everything re-scored on one absolute scale, deduped, merged
      ──► intent-aware, stake-neutral retrieval rank
      ──► retained evidence + OpenUncertainty Stage 2
      ──► claim-scoped structured joins where available
              (Brazilian polls: official TSE/PesqEle registration metadata)
      ──► extractive answer, destination, or explicit abstention
      ──► background: crawl & index external results       ← the flywheel

Navigational destinations that cannot be text-crawled are retained in a
separate metadata cache. They can route `claude` or `perplexity`, but cannot
count as factual evidence.
```

## CLI tour

```bash
oi seed starter                      # bootstrap corpus (~80 curated pages)
oi seed sitemap <url>                # crawl a whole sitemap
oi sentinel start --feeds <rss,...>  # continuous freshness daemon
oi search "<query>" [--min-trust neutral] [--site <domain>]
oi search "<query>" --answer --explain
oi network node --http-port 8001     # resident node + HTTP API + P2P
oi index info                        # health: consistency, domains, quality
oi index reembed                     # migrate after an embedder upgrade
oi wallet create && oi stake <domain> --amount <lamports>
oi verify <url>                      # trust-check any URL
```

## Architecture

```
crates/
├── oi-types     # Shared types, error codes, protobuf
├── oi-index     # HNSW + BM25 hybrid engine, dedup, audit, storage locking
├── oi-embed     # Arctic Embed S (ONNX, 384-dim) + caching
├── oi-crawl     # Distiller, chunker, quality gates, sentinel daemon
├── oi-network   # libp2p: gossip, query fan-out, verified replication
├── oi-sdk       # Client: blended search, learning, spell-correction, stakes
├── oi-staking   # Solana Anchor program ($TRUTH), disputes, emissions
├── oi-cli       # The `oi` binary
└── oi-facts     # Entity/fact extraction (GLiNER ONNX)
```

## Benchmarks (reproducible: `bench/`)

| Benchmark | Result |
|-----------|--------|
| BEIR scifact nDCG@10 | **0.6980** (beats published Arctic-S 0.6974) |
| BEIR nfcorpus nDCG@10 | 0.3322 |
| Agent retrieval accuracy (50 q) | 82% |
| Live production-path dogfood | 24 semantic contracts × 4 natural forms = 96 requests across navigation, action, tool, current facts, factual relations, due diligence, health/tax guidance, definition, price, local, and recommendation intents |
| Latest complete cold matrix, concurrency 4 | **96/96**; 0→146 docs; 1.6 qps; p50 0.93s; p95 5.09s; p99 5.94s; foreground hydration p99 4.30s; background crawl p99 2.74s; real embedding, discovery, crawling, evidence, decision, and answers |
| First three-pass resident/revalidation matrix on the same corpus | **288/288**; 146→177 docs; 12.3 qps; p50 3.2ms; p95 1.24s; p99 2.40s; background crawl p99 1.70s; passes every declared resident quality/latency/throughput gate |
| Resident concurrency soak, 16 workers | **960/960**; 177→210 docs; 26.9 qps; p50 2.9ms; p95 0.57s; p99 2.65s; background crawl p99 1.01s; no warmup and no mocks |
| Sustained keyless-provider probe, 3 classes × 10 rounds, concurrency 4 | Bing 30/30; Brave public HTML 12/30 before rate limiting; sustained redundancy gate failed with 1/2 required providers |
| Sustained credentialed-provider probe, 3 classes × 10 rounds, concurrency 4 | **Bing 30/30 and Serper 30/30**; p50 293ms and 1.03s respectively; 2/2 availability gate passed for this configuration |
| Real failover chaos without Bing, 24 canonical cases | 17/24 (70.8%); Brave public HTML and DuckDuckGo both rate-limited; safe abstention prevented fabricated results but recall was not production-ready |
| Seen Google multi-intent development regression, 9 cases | Google 74.1%, OpenSonarX 100.0%; 4 wins, 5 ties, 0 losses on policy-authored candidate contracts; **not confirmatory evidence** after these cases drove implementation changes |
| Independent MS MARCO answer-seeking holdout, 48 cases | OpenSonarX non-empty 40/48 (83.3%); Google visible organic 48/48 (100%); relevance/trust still unjudged, so superiority is false |
| Independent current/high-impact holdout, 40 cases | OpenSonarX non-empty 21/40 (52.5%): 16/25 current Trends BR and 5/15 health/legal/finance; subsequent Google capture was blocked and is treated as missing data |
| Disjoint post-fix MS MARCO holdout, 48 cases | OpenSonarX non-empty 13/48 (27.1%); real Brave 429 and DuckDuckGo 403/timeout exposed an E2E availability failure, not a clean ranking estimate |
| Blind pooled v10/v11 diagnostics, 48 disjoint cases each | OpenSonarX lost to raw Serper: acceptable top-one gaps −12.5/−6.3 points; relevance nDCG gaps −0.099/−0.086; authority nDCG gaps −0.150/−0.118 |
| Blind pooled v12/v13 diagnostics, 48 disjoint cases each | **Parity repeated** after bounded per-provider depth and query-relative source roles: acceptable top-one gaps −2.1/0.0 points; relevance nDCG gaps +0.008/+0.001; authority nDCG gaps −0.007/−0.024; 0% severe top-one errors on both systems in both runs; all CIs include zero |
| Exploratory market latency audit, 9 intents × 3 rounds | Google browser wall→first organic p50/p95/p99 215/407/414ms; OpenSonarX backend `search_ms` 1.96/31.02/2400.04ms; 8/9 queries text-identical; deliberately **not comparable** because the measured surfaces and execution protocols differ |
| Aggregate mission-readiness gate | **FAIL**: credentialed coverage and provider availability now pass in one answer-seeking slice, but blinded market superiority, comparable latency, heterogeneous generalization, real-cash prediction, and live calibration are not established |
| Google election-poll snapshot replay | policy nDCG@5 0.8955→0.9985; hard-risk@3 18.52%→3.70%; independent clusters@3 regressed 2.44→2.22 |
| 100k-passage ingest | ~1,400 docs/s; idempotent re-ingest 0.6s |

## Status & honest limitations

- Freshness for uncovered topics depends on external engines (credential-free Bing, Brave public HTML, and DuckDuckGo; authenticated Serper or Brave API when configured) until your index — or the network — covers them.
- The keyless serving waterfall is Bing first, Brave public HTML second, and DuckDuckGo as a delayed tertiary hedge. Its sustained concurrency-four gate still fails with only Bing healthy: Brave passed 12/30 before HTTP 429, and a no-Bing Brave/DuckDuckGo chaos run passed 17/24 before both rate-limited. In the credentialed configuration, Serper and Bing each passed 30/30, so the observed availability-redundancy blocker is removed. This does not prove upstream index independence. Semantically poisoned batches are discarded before crawling, and circuit breakers, coalesced acquisition, bounded deadlines, retained coverage, and abstention make failures safer; they cannot manufacture missing recall.
- The P2P network is young: replication is integrity-verified but not yet signature-authenticated; run peering with nodes you trust.
- Legacy trust tiers and `--min-trust` are compatibility metadata, not calibrated probabilities; their feedback pipeline remains Sybil-prone ([details](docs/SECURITY-ECONOMICS.md)).
- OpenUncertainty is an MVP with synthetic estimator tests and real production-path evidence/abstention validation. Informational/exploratory result slates pass through evidential Stage 2, with both retrieval and decision ranks exposed; verified navigation, transactions, and tools remain outside claim reliance, while an ambiguous short query without a canonical destination is assessed as informational. A relevance guard prevents trust from promoting a materially off-topic candidate. Query-relative source roles now break otherwise-identical cold-source ties inside that band without fabricating probability. Cold external candidates still usually lack source history, generalized lineage, and retained claim evidence. The real-cash predictive thesis and the $TRUTH settlement layer are not live; every live result had `cash_log_bayes_factor = 0`.
- The claim graph is partial. Brazilian poll answers join the exact retained TSE registration, mark one underlying measurement and its editorial derivatives, and expose known independence separately from unresolved dependency uncertainty. A second bounded discovery pass can correct an alternate-slate headline (observed `39–32`) to the unqualified primary scenario (`39–33`) only after the normal two-family and registry-lineage gates pass. Corroborated factual relations share a stable normalized claim signature and report editorial-family count, while explicitly leaving observation count and derivation independence unknown. Due diligence, thematic events, ownership/funding, quotation, and broader claim classes still lack resolved lineage.
- PesqEle verifies registration and methodology metadata; it does **not** validate the percentages reported by publishers, sampling accuracy, or absence of methodological bias.
- The live answer layer is intentionally narrow: structured poll facts and simple factual relations can be extractive; generic news synthesis and legal/operational legitimacy checks currently abstain.
- The current 96-request and nine-market-case suites are development regressions, not generalization evidence. Every inspected holdout is burned before the next category-level fix, and later manifests reject prior IDs, normalized queries, and hash drift. The blind evaluator pools and randomizes OpenSonarX/raw-Serper slates, keeps the system key private, requires complete judgments, uses pooled-IDCG nDCG and paired bootstrap intervals, and cannot pass the claim gate below 1,000 queries/eight strata/two independent external judges. One system-blind local-model diagnostic found clear OpenSonarX losses on v10/v11 and parity on the further-disjoint v12/v13 samples. That is evidence of meaningful progress in one English answer-seeking stratum, not more trustworthy general-purpose search than Google: the judge saw only URL/title/snippet, was not external to implementation, the samples are small, heterogeneous intents/languages remain absent, and raw Serper is not the complete Google product surface. Therefore broad superiority remains false. The three-round latency audit also keeps `market_latency_compared=false` because browser wall time and backend `search_ms` are different surfaces. See the [generalization holdout report](docs/experiments/GENERALIZATION-HOLDOUT-2026-08-22.md).

See the dated [live end-to-end validation report](docs/experiments/LIVE-PRODUCTION-E2E-2026-08-21.md) for development-path details and the [generalization holdout report](docs/experiments/GENERALIZATION-HOLDOUT-2026-08-22.md) for the independent mission audit.

## License

MIT
