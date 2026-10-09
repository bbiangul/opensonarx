# OpenSonarX

Decentralized, AI-first search protocol. Publishers stake $TRUTH tokens to vouch for content quality.

## Mission
We are building infrastructure for the best, most reliable, and most up-to-date web search results for AI Agents and LLMs. Not ads for humans — infra for LLMs to search and answer users accurately, free of spam and AI slop.

## Stack
- Rust workspace (Cargo)
- Solana native program (`solana_program`, staking) — not Anchor
- libp2p (P2P networking)
- Protobuf (wire protocol via prost)
- HNSW + Tantivy (hybrid vector/BM25 search)

## Crate Layout
- `oi-types` — shared types, traits, error codes (all other crates depend on this)
- `oi-staking` — off-chain mirror of staking and economics; the on-chain program is `programs/opensonarx-staking`
- `oi-index` — HNSW + BM25 hybrid search engine
- `oi-network` — libp2p P2P layer
- `oi-crawl` — Content distiller + crawl pipeline
- `oi-sdk` — Rust SDK core: blended search, answer assembly, model backends
- `oi-embed` — Embedding pipelines (mock, MiniLM, CLIP via ONNX)
- `oi-cli` — CLI binary (`oi`)

## Commands
- `cargo build` — build all crates
- `cargo test` — run all tests
- `cargo clippy` — lint
- `cargo run --bin oi -- <subcommand>` — run CLI

## Conventions
- All public types go through `oi-types`
- Error codes follow the E1000-E6002 scheme (see oi-types/src/error.rs)
- Protobuf definitions live in `proto/opensonarx.proto`, generated via `prost-build`
- Tests in each crate cover critical paths from Tech Addendum Section 7.2
- No term, phrase or domain lists that decide meaning in `crates/`. Language understanding (what a request asks, what a passage claims, support vs contradiction) goes through a model behind the `ClaimExtractor` / `StanceJudge` traits in `oi-types`; deterministic code only checks what is reproducible from retained bytes (verbatim quote location, hashes, hosts, dates, registry joins by exact ID)
- No benchmark query or expected answer may appear in `crates/`. New failure modes become cases in `bench/fixtures/`, not rules in code
