# Adversarial Analysis: Retrieval, Uncertainty, and Economics

**Status:** 2026-10-08, aligned with whitepaper v2.8; financial deployment unvalidated
**Legend:** `SHIPPED` means present and tested; `PARTIAL` means some defenses exist; `OPEN` means the stated property is not yet adequate; `RETRACTED` means legacy code or documentation must not be treated as the current design.

This document prevents demo-grade mechanisms from being mistaken for production trust. The normative threat model is in whitepaper v2.8 §10. Historical results describe their original code versions, not fresh market tests.

## Threat model

Adversaries may try to buy visibility or confidence, poison calibration, manufacture feedback or receipts, hide common funding/derivation, hedge apparent exposure, manipulate settlement, suppress competitors, poison replicated content, or farm economic rewards.

## 1. Capital, reputation, or DNS buys retrieval rank

**Attack:** add domain/entity stake, reputation, or DNS status to move a weak document above a more relevant one.

**Defense — PARTIAL:** `compute_score` excludes stake, reputation, and DNS verification. The old 1.05×/0.95× DNS multiplier has been removed. Scorer tests, `stake_ablation.rs`, `signal_comparison.rs`, and a focused non-zero `HybridIndex` registry/no-registry test return unchanged default-query scores and order. Registry data can still be returned as metadata.

**Remaining gate:** run the registry ablation on representative non-zero corpora and audit crate/API boundaries for alternate pruning or scoring paths. The legacy explicit `min_trust` filter is post-ranking and caller-selected; it is not OpenUncertainty and must not be presented as calibrated reliance.

## 2. Capital buys a trusted label

**Attack:** combine stake, DNS, and farmed reputation to unlock `trusted`/`highly_trusted`, then rely on clients that filter by the label.

**Defense — RETRACTED AS A SECURITY PROPERTY:** `compute_verdict` gates and tests still exist for compatibility, but the scalar label is not a calibrated probability and is not the v2.4 trust algorithm. Capital alone is pinned below `trusted`, yet the reputation/vote inputs remain Sybil-prone. A caller explicitly using `--min-trust` accepts this legacy limitation.

**Replacement:** claim/task/horizon-specific OpenUncertainty with deterministic, calibrated, or evidential-only output. No legacy verdict may be relabeled as `p_reliable`.

## 3. DNS or domain lapse launders identity

**Attack:** bind a domain temporarily, inherit its perceived legitimacy, or keep history/funds after transfer.

**Defense — PARTIAL:** DNS is provenance metadata only and no longer moves rank. The v2.4 design binds financial authority to an identity key rather than the domain, records binding intervals, requires periodic re-verification, freezes new domain-authorized use on a change, and requires prior-key continuity for history transfer.

**Remaining gate:** durable DNS observations, key hierarchy, cooldown/freeze behavior, and transfer tests are not fully implemented.

## 4. Sybil feedback or selected audits poison calibration

**Attack:** create identities that submit positive feedback, report only easy successes, or omit failures.

**Defense — OPEN:** raw feedback has no direct retrieval or uncertainty multiplier. Only eligible randomized outcomes may update the MVP source history. A production model requires logged non-zero sampling propensity, bounded inverse-propensity or doubly robust estimation, immutable cohort manifests, and forward evaluation.

Stake-weighting feedback is not the fix: it merely turns a Sybil problem into purchased influence. Until the sampling and identity model is validated, feedback remains telemetry.

## 5. Wealth, hedging, or self-funding buys confidence

**Attack:** post a huge nominal position, borrow it from an interested party, recover it through rebates/side payments, hedge elsewhere, split it among controlled wallets, or treat losses as advertising.

**Defense — PARTIAL:** MVP-1 requires exact scope, pre-outcome timing, positive loss, lock time, coverage, funding independence, unhedged exposure and deterministic/event-bound settlement. Amount is log-normalized and capped at 100× the reference amount. The former marginal cash update is disabled: stronger sources/easier claims selecting cash cannot receive their aggregate advantage again on top of source history. Cash diagnostics have zero probability authority. The `±ln(3)` cap is reserved for a future conditional/joint model, not proof of safety or predictive increment.

**Remaining gate:** beneficial-owner/funder resolution, hedge and rebate detection, correlated-position collapse, wealth slices, outcome manipulation, and real-world predictive value remain open.

## 6. Subjective dispute slashes or suppresses a competitor

**Attack:** file a “misinformation,” spam, or quality dispute, bribe/collude with jurors, and move funds or hide content before objective resolution.

**Defense — RETRACTED:** subjective jury outcomes have no capital authority in v2.4. Capital may move only on a versioned Grade-A deterministic predicate with retained inputs and reproducible traces. Divergent traces, missing artifacts, or ambiguous settlement pause the class and return commitment/challenge bonds as unresolved. Human/model review may add evidence but cannot decide the bond.

**Remaining gate:** the deterministic execution/fraud-proof path and coalition-safe payouts are specified but not production-implemented or audited.

## 7. Replication poisoning

**Attack:** return spam or malformed chunks attributed to another source.

**Defense — PARTIAL:** replicated chunks validate declared content hashes, URL/domain consistency, size bounds, and embedding dimension before ingestion. This blocks malformed or hash-inconsistent payloads.

**Remaining gate:** a peer can still serve self-consistent junk. Identity-key chunk signatures, source authorization, re-crawl sampling, and a lawful artifact policy remain required.

## 8. Reward and receipt farming

**Attack:** one beneficial cluster pays, serves, verifies, sponsors, and receives rebates or emissions, recovering more than its external cost.

**Defense — OPEN:** every recoverable role must enter the coalition inequality:

```
fee + irreversible_external_cost > total_recoverable_cluster_payout
```

The historical 85/5/10 split does not prove this once rebates, witnesses, sponsors, emissions, or side payments are included. Usage receipts also require payer authorization, service signatures, license binding, unique nonces, replay checks, privacy aggregation, and capped subsidies.

## 9. Dependency/citogenesis manufactures corroboration

**Attack:** copy one unsupported origin into many pages, models, languages, wallets, or ostensibly independent outlets.

**Defense — OPEN:** exact/near duplicates and known citation/derivation/funding relationships must collapse into dependency components. Unknown common ancestry widens uncertainty and increases ignorance. Multiple cash positions from one beneficial/funding component are one conviction observation, not corroboration.

Common ancestry and funding can remain hidden; this is a core inference problem, not a solved Sybil claim.

## Summary

### October corrections and unresolved activation

Observed descriptive mass is bounded; a fixed missing requirement cannot disappear
through repeated supportive volume. A generic `critical_missing` flag blocks
probabilities and complete reliance, including when a narrow deterministic check
passes. Full search-policy checklist wiring and private/paid coverage tests remain
pending. A numeric floor does not discover hidden derivations.

The proposed dispute bounty increases from US$10 to US$30: correct challengers net
US$28 before investigation and US$18 after illustrative US$10 investigation. With
no inconclusive cases, `EV=40q-12-C`; high costs or ambiguity can still defeat the
incentive. The restricted reserve is not consumer insurance and must be outside
the entire attacker coalition. Production escrow/payouts remain unimplemented.
The full model, conservation/sensitivity tests and approval gates are in
`scripts/economics_pilot.py` and `docs/experiments/PILOT-GATES-2026-10-08.md`.

P0 licensed delivery chooses explicit price/payee/cache terms, forbids payable
free/citation events, validates publisher authority, refunds failed delivery and
requires replay-safe authorization. This is a specification, not a secure live
gateway. Publishing a predicate or license does not prove enforcement.

| Attack | Status |
|---|---|
| Capital/reputation/DNS buys Stage-1 rank | **PARTIAL** — focused tests pass; representative audit pending |
| Capital buys legacy trust verdict | **RETRACTED as security** — compatibility only |
| DNS/domain transfer launders identity | **PARTIAL** |
| Sybil feedback/selective calibration | **OPEN** |
| Wealth/hedging/self-funding buys confidence | **PARTIAL MVP; real defenses OPEN** |
| Subjective dispute moves capital | **RETRACTED** |
| Replication poisoning | **PARTIAL** |
| Reward/receipt farming | **OPEN** |
| Citogenesis/common funding | **OPEN** |

## Before real capital or a production confidence feature

1. Complete representative Stage-1 registry/dependency ablations.
2. Ship deterministic predicate execution, retained artifacts, unresolved refunds, and an independent audit.
3. Pre-register one observable vertical with immutable forward splits and logged sampling propensities.
4. Implement beneficial-owner/funder components, hedge/rebate accounting, and manipulation slices.
5. Require held-out Brier/log-loss/calibration/selective-risk improvement over the no-cash model, including a positive source/event/funder cluster-bootstrap lower bound.
6. Prove coalition-negative rewards under the cheapest modeled multi-role strategy.
7. Complete jurisdiction-specific payment, custody, tax, gambling/securities, privacy, and consumer-protection review.

Failure of the cash feature does not invalidate stake-neutral retrieval or typed evidence. It means economic conviction stays out of the estimator.

## Local economics audit — 9 October 2026

See [the economics audit](experiments/ECONOMICS-REVIEW-2026-10-09.md).
Subsidies are excluded from operating-profit gates. Role payouts must exceed
each role's costs; gross rewards alone do not establish participation incentives.
Ambiguous resolution can eliminate challenger EV. Coalition defenses must include
all bonuses/rebates and reserve recovery, not just the nominal dispute payout.
Admission must refuse commitments beyond available collateral; correlated losses
need explicit ownership and funding. Consumer indemnity remains absent from the
illustrative dispute split. The scenario model is not a financial ledger or
production validation. Legacy token-channel defaults do not define P0 stablecoin fees.

The [subsequent local implementation](experiments/ECONOMICS-IMPLEMENTATION-2026-10-09.md)
adds checked integer price/payout calculations and in-memory collateral admission,
cluster caps and replay refusal. It is not durable custody, authenticated license
delivery or a deterministic oracle. A caller-supplied cluster or loss estimate is
not proof of common ownership or risk. These guards do not unlock economic
confidence or establish general coalition resistance.
