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

The two bonds were made by the end-to-end demo (`scripts/demo.py --rpc
https://api.devnet.solana.com`), which then read each stake account back
from the chain, mirrored it into the node's registry and checked that the
running node showed it without changing the claims it returns or their
order. Both runs passed all 18 checks; one is kept in
[DEMO-RUN.txt](DEMO-RUN.txt). The first transaction was also confirmed
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
- Only the V1 `Stake` instruction has been sent on devnet. Unstake,
  disputes, jury votes, slashing and rewards are exercised by the tests in
  the Solana runtime, not on a public cluster.
- Nodes being paid or penalised on chain for ingestion work is design.

## Reproduce

The whole demo, bonding on devnet with a funded wallet at
`~/.config/solana/id.json`:

```bash
scripts/demo.py --rpc https://api.devnet.solana.com
```

Only the bond:

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
