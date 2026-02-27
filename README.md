# OpenSonarX

**Free, open-source search engine built for AI agents.** Returns full content with trust signals — so LLMs don't have to fetch and parse web pages themselves.

## The Problem

Today, when an AI agent needs to answer a question from the web, it:

1. Calls a search API → gets 10 links with thin snippets (~250 tokens)
2. Fetches 3 pages → parses HTML, strips boilerplate (~6,500 tokens)
3. Sends all of it to the LLM → expensive, slow, full of noise

**This costs ~$20,000/month per 1M queries** in input tokens alone. Most of those tokens are navigation menus, cookie banners, and ads.

## The Solution

OpenSonarX pre-crawls the web, extracts clean content, and returns only the sentences relevant to your query — ready for RAG synthesis.

| Approach | Median Tokens | Cost/1M Queries (Sonnet) |
|----------|:------------:|:------------------------:|
| Search API + fetch 3 pages | 6,448 | $19,978 |
| **OpenSonarX** | **835** | **$2,480** |

**8x fewer tokens. 8x lower cost. Same answer quality.**

> Benchmarked across 50 queries (tutorials, concepts, devops, AI/ML, product docs). [Full results →](COMPARISON.md)

## Install

```bash
curl -fsSL https://opensonarx.com/install.sh | sh
```

Or build from source:

```bash
git clone https://github.com/bbiangul/opensonarx.git
cd opensonarx
cargo build --release --bin oi
```

## Quick Start

```bash
# Search (works immediately — DuckDuckGo blended results, no API key needed)
oi search "how to deploy a Next.js app to Vercel"

# Seed your local index with content
oi seed sitemap https://docs.rs/sitemap.xml

# Run a P2P node with HTTP API
oi network node --port 4001 --http-port 8001

# Search through your node
oi search "rust async" --node http://localhost:8001
```

## How It Works

```
Agent query → OpenSonarX
                ├── HNSW vector search (384-dim MiniLM embeddings)
                ├── BM25 keyword search (TF-IDF)
                ├── Reputation-weighted ranking
                └── Returns: clean Markdown + trust signals
```

1. **Sentinel nodes crawl** the web → extract content → chunk → embed → index
2. **AI agents search** → get ranked results with full content, not just links
3. **Feedback signals** from agents improve ranking over time

## Architecture

```
crates/
├── oi-types     # Shared types, error codes, protobuf
├── oi-index     # HNSW + BM25 hybrid search engine
├── oi-embed     # MiniLM (ONNX, 384-dim) embeddings
├── oi-crawl     # Content distiller + crawl pipeline
├── oi-network   # libp2p P2P layer (Kademlia, gossipsub)
├── oi-sdk       # SDK core (wires index + embedder + engines)
├── oi-staking   # Solana Anchor program ($TRUTH token) [Phase 3]
├── oi-cli       # CLI binary (oi)
└── oi-facts     # Entity extraction (GLiNER ONNX)
```

## Why OpenSonarX?

| Feature | Traditional Search APIs | OpenSonarX |
|---------|------------------------|------------|
| **Output** | Links + snippets | Full Markdown content |
| **Token cost** | ~6,500 tokens/query | ~835 tokens/query |
| **Fetching pages** | Agent must fetch & parse | Pre-crawled and indexed |
| **Trust signals** | None | Reputation score, feedback signals |
| **Content quality** | Raw HTML with ads/nav | Clean, query-relevant content |
| **Cost** | Paid API + LLM tokens | Free and open source |
| **Infrastructure** | Centralized | Decentralized P2P network |

## Roadmap

| Phase | Status | Description |
|-------|--------|-------------|
| **Phase 1** | Done | Hybrid search engine (HNSW + BM25), content distiller, CLI |
| **Phase 2** | Done | P2P network (libp2p), blended search, sentinel crawler |
| **Phase 3** | Planned | $TRUTH token staking on Solana, dispute/jury system, stake-weighted ranking |

## Documentation

Full docs at [docs.opensonarx.com](https://docs.opensonarx.com)

## License

MIT
