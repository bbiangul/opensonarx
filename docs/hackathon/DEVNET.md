# Solana deployment status

Program ID: `5psb7XSg7ZkzPXuD9GirjYRyz3Nh5CbXEQLg84vpuE9c`

## What has run (11 October 2026)

- The program compiles to SBF (`cargo build-sbf`, 339 KB).
- Deployed to a local `solana-test-validator` and exercised with real
  transactions through the validator's RPC:
  - `solana program deploy` of the SBF binary;
  - a `Stake` transaction bonding 0.25 SOL to a domain, sent by
    `crates/oi-staking/examples/chain_bond.rs`;
  - the stake account read back from its PDA (`["stake", domain]`);
  - the bond mirrored into the local registry and shown in the `BOND` column
    of `oi search` next to every result from that domain.
- 84 tests against the program inside the Solana runtime
  (`cargo test --manifest-path tests/integration/Cargo.toml --test solana_e2e`).

## What has not run

**The program is not on devnet yet.** The devnet and testnet faucets refused
every airdrop request from the deploy wallet on 11 October (rate limit), and
deployment needs about 2.4 SOL. Nothing in the code is waiting on this: the
same script deploys to devnet once the wallet is funded.

This file is updated with the devnet transaction signatures when that
happens.

## Reproduce

Local validator:

```bash
solana-test-validator --reset --quiet &
solana airdrop 10 --url http://127.0.0.1:8899
scripts/chain_bond_demo.sh http://127.0.0.1:8899 rome2rio.com 250000000
oi --open search "new york to washington dc rome2rio"
```

Devnet, with a funded wallet at `~/.config/solana/id.json`:

```bash
scripts/chain_bond_demo.sh https://api.devnet.solana.com rome2rio.com 250000000
```

The script deploys the program if the cluster does not have it, sends the
bond if the domain has none, reads it back and mirrors it into
`$OPENSONARX_DATA_DIR/stakes.json`.

## What the bond is

A bond says someone locked value behind a domain. It does not say they own
it: the mirrored record is marked as not ownership-verified, and DNS proof
(`oi-verify=<wallet>` in a TXT record) is a separate check. The bond is
displayed with the source and never changes its rank.
