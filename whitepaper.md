# OpenSonarX: A Decentralized, Stake-Weighted Search Protocol for AI Agents

**Version 1.1 — February 2026**

**Authors:** OpenSonarX Core Team

---

> *"Stake truth, burn spam, earn trust."*

---

## Abstract

The proliferation of large language models (LLMs) and autonomous AI agents has created urgent demand for a search infrastructure layer that is accurate, spam-resistant, and economically aligned with content quality rather than advertising revenue. Existing search engines optimize for human click-through rates and ad placement; they are ill-suited to serve machine consumers that require factual, verifiable, and up-to-date information at API speed.

OpenSonarX is a decentralized search protocol in which publishers stake **$TRUTH** tokens to vouch for the quality of their content. Staked content is indexed in a hybrid vector-and-keyword search engine, distributed across a peer-to-peer network, and ranked by a multi-signal formula that rewards relevance, economic commitment, and community reputation. A system of sentinels, disputes, and commit-reveal juries enforces quality standards on-chain, slashing dishonest actors and burning tokens to maintain long-term deflation. Governance is fully on-chain, with time-locked proposals and anti-flash-loan protections.

This paper presents the protocol architecture, the economic model, the governance framework, the search algorithm, and the formal security properties of OpenSonarX.

---

## Table of Contents

1. [Introduction & Motivation](#1-introduction--motivation)
2. [Protocol Overview](#2-protocol-overview)
3. [System Architecture](#3-system-architecture)
4. [Hybrid Search Engine](#4-hybrid-search-engine) — HNSW, BM25, Fusion, Embeddings, Reranking, SimHash, Diversity
5. [Crawl & Content Pipeline](#5-crawl--content-pipeline) — Distillation, Quality Gate, Discovery, Frontier, Adapters, Spider, Daemon, Drift, Facts
6. [Peer-to-Peer Network](#6-peer-to-peer-network) — Transport, Gossip, Query Protocol, Fanout, Replication
7. [Staking & Economic Model](#7-staking--economic-model)
8. [Dispute Resolution & Slashing](#8-dispute-resolution--slashing)
9. [State Channels & Micropayments](#9-state-channels--micropayments)
10. [Governance (DAO)](#10-governance-dao)
11. [Token Supply & Emission Schedule](#11-token-supply--emission-schedule)
12. [Ranking Formula — Formal Specification](#12-ranking-formula--formal-specification)
13. [Security Analysis](#13-security-analysis)
14. [Roadmap](#14-roadmap)
15. [Conclusion](#15-conclusion)
16. [Appendix A — Protocol Parameters](#appendix-a--protocol-parameters)
17. [Appendix B — Error Code Taxonomy](#appendix-b--error-code-taxonomy)
18. [Appendix C — Wire Protocol (Protobuf)](#appendix-c--wire-protocol-protobuf)

---

## 1. Introduction & Motivation

### 1.1 The Problem

Modern web search was designed for humans browsing the web. Revenue flows from advertisers, not from the quality of information returned. This misalignment produces three systemic failures:

1. **Ad-driven ranking distortion.** Search engines optimize for engagement and ad revenue, not factual accuracy. Results that generate clicks are promoted over results that provide correct answers.

2. **AI slop and SEO spam.** The cost of producing low-quality, machine-generated content has collapsed. Search indexes are increasingly polluted with formulaic, keyword-stuffed pages that game ranking algorithms but provide no genuine informational value.

3. **Opaque, centralized gatekeeping.** A small number of corporations control which content is discoverable. Publishers have no verifiable, permissionless mechanism to signal content quality or earn ranking on merit.

### 1.2 The Opportunity

LLMs and AI agents are rapidly becoming the primary consumers of web information. Unlike human users, these machine consumers do not click ads, do not respond to engagement bait, and require structured, accurate, and citation-worthy content. They need infrastructure — not advertisements.

### 1.3 The OpenSonarX Thesis

OpenSonarX introduces an economic primitive — **stake-weighted search** — to align incentives across publishers, curators, quality enforcers, and AI consumers:

- **Publishers** stake $TRUTH tokens on their domains, creating a verifiable economic bond. Quality content earns staking rewards; spam risks slashing.
- **Sentinels** monitor content quality, file disputes against bad actors, and earn protocol fees for enforcement.
- **AI Agents** pay for search results through state channels, with a portion of every payment burned to make spam economically irrational.
- **Governance** is on-chain, with all protocol parameters adjustable by token-weighted voting subject to timelocks and quorum requirements.

The core thesis: **quality content is profitable; spam is unprofitable.** The protocol enforces this through staking, slashing, burning, and decayed emissions that transition the network from subsidy-driven growth to a self-sustaining fee economy over approximately three years.

---

## 2. Protocol Overview

```
┌─────────────────────────────────────────────────────────────────┐
│                        AI Agent / LLM                           │
│              Queries the network, pays via state channels        │
└──────────────────────────┬──────────────────────────────────────┘
                           │ HTTPS / libp2p
                           ▼
┌─────────────────────────────────────────────────────────────────┐
│                      OpenSonarX Gateway                          │
│    REST API · Magic-Link Auth · Quota · Billing · Blended Search│
└──────────┬──────────────────────────────────┬───────────────────┘
           │                                  │
     ┌─────▼──────┐                    ┌──────▼──────┐
     │  oi-sdk     │                    │ oi-network  │
     │  Core SDK   │                    │ libp2p P2P  │
     │  + Gateway  │                    │ Kad + Gossip│
     │  + Auth     │                    │ + Fanout    │
     │ ┌─────────┐ │                    └─────────────┘
     │ │oi-index │ │
     │ │HNSW+BM25│ │
     │ │+Reranker│ │
     │ │+SimHash │ │
     │ └─────────┘ │
     │ ┌─────────┐ │
     │ │oi-embed │ │
     │ │MiniLM   │ │
     │ │Arctic M/L│ │
     │ │CLIP+Rnk │ │
     │ └─────────┘ │
     │ ┌─────────┐ │
     │ │oi-crawl │ │
     │ │Distiller│ │
     │ │+Spider  │ │
     │ │+Daemon  │ │
     │ └─────────┘ │
     │ ┌─────────┐ │
     │ │oi-facts │ │
     │ │GLiNER   │ │
     │ └─────────┘ │
     └──────┬──────┘
            │ Solana RPC
            ▼
┌─────────────────────────────────────────────────────────────────┐
│                    Solana Blockchain                             │
│   $TRUTH Token · Staking Program · Disputes · Governance        │
│   State Channels · Sentinel Registry · Emission Controller      │
└─────────────────────────────────────────────────────────────────┘
```

**Figure 1.** High-level protocol architecture. AI agents query the gateway or P2P network directly. The SDK orchestrates hybrid search, embedding, entity extraction, and crawling. The gateway client provides REST access with magic-link authentication and quota management. All economic state (staking, disputes, governance, payments) is settled on Solana.

---

## 3. System Architecture

### 3.1 Crate Topology

OpenSonarX is implemented as a Rust workspace with nine crates, each responsible for a single concern:

| Crate | Responsibility |
|-------|---------------|
| **oi-types** | Shared types, traits, error codes (E1000–E6012). All other crates depend on this. |
| **oi-index** | HNSW (vector) + BM25 (keyword) hybrid search engine with multi-pass ranking, SimHash near-duplicate detection, cross-encoder reranking, and result diversity enforcement. |
| **oi-embed** | Embedding pipelines: MiniLM (Snowflake Arctic Embed S, 384-dim, INT8 ONNX), Snowflake Arctic Embed M and L (feature-gated), CLIP (512-dim, feature-gated), cross-encoder reranker (feature-gated), and batch inference. |
| **oi-facts** | Entity and fact extraction via GLiNER (ONNX, feature-gated). Extracts named entities and structured facts at crawl time for entity-boosted ranking and structured search. |
| **oi-crawl** | Content distillation pipeline: fetcher, HTML→Markdown extractor, quality gate (spam + slop detection), chunker, embedder. Includes a BFS spider, priority-based URL frontier, content drift detection, RSS/Atom/sitemap discovery engines, platform adapters (YouTube, Reddit, etc.), and a sentinel crawl daemon for continuous background indexing. |
| **oi-network** | libp2p networking: Kademlia DHT, gossipsub pub/sub, custom request-response query protocol, distributed query fanout with local+remote result merging. |
| **oi-staking** | Solana Anchor program: stake, unstake, disputes, jury voting, governance, state channels, emissions. |
| **oi-sdk** | SDK core that wires index + embedder + staking + crawl into a unified client. Includes a REST gateway client, magic-link authentication, and a query leaderboard. |
| **oi-cli** | Command-line binary (`oi`): search, stake, sentinel, seed (docs-rs, MDN, custom sitemaps), wallet, dispute, governance, feedback, network (P2P node with HTTP API). |

### 3.2 Data Flow

A search query traverses the following path:

```
Query ("rust async programming")
  │
  ▼
[1] Embedding ─── MiniLM (384-dim, INT8 quantized)
  │                Query prefix: "Represent this sentence for
  │                searching relevant passages: "
  ▼
[2] Hybrid Search
  │  ├── HNSW vector search (ef_search=50, pool=top_k×2)
  │  └── BM25 keyword search (k1=1.2, b=0.75)
  │
  ▼
[3] Score Fusion ─── Weighted sum: 0.7·semantic + 0.3·BM25
  │                  (or Reciprocal Rank Fusion, k=60)
  ▼
[4] Filter Pass ─── content_type, freshness, entities, site
  │
  ▼
[5] Cross-Encoder Rerank (optional)
  │  └── Reranker rescores top candidates using full query-document
  │      attention (when a Reranker is configured)
  │
  ▼
[6] Diversity Enforcement
  │  └── Max 3 results per domain, max 2 per URL
  │
  ▼
[7] Stake & Reputation Enrichment
  │  ├── Batch lookup domain/entity stakes from Solana
  │  ├── Wilson score from accumulated feedback
  │  └── UGC platform passthrough = 0 (entity-only staking)
  │
  ▼
[8] Final Ranking ─── score = base × stake_boost × reputation
  │                           × quality × freshness × dns
  ▼
[9] Response ─── Ranked results with scores, stake info,
                 content hashes, provenance metadata, and
                 per-stage timing breakdown
```

**Figure 2.** Query processing pipeline from embedding through multi-pass ranking.

---

## 4. Hybrid Search Engine

### 4.1 Design Rationale

Pure vector search captures semantic meaning but misses exact keyword matches. Pure BM25 captures lexical relevance but fails on synonyms and paraphrases. OpenSonarX combines both in a hybrid architecture that empirically outperforms either method alone.

Parameter sweep results on the BEIR benchmark showed that a 70/30 semantic-to-BM25 weighting achieved the best Recall@10 among tested configurations.

### 4.2 HNSW Vector Index

The vector index implements Hierarchical Navigable Small World (HNSW) graphs with the following parameters:

| Parameter | Value | Description |
|-----------|-------|-------------|
| M | 16 | Maximum bidirectional connections per node per layer |
| M_max0 | 32 | Maximum connections on the ground layer (layer 0) |
| ef_construction | 200 | Beam width during index construction |
| ef_search | 50 | Beam width during query-time search |
| MAX_LEVEL | 16 | Maximum number of hierarchical layers |
| level_mult | 1/ln(M) | Probabilistic level assignment multiplier |

Level assignment for each new node follows a geometric distribution:

$$\ell = \lfloor -\ln(U) \cdot m_L \rfloor, \quad U \sim \text{Uniform}(0,1), \quad m_L = \frac{1}{\ln(M)}$$

where M = 16 and the expected number of layers scales logarithmically with the corpus size.

### 4.3 BM25 Keyword Index

The keyword index implements Okapi BM25 with standard parameters:

$$\text{BM25}(q, d) = \sum_{t \in q} \text{IDF}(t) \cdot \frac{tf(t,d) \cdot (k_1 + 1)}{tf(t,d) + k_1 \cdot \left(1 - b + b \cdot \frac{|d|}{\text{avgdl}}\right)}$$

where:

$$\text{IDF}(t) = \ln\left(\frac{N - n(t) + 0.5}{n(t) + 0.5} + 1\right)$$

Parameters: k₁ = 1.2, b = 0.75. Tokenization: lowercase, split on non-alphanumeric boundaries, filter tokens with length ≤ 1.

### 4.4 Score Fusion

Two fusion methods are supported:

**Weighted Sum (default):**

$$S_{\text{fused}} = w_s \cdot \hat{s}_{\text{vec}} + w_b \cdot \hat{s}_{\text{bm25}}$$

where ŝ denotes min-max normalized scores, w_s = 0.7, w_b = 0.3.

**Reciprocal Rank Fusion (RRF):**

$$S_{\text{RRF}} = \frac{1}{k + r_{\text{vec}}} + \frac{1}{k + r_{\text{bm25}}}$$

where k = 60 (smoothing constant) and r denotes the rank position in each retrieval list.

### 4.5 Embedding Model

| Property | Value |
|----------|-------|
| Model | Snowflake Arctic Embed S |
| Dimensions | 384 |
| Parameters | 33M |
| Quantization | INT8 (ONNX Runtime) |
| Pooling | CLS token |
| Max sequence length | 256 tokens |
| nDCG@10 (BEIR) | 51.98 |
| Query prefix | "Represent this sentence for searching relevant passages: " |

**Additional Embedding Models (feature-gated):**

| Model | Feature Flag | Dimensions | Use Case |
|-------|-------------|------------|----------|
| Snowflake Arctic Embed M | `arctic-m` | 768 | Higher-quality retrieval for larger indexes |
| Snowflake Arctic Embed L | `arctic-l` | 1024 | Maximum retrieval quality |
| CLIP | `clip` | 512 (projected to 384) | Multimodal text + image unified search |

All ONNX models share a configurable thread pool (default: min(4, available_cores), overridable via `OI_THREADS` env var).

### 4.6 Cross-Encoder Reranking

When a cross-encoder reranker is configured (feature flag `reranker`), the top candidates from score fusion are rescored using full query-document attention. Unlike bi-encoder embeddings (which encode query and document independently), the cross-encoder jointly attends to both, producing more accurate relevance scores at the cost of higher latency. Reranking is applied after fusion and filtering but before stake enrichment, and its execution time is tracked in the per-query timing breakdown.

### 4.7 Near-Duplicate Detection (SimHash)

At ingest time, the index performs two levels of duplicate detection:

1. **Exact deduplication:** SHA-256 content hashes reject byte-identical documents.
2. **Near-duplicate detection:** 64-bit SimHash fingerprints computed from character trigrams. Two documents with Hamming distance ≤ 8 (out of 64 bits) are considered near-duplicates and rejected. The `SimHashIndex` uses band-based Locality-Sensitive Hashing (LSH) for O(1) average-case duplicate lookups rather than O(n) linear scan.

### 4.8 Result Diversity

To prevent a single source from dominating results, the index enforces per-query diversity limits:

- **MAX_PER_DOMAIN = 3** — at most 3 results from any single domain.
- **MAX_PER_URL = 2** — at most 2 results from any single URL (prevents one large page, e.g., release notes, from consuming all domain slots).

---

## 5. Crawl & Content Pipeline

### 5.1 Pipeline Stages

```
URL Queue (BFS Frontier)
  │
  ▼
[1] Fetcher ─── HTTP client, robots.txt compliance
  │
  ▼
[2] Extractor ─── HTML → Markdown, date parsing, content type inference
  │
  ▼
[3] Quality Gate ─── Spam detection + AI slop detection + length checks
  │                  Reject if spam_score > 0.7 or slop_score > 0.7
  ▼
[4] Hasher ─── SHA-256 of markdown content (deduplication, provenance)
  │
  ▼
[5] Chunker ─── Token-based splitting: max_tokens=512, overlap=128
  │
  ▼
[6] Embedder ─── MiniLM 384-dim INT8 vectors per chunk
  │
  ▼
[7] Indexer ─── Insert into HNSW + BM25 hybrid index
```

**Figure 3.** Content distillation pipeline from URL to indexed, searchable chunks.

### 5.2 Quality Gate

The quality gate implements three independent detectors that produce scores in [0, 1]. Content is rejected if any score exceeds 0.7.

**Spam Detection** (three averaged signals):

1. **Keyword density:** If any single non-stopword exceeds 10% frequency:

$$s_1 = \min\left(\frac{f_{\max} - 0.10}{0.40},\ 1.0\right)$$

2. **Trigram repetition:** If any trigram appears more than 3 times:

$$s_2 = \min\left(\frac{c_{\max} - 3}{10},\ 1.0\right)$$

3. **Capitalization ratio:** If uppercase characters exceed 30%:

$$s_3 = \frac{r_{\text{caps}} - 0.30}{0.70}$$

$$\text{spam\_score} = \frac{s_1 + s_2 + s_3}{3}$$

**AI Slop Detection** (three averaged signals):

1. **Formulaic patterns:** A dictionary of 20 known LLM-generation markers ("in today's rapidly evolving", "game-changer", "paradigm shift", "synergy", etc.):

$$s_1 = \min\left(\frac{\text{hits}}{3},\ 1.0\right)$$

2. **Vocabulary diversity:** Ratio of unique words to total words:

$$s_2 = \begin{cases} 1 - \frac{d}{0.3} & \text{if } d < 0.3 \\ 0 & \text{otherwise} \end{cases}$$

3. **Sentence length uniformity:** Coefficient of variation of sentence lengths:

$$s_3 = \begin{cases} 1 - \frac{\text{cv}}{0.2} & \text{if cv} < 0.2 \\ 0 & \text{otherwise} \end{cases}$$

$$\text{slop\_score} = \frac{s_1 + s_2 + s_3}{3}$$

**Quality Score:**

$$Q = \min\left(\frac{w}{500},\ 1.0\right)$$

where w = word count. Content quality profiles impose minimum character lengths (Standard: 100, SocialPost: 20, VideoDescription: 30) and maximum link ratios (80%, 95%, 90% respectively).

### 5.3 Content Discovery Engines

The crawl pipeline supports three discovery methods for finding new URLs to crawl:

- **RSS/Atom feeds** — Parses both RSS 2.0 and Atom feeds to discover new content from subscribed sources. Feed entries include publication dates for freshness-aware scheduling.
- **Sitemap parsing** — Extracts URLs from `sitemap.xml` files, including `<lastmod>` timestamps for change detection.
- **Link extraction** — Follows same-domain HTML links discovered during page crawling.

Discovery source affects crawl priority: Feed URLs receive the highest priority (3×), followed by Sitemap (2×), then Link-discovered URLs (1×).

### 5.4 URL Frontier

URLs are scheduled for crawling via a priority queue (the *frontier*) that orders URLs by a composite priority score incorporating:

- **Domain stake** — higher-staked domains are crawled first.
- **Discovery source** — feeds and sitemaps take precedence over links.
- **Recency** — recently added URLs are prioritized within the same priority tier.

The frontier enforces per-domain rate limits and is optionally backed by SQLite for persistence across restarts.

### 5.5 Platform Adapters

For user-generated content (UGC) platforms, platform-specific adapters implement a `PlatformAdapter` trait that handles:

- **Entity extraction** — Mapping URLs to entity references (e.g., YouTube channels, subreddits).
- **Content discovery** — Fetching content via platform-specific RSS feeds, JSON APIs, or HTML scraping.
- **Structured extraction** — Producing normalized `CrawlOutput` regardless of the source platform.

Adapters exist for YouTube, Reddit, and other major UGC platforms. UGC content receives zero domain-stake passthrough (only entity-level stakes earn rewards), as documented in Section 7.

### 5.6 BFS Spider

The `Spider` performs one-shot site crawling via breadth-first search from seed URLs. Configurable parameters:

| Parameter | Default | Description |
|-----------|---------|-------------|
| `max_depth` | 3 | Maximum link-follow depth from seed URLs |
| `max_pages` | 1,000 | Maximum pages to crawl per spider run |
| `concurrency` | 4 | Number of concurrent crawl workers |
| `respect_robots` | true | Whether to obey robots.txt |
| `same_domain_only` | true | Only follow links on the same domain |

### 5.7 Sentinel Crawl Daemon

The sentinel daemon is a continuous background process that discovers and indexes content from staked domains. It:

- Polls RSS/Atom feeds and homepage links on configurable intervals.
- Enforces per-domain and per-epoch crawl budgets (`max_urls_per_domain`, `max_urls_per_epoch`).
- Emits `CrawlEvent` messages (`ContentDiscovered`, `ContentUpdated`) for the network layer to broadcast via gossip.

### 5.8 Content Drift Detection

The daemon detects when previously crawled content has changed:

- Re-fetches pages periodically and computes new SHA-256 hashes.
- Compares against stored hashes; if different, compares embedding vectors via cosine similarity.
- Pages with significant content drift are re-indexed and a `ContentUpdated` event is broadcast.

### 5.9 Entity & Fact Extraction (oi-facts)

When the `oi-facts` crate is enabled (feature-gated behind `onnx`), the pipeline extracts named entities and structured facts from crawled content using a GLiNER model (ONNX Runtime). Extracted entities are:

- Normalized and deduplicated.
- Stored alongside chunk metadata for entity-boosted ranking (see Section 12).
- Used for structured search filters (e.g., filtering results by entity type or name).

---

## 6. Peer-to-Peer Network

### 6.1 Transport Stack

| Layer | Technology |
|-------|-----------|
| Transport | TCP |
| Encryption | Noise (XX handshake) |
| Multiplexing | Yamux |
| Identity | Ed25519 keypairs |
| Discovery | Kademlia DHT (memory-backed) |
| Pub/Sub | Gossipsub |
| Query | Custom request-response (`/opensonarx/query/1.0.0`) |

### 6.2 Gossip Topics

Nodes subscribe to three gossip topics for protocol coordination:

- `/opensonarx/heartbeat` — Node liveness, shard counts, query throughput, uptime percentage (30-day rolling).
- `/opensonarx/content-announce` — Sentinel announces new crawled content (URL, domain, SHA-256 hash, chunk count, shard ID).
- `/opensonarx/stake-events` — On-chain stake/unstake/slash events broadcast for local cache invalidation.

### 6.3 Query Protocol

Distributed search uses the `/opensonarx/query/1.0.0` request-response protocol:

```
┌──────────┐    QueryRequest (protobuf)     ┌──────────┐
│  Client  │ ──────────────────────────────► │  Node    │
│          │                                 │          │
│          │ ◄────────────────────────────── │          │
└──────────┘    QueryResponse (protobuf)     └──────────┘
                + StateChannelTicket
```

QueryRequest carries a 384-byte INT8 embedding, filters, and an optional signed state channel ticket for payment. The response includes ranked results with full provenance metadata (content hashes, stake info, extraction quality).

### 6.4 Distributed Query Fanout

When a node receives a query it cannot fully answer from its local index (e.g., it holds only a subset of shards), the query is fanned out to remote peers:

1. The node executes the query against its local HNSW+BM25 index.
2. It simultaneously forwards the `QueryRequest` to peers known to hold relevant shards (discovered via `ShardAnnounce` gossip).
3. Remote peers return their local `QueryResponse` results.
4. The originating node merges local and remote results, deduplicates, and re-ranks the combined set before returning the final response.

The `PendingFanout` struct tracks in-flight distributed queries, including expected/received remote responses and a creation timestamp for timeout handling.

### 6.5 Shard Replication

Large indexes are sharded across nodes. The `ShardAnnounce` gossip message advertises which shards a node holds, its vector count, and its last sync block. The `ReplicationReq`/`ReplicationResp` protocol enables bulk shard transfer with per-document chunks (content, INT8 embedding, metadata, content hash, sequence number). Each ingested chunk receives a monotonically increasing sequence number for incremental replication — peers can request only chunks newer than their last sync point.

---

## 7. Staking & Economic Model

### 7.1 Participant Roles

The protocol defines three staking roles with distinct incentives and risk profiles:

#### 7.1.1 Publishers

Publishers stake $TRUTH on domains they control (e.g., `example.com`). This stake serves as an economic bond vouching for content quality.

- **Earns:** Pro-rata share of 70% of epoch emissions.
- **Risk:** Graduated slashing if the domain is disputed and found guilty.
- **Constraint:** Minimum stake enforced (`config.min_stake`). DNS verification available for enhanced ranking (+5% bonus).
- **Lock:** Cannot unstake during active disputes. 7-day cooldown after initiating unstake.

#### 7.1.2 Curators

Curators stake on domains they do not own, acting as decentralized quality signals.

- **Earns:** Pro-rata share of staker emissions, capped at 15% APY (1500 bps) to prevent whale-gaming of the emission pool.
- **Risk:** Same slashing exposure as publishers on their staked domains.
- **UGC Platforms:** Curators receive zero passthrough on user-generated content platforms (YouTube, Reddit, X, etc.). Only entity-level stakes (e.g., individual channels) earn rewards.

#### 7.1.3 Sentinels

Sentinels are quality enforcement agents. They stake into a global sentinel pool (not domain-specific) and earn rewards for monitoring the network.

- **Earns:** 30% of epoch emissions (pro-rata by sentinel stake) + 5% of state channel settlement fees + flat jury vote rewards.
- **Powers:** Can file disputes against domains, triggering commit-reveal jury voting.
- **Trusted Reporters:** Sentinels with stake ≥ 10× `jury_min_stake` receive a 50% discount on dispute bonds.
- **Constraint:** Minimum stake of `config.jury_min_stake`.

### 7.2 Staking Mechanics (On-Chain)

All staking state is managed by a Solana Anchor program. Key program-derived addresses (PDAs):

| PDA | Seeds | Purpose |
|-----|-------|---------|
| ProtocolConfig | `["config"]` | Global protocol parameters, emission state, reward indices |
| StakeAccountV2 | `["stake", domain, staker]` | Per-staker-per-domain position |
| DomainRecord | `["domain", domain]` | Aggregate domain state (total staked, publisher/curator counts, blacklist flag) |
| SentinelAccount | `["sentinel", pubkey]` | Per-sentinel stake position |
| StateChannelAccount | `["channel", payer, payee]` | Bidirectional payment channel |
| DisputeAccount | `["dispute", domain, nonce]` | Active dispute state, jury votes, severity |
| GovernanceProposal | `["proposal", nonce]` | DAO proposal with vote tallies and timelock |

### 7.3 Reward Distribution

Rewards are distributed via a global reward index pattern (similar to Synthetix StakingRewards), using 10^18 fixed-point precision to avoid rounding errors:

$$R_{\text{per\_token}} = R_{\text{per\_token}}^{\text{prev}} + \frac{\Delta_{\text{rewards}}}{\text{total\_staked}}$$

Each staker's claimable reward is:

$$\text{claimable}_i = \text{stake}_i \times (R_{\text{per\_token}} - \text{debt}_i)$$

where debt_i is set to the current R_per_token at the time of staking or last claim. An analogous index exists for sentinels.

### 7.4 Emission Split

```
UpdateRewardIndex (permissionless, anyone can call)
  │
  │  Caps at: min(requested, distributable, epoch_budget, supply_headroom)
  │
  ├──── 70% ────► Staker Reward Index
  │               Divided proportionally by total_staked
  │
  └──── 30% ────► Sentinel Reward Index
        + fees    Divided proportionally by total_sentinel_staked
```

**Safety:** If no stakers or sentinels exist, emissions are not counted against the supply cap — they remain available for later distribution.

---

## 8. Dispute Resolution & Slashing

### 8.1 Dispute Lifecycle

```
┌─────────────┐
│  Sentinel    │
│  files       │──── Posts dispute_bond as collateral
│  dispute     │     (50% discount for trusted reporters)
└──────┬──────┘
       │
       ▼
┌─────────────────────────────────┐
│  Commit Phase                   │
│  Jurors submit hash(vote|salt)  │
│  Eligibility: stake ≥ min +    │
│  7-day stake age                │
└──────┬──────────────────────────┘
       │
       ▼
┌─────────────────────────────────┐
│  Reveal Phase                   │
│  Jurors reveal vote + salt      │
│  Must match committed hash      │
└──────┬──────────────────────────┘
       │
       ▼
┌─────────────────────────────────┐
│  Resolution                     │
│  Quorum check: total jury       │
│  weight ≥ min_jury_weight       │
│  (default: 1,000 $TRUTH)        │
└──────┬──────────────────────────┘
       │
       ├── GUILTY ──────────────────────────────────────┐
       │                                                │
       │   Graduated Slashing:                          │
       │     Low severity:   50% × slash_pct            │
       │     Medium severity: 100% × slash_pct          │
       │     High severity:  150% × slash_pct           │
       │                                                │
       │   Slashed tokens:                              │
       │     50% burned (deflationary)                  │
       │     50% to protocol revenue vault              │
       │                                                │
       │   Reporter: full bond returned                 │
       │                                                │
       └── INNOCENT ────────────────────────────────────┐
                                                        │
           Reporter loses bond (deters spam disputes)   │
           Bond stays in vault as protocol revenue      │
           Domain stakers unaffected                    │
```

**Figure 4.** Dispute resolution flow with commit-reveal jury voting.

### 8.2 Jury Incentives

Jury rewards are designed to be neutral — jurors earn a **flat fee** (`jury_vote_reward`, minted from supply) per winning vote, regardless of the verdict outcome. This eliminates profit motive from predatory slashing.

- Per-epoch jury reward cap: `max_jury_rewards_per_epoch` (default: 50,000 $TRUTH)
- This prevents jury farming through fabricated disputes.

### 8.3 Anti-Frontrunning

Stakers **cannot unstake** during an active dispute against their domain (`governance_lock_until` set to dispute deadline). This prevents front-running slashing by withdrawing early.

---

## 9. State Channels & Micropayments

### 9.1 Channel Architecture

AI agents pay for search results through bidirectional state channels on Solana, enabling high-throughput micropayments without per-query on-chain transactions.

```
AI Agent (payer) ◄──── state channel ────► Publisher Node (payee)

OpenChannel:   Payer deposits $TRUTH into channel PDA
Queries:       Off-chain signed tickets (monotonic nonce, amount, expiry)
SettleChannel: Either party submits final ticket on-chain
```

### 9.2 Settlement Fee Structure

On settlement, the channel payment is split:

| Allocation | Percentage | Purpose |
|-----------|-----------|---------|
| **Burned** | 10% (configurable via `burn_pct`) | Deflationary pressure; makes spam unprofitable |
| **Sentinel fee** | 5% (configurable via `sentinel_fee_pct`) | Accumulated into sentinel reward index |
| **Payee** | Remainder (85%) | Publisher/node operator revenue |

**Why burn?** AI agents pay for search results. Burning a portion of every payment ensures that spam operators spend more than they earn — the cost of staking plus the burn on settlements exceeds any revenue from serving low-quality content.

### 9.3 Ticket Format

```protobuf
message StateChannelTicket {
  bytes  payer_pubkey    = 1;  // 32 bytes Ed25519
  bytes  payee_pubkey    = 2;  // 32 bytes Ed25519
  uint64 amount_lamports = 3;  // cumulative spend
  uint64 nonce           = 4;  // monotonically increasing
  bytes  signature       = 5;  // Ed25519 over fields 1-4
  uint64 expiry_slot     = 6;  // Solana slot deadline
}
```

Replay protection is enforced by the monotonic nonce — the on-chain program only accepts tickets with a nonce strictly greater than the last settled nonce.

---

## 10. Governance (DAO)

### 10.1 Proposal Lifecycle

```
[1] Create Proposal
    │  Proposer deposits proposal_deposit (100 $TRUTH default)
    │  Specifies: param_key, param_value, description_hash
    │
    ▼
[2] Voting Period (24 hours)
    │  Vote weight = staked amount
    │  Eligibility: 7-day stake age (anti-flash-loan)
    │  Voting locks ALL positions (per-wallet lock)
    │
    ▼
[3] Execution Timelock (24 hours default, min 1 hour)
    │  Rage-quit window: stakers can exit before changes take effect
    │
    ▼
[4] Execution
    │  Parameter updated on-chain
    │  Proposal deposit refunded to proposer
    │
   [OR]
    │
    ▼
[4'] Failure / Cancellation
    Deposit forfeited (anti-spam)
```

**Figure 5.** Governance proposal lifecycle.

### 10.2 Governable Parameters

All critical protocol parameters are adjustable through governance, subject to bounded ranges that prevent zeroing safety mechanisms:

| Parameter | Default | Min | Max | Description |
|-----------|---------|-----|-----|-------------|
| `burn_pct` | 10% | 0% | 100% | Channel settlement burn rate |
| `sentinel_fee_pct` | 5% | 0% | 50% | Channel fee to sentinels |
| `slash_pct` | configurable | 5% | 100% | Base slash rate (graduated by severity) |
| `staker_emission_pct` | 70% | 0% | 100% | Staker share of emissions |
| `curator_yield_cap_bps` | 1500 | 0 | 10000 | 15% APY cap for curators |
| `execution_delay_secs` | 86,400 | 3,600 | 2,592,000 | Timelock: 24h default (min 1h, max 30d) |
| `proposal_deposit` | 100 | 0 | 1B | Anti-spam deposit |
| `min_jury_weight` | 1,000 | 0 | 10B | Minimum jury quorum |
| `max_total_supply` | 1,000,000,000 | ≥ total_minted | — | Hard supply cap |
| `jury_vote_reward` | 1,000 | 0 | 1B | Flat reward per winning juror |
| `max_jury_rewards_per_epoch` | 50,000 | 0 | 1B | Per-epoch jury reward budget |
| `min_epoch_duration` | 604,800 | 0 | 31,536,000 | 7 days between epoch advances |

### 10.3 Anti-Governance-Attack Measures

1. **7-day stake age for voting** — prevents flash-loan governance attacks where an attacker borrows tokens, votes, and returns them in the same block.
2. **24-hour execution timelock** (minimum 1 hour, cannot be zeroed) — gives the community time to react to malicious proposals and exit positions.
3. **Per-wallet governance lock** — voting locks all staking positions until the vote period ends, preventing vote-then-dump strategies.
4. **Proposal deposit** (100 $TRUTH, forfeited on cancellation) — prevents spam proposals.
5. **Parameter floors** — critical safety parameters (slash_pct ≥ 5%, execution_delay ≥ 1h) cannot be zeroed through governance.

---

## 11. Token Supply & Emission Schedule

### 11.1 Supply Cap

$TRUTH has a hard maximum supply of **1,000,000,000 tokens** (1 billion), enforced on every mint instruction. This cap is itself governable (can be raised or lowered, but never below total_minted).

### 11.2 Emission Decay

Emissions follow a geometric decay of 1.5% per epoch (minimum 7-day epochs):

$$E_n = E_0 \times (1 - \delta)^n$$

where E₀ = 10,000,000 $TRUTH and δ = 0.015 (1.5% decay per epoch).

| Epoch | Time | Max Emission per Epoch |
|-------|------|----------------------|
| 0 | Week 0 | 10,000,000 |
| 1 | Week 1 | 9,850,000 |
| 52 | ~Year 1 | ~4,557,000 |
| 104 | ~Year 2 | ~2,077,000 |
| 156 | ~Year 3 | ~946,000 |
| 208 | ~Year 4 | ~431,000 |
| ∞ | — | → 0 |

### 11.3 Cumulative Supply Curve

The total emitted supply after n epochs (assuming full distribution each epoch):

$$S_n = E_0 \sum_{k=0}^{n-1}(1-\delta)^k = E_0 \cdot \frac{1 - (1-\delta)^n}{\delta}$$

Theoretical maximum cumulative emission (n → ∞):

$$S_\infty = \frac{E_0}{\delta} = \frac{10{,}000{,}000}{0.015} = 666{,}666{,}667 \text{ } \$TRUTH$$

This leaves 333,333,333 $TRUTH of supply headroom under the 1B cap for treasury, grants, and future governance decisions.

### 11.4 Deflationary Forces

Two mechanisms create deflationary pressure that counteracts emissions:

1. **Channel settlement burn:** 10% of every AI agent payment is burned permanently.
2. **Slash burn:** 50% of slashed tokens from guilty dispute verdicts are burned.

The protocol transitions from emission-driven (early) to fee-driven (mature) economics over approximately 3 years:

```
Phase 1 (Year 0-1):  High emissions attract stakers → builds index quality
                      Low burn rate (10%) attracts publishers
                      5% sentinel fee bootstraps quality enforcement

Phase 2 (Year 1-2):  Decaying emissions create scarcity → token appreciates
                      Governance can raise burn/sentinel fees as network grows

Phase 3 (Year 2-3):  Channel fees sustain sentinels → quality maintained
                      Emissions become marginal

Phase 4 (Year 3+):   Zero-emission, fully fee-driven economy
                      Ongoing burns maintain deflationary pressure
```

---

## 12. Ranking Formula — Formal Specification

The final ranking score for a search result d against query q is computed as:

$$\text{Score}(q, d) = S_{\text{base}} \times B_{\text{stake}} \times R \times Q \times F \times D$$

where each factor is defined below.

### 12.1 Base Relevance Score

$$S_{\text{base}} = w_s \cdot \hat{s}_{\text{vec}}(q, d) + w_b \cdot \hat{s}_{\text{bm25}}(q, d)$$

with w_s = 0.7, w_b = 0.3, and hat denoting min-max normalization within the candidate set.

### 12.2 Stake Boost

$$B_{\text{stake}} = \begin{cases} 1 + 0.1 \cdot \log_2(\sigma_{\text{eff}}) & \text{if } \sigma_{\text{eff}} \geq 1 \\ 1.0 & \text{otherwise} \end{cases}$$

capped at B_stake ≤ 3.0, where:

$$\sigma_{\text{eff}} = \sigma_{\text{entity}} + \sigma_{\text{domain}} \times \begin{cases} 0.0 & \text{if UGC platform} \\ 0.3 & \text{otherwise} \end{cases}$$

The logarithmic scaling ensures diminishing returns — doubling stake provides only a marginal ranking boost, discouraging whales from dominating results purely through capital.

### 12.3 Reputation Factor

$$R \in [0, 3.0]$$

Derived from the Wilson score lower bound of accumulated feedback signals (positive/negative), or from stake-derived reputation as a fallback.

### 12.4 Quality Multiplier

$$Q = (0.6 + 0.6 \cdot q_{\text{score}}) \times (1 - 0.3 \cdot s_{\text{spam}}) \times (1 - 0.2 \cdot s_{\text{slop}})$$

where q_score, s_spam, s_slop ∈ [0, 1]. High-quality, non-spam, non-slop content achieves Q ≈ 1.2. Spammy content can be driven to Q ≈ 0.

### 12.5 Freshness Multiplier

$$F = \begin{cases} 1.15 & \text{if age} < 7 \text{ days} \\ 1.08 & \text{if age} < 30 \text{ days} \\ 1.03 & \text{if age} < 90 \text{ days} \\ 1.00 & \text{otherwise} \end{cases}$$

### 12.6 DNS Verification Multiplier

$$D = \begin{cases} 1.05 & \text{if DNS verified} \\ 0.95 & \text{if known unverified} \\ 1.00 & \text{if unknown} \end{cases}$$

---

## 13. Security Analysis

### 13.1 Threat Model

| Threat | Mitigation |
|--------|-----------|
| **Spam flooding** | Minimum stake requirement + quality gate (spam/slop detection) + slashing risk |
| **Stake-and-dump** | 7-day unstake cooldown + dispute lock (cannot unstake during active dispute) |
| **Flash-loan governance** | 7-day stake age requirement for voting eligibility |
| **Malicious proposals** | 24-hour execution timelock + parameter floors + proposal deposit |
| **Vote manipulation** | Commit-reveal jury voting prevents last-minute vote swinging |
| **Predatory slashing** | Flat jury rewards (no profit from verdict outcome) + dispute bond at risk |
| **Jury farming** | Per-epoch jury reward cap (max_jury_rewards_per_epoch) |
| **Single-juror verdicts** | Minimum jury quorum (min_jury_weight = 1,000 $TRUTH) |
| **Whale emission gaming** | Curator yield cap (15% APY) + UGC passthrough = 0 |
| **Phantom rewards** | Supply headroom cap prevents accumulating unmintable reward debt |
| **Payment spam** | Channel settlement burn (10%) makes spam queries unprofitable |
| **Sybil attacks** | Stake-weighted participation; cost of attack scales linearly with capital |

### 13.2 Economic Security Invariants

1. **Spam unprofitability:** For any spammer staking S tokens, the expected loss from slashing (probability p × graduated rate × S) plus settlement burns exceeds the expected revenue from serving low-quality results.

2. **Sentinel incentive compatibility:** Sentinels earn flat fees regardless of verdict, removing incentive for frivolous or predatory disputes. False reporting costs the dispute bond.

3. **Governance safety:** The execution timelock ensures that even if a malicious proposal passes, affected stakers have time to exit. Parameter floors prevent disabling critical safety mechanisms.

4. **Deflationary convergence:** As emissions decay to zero, the protocol converges to a steady state where burns from channel settlements and slashing provide ongoing deflationary pressure, while sentinel fees sustain quality enforcement.

---

## 14. Roadmap

| Phase | Milestone | Status | Description |
|-------|-----------|--------|-------------|
| **Phase 0** | Local Stack | **Complete** | In-memory gateway, mock staking, hybrid search (HNSW+BM25), CLI, content distillation pipeline, quality gate. |
| **Phase 1** | Embedding & Retrieval | **Complete** | MiniLM INT8 embeddings, batch inference, SimHash dedup, cross-encoder reranking, Arctic M/L model support, CLIP multimodal (feature-gated). |
| **Phase 2** | Crawl Infrastructure | **Complete** | BFS spider, sentinel crawl daemon, URL frontier with stake-weighted priority, RSS/Atom/sitemap discovery, platform adapters, content drift detection, SQLite persistence. |
| **Phase 3** | Solana Program | **Complete** | Anchor staking program with stake/unstake, disputes, commit-reveal jury voting, governance, state channels, emissions, sentinel registry. All PDAs and instructions implemented. |
| **Phase 4** | P2P Network | **Complete** | libp2p mesh (Kademlia + gossipsub), distributed query fanout, shard replication with incremental sync, content/stake/heartbeat gossip. |
| **Phase 5** | SDK & Gateway | **Complete** | REST gateway client, magic-link authentication, query leaderboard, content seeding (docs-rs, MDN, custom sitemaps), entity/fact extraction (GLiNER). |
| **Phase 6** | Mainnet Beta | Planned | $TRUTH token launch, emission schedule activation, sentinel onboarding, production deployment. |
| **Phase 7** | Fee Economy | Planned | Transition from emission-driven to fee-driven sustainability. Community governance activation. |

---

## 15. Conclusion

OpenSonarX introduces a new paradigm for web search — one designed for machines rather than humans, and for truth rather than advertising. By requiring publishers to stake economic value, enforcing quality through decentralized sentinel networks, and aligning all participants through carefully designed token mechanics, the protocol creates a search layer where accuracy is profitable and spam is punished.

The 1.5% weekly emission decay provides a ~3-year runway to bootstrap network effects, while the burn-on-settlement and slash-and-burn mechanisms ensure long-term deflationary pressure. Governance is fully on-chain, with multiple layers of protection against flash-loan attacks, malicious proposals, and parameter manipulation.

As LLMs and AI agents become the dominant consumers of web information, the need for a search infrastructure layer that is accurate, verifiable, and economically aligned with quality has never been greater. OpenSonarX is that infrastructure.

---

## Appendix A — Protocol Parameters

| Parameter | Default | Range | Notes |
|-----------|---------|-------|-------|
| `max_total_supply` | 1,000,000,000 | ≥ total_minted | Hard cap on mintable $TRUTH |
| `epoch_max_emission` (E₀) | 10,000,000 | — | Initial epoch emission |
| `emission_decay` (δ) | 1.5% | — | Per-epoch geometric decay |
| `min_epoch_duration` | 604,800s (7d) | 0–365d | Minimum time between epoch advances |
| `burn_pct` | 10% | 0–100% | Channel settlement burn |
| `sentinel_fee_pct` | 5% | 0–50% | Channel fee to sentinels |
| `slash_pct` | configurable | 5–100% | Base slash rate |
| `staker_emission_pct` | 70% | 0–100% | Staker share of emissions |
| `curator_yield_cap_bps` | 1500 | 0–10000 | 15% APY cap for curators |
| `cooldown_secs` | 604,800 (7d) | — | Unstake cooldown period |
| `execution_delay_secs` | 86,400 (24h) | 3,600–2,592,000 | Governance timelock |
| `proposal_deposit` | 100 | 0–1B | Anti-spam deposit |
| `min_jury_weight` | 1,000 | 0–10B | Minimum jury quorum |
| `jury_vote_reward` | 1,000 | 0–1B | Flat reward per winning juror |
| `max_jury_rewards_per_epoch` | 50,000 | 0–1B | Per-epoch jury reward cap |
| `semantic_weight` (w_s) | 0.7 | — | Hybrid search: semantic weight |
| `bm25_weight` (w_b) | 0.3 | — | Hybrid search: BM25 weight |
| `ef_search` | 50 | — | HNSW query beam width |
| `M` | 16 | — | HNSW max connections per layer |
| `ef_construction` | 200 | — | HNSW build beam width |
| `k1` | 1.2 | — | BM25 term frequency saturation |
| `b` | 0.75 | — | BM25 document length normalization |
| `max_tokens` | 512 | — | Chunk size (tokens) |
| `overlap` | 128 | — | Chunk overlap (tokens) |
| `pool_multiplier` | 2 | — | HNSW candidate pool: top_k × this |
| `MAX_PER_DOMAIN` | 3 | — | Max results per domain per query |
| `MAX_PER_URL` | 2 | — | Max results per URL per query |
| `simhash_threshold` | 8 | — | Hamming distance for near-duplicate detection |
| `spider_max_depth` | 3 | — | BFS spider max link-follow depth |
| `spider_max_pages` | 1,000 | — | BFS spider max pages per crawl |
| `spider_concurrency` | 4 | — | BFS spider concurrent workers |
| `OI_THREADS` | min(4, cores) | — | ONNX intra-op thread count (env var) |

## Appendix B — Error Code Taxonomy

| Range | Category | Codes |
|-------|----------|-------|
| E1000–E1004 | Query | No results, timeout, shard unavailable, partial results, invalid filter |
| E2000–E2003 | Payment | Insufficient balance, ticket rejected, ticket expired, settlement failed |
| E3000–E3009 | Staking | Domain not verified, already staked, cooldown active, below minimum, blacklisted, commit/reveal/voting errors |
| E4000–E4003 | Dispute | Already active, bond insufficient, reporter cooldown, jury not eligible |
| E5000–E5002 | Network | No peers, DHT lookup failed, node unreachable |
| E6000–E6012 | Auth/Gateway | Invalid wallet/key/signature, quota exceeded, session/magic-link expired, email unverified, free tier exhausted, Stripe/fiat bond errors |

Retryable errors: E1001 (timeout), E1002 (shard unavailable), E2001–E2002 (ticket), E2003 (settlement), E5000–E5002 (network), E6007 (free tier exhausted).

## Appendix C — Wire Protocol (Protobuf)

The OpenSonarX wire protocol uses Protocol Buffers (protobuf) via `prost` for all P2P communication. Three protocol streams are defined:

| Protocol | Path | Direction | Purpose |
|----------|------|-----------|---------|
| Query | `/opensonarx/query/1.0.0` | Request-Response | Distributed search queries |
| Feedback | `/opensonarx/feedback/1.0.0` | Request-Response | Relevance feedback signals |
| Gossip | `/opensonarx/gossip/1.0.0` | Pub/Sub | Content announcements, heartbeats, stake events, shard metadata |

Key message types:

- **QueryRequest:** request_id (UUID), embedding (384 bytes INT8), query text, filters, limit, state channel ticket, nonce.
- **QueryResponse:** request_id, metadata (latency, shard, node version), ranked results with full provenance.
- **SearchResult:** result_id, rank, title, URL, snippet (200 chars), content markdown, scores (semantic, BM25, final), stake info (amount, USD, type, reputation), utility info (agent score, feedback counts), freshness info (crawled/published timestamps, content hash), extraction quality, source attribution.
- **StateChannelTicket:** payer/payee pubkeys, cumulative amount, monotonic nonce, Ed25519 signature, expiry slot.
- **ContentAnnouncement:** sentinel_id, URL, domain, content SHA-256, crawled_at, chunk count, shard_id.
- **Heartbeat:** node_id, timestamp, shard count, vectors stored, queries served, average latency, uptime percentage.

---

*OpenSonarX is open source under the MIT License.*
*Repository: https://github.com/bbiangul/opensonarx*
