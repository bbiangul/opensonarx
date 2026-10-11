# Solana deployment status

**As of 11 October 2026 the program is not deployed to a public cluster.**
The program ID in `programs/opensonarx-staking/src/lib.rs` is still the
placeholder, and the CLI's `oi stake` writes to a local registry file.

What is verified today is the program's logic inside the Solana runtime:

```bash
cargo test --manifest-path tests/integration/Cargo.toml --test solana_e2e
# 84 passed
```

`test_full_pipeline_with_solana` submits a `Stake` transaction, reads the
stake account back from its PDA, ingests a page from the bonded domain and
checks that the search result carries the bond.

This file is updated with the program ID, the cluster and the transaction
signatures once the devnet deployment is done.
