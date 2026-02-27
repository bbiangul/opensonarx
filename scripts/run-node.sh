#!/usr/bin/env bash
# run-node.sh — Demo: crawl a real website and search it locally.
#
# Usage:
#   bash scripts/run-node.sh [domain] [query]
#
# Defaults:
#   domain = rust-lang.org
#   query  = "rust programming"
set -euo pipefail

DOMAIN="${1:-rust-lang.org}"
QUERY="${2:-rust programming}"
CRAWL_SECONDS=30
BINARY="cargo run --release --bin oi --"

echo "=== OpenSonarX Web3 Node Demo ==="
echo "  Domain: $DOMAIN"
echo "  Query:  $QUERY"
echo ""

# 1. Build
echo ">>> Building oi-cli (release)..."
cargo build --release -p oi-cli
echo ""

# 2. Start sentinel in the background
echo ">>> Starting sentinel crawler for $DOMAIN..."
$BINARY sentinel start --domains "$DOMAIN" &
SENTINEL_PID=$!

# Give it time to crawl
echo ">>> Crawling for ${CRAWL_SECONDS}s..."
sleep "$CRAWL_SECONDS"

# 3. Stop the sentinel
echo ""
echo ">>> Stopping sentinel (pid $SENTINEL_PID)..."
kill -INT "$SENTINEL_PID" 2>/dev/null || true
wait "$SENTINEL_PID" 2>/dev/null || true
echo ""

# 4. Search the local index
echo ">>> Searching local index for: $QUERY"
$BINARY search "$QUERY"
echo ""

echo "=== Done ==="
