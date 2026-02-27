# oi-sdk

Rust SDK for the OpenSonarX decentralized search protocol.

## Modules

- **client** -- `OpenSonarXClient` wires together the hybrid search index, stake registry, and wallet into a single entry point for searching, ingesting content, and managing stakes.
- **config** -- `SdkConfig` holds bootstrap peers, wallet path, RPC URL, program ID, network selection, and timeout settings.
- **wallet** -- `Wallet` provides keypair management with SHA-256-derived keys (demo/testing; production would use real Solana keypairs).

## Quick start

```rust
use oi_sdk::{OpenSonarXClient, SdkConfig};
use oi_types::QueryFilters;

let client = OpenSonarXClient::new(SdkConfig::default());

// Search
let results = client.search("rust programming", 5, &QueryFilters::default()).unwrap();

// Stake
let tx = client.stake("example.com", 1_000_000).unwrap();

// Ingest content
let chunks = vec![/* ContentChunk instances */];
client.ingest(&chunks).unwrap();
```

## Testing

```bash
cargo test -p oi-sdk
```
