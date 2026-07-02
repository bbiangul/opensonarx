# Adversarial Analysis: Trust & Economics Layer

Status of known attack vectors against the stake/trust system, as of
2026-07-02. Each entry states the attack, the current defense, whether it is
**enforced in code** (with tests) or **open**, and what closing it requires.
This document exists so nobody mistakes the demo-grade parts for
production-grade parts.

## Threat model

Adversaries want one of: (a) rank spam/malicious content highly, (b) acquire
an unearned trust label, (c) poison other nodes' indexes, (d) extract token
rewards without contributing, or (e) suppress a competitor's content.

## Attack vectors

### 1. Buy rank with capital ("pay-to-win")

**Attack**: stake heavily on a spam domain so its content outranks organic
results.

**Defense — ENFORCED**: stake and reputation are excluded from the ranking
score entirely; they feed only the trust *label*. Validated empirically:
`crates/oi-index/tests/stake_ablation.rs` shows mixing stake into ranking
degrades retrieval, and `test_no_stake_no_boost`-family tests pin the
stake-neutral formula. The external-result path also carries no stake boost
(removed 2026-07-01).

### 2. Buy the "trusted" label with capital

**Attack**: whale-stake a domain to acquire the trusted verdict that agents
filter on.

**Defense — ENFORCED**: `compute_verdict` requires earned reputation (>=1.3)
AND community votes (>=10) AND DNS-proven domain ownership for "trusted";
`test_verdict_capital_alone_buys_nothing_above_neutral` pins a $1M stake with
no reputation at "neutral". Caveat: reputation/votes currently come from a
stub feedback system (see #4), so today the gate is closed mostly because the
inputs don't exist yet.

### 3. Stake a domain you don't own

**Attack**: bond capital to `nytimes.com` to inherit its perceived
legitimacy, or to get *their* domain disputed/slashed.

**Defense — ENFORCED (new)**: staking checks for a DNS TXT record
(`oi-verify=<wallet>`) via DoH at stake time; unverified stakes are accepted
but capped at "neutral" (`test_verdict_trusted_requires_dns_proof`). Residual
gap: DNS is checked once at stake time — a domain sold/transferred keeps its
old verification until re-checked. Needs periodic re-verification.

### 4. Wash reputation / Sybil feedback

**Attack**: register many identities, submit positive feedback on your own
domain until reputation >=1.3 and votes >=10, unlocking "trusted".

**Defense — OPEN**. Feedback is a local stub (`client.feedback` logs and
returns). The on-chain design (Wilson-score reputation, jury voting, dispute
bonds) exists in `programs/opensonarx-staking` with unit tests for the math,
but no Sybil resistance is enforced end-to-end: nothing prevents one wallet
farming votes from N sock-puppet wallets. Closing this needs
feedback-weighted-by-stake (skin-in-the-game voting), per-identity rate
limits, and adversarial testing on devnet. **Until then, verdict tiers above
"neutral" should not be treated as Sybil-resistant.**

### 5. Poison peers via replication

**Attack**: run a node, respond to replication requests with spam chunks
attributed to reputable domains, or malformed chunks that break ingestion.

**Defense — PARTIALLY ENFORCED (new)**: replicated chunks are verified before
ingest — sha256(content) must match the declared hash, domain must match the
URL host, content bounded, embedding dimension checked (one bad vector
previously aborted a whole 1000-chunk batch). Tests in
`oi-network/src/event_loop.rs`. Residual gap: a peer can still send
*self-consistent* junk (correct hashes over garbage text). Closing fully
needs chunk signatures against announced node keys plus spot-check
re-crawling of replicated URLs, and slashing for verified poisoning.

### 6. Dispute griefing

**Attack**: file disputes against a competitor to tag them "disputed"
(which overrides every other verdict) and force their stake into limbo.

**Defense — PARTIAL**: disputes require a bond that is forfeited to the
protocol when the jury votes innocent (economics covered by
`oi-staking` tests: `resolve_dispute` splits). The trusted-reporter bond
discount is on-chain-tested. Residual gap: jury selection/collusion
resistance is untested against real adversaries, and "disputed" takes effect
at filing (before adjudication) — cheap temporary suppression if bonds are
mispriced. `test_verdict_disputed_overrides_everything` pins the (intended
but abusable) override.

### 7. Reward extraction without contribution

**Attack**: idle nodes claim emissions; peers leech replication without
serving.

**Defense — PARTIAL**: tit-for-tat replication accounting exists
(`record_replication_sent/received`, `min_contribution_ratio`) and emission
decay is on-chain-mirrored and tested. Residual gap: query-serving rewards
(state channels) are stubbed off-chain; uptime/contribution proofs are
self-reported.

## Summary table

| # | Attack | Status |
|---|--------|--------|
| 1 | Capital buys rank | **Closed** (tested) |
| 2 | Capital buys label | **Closed** (tested, pending #4 inputs) |
| 3 | Stake unowned domain | **Closed at stake time** (needs re-verification) |
| 4 | Sybil feedback → trusted | **OPEN** |
| 5 | Replication poisoning | **Partial** (integrity yes, authenticity no) |
| 6 | Dispute griefing | **Partial** (bond economics yes, timing/collusion no) |
| 7 | Reward leeching | **Partial** |

## Before any token has value

1. Sybil-resistant feedback (stake-weighted, rate-limited, devnet-tested).
2. Chunk signatures + spot-check verification + poisoning slashes.
3. Periodic DNS re-verification.
4. Dispute effects deferred until adjudication or bond scaled to target stake.
5. External adversarial review of the on-chain program.
