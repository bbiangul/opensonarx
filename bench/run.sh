#!/usr/bin/env bash
set -euo pipefail

DOCS=${1:-10000}
PORT=3333
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT_DIR="$(dirname "$SCRIPT_DIR")"

echo "=== OpenSonarX Bench Suite ==="
echo "Documents: $DOCS"
echo ""

# 1. Build release binary
echo "[1/4] Building bench server (release)..."
cargo build --release -p oi-bench 2>&1

# 2. Start server
echo "[2/4] Starting bench server with $DOCS docs on port $PORT..."
"$ROOT_DIR/target/release/oi-bench" -n "$DOCS" -p "$PORT" &
SERVER_PID=$!

# Cleanup on exit
cleanup() {
    echo ""
    echo "Stopping server (PID $SERVER_PID)..."
    kill "$SERVER_PID" 2>/dev/null || true
    wait "$SERVER_PID" 2>/dev/null || true
}
trap cleanup EXIT

# 3. Wait for health check
echo "[3/4] Waiting for server..."
for i in $(seq 1 30); do
    if curl -sf "http://localhost:$PORT/health" > /dev/null 2>&1; then
        echo "Server ready!"
        break
    fi
    if [ "$i" -eq 30 ]; then
        echo "ERROR: Server failed to start after 30s"
        exit 1
    fi
    sleep 1
done

# Show stats
curl -s "http://localhost:$PORT/stats" | python3 -m json.tool 2>/dev/null || \
    curl -s "http://localhost:$PORT/stats"
echo ""

# 4. Run k6
echo "[4/4] Running k6 load test..."
if command -v k6 &> /dev/null; then
    k6 run "$SCRIPT_DIR/k6/search.js"
else
    echo "k6 not found. Install with: brew install k6"
    echo "Running a simple curl test instead..."
    echo ""

    for q in "rust programming" "python data science" "kubernetes deployment"; do
        echo "Query: $q"
        curl -s -X POST "http://localhost:$PORT/search" \
            -H "Content-Type: application/json" \
            -d "{\"query\": \"$q\", \"top_k\": 10}" | python3 -c "
import sys, json
data = json.load(sys.stdin)
print(f'  Results: {data[\"total\"]}, Query time: {data[\"query_ms\"]:.2f}ms')
for r in data['results'][:3]:
    print(f'    #{r[\"rank\"]} [{r[\"score\"]:.4f}] {r[\"title\"][:60]}')
" 2>/dev/null || echo "  (python3 not available for pretty output)"
        echo ""
    done
fi

echo ""
echo "=== Done ==="
