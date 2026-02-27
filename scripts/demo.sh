#!/usr/bin/env bash
# E2E Web3 Simulation Demo — Solana to Search to Feedback
# Usage: ./scripts/demo.sh [BASE_URL]
#   BASE_URL defaults to http://localhost:8080

set -euo pipefail

BASE="${1:-http://localhost:8080}"
ADMIN_KEY="test-admin-key"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m' # No Color

log()  { echo -e "${CYAN}[$1]${NC} $2"; }
ok()   { echo -e "  ${GREEN}✓${NC} $1"; }
fail() { echo -e "  ${RED}✗${NC} $1"; exit 1; }

echo -e "${YELLOW}══════════════════════════════════════════════════════════${NC}"
echo -e "${YELLOW}  OpenSonarX E2E Web3 Simulation Demo${NC}"
echo -e "${YELLOW}  Base URL: ${BASE}${NC}"
echo -e "${YELLOW}══════════════════════════════════════════════════════════${NC}"
echo

# --- Step 1: Admin seeds content ---
log "ADMIN" "Seeding 5 content chunks across 3 domains..."
INGEST_RESP=$(curl -s -w "\n%{http_code}" -X POST "${BASE}/v1/admin/ingest" \
  -H "Content-Type: application/json" \
  -H "X-Admin-Key: ${ADMIN_KEY}" \
  -d '{
    "chunks": [
      {"chunk_id":"rust1","content":"Rust systems programming language for safe concurrency and memory safety","url":"https://rust-lang.org/learn","title":"Learn Rust","domain":"rust-lang.org"},
      {"chunk_id":"rust2","content":"Rust async runtime with tokio for high performance network applications","url":"https://tokio.rs/docs","title":"Tokio Docs","domain":"tokio.rs"},
      {"chunk_id":"py1","content":"Python data science and machine learning frameworks for AI research","url":"https://python.org/about","title":"About Python","domain":"python.org"},
      {"chunk_id":"rust3","content":"Rust ownership and borrowing for memory safety without garbage collector","url":"https://rust-lang.org/book","title":"The Rust Book","domain":"rust-lang.org"},
      {"chunk_id":"tokio1","content":"Async IO framework for Rust providing runtime for reliable network apps","url":"https://tokio.rs/tutorial","title":"Tokio Tutorial","domain":"tokio.rs"}
    ]
  }')
HTTP_CODE=$(echo "$INGEST_RESP" | tail -1)
BODY=$(echo "$INGEST_RESP" | head -n -1)
[ "$HTTP_CODE" = "200" ] && ok "Ingested: $(echo "$BODY" | jq -r '.ingested') chunks" || fail "Ingest failed ($HTTP_CODE): $BODY"

# --- Step 2: Anonymous Dev searches ---
echo
log "ANON DEV" "Searching 'rust async programming' without authentication..."
SEARCH_RESP=$(curl -s -w "\n%{http_code}" -X POST "${BASE}/v1/search" \
  -H "Content-Type: application/json" \
  -d '{"query":"rust async programming"}')
HTTP_CODE=$(echo "$SEARCH_RESP" | tail -1)
BODY=$(echo "$SEARCH_RESP" | head -n -1)
[ "$HTTP_CODE" = "200" ] && ok "Got $(echo "$BODY" | jq '.results | length') results" || fail "Anonymous search failed ($HTTP_CODE)"
echo "$BODY" | jq -r '.results[] | "    \(.rank). [\(.url)] score=\(.final_score) stake=\(.stake_truth)"' 2>/dev/null || true

# --- Step 3: Create Publisher user ---
echo
log "ADMIN" "Creating Publisher user..."
PUB_RESP=$(curl -s -w "\n%{http_code}" -X POST "${BASE}/v1/admin/users" \
  -H "Content-Type: application/json" \
  -H "X-Admin-Key: ${ADMIN_KEY}" \
  -d '{"email":"publisher@rustlang.org"}')
HTTP_CODE=$(echo "$PUB_RESP" | tail -1)
BODY=$(echo "$PUB_RESP" | head -n -1)
PUB_KEY=$(echo "$BODY" | jq -r '.api_key')
[ "$HTTP_CODE" = "200" ] && ok "Publisher API key: ${PUB_KEY:0:20}..." || fail "Create publisher failed ($HTTP_CODE)"

# --- Step 4: Register stake for rust-lang.org ---
echo
log "ADMIN" "Registering 5 SOL stake for rust-lang.org..."
STAKE_RESP=$(curl -s -w "\n%{http_code}" -X POST "${BASE}/v1/admin/stakes" \
  -H "Content-Type: application/json" \
  -H "X-Admin-Key: ${ADMIN_KEY}" \
  -d '{"domain":"rust-lang.org","amount_lamports":5000000,"wallet":"Publisher111111111111111111111111111111111111"}')
HTTP_CODE=$(echo "$STAKE_RESP" | tail -1)
BODY=$(echo "$STAKE_RESP" | head -n -1)
[ "$HTTP_CODE" = "200" ] && ok "Stake registered for $(echo "$BODY" | jq -r '.domain')" || fail "Register stake failed ($HTTP_CODE)"

# --- Step 5: Publisher searches (should see stake boost) ---
echo
log "PUBLISHER" "Searching 'rust async programming' with authentication..."
SEARCH_RESP=$(curl -s -w "\n%{http_code}" -X POST "${BASE}/v1/search" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer ${PUB_KEY}" \
  -d '{"query":"rust async programming"}')
HTTP_CODE=$(echo "$SEARCH_RESP" | tail -1)
BODY=$(echo "$SEARCH_RESP" | head -n -1)
[ "$HTTP_CODE" = "200" ] && ok "Got $(echo "$BODY" | jq '.results | length') results (with stake enrichment)" || fail "Publisher search failed ($HTTP_CODE)"
echo "$BODY" | jq -r '.results[] | "    \(.rank). [\(.url)] score=\(.final_score) stake=\(.stake_truth)"' 2>/dev/null || true

# --- Step 6: Create Searcher user ---
echo
log "ADMIN" "Creating Searcher user..."
SRCH_RESP=$(curl -s -w "\n%{http_code}" -X POST "${BASE}/v1/admin/users" \
  -H "Content-Type: application/json" \
  -H "X-Admin-Key: ${ADMIN_KEY}" \
  -d '{"email":"searcher@example.com"}')
HTTP_CODE=$(echo "$SRCH_RESP" | tail -1)
BODY=$(echo "$SRCH_RESP" | head -n -1)
SRCH_KEY=$(echo "$BODY" | jq -r '.api_key')
[ "$HTTP_CODE" = "200" ] && ok "Searcher API key: ${SRCH_KEY:0:20}..." || fail "Create searcher failed ($HTTP_CODE)"

# --- Step 7: Searcher submits feedback ---
echo
log "SEARCHER" "Submitting feedback: rust1=useful, tokio1=useful, py1=not useful..."

for SIGNAL in \
  '{"result_id":"rust1","useful":true,"query":"rust async programming","result_url":"https://rust-lang.org/learn","result_rank":1}' \
  '{"result_id":"tokio1","useful":true,"query":"rust async programming","result_url":"https://tokio.rs/tutorial","result_rank":2}' \
  '{"result_id":"py1","useful":false,"query":"rust async programming","result_url":"https://python.org/about","result_rank":3}'; do
  FB_RESP=$(curl -s -w "\n%{http_code}" -X POST "${BASE}/v1/feedback" \
    -H "Content-Type: application/json" \
    -H "Authorization: Bearer ${SRCH_KEY}" \
    -d "$SIGNAL")
  HTTP_CODE=$(echo "$FB_RESP" | tail -1)
  [ "$HTTP_CODE" = "200" ] || fail "Feedback failed ($HTTP_CODE)"
done
ok "3 feedback signals submitted"

# --- Step 8: Verify feedback stats ---
echo
log "SEARCHER" "Checking feedback stats..."
STATS_RESP=$(curl -s -w "\n%{http_code}" "${BASE}/v1/feedback/stats" \
  -H "Authorization: Bearer ${SRCH_KEY}")
HTTP_CODE=$(echo "$STATS_RESP" | tail -1)
BODY=$(echo "$STATS_RESP" | head -n -1)
[ "$HTTP_CODE" = "200" ] && ok "Stats: total=$(echo "$BODY" | jq '.total_signals'), positive=$(echo "$BODY" | jq '.positive_signals'), negative=$(echo "$BODY" | jq '.negative_signals')" || fail "Stats failed ($HTTP_CODE)"

# --- Step 9: Trust check on staked domain ---
echo
log "PUBLISHER" "Verifying trust for rust-lang.org..."
TRUST_RESP=$(curl -s -w "\n%{http_code}" "${BASE}/v1/verify?url=https://rust-lang.org/learn" \
  -H "Authorization: Bearer ${PUB_KEY}")
HTTP_CODE=$(echo "$TRUST_RESP" | tail -1)
BODY=$(echo "$TRUST_RESP" | head -n -1)
IS_STAKED=$(echo "$BODY" | jq -r '.is_staked')
[ "$HTTP_CODE" = "200" ] && [ "$IS_STAKED" = "true" ] && ok "rust-lang.org: STAKED (trusted)" || fail "Trust check failed"

# --- Step 10: Trust check on unstaked domain ---
log "PUBLISHER" "Verifying trust for python.org..."
TRUST_RESP=$(curl -s -w "\n%{http_code}" "${BASE}/v1/verify?url=https://python.org/about" \
  -H "Authorization: Bearer ${PUB_KEY}")
HTTP_CODE=$(echo "$TRUST_RESP" | tail -1)
BODY=$(echo "$TRUST_RESP" | head -n -1)
IS_STAKED=$(echo "$BODY" | jq -r '.is_staked')
[ "$HTTP_CODE" = "200" ] && [ "$IS_STAKED" = "false" ] && ok "python.org: NOT STAKED (unverified)" || fail "Trust check failed"

echo
echo -e "${GREEN}══════════════════════════════════════════════════════════${NC}"
echo -e "${GREEN}  All 10 steps passed! E2E Web3 simulation complete.${NC}"
echo -e "${GREEN}══════════════════════════════════════════════════════════${NC}"
