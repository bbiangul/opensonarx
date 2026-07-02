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

# 2. Search — local results blend with live web (DuckDuckGo needs no key;
#    export SERPER_API_KEY=... to use Google-quality results instead)
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

**Transparent for agents.** Every result carries comparable absolute scores (semantic + BM25 + final), full Markdown content, query-relevant extracted spans, and a trust verdict — no opaque rankings, no second fetch.

**Trust = skin in the game, never rank.** Publishers bond $TRUTH on domains they prove ownership of (DNS TXT record). Stake **never** moves a ranking score — our ablation benchmarks show capital-as-relevance degrades retrieval. It feeds an auditable verdict (`unverified → neutral → trusted → highly_trusted`) that agents can floor with `--min-trust`. A $1M stake with no earned reputation is still just "neutral". Honest status of every attack vector: [docs/SECURITY-ECONOMICS.md](docs/SECURITY-ECONOMICS.md).

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
          external engines (Serper → Brave → DuckDuckGo, parallel)
      ──► everything re-scored on one absolute scale, deduped, merged
      ──► results (content + scores + trust verdicts)
      ──► background: crawl & index the external results   ← the flywheel
```

## CLI tour

```bash
oi seed starter                      # bootstrap corpus (~80 curated pages)
oi seed sitemap <url>                # crawl a whole sitemap
oi sentinel start --feeds <rss,...>  # continuous freshness daemon
oi search "<query>" [--min-trust neutral] [--site <domain>]
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
| Dogfood battery vs mainstream search (15 q) | 15/15 parity |
| Warm node, covered query | 1–10ms |
| 100k-passage ingest | ~1,400 docs/s; idempotent re-ingest 0.6s |

## Status & honest limitations

- Freshness for uncovered topics depends on external engines (DuckDuckGo free; Serper/Brave by API key) until your index — or the network — covers them.
- The P2P network is young: replication is integrity-verified but not yet signature-authenticated; run peering with nodes you trust.
- Trust tiers above "neutral" require earned reputation, whose Sybil-resistant feedback pipeline is still in development ([details](docs/SECURITY-ECONOMICS.md)).
- The $TRUTH token layer is devnet-stage design, not live.

## License

MIT
