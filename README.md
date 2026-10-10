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

**Self-building.** Misses become coverage: external results are crawled, distilled to clean Markdown, quality-gated (length, link ratio, repetition statistics), deduplicated (exact + SimHash), and indexed — automatically, after your results are already shown. A sentinel daemon (`oi sentinel start --feeds <rss>`) keeps subscribed feeds fresh continuously.

**Fast where it counts.** Resident node answers covered queries in ~1–10ms (100× faster than commercial search APIs). Blended live-web queries take ~2s. Cold CLI runs pay ~1s of model load.

**Transparent for agents.** Every result carries comparable absolute scores (semantic + BM25 + final), full Markdown content, query-relevant extracted spans, and Stage-2 decision metadata.

**Answers only from retained evidence.** With a model configured (`OI_ANSWER_MODEL`, served locally by Ollama or hosted via `OI_ANSWER_PROVIDER=openrouter`), `oi search "<query>" --answer --explain` fetches the top results, has the model propose claims with quotes, judge each source's stance, and check that each claim answers what was asked, and then verifies every quote verbatim against the retained text before citing it. A claim that is supported but about something other than what was asked is listed as related evidence, never as the answer. A claim needs matching quotes from two distinct source families to be presented as an answer; a contradicting source makes it `contested`. Reading language is the model's job; there are no keyword, phrase, or domain lists deciding what a query means or which sources count. Without a model, `--answer` returns the result slate and claims nothing.

**Trust is claim-specific, never purchased rank.** Domain stake, reputation, and DNS verification never move retrieval scores. Experimental OpenUncertainty reports support, conflict, and ignorance per claim and never invents a probability; a missing critical requirement blocks a complete answer no matter how many pages agree. Two pages of one registrable domain count as one source. Cash has zero effect on any estimate. Legacy `trust_verdict` / `--min-trust` remains compatibility metadata, not calibrated confidence. See the [MVP-1 protocol](docs/OPENUNCERTAINTY-MVP1.md), [scoped pilot and stop gates](docs/experiments/PILOT-GATES-2026-10-08.md), [historical poll experiment](docs/experiments/BRAZIL-ELECTION-POLLS-2026-08-21.md), and [security economics](docs/SECURITY-ECONOMICS.md).

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
      ──► stake-neutral retrieval rank + Stage-2 metadata
      ──► background: crawl & index external results       ← the flywheel

with --answer and a configured model:
      ──► fetch result pages that have no retained text
      ──► model: propose claims + quotes; judge each source's stance
      ──► verify every quote verbatim in retained text; count source families
      ──► answer, destination, or explicit abstention with the checklist
```

## CLI tour

```bash
oi seed starter                      # bootstrap corpus (~80 curated pages)
oi seed sitemap <url>                # crawl a whole sitemap
oi sentinel start --feeds <rss,...>  # continuous freshness daemon
oi search "<query>" [--min-trust neutral] [--site <domain>]
OI_ANSWER_MODEL=<ollama model> oi search "<query>" --answer --explain
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
├── oi-sdk       # Client: blended search, answer assembly, model backends, learning
├── oi-staking   # Off-chain staking/economics mirror (on-chain: programs/)
├── oi-cli       # The `oi` binary
└── oi-facts     # Entity/fact extraction (GLiNER ONNX)
```

## Benchmarks (reproducible: `bench/`)

| Benchmark | Result |
|-----------|--------|
| BEIR scifact nDCG@10 | **0.6980** (beats published Arctic-S 0.6974) |
| BEIR nfcorpus nDCG@10 | 0.3322 |
| Agent retrieval, keyword match in top 5 (50 q, 2026-10-09) | 47/50 (94%) |
| 100k-passage ingest | ~1,400 docs/s; idempotent re-ingest 0.6s |
| Answer path, 20 development cases, `claude-haiku-5.5` (2026-10-09) | 20/20 as expected. Cases written and inspected by the implementer |
| Answer path, SQuAD 2.0 holdout D, 200 unseen cases, one source each (2026-10-09) | Answerable: 90 of 100 matched a gold answer, 6 abstained. Unanswerable: **11 of 100 were answered anyway** |

Results measured before 9 October 2026 on the retired keyword-rule answer layer (the 96-request matrices, the Google and Serper comparisons, the MS MARCO holdouts v1–v13) are kept as history in [docs/experiments/](docs/experiments/). They describe code that no longer exists and are not claims about the current system.

## Status & honest limitations

- **General superiority over market search is not demonstrated.** That claim still needs the pre-registered gate: at least 1,000 queries in eight strata judged by two independent external reviewers ([pilot gates](docs/experiments/PILOT-GATES-2026-10-08.md)).
- **The answer path still answers some questions it should not.** On SQuAD 2.0 questions the source text cannot answer, it emitted an answer 11% of the time (22–34% before the responsiveness checks). Those answers quote real text about something close to what was asked. Four disjoint 200-case samples, English, one small model, one source per case ([record](docs/experiments/RESTRUCTURE-2026-10-09.md)).
- **Answer latency depends on the model.** The answer path alone took a median of 4–6 s per question with a hosted small model and 22–41 s end to end with a local 20B model; the proposed ceiling is 5 s end to end, which has not been measured with the hosted model. Plain search is unaffected.
- **Model output is evidence, not fact.** Quotes are verified verbatim, so a model cannot cite text that is not there, but whether a quote supports a claim is still a model judgement. Source independence is reported as unresolved unless identical wording shows a shared origin.
- **No probabilities are shown.** The machinery to attach a calibrated probability to a complete answer exists and is off: two calibration tables were fitted and both failed the acceptance test on held-out data, the second in a pre-registered replication ([record](docs/experiments/ANSWER-CALIBRATION-2026-10-10.md)). Answers report support, conflict and ignorance, not a percentage.
- Freshness for uncovered topics depends on external engines (credential-free Bing, Brave public HTML, and DuckDuckGo; authenticated Serper or Brave API when configured). The keyless engines rate-limit under sustained load; production should configure an authenticated provider.
- The P2P network is young: replication is integrity-verified but not yet signature-authenticated; run peering with nodes you trust.
- Trust tiers and `--min-trust` are compatibility metadata, not calibrated probabilities; their feedback pipeline remains Sybil-prone ([details](docs/SECURITY-ECONOMICS.md)).
- The TSE/PesqEle join attaches the official registration record when a cited page prints its exact registration ID. It verifies registration metadata; it does **not** validate reported percentages.
- The $TRUTH settlement layer, protected paid delivery, and any cash signal are not live.

## License

MIT
