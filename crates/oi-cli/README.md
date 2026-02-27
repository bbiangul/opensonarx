# oi-cli

Command-line interface for the OpenSonarX decentralized search protocol.

## Installation

```bash
cargo install --path crates/oi-cli
```

## Usage

```
oi <command> [options]
```

### Global Flags

| Flag | Short | Description | Default |
|------|-------|-------------|---------|
| `--output` | `-o` | Output format: `table`, `json`, `ndjson` | `table` |
| `--wallet` | `-w` | Path to wallet keypair file | none |
| `--network` | `-n` | Network: `mainnet`, `devnet`, `localnet` | `mainnet` |
| `--rpc` | | Solana RPC URL override | auto |
| `--peers` | | Bootstrap peer multiaddrs | none |
| `--timeout` | `-t` | Request timeout in seconds | `30` |
| `--quiet` | `-q` | Suppress non-essential output | off |
| `--verbose` | `-v` | Verbose logging | off |
| `--yes` | | Skip confirmation prompts | off |

### Commands

#### Search

```bash
oi search "rust programming" -k 10 --min-stake 100 --freshness 1d --type text
```

Search the OpenSonarX network. Returns results ranked by hybrid semantic + BM25 scoring, weighted by publisher stake.

#### Feedback

```bash
oi feedback <result_id> --useful
```

Submit feedback on a search result to improve future rankings.

#### Stake / Unstake

```bash
oi stake example.com --amount 1000000
oi unstake example.com
```

Stake `$TRUTH` tokens for a domain or begin the 7-day unstaking cooldown.

#### Domain Management

```bash
oi domain verify example.com
oi domain list
oi domain info example.com
oi domain curate example.com --amount 500000
```

Verify DNS ownership, list staked domains, get domain details, or curate (stake for) another publisher's domain.

#### Disputes

```bash
oi dispute open example.com --bond 100000 --reason "spam content"
oi dispute list
oi dispute info <dispute_id>
```

Open a dispute against a domain, list active disputes, or view dispute details.

#### Jury

```bash
oi jury register
oi jury vote <dispute_id> --verdict uphold
oi jury status
```

Register as a juror, vote on disputes, or check eligibility status.

#### Yield

```bash
oi yield show
oi yield history
```

View current yield earnings and history from staked domains.

#### Network

```bash
oi network node -p 4001 -b /ip4/1.2.3.4/tcp/4001/p2p/QmPeerId
oi network status
oi network peers
```

Run an OpenSonarX P2P node, check network status, or list connected peers.

#### Wallet

```bash
oi wallet create
oi wallet balance
oi wallet address
oi wallet import /path/to/keypair.json
```

Create, inspect, or import wallets for signing transactions.

## Output Formats

All commands support three output formats via `-o`:

- **table** (default): Human-readable column-aligned output
- **json**: Pretty-printed JSON, suitable for piping to `jq`
- **ndjson**: Newline-delimited JSON, one object per line (suitable for streaming)

## Examples

```bash
# Search with JSON output
oi search "decentralized AI" -o json -k 3

# Stake on devnet
oi -n devnet -w ~/.config/oi/keypair.json stake mysite.com --amount 1000000

# Run a node with bootstrap peers
oi network node -p 4001 --peers /ip4/1.2.3.4/tcp/4001/p2p/QmPeerId

# Check wallet address
oi -w ~/.config/oi/keypair.json wallet address
```
