# Solana deployment status

Program ID: `5psb7XSg7ZkzPXuD9GirjYRyz3Nh5CbXEQLg84vpuE9c`

## On devnet (12 October 2026)

The program is deployed on Solana devnet and has taken stake transactions
there. Everything below is devnet, with test SOL. Nothing is on mainnet.

| What | Where |
|---|---|
| Program | [`5psb7XSg7ZkzPXuD9GirjYRyz3Nh5CbXEQLg84vpuE9c`](https://explorer.solana.com/address/5psb7XSg7ZkzPXuD9GirjYRyz3Nh5CbXEQLg84vpuE9c?cluster=devnet) |
| Deploy transaction | [`5s3sV5mZ…SatqZSDf`](https://explorer.solana.com/tx/5s3sV5mZxETr6YJ5hEzwKV2mXFyJVy1y9yqq1nhT6uzQQWBcG5yyaFmvkJ4jKQpgNoXDmFS5o9eYgSsqSatqZSDf?cluster=devnet), slot 510299573 |
| Upgrade authority and staker | `CmHFGwAKPquiQCwVRowQJ71ScGcxXihaXxTBbiYmK81h` (a wallet used only for this) |
| Bond of 0.25 SOL on `consumerfinance.gov` | stake account [`CGPyudzz…aNLzCCw`](https://explorer.solana.com/address/CGPyudzzksPULCGi5HHuSayZhKbGhkkVLFAJ8aNLzCCw?cluster=devnet), transaction [`5i2UsLBz…RuQrMtF`](https://explorer.solana.com/tx/5i2UsLBzVgBPGxKSw2U2Rn2qSKhMpGd8cLRFJvj86fStbNVYHYgp3EFFL2CYT2cLQiwCAJxXzXY8k4oFqRuQrMtF?cluster=devnet) |
| Bond of 0.25 SOL on `worldatlas.com` | stake account [`AtJp8i1R…hMKKtvx1f`](https://explorer.solana.com/address/AtJp8i1Rh6Na6STy57JFf6z3esCoLMrJ6j4hMKKtvx1f?cluster=devnet), transaction [`2adsmYi2…32z8ybf`](https://explorer.solana.com/tx/2adsmYi2jHGqA6LDsNgVWC6zX4Wm6j6tfVo2ahMwME7LzhiNLU2gTcTQS4gjnTJ3BmxWFbDD14fUotQG232z8ybf?cluster=devnet) |

### Node operators: a bond, a refusal, a slash

The protocol is initialized on devnet (authority: the wallet above; test
token mint `P7Tx7UnWFYLJvkcpfszojnZQKmDf5MJcaKTa84KTA6o`; minimum bond
1,000,000 units of that token). In the demo run kept in
[DEMO-RUN.txt](DEMO-RUN.txt):

| What | Where |
|---|---|
| Operator of the honest node locks 1,000,000 units | wallet `8nKYxqSibwKu2Q1BdSNY4NhLrvdQgiDdhjAgGFxXXm4Q`, transaction [`UJxjUEoS…ae7x1`](https://explorer.solana.com/tx/UJxjUEoSh4Rrkc3Ri8vKESxADqhxznGwBe7XwYqwTiyuBpTgmBJnVowbMEopMGsPfYXvwqbmMVkuUk3Rn2ae7x1?cluster=devnet) |
| Operator of the dishonest node locks 1,000,000 units | wallet `5BGpSxtoa8chqatFBmpTgQoNazkptKBm56dEXnvPT7y6`, transaction [`3aTfVQwc…52p1iU`](https://explorer.solana.com/tx/3aTfVQwcxedRyXECDJxMppRm5u898U439fct3Z1bWxb5Jc14DNN9Gi5rpsW6dteESg44rCn1m35MjNH4ZC52p1iU?cluster=devnet) |
| Its bond is slashed, the evidence hash in a memo | transaction [`e6S47ATT…ty73gZ`](https://explorer.solana.com/tx/e6S47ATTE18STGMAgjnpPWf9KXHwKZh4KdWF9s6jiS2ErsbwwRWgvvDcSz4ViSfbhE1zpH6aT77A9HDhfty73gZ?cluster=devnet) |

Read back from the chain afterwards: the dishonest operator's bond account
shows 1,000,000 staked, 1,000,000 slashed; the honest operator's shows
1,000,000 staked, none slashed. The slash transaction's log carries the
memo `opensonarx refused claim, evidence sha256 9625…7f99`.

What decides the slash is off chain. A replica found that the quote was not
in the page, an auditor fetched the page again and confirmed it, and the
protocol authority sent `SlashSentinel`. The program checks only that the
sender is the authority; a slash signed by another wallet was rejected in
the same run. The bond is in a test token with no value.

### Publishers: a bond behind a domain

The two bonds were made by the end-to-end demo (`scripts/demo.py --rpc
https://api.devnet.solana.com`), which then read each stake account back
from the chain, mirrored it into the node's registry and checked that the
running node showed it without changing the claims it returns or their
order. The first transaction was also confirmed
separately (`solana confirm`: finalized; the stake account is owned by the
program and holds the 0.25 SOL plus rent).

The domains are the ones the demo's search happened to return. We do not own
them and the bonds say nothing about their publishers: they show the
mechanism. See "What the bond is" below.

## Also run

- The same flow on a local `solana-test-validator`, which is what the demo
  uses when no `--rpc` is given. There it generates a throwaway wallet and
  program address, so it needs no keys of the operator.
- 84 tests against the program inside the Solana runtime
  (`cargo test --manifest-path tests/integration/Cargo.toml --test solana_e2e`).

## Not done

- Mainnet.
- Sent on devnet: `Stake`, `InitializeProtocol`, `MintTruth`,
  `StakeSentinel` and `SlashSentinel`. Unstake and withdrawal, disputes,
  jury votes and rewards are exercised by the tests in the Solana runtime,
  not on a public cluster.
- A slash that does not depend on the authority: evidence submitted by
  anyone and decided by a jury or by the chain. The program has a dispute
  and jury flow for domains; it is not connected to node operators.
- Nodes being paid for ingestion work.

## Reproduce

The whole demo, bonding on devnet with a funded wallet at
`~/.config/solana/id.json`:

```bash
scripts/demo.py --rpc https://api.devnet.solana.com
```

A node operator's bond by hand (the tool is
`crates/oi-staking/examples/chain_node_bond.rs`; `join` and `slash` need the
protocol authority's keypair):

```bash
cargo run -p oi-staking --features solana --example chain_node_bond -- \
  show --rpc https://api.devnet.solana.com \
  --program 5psb7XSg7ZkzPXuD9GirjYRyz3Nh5CbXEQLg84vpuE9c \
  --wallet 5BGpSxtoa8chqatFBmpTgQoNazkptKBm56dEXnvPT7y6
```

Only a publisher's bond:

```bash
scripts/chain_bond_demo.sh https://api.devnet.solana.com example.org 250000000
```

The script deploys the program if the cluster does not have it, sends the
bond if the domain has none, reads it back and mirrors it into
`$OPENSONARX_DATA_DIR/stakes.json`.

## What the bond is

A bond says someone locked value behind a domain. It does not say they own
it: the mirrored record is marked as not ownership-verified, and DNS proof
(`oi-verify=<wallet>` in a TXT record) is a separate check. The bond is
displayed with the source and never changes its rank.
