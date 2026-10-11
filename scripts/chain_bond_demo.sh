#!/usr/bin/env bash
# Deploy the staking program to a Solana cluster (if it is not there yet),
# bond SOL to a domain, read the bond back from the chain and mirror it into
# the local stake registry so `oi search` shows it.
#
# Usage: scripts/chain_bond_demo.sh <rpc-url> <domain> <lamports>
#   scripts/chain_bond_demo.sh http://127.0.0.1:8899 rome2rio.com 250000000   # solana-test-validator
#   scripts/chain_bond_demo.sh https://api.devnet.solana.com rome2rio.com 250000000
#
# Needs the Solana CLI, a funded keypair at ~/.config/solana/id.json and the
# program keypair (default ~/.config/solana/opensonarx_staking-keypair.json,
# override with PROGRAM_KEYPAIR). The registry file is
# $OPENSONARX_DATA_DIR/stakes.json (default ~/.opensonarx/stakes.json).

set -euo pipefail

RPC="${1:?rpc url}"
DOMAIN="${2:?domain}"
LAMPORTS="${3:?lamports}"
WALLET="${WALLET:-$HOME/.config/solana/id.json}"
PROGRAM_KEYPAIR="${PROGRAM_KEYPAIR:-$HOME/.config/solana/opensonarx_staking-keypair.json}"
DATA_DIR="${OPENSONARX_DATA_DIR:-$HOME/.opensonarx}"
PROGRAM_ID="$(solana-keygen pubkey "$PROGRAM_KEYPAIR")"
BOND=(cargo run -q -p oi-staking --features solana --example chain_bond --)

echo "cluster: $RPC"
echo "program: $PROGRAM_ID"

if ! solana program show "$PROGRAM_ID" --url "$RPC" >/dev/null 2>&1; then
  echo "program not found on this cluster; building and deploying"
  cargo build-sbf --manifest-path programs/opensonarx-staking/Cargo.toml
  solana program deploy target/deploy/opensonarx_staking.so \
    --program-id "$PROGRAM_KEYPAIR" --keypair "$WALLET" --url "$RPC"
fi

if "${BOND[@]}" show --rpc "$RPC" --program "$PROGRAM_ID" --domain "$DOMAIN" | grep -q "^no bond"; then
  "${BOND[@]}" stake --rpc "$RPC" --program "$PROGRAM_ID" --keypair "$WALLET" \
    --domain "$DOMAIN" --lamports "$LAMPORTS"
fi

mkdir -p "$DATA_DIR"
"${BOND[@]}" show --rpc "$RPC" --program "$PROGRAM_ID" --domain "$DOMAIN" \
  --mirror "$DATA_DIR/stakes.json"
