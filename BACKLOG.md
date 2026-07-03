# OpenSonarX Backlog — The Content Credibility Market

## The thesis (why this exists)

AI broke the web's business model. Models trained on creators' work paid them
nothing; now agents answer from that work without ever loading the page — no
visit, no ad impression, no subscription prompt, no revenue. The creators who
made the web worth reading are being disintermediated by the machines reading
it.

OpenSonarX is the corrective: **a market where agent usage pays creators, and
creator skin-in-the-game tells agents what to trust.** Think Kalshi, but the
contract isn't "will X happen" — it's "this content is good," priced by
capital at risk and settled by real agent usage and adjudicated disputes.

Two sides, one loop:

```
CREATORS                                     AGENTS / OPERATORS
  stake on their content  ──────────────►  filter by trust (min_trust)
  (bond: slashable if slop/false)            pay per query (fees/channels)
        ▲                                          │
        │            USAGE LEDGER                  │
        └──── earn when agents actually ◄──────────┘
              use their content (settlement
              by revealed demand, not clicks)
```

Design invariants (already enforced, keep forever):
- **Stake never buys rank.** Relevance decides order; capital prices trust.
- **Trust labels require proof** (DNS ownership) and earned reputation —
  money alone caps at "neutral."
- **Every claim in this file must ship with tests or an honest OPEN marker.**

---

## Phase 1 — Creator earnings loop (off-chain mirror) ← IN PROGRESS

The payoff leg. Track which creator's content each query actually used, and
accrue credits to them. Off-chain first (same mirror pattern as stakes.json);
on-chain settlement via the existing state channels when fees go live.

- [x] **1.1 Usage ledger**: on every search, record attribution for the
  local-content results served (domain, URL, count, whether stake-backed);
  persist `~/.opensonarx/creator_ledger.json`; survives restarts.
- [x] **1.2 `oi yield creators`**: creator-facing view — "agents used your
  content N times (M in top-3); accrued credits X" per domain, JSON output
  for dashboards.
- [ ] **1.3 Fee plumbing**: denominate credits in query-fee units and settle
  through the existing `SettleChannel` split (publisher 85% / sentinel 5% /
  burn 10% — already on-chain-tested). Map ledger drains → channel payouts.
- [ ] **1.4 Node attribution**: the resident node ledger records per-query
  usage server-side (agents shouldn't self-report); expose
  `/v1/creators/:domain/earnings`.
- [ ] **1.5 Anti-gaming**: usage credits only accrue from queries the ledger
  owner did not originate (no self-query farming); rate-normalize per
  querying identity; cap per-query attribution.

## Phase 2 — Resolvable bets (claims, not vibes)

"This domain is good" can't be settled; Kalshi contracts work because they
resolve. Move the unit of betting toward adjudicable positions.

- [ ] **2.1 Claim-level staking**: use oi-facts extraction to let creators
  stake on specific factual claims in their content ("assertions on this
  page stand"). Disputes then target a claim, not a reputation.
- [ ] **2.2 Dispute → resolution tightening**: "disputed" verdict takes
  effect only after adjudication (or bond scaled to target stake) — kill
  cheap suppression-by-filing (SECURITY-ECONOMICS #6).
- [ ] **2.3 Usage-as-settlement**: a stake position "pays out" (reputation +
  fee share) when agents keep using the content and no dispute lands —
  revealed demand is the continuous oracle between jury events.

## Phase 3 — Two-sided market (shorts)

- [ ] **3.1 Standing shorts**: let anyone maintain an open short on a domain
  or claim (continuous bond, not the current one-shot dispute). Shorts pay
  out on confirmed slop/falsehood; expire worthless otherwise. Attackers of
  slop fund the defense.
- [ ] **3.2 Verdict pricing**: expose long/short open interest per domain as
  a market signal alongside the verdict — the "price" of credibility.

## Phase 4 — Settlement oracle hardening (pre-token blockers)

From SECURITY-ECONOMICS.md — non-negotiable before credits are redeemable:

- [ ] **4.1 Sybil-resistant feedback**: stake-weighted, rate-limited,
  identity-bound agent feedback; devnet adversarial testing.
- [ ] **4.2 Chunk signatures**: replicated content signed by origin node keys
  (wallet Ed25519 machinery exists); spot-check re-crawls; slash poisoners.
- [ ] **4.3 Periodic DNS re-verification** (ownership can change hands).
- [ ] **4.4 External adversarial review** of the on-chain program.

## Phase 5 — Creator experience

Creators must see the deal working without reading a whitepaper.

- [ ] **5.1 Creator onboarding**: `oi creator register <domain>` = wallet +
  DNS TXT + stake in one guided flow.
- [ ] **5.2 Creator dashboard**: usage over time, earnings, trust tier, what
  queries surfaced their content (the "analytics" ads used to justify).
- [ ] **5.3 Content freshness pull**: staked domains get priority sentinel
  crawling (stake as a crawl-quality-of-service signal — not rank).
- [ ] **5.4 Opt-out/robots respect audit**: creators who don't want in are
  excluded cleanly; provable non-inclusion.

## Phase 6 — Human surface (users, not just agents)

The market needs all three participants: creators (supply), agents
(high-volume demand), and humans (demand + the feedback oracle + the
constituency that makes creator compensation politically real).

- [x] **6.1 Node web UI**: minimal search page served at `/` by the resident
  node — trust badges, stake amounts, min_trust toggle, latency shown.
  `docker compose up` → open http://localhost:8001.
- [ ] **6.2 Feedback from the UI**: thumbs on results → the same feedback
  pipeline agents use (humans are the highest-quality settlement signal).
- [ ] **6.3 Creator dashboard page**: `/creators` on the node — usage,
  credits, trust tier per domain (reads the creator ledger).
- [ ] **6.4 Packaged app**: desktop tray app or PWA wrapping the node
  (index + UI in one install) — the "own your search" consumer story.

## Engineering debt (carried forward)

- [ ] Import fidelity: same-corpus restore loses ~16% to SimHash re-screen.
- [ ] `-o` flag shadowing (global format vs export path).
- [ ] Shard system is nominal (everything topic-general); real topic sharding
  before 1M+ docs.
- [ ] Embedder upgrade evaluation (Arctic-S is the quality ceiling;
  reembed path exists).
- [ ] Week-long unattended soak test (sentinel + node).
