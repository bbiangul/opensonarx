# oi-types

Shared types, traits, and error codes for the OpenSonarX protocol. All other crates depend on this.

## Modules

| Module | Purpose |
|--------|---------|
| `types` | Core data structures: `StakeInfo`, `SearchResultItem`, `QueryFilters`, `CrawlOutput`, `PeerInfo`, `MetricsSnapshot` |
| `traits` | Interface contracts: `StakeRegistry`, `SearchIndex`, `StateChannel`, `CrawlPipeline`, `EconomicsEngine`, `MetricsCollector` |
| `error` | Error codes E1000-E6002, exit codes, retryability flags |
| `embed` | Deterministic SHA256-seeded text-to-embedding (384-dim) |
| `economics` | Protocol economics data types: emission, burn, revenue share, yield |
| `proto` | Protobuf-generated types (prost) from `proto/opensonarx.proto` |

## Error Code Ranges

| Range | Category |
|-------|----------|
| E1000-E1004 | Query errors |
| E2000-E2003 | Payment/ticket errors |
| E3000-E3009 | Stake + commit-reveal errors |
| E4000-E4003 | Dispute errors |
| E5000-E5002 | Network errors |
| E6000-E6002 | Auth errors |
