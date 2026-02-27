# OpenSonarX Quickstart

Run the OpenSonarX search protocol locally in under 5 minutes.

## Prerequisites

| Tool | Version | Install |
|------|---------|---------|
| Rust | 1.85+ | `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \| sh` |
| Solana CLI | 1.18+ | `sh -c "$(curl -sSfL https://release.anza.xyz/v1.18.26/install)"` (optional, Phase 3) |

---

## 1. Build

```bash
cargo build --release --bin oi
```

---

## 2. What the stack looks like

```
┌─────────────────────────────────────────────────────────┐
│                      oi (CLI)                           │
│  search · feedback · sentinel · seed · network           │
└────────────┬────────────────────────┬───────────────────┘
             │ HTTP (--node)          │ libp2p
             ▼                        ▼
┌────────────────────┐    ┌───────────────────────┐
│   Node HTTP API    │    │   oi-network (P2P)    │
│   --http-port      │    │   Kad DHT, gossipsub  │
│                    │    │   request-response    │
└────────────────────┘    └───────────────────────┘
         │
    ┌────┴────────────────────────────┐
    │         oi-sdk (core)           │
    │  ┌──────────┐ ┌──────────────┐  │
    │  │ oi-index │ │ oi-embed     │  │
    │  │ HNSW+BM25│ │ MiniLM(ONNX)│  │
    │  └──────────┘ └──────────────┘  │
    │  ┌──────────┐                   │
    │  │ oi-crawl │                   │
    │  │ fetcher, │                   │
    │  │ sentinel │                   │
    │  └──────────┘                   │
    └─────────────────────────────────┘
```

**Key crates** (all under `crates/`):

| Crate | What it does | Reference |
|-------|-------------|-----------|
| `oi-types` | Shared types, error codes (E1000-E6007), protobuf types | `crates/oi-types/src/error.rs` |
| `oi-index` | Hybrid HNSW (vector) + BM25 (keyword) search engine | `crates/oi-index/src/hybrid.rs` |
| `oi-embed` | Embedding pipelines: MiniLM (ONNX, 384-dim), CLIP (512-dim) | `crates/oi-embed/src/minilm.rs` |
| `oi-crawl` | Content distiller: fetch, extract, chunk, quality score | `crates/oi-crawl/src/pipeline.rs` |
| `oi-network` | libp2p layer: Kademlia DHT, gossipsub, query protocol | `crates/oi-network/src/node.rs` |
| `oi-sdk` | SDK core that wires index + embedder + engines | `crates/oi-sdk/src/client.rs` |
| `oi-staking` | Solana Anchor program (Phase 3 — not yet active) | `programs/opensonarx-staking/src/lib.rs` |
| `oi-cli` | CLI binary `oi`: search, seed, sentinel, network | `crates/oi-cli/src/main.rs` |

---

## 3. Search

```bash
# Local search (uses local index + DuckDuckGo blended results)
oi search "rust async programming" -k 5

# Search via a running P2P node
oi search "rust async programming" --node http://localhost:8001
```

Results are ranked by hybrid score (semantic + keyword) weighted by reputation from feedback signals.

---

## 4. Blended search (external engines)

DuckDuckGo is always available as a free fallback — no API key needed. For higher-quality results, configure paid engines in `~/.opensonarx/config.toml`:

```toml
[[search.engines]]
name = "google"
priority = 1
api_key = "your-serper-key"

[[search.engines]]
name = "brave"
priority = 5
api_key = "your-brave-key"
```

Results from all engines are interleaved with local results using reciprocal rank fusion.

---

## 5. Seed content

```bash
# Crawl a sitemap
oi seed sitemap https://docs.rs/sitemap.xml

# Crawl URLs from a file
oi seed urls urls.txt

# Spider a site (follow links)
oi seed crawl https://rust-lang.org --depth 3 --max-pages 1000

# Seed from docs.rs (top N crates)
oi seed registry docs-rs --top 100
```

---

## 6. Sentinel node (crawl daemon)

The Sentinel continuously crawls domains and feeds the local index.

```bash
# Start crawling (4 concurrent workers, 2 domains)
oi sentinel start --concurrency 4 --domains "rust-lang.org,tokio.rs"

# Add more domains on the fly
oi sentinel add-domain docs.rs

# Subscribe to an RSS feed
oi sentinel add-feed https://blog.rust-lang.org/feed.xml

# Check crawl stats
oi sentinel status
```

Quick demo script that crawls a domain for 30 seconds and searches it:

```bash
bash scripts/run-node.sh rust-lang.org "rust programming"
```

---

## 7. P2P network node

Run a libp2p node to join the decentralized search network:

```bash
# Start a node on port 4001 with HTTP API on 8001
oi network node --port 4001 --http-port 8001

# Join an existing network by bootstrapping
oi network node --port 4001 --bootstrap /ip4/1.2.3.4/tcp/4001/p2p/12D3KooW...

# Start with integrated sentinel crawler
oi network node --port 4001 --http-port 8001 --sentinel --domains "rust-lang.org,tokio.rs"

# Search through the running node
oi search "rust async" --node http://localhost:8001

# Health check
curl http://localhost:8001/health
```

P2P uses libp2p with Kademlia DHT for peer discovery, gossipsub for broadcasting, and a custom request-response protocol (`/opensonarx/query/1.0.0`) for search queries.

---

## 8. Docker Compose

```bash
docker compose up
# Services:
#   P2P Node -> localhost:4001 (P2P), localhost:8001 (HTTP API)
```

---

## 9. Embedding models

The SDK defaults to **MiniLM** (Snowflake Arctic Embed S, 384-dim, INT8 quantized via ONNX). The model downloads automatically on first run (~30MB).

| Model | Feature flag | Dimensions | Use case |
|-------|-------------|-----------|----------|
| MiniLM | `onnx` (default) | 384 | Text search, production use |
| CLIP | `clip` | 512 | Image + text multimodal search |

---

## 10. CLI config file

The CLI reads persistent settings from `~/.opensonarx/config.toml`:

```toml
[network]
bootstrap_peers = ["/ip4/1.2.3.4/tcp/4001/p2p/12D3KooW..."]

[[search.engines]]
name = "google"
priority = 1
api_key = "your-serper-key"
```

CLI flag precedence: **CLI flags > config file > defaults**.

---

## 11. Running tests

```bash
# All tests
cargo test

# Specific crate
cargo test -p oi-index
cargo test -p oi-sdk
cargo test -p oi-network

# Lint
cargo clippy --all
```
