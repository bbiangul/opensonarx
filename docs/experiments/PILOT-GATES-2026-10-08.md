# OpenSonarX: scoped validation and stop gates

**Status: proposed pre-registration, not an executed or approved capital pilot.**
**Specification: whitepaper v2.8, 8 October 2026.**

## 1. First product and dependencies

The first product is an operator-run text/structured-data API returning an answer
with claim-level evidence, contradictions, missing requirements, source locators,
freshness, and a reproducible decision trace. The first proposed buyer is a team
whose agent checks protocol documentation against publicly observable execution
records. Election-poll lineage is a second evaluation family, not another product.
Broad general search remains the mission. A crypto-only demo cannot validate it.

The minimum workflow uses replaceable discovery providers, authorized sources,
and the local evidence/decision pipeline. No token, publisher enrollment for all
web content, global node network, training exports, or broad multimodal ingestion
is a prerequisite. Protected licenses follow only after free/attribution serving
is useful. Real payments require separate approvals. No money is moved by this plan.

## 2. Frozen protocol and evaluation responsibilities

Before any confirmatory capture, retain hashes of code, policy, budgets, query
manifest, corpus snapshots, targets, metric formulas and exclusions. Name actual
people for these roles before launch: implementation owner, independent evaluation
owner, rights/security reviewer, and economics reviewer. These are required roles,
not appointments of people who have agreed to participate. Fix a four-week pilot
window only after recruitment, provider access, and authorization are confirmed.

Use 240 initial queries to estimate variance and cost. They become development
data after inspection. Then freeze a new confirmatory manifest of at least 1,000
queries across at least eight strata (at least 100 per stratum): navigation,
transactions, factual relations, current events, due diligence, polls,
high-impact information, and open-ended exploration. Include pt-BR and English
and time/family/source-disjoint queries. Two external blinded reviewers assess
full answers, not only snippets. Tune nothing against inspected holdouts. Failed
holdouts are burned; report failures and register fresh confirmatory sets.

Compare market search with its actual answer surface where available, raw
discovery-provider results, a strong search-plus-LLM baseline, and agent-search
evidence/confidence APIs where access permits. Missing baseline access is disclosed
and prevents claims about that product. Fix equivalent budgets, time windows,
locales and latency endpoints. Capture rate limits, no-results and abstentions.

## 3. Proposed numerical go/no-go criteria

These are acceptance targets, not results or a universal sample-size calculation.
Re-estimate power on the development pilot, then freeze final thresholds before
the independent evaluation. If uncertainty is too wide, report **inconclusive**.

| Gate | Required observation |
|---|---|
| Answer reliability | At matched answer coverage, at least 20% relative reduction in material factual error versus the declared strongest baseline, with a positive lower bound on paired cluster-bootstrap 95% improvement. Also publish absolute errors, uncertainty intervals and risk/coverage curves. No-answer cases never count as correct answers. |
| Useful coverage | Answer coverage no more than 5 percentage points below baseline at the declared service policy. Measure correct destination/action success separately for navigation and transactions. |
| Harm slices | No new severe supported-as-fact error in the predetermined attack suite. Report every stratum and any deterioration instead of hiding it in an average. Zero observed severe errors is not proof of zero population risk. |
| Critical evidence | Missing authorization, essential date/entity/measurement fields, inaccessible material source, or unresolved contradiction must prevent a complete claim endorsement. Repeat a supportive item 1/10/100/1,000 times without reducing the recorded unmet checklist. |
| Evidence-unit fidelity | At least 99% recovery of required source locators and qualifying context on an independent annotated set. Any materially reversed negation, wrong entity/date or lost scope blocks that adapter release pending repair and fresh holdout. Compare ordinary chunks and full documents. |
| Latency | Search-only cold p95 <= 2 s and p99 <= 5 s. Full evidence-bearing answers cold p95 <= 5 s and p99 <= 10 s. Measure end-to-end endpoints, timeouts and load. These are proposed ceilings; historical August search p95/p99 do not pass the search target. |
| Budget and service failure | Every authorization obeys the payer's cap. No protected bytes, derived snippets or cache entries cross the public boundary in the declared security suite. Both configured providers pass sustained tests, and a separately measured provider-loss test preserves the declared availability target. |
| Buyer usefulness | At least three recruited teams, with at least two independently choosing paid reuse after the trial and at least two paid weekly billing cycles. This validates only initial demand, not market size or retention at scale. |
| Lifecycle economics | Positive contribution after content, connectors, nodes, extraction, revalidation, corrections, audits, payment fees and support. At least 30% modeled operating margin at the agreed pilot price after fixed-cost allocation, with measured inputs and a stress scenario. Report startup-volume fixed costs, creator net receipts and uncompensated work. |
| Challenges | For measured investigation-cost distributions and observed precision/resolution, independent challengers have positive expected net return. Pre-fund verification; no bounty may depend on optional future rewards or inaccessible reserve withdrawals. Inconclusive outcomes and correlations count. |

No pilot gate unlocks a token, cash confidence, general-superiority claim, training
benefit or decentralization by implication. Each requires its own evidence.

## 4. Conditional cash experiment, separate from content payment

The former marginal bucket log-odds update is disabled. The cohort may still record
eligible positions for research, but the production probability contribution stays
zero. Ordinary content payments, stake or token rewards remain ineligible.

First validate a frozen no-cash baseline using source histories, non-financial
evidence and declared claim difficulty. Define a joint predictive model or a
conditional residual update `logit m1(x,b) - logit m0(x)` where `x` is the *same*
pre-outcome baseline feature vector, target, policy and timestamp. Train with
cross-fitting and hold calibration/outcome labels out of feature construction.
Do not transfer a marginal cash/no-cash odds ratio onto an unrelated source prior.
Test whether source confidence/easy-claim selection alone produces the apparent
cash effect. Matching and propensity correction do not guarantee identification
when confounders are unobserved or positivity fails; then disable that slice.

Pre-register source/event/funder/derivation component splits, both known-source
forward and new-cluster performance, cash-free/self-reported-confidence controls,
wealth and hedge-observability slices. Minimum initial cohort target: 5,000
resolved claims, 500 independent sources and 12 weeks. Final power depends on
variance and the minimum effect. Missing evidence or an unobservable target stays
evidential-only regardless of a wallet's capital.

Activation requires at least 3% relative Brier improvement over the same no-cash
baseline with positive cluster-bootstrap 95% lower bound, better log loss, ECE
no worse by >0.01, interval and selective-risk validation, and no material
manipulation/wealth-slice regression. These are proposed research thresholds.
Also report eligible-query share, benefit per added dollar and latency, and
compare spending the same budget on additional primary evidence. A successful
escrow or licensed-delivery transaction does not pass this gate.

## 5. Publisher serving profile P0

P0 pays for **authorized licensed delivery**, not citation count or model-rated
usefulness. Before delivery, the buyer accepts resource/version, price, scope,
payee allocation, retention, and cache-reuse window. Within that licensed window,
cache reuse has no new content charge unless the accepted license explicitly
sets a per-use price. Expired resources require revalidation/renewal before reuse.
The API service charge remains separate. Training permission is never inferred.

Open/attribution resources generate nonpayable receipts. Connector work has a
separate agreed fee and no automatic authorship or content-rights claim. Multiple
sources receive only their explicitly authorized delivery fees; citations do not
create another debt. A signature, first crawl, or earliest anchor alone does not
establish rights. Onboarding requires verified authority and accepted rights
documentation with conflicts held for review. Metadata and public factual
statements are not treated as owned merely because they have a protocol ID.

No automatic royalty for causal contribution is attempted in P0. Later royalty
allocation is another market design. No receipts mint token rewards. Replayed
nonces cannot charge twice. Failed delivery triggers refund; usage-after-delivery
compliance is contractual/monitorable, not cryptographically guaranteed.

Coverage records separately count acquired support/conflict, known inaccessible
resources, undiscovered coverage, and critical unmet requirements. An agent may
request more budget but never silently charge or turn an access failure into
absence of a contradiction. A required inaccessible source blocks completion.
This profile is specified, not yet implemented as a protected production gateway.

## 6. Economic redesign and finite limits

`python3 scripts/economics_pilot.py --test` verifies scenario arithmetic.
`python3 scripts/economics_pilot.py` prints inputs and sensitivity, without funds.

For a US$100 author bond, US$10 challenge deposit and US$2 fee, the proposed
proceeding distribution is US$40 to the challenger, US$2 to the verifier and US$70
to a restricted reserve. The US$40 includes the returned US$10 deposit and a
US$30 bounty. No consumer indemnity is defined. Other outcomes conserve the same
US$112: not proceeding pays 102/0/2/8; unresolved pays 100/10/2/0, in
author/challenger/verifier/reserve order.

With no unresolved cases, investigation cost `C`, and challenge precision `q`,
`EV = 40q - 12 - C`. At `C=10`, break-even is 55%; a correct challenge nets US$18,
and 90% precision gives US$14 expected gain per attempt. With an unresolved share
`u`, `EV = (1-u)[30q - 10(1-q)] - 2 - C`. Expensive investigations can still fail.
Measure actual costs and never assume AI investigation is free. Reserve assets
must stay inaccessible to the attacker's full coalition; colluding reviewers,
subsidies and manufactured results still require adversarial validation.

The old US$700 monthly underwriting premium is not viable at 100% detection under
its own US$100/5,000-commitment assumptions: the subtotal is US$858.33. A US$1,030
premium adds a 20% cost markup but still excludes administration, legal/custody,
correlated errors and tail capital. It is not an offered price or solvency result.

The earlier query scenario had US$0.0177 total modeled cost at one million queries.
An illustrative US$0.003 incremental policy/extraction/maintenance cost raises it
to US$0.0207: US$0.02 pricing loses US$700/month; US$0.03 yields US$9,300/month
before any still-unmeasured cost. The 30% margin price would be US$0.02957. These
inputs are placeholders. Measure actual itemized costs and willingness to pay,
including far lower startup volume, rather than adopting the attractive scenario.

## 7. Demonstration and governance

The demo must show a useful positive answer and a necessary abstention, the exact
source locator and version, a repeated-origin case and the decision trace. Show
the real competitor response under the same recorded query, time and budget.
Illustrations are explicitly labeled and never substituted for live results.

The operator controls provider configuration, labels, policy, model releases,
authorization and availability today. Publish versioned changes and overrides;
bind each answer to its versions and preserve appeal/correction records. Chain
settlement does not decentralize these decisions. Public independent settlement
is justified only after actual parties need it and deterministic traces,
security, custody and rights reviews pass. Training, multimodal and token work
stay off the first-release critical path.
