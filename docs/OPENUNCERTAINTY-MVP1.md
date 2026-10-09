# OpenUncertainty MVP-1: Cash-at-Risk Validation Protocol

**Status:** corrected local safeguards; marginal cash uplift disabled; real-world thesis unvalidated

**Date:** 8 October 2026 (historical results below are August 2026)
**Scope:** claim-level uncertainty and Stage-2 decision ranking; never Stage-1 retrieval

## 1. Thesis

The hypothesis is not “money proves truth.” It is:

> For a precisely scoped claim with an observable outcome, a pre-outcome position carrying genuine, non-refundable, unhedged economic loss may reveal conviction and may improve out-of-sample reliability estimates after source history and other evidence are controlled for.

This is falsifiable. The cash contribution may be positive, zero, or negative. If comparable held-out outcomes do not show predictive value, OpenUncertainty assigns it zero effect. Ordinary retrieval payments, citation revenue, domain stake, popularity, wallet balance, and capital unrelated to the exact claim are never eligible.

## 2. What MVP-1 implements

The reference implementation is in `crates/oi-index/src/uncertainty.rs`. It includes:

- `DETERMINISTIC`, `CALIBRATED`, and `EVIDENTIAL_ONLY` output modes;
- typed Beta-Binomial source history over eligible observable outcomes;
- descriptive support/conflict/ignorance plus explicit known independence, unresolved dependency, staleness, and OOD fields;
- a claim-specific `ConvictionSignal` with amount, maximum loss, lock time, predicate coverage, funding independence, hedge discount, timing, and settlement grade;
- logarithmic amount normalization and a 100× reference-amount saturation cap;
- descriptive cash/outcome bucket associations with zero probability authority;
- zero effect when either calibration bucket is sparse;
- positive, zero, or negative diagnostic associations, never added to source history;
- an MVP-1 absolute cash contribution cap of `ln(3)`, limiting the odds change to 3× in either direction;
- 90% approximate probability intervals and sparse/OOD fallback;
- Stage-2 risk-aware reranking that preserves the original retrieval rank; and
- Brier score, log loss, and expected calibration error (ECE).

The current numerical estimate uses typed source history only. The marginal bucket update is disabled because stronger sources/easier claims may select cash, and adding their aggregate advantage to source history can double count information. A future joint or conditional model must validate the incremental contribution against the same frozen no-cash evidence baseline. Non-financial evidence still controls fallback, interval widening and decision penalties rather than learned Bayes factors. Observed descriptive mass is capped at one before adding policy missing mass; multiplying supporting pages cannot wash away a fixed missing requirement. `critical_missing` prevents complete reliance and a probability despite strong history; narrow deterministic outcomes retain their scope and the missing flag. The generic estimator supports this flag, but search's complete typed checklist integration is still pending. Narrow poll lineage and factual claim signatures remain implemented; generalized graphs, conditional cash, durable outcomes, propensity correction, distributed joins and legal/custody are not complete.

## 3. Effective economic exposure

An amount is eligible only if it was posted before the outcome on a deterministic or event-bound predicate and carries real positive loss, duration, scope, funding independence, and unhedged exposure.

```
amount = log(1 + min(bond_usd, reference_bond * cap) / reference_bond)
         / log(1 + cap)
time   = min(lock_days / reference_lock_days, 1)

effective_exposure = amount * max_loss_fraction * time
                     * predicate_coverage * funding_independence
                     * unhedged_fraction
```

Each component is clamped to `[0,1]`. Fully hedged, refundable, post-outcome, self-funded, weakly scoped, or evidence-only positions contribute zero or approach zero. Multiple positions with the same beneficial owner or funding cluster must be collapsed before calibration.

## 4. Reproduce the synthetic benchmark

From the repository root:

```bash
cargo run --release -p oi-bench -- uncertainty \
  --train-cases 20000 \
  --eval-cases 20000 \
  --throughput-claims 1000000 \
  --output json
```

The **retired marginal model's historical August** fixed-seed run produced:

| Measurement | No cash | Calibrated cash |
|---|---:|---:|
| Brier score | 0.217515 | 0.194467 |
| Log loss | 0.627475 | 0.574367 |
| ECE | 0.120846 | 0.014397 |

- relative Brier improvement: **10.60%**;
- relative log-loss improvement: **8.46%**;
- uninformative-cash negative control, maximum `|log BF|`: **0.0532**;
- whale guard after a 2-pass/98-failure history, including a USD 1 billion nominal position: `p(reliable) = 0.0833`;
- latest one-million-estimate run: **153.8 ms**, approximately **153.8 ns/claim** or **6.50 million estimates/second** (wall-clock throughput is expected to vary by machine and run).

The synthetic generator deliberately makes higher eligible cash buckets more predictive. Those historical results describe recovery of an injected marginal relationship, not conditional incremental information or real cash. They are not current benchmark expectations. After the October correction, the command emits `cash_model_status = disabled_marginal_association_not_incremental_evidence`; the legacy `calibrated_cash` field equals the no-cash predictions, and expected cash improvement is zero. Bucket associations remain available through `marginal_log_odds` for diagnostics only. The throughput number excludes retrieval, network, storage, extraction, joins and outcome ingestion. Never interpret disabled-effect negative controls as validation of a functioning conditional model.

### 4.1 Run a forward JSONL cohort

The evaluator now accepts a real forward cohort:

```bash
cargo run --release -p oi-bench -- uncertainty-dataset \
  --dataset bench/fixtures/uncertainty-example.jsonl \
  --output json
```

Each JSONL record uses this schema:

```json
{
  "claim_ref": "event:123",
  "source_ref": "publisher-key:abc",
  "evaluation_cluster_ref": "publisher-key:abc|event:123|funder:xyz",
  "claim_class": "event_bound_example",
  "task": "forecast",
  "horizon": "P30D",
  "policy_id": "pilot-v1",
  "split": "train",
  "committed_at_unix": 1700000000,
  "outcome_at_unix": 1700100000,
  "reliable": true,
  "eligible_for_calibration": true,
  "evidence": {
    "support_mass": 1.0,
    "conflict_mass": 0.0,
    "missing_mass": 0.5,
    "independence": 1.0,
    "dependency_uncertainty": 0.0,
    "staleness_uncertainty": 0.0,
    "distribution_shift": 0.0
  },
  "conviction": {
    "bond_usd": 1000.0,
    "max_loss_fraction": 1.0,
    "lock_duration_days": 30.0,
    "predicate_coverage": 1.0,
    "funding_independence": 1.0,
    "unhedged_fraction": 1.0,
    "posted_before_outcome": true,
    "settlement_grade": "event_bound"
  }
}
```

MVP-1 requires exactly one claim-class/task/horizon scope per file, unique claim IDs, a non-empty `evaluation_cluster_ref`, commitment before outcome, both `train` and `evaluation` records, and the earliest evaluation commitment strictly after the latest training outcome. The cluster reference must be the conservative connected-component ID of the graph linking any shared source, event, funder, beneficial owner, or derivation path, so correlated observations are resampled together. Source histories and cash calibration are derived only from eligible training records. Sparse and new sources remain unscored in probabilistic metrics, so the report exposes calibrated coverage rather than silently dropping that limitation.

The command reports source-history metrics, invalid positions, coverage, an explicit disabled cash-model status and paired cluster-bootstrap differences. Legacy `calibrated_cash` predictions now equal source-only predictions; no enabled conditional model is evaluated. The screen cannot validate that model. `predictive_thesis_validated = false` remains mandatory until conditional baseline/evidence models, independent forward evaluation, manipulation/wealth slices, outcome integrity and pre-registration pass. See [the scoped pilot protocol](experiments/PILOT-GATES-2026-10-08.md) for matched-baseline residuals, contextual fidelity, serving terms, full costs and numerical stop gates.

### 4.2 Replay the Brazil election-poll Google benchmark

The first real search vertical replays a dated, signed-out Brazilian Google candidate set through OpenUncertainty Stage 2:

```bash
cargo run --release -p oi-bench -- election-polls \
  --dataset bench/fixtures/brazil-election-polls-google-2026-08-21.json \
  --output table
```

The nine-query/45-candidate snapshot improved the declared policy-utility nDCG@5 from `0.8955` to `0.9985` and reduced hard temporal, geographic, or not-a-poll risk@3 from `18.52%` to `3.70%`. It also exposed a list-level dependency-diversity regression (`2.44` to `2.22` independent clusters@3), which remains a production gap. The experiment uses Google's candidates, policy-authored labels, evidential-only estimates, and no eligible cash positions; it validates neither independent retrieval nor the cash thesis. See the [full experiment report](experiments/BRAZIL-ELECTION-POLLS-2026-08-21.md).

### 4.3 Run the live no-mock search path

The production-path harness starts with an empty persistent directory and exercises the real embedder, local retrieval, live providers, crawl, extraction, evidence hydration, Stage 2, answer construction, and TSE/PesqEle join:

```bash
E2E_DIR="$(mktemp -d /tmp/opensonarx-live.XXXXXX)"
target/release/oi-bench live-search \
  --data-dir "$E2E_DIR" --passes 1 --concurrency 4 \
  --query-variants --top-k 5 --answer --summary-only --output table
```

The 22 August suite now contains 24 semantic cases and four natural surface forms, or 96 requests. The latest complete empty-index run passed `96/96`, grew from `0` to `146` documents, and measured 59.05 s wall time at concurrency four, 1.63 qps, p50 0.93 s, p95 5.09 s, and p99 5.94 s; foreground hydration p99 was 4.30 s and background-crawl p99 was 2.74 s. The final cold defect was an acquisition deadlock in the Brazilian presidential-poll case: a broad query needed first- and second-round corroboration before an answer could become extractive, while round completion previously required an already-extractive answer. Broad Brazilian presidential queries with insufficient evidence now open one bounded two-round pass, and the crawl scheduler reserves enough distinct reporting families for both round slots. The visible poll slate then preserves the one Stage-2 measurement representative, interleaves citations across first- and second-round claims, and moves same-family derivatives behind independent editorial families. A fresh isolated query, all four concurrent surface forms, and the complete 96-request rerun passed without weakening the two-family or exact PesqEle lineage gates.

Reusing that exact cold directory for three passes passed `288/288`, grew from `146` to `177` documents, and measured 23.33 s wall time, 12.35 qps, p50 3.2 ms, p95 1.24 s, and p99 2.40 s; foreground hydration p99 was zero and background-crawl p99 was 1.70 s. The coherent chain therefore passes every declared resident quality, latency, crawl, and throughput gate on the first resident/revalidation matrix. A subsequent no-warmup concurrency-16 soak passed `960/960`, grew from `177` to `210` documents, and measured 26.95 qps, p50 2.9 ms, p95 0.57 s, p99 2.65 s, and background-crawl p99 1.01 s. This is useful single-process concurrency evidence, not a distributed or long-duration scale claim. A preceding same-code-path diagnosis, before temporal background-crawl deduplication, passed quality but reached only 8.71 qps while growing by 64 documents. The background flywheel now skips the same retained exact URL for five minutes, while still acquiring new URLs and leaving stricter foreground claim-freshness policies unchanged; crawl outputs are ingested in one atomic batch to reduce reader/writer contention. A 30-second process-local acquisition cache still coalesces only successful policy-qualified work. Unsupported thematic-news synthesis no longer blocks the result slate on foreground hydration, due diligence uses one recent/contextual provider matrix instead of repeating it as a news retry, and factual incomplete-list abstention is reused only with recent multi-family evidence plus a relation-legible public audit trail. None of these mechanisms turns failed acquisition, a provider snippet, stale evidence, or an incomplete enumeration into a positive answer. These are system measurements, not estimator-only throughput.

For a current Brazilian presidential-poll query, the answer retained the editorial claim, joined exactly `BR-04496/2026`, and emitted machine-readable lineage with one underlying measurement and multiple reporting families. The live TSE/PesqEle client retries one empty JSF response with a fresh session, and institute selection uses the strongest shared context instead of an arbitrary headline token. Once retained editorial evidence already identifies the institute, the critical path uses one institute-specific discovery query instead of blocking on a broad registry listing plus noisy registration-ID disjunctions; the exact record is still mandatory at the final join. The join explicitly says the official registry verifies registration/method metadata and does not verify publisher-reported percentages, realized sampling accuracy, or methodological bias. Known derivatives receive `independence = 0`; unresolved ancestry remains a separate `dependency_uncertainty` input. A broad poll query whose provisional answer lacks first- or second-round coverage opens one bounded, traceable scenario-specific discovery pass. A later audit also caught a same-page alternate slate reporting `39–32`; the unqualified primary-scenario pass recovered two independent editorial families for `39–33` before replacing it. Section-boundary parsing prevents percentages from adjacent slates being joined. If the missing or primary scenario still lacks normal corroboration and lineage, the answer remains partial rather than inferring the result.

For factual relations, compatible citations share one stable `factual_relation_claim` signature across Portuguese/English paraphrases, but `known_independent_observations` remains absent and unresolved derivation channels remain visible. The display and answer predicates are directional: “content created by Claude,” “Founders Fund,” and a creator profile unrelated to the requested company cannot answer “who created/founded Anthropic?”. A bounded MediaWiki Action API query may supply auditable reference leads when general discovery fails, but those leads remain one reference family, never count as independent web-provider redundancy, and never become evidence until fetched and retained. Founder completion queries may combine distinct names extracted from separate retained biographies, but names are only discovery hints. If retained pages still indicate an incomplete enumeration, the system returns useful relation-legible leads and abstains instead of presenting a partial list as complete. This is useful grouping and fail-closed retrieval, not an equivalent registry-resolved graph.

The live harness is a contract suite over 24 semantic cases and four surface forms, not a broad relevance benchmark. Because those cases drove implementation, they are now explicitly classified as a seen development regression suite. Their passing cold and resident results do not validate real cash, Google-scale recall, or market superiority. Every live cash contribution remained zero.

Provider health is measured separately with `oi-bench engine-probe`; it makes live requests to each raw adapter, requires at least one topically relevant HTTP result, and now measures explicit provider concurrency. The default keyless waterfall uses Bing first, Brave public HTML second, and DuckDuckGo as a delayed tertiary hedge. The Brave adapter identifies itself as OpenSonarX and parses only public web-result cards; it does not impersonate a browser and is kept distinct from the authenticated Brave API adapter. Brave public HTML passed a nine-attempt serial probe and a short concurrency-four burst, but the sustained three-class, ten-round, concurrency-four gate exposed rate limiting: Bing passed `30/30`, Brave public HTML passed `12/30`, and the two-provider gate exited non-zero with only Bing healthy. A real no-Bing chaos run over the 24 canonical cases used only Brave public HTML and DuckDuckGo; it passed `17/24` before both providers rate-limited, with seven safe no-result/abstention failures. This makes contracted provider redundancy an observed deployment blocker, not an inferred caveat. Public HTML paths are useful bootstrap capacity but cannot be represented as production SLAs.

A later credentialed probe used the same production adapter path and explicit concurrency. Bing and Serper each passed `30/30`; their measured p50 latencies were 293 ms and 1.03 s, respectively. This removes the observed two-adapter availability blocker for that tested configuration. It does not establish independent upstream indexes, independent failure domains, or a quality advantage.

A separate dated, signed-out `pt-BR`/Brazil Google snapshot covers nine canonical queries across navigation, transactions, tools, typo resolution, due diligence, current polls, factual origin, founder completeness, and generic news. A reproducible comparator applies the same URL/surface/diversity contracts to Google's captured top five and the no-mock OpenSonarX report. On the final coherent retained corpus, Google scored 74.1% and OpenSonarX 100.0%, with four wins, five ties, and no losses. Because these cases drove implementation, this is a seen development regression, not confirmatory evidence. Pix now places the claim-matching Banco Central page first; the incomplete Anthropic-founder query remains an explicit abstention while still presenting relation-legible leads. The fixture excludes Google AI Overview, knowledge panels, maps, shopping, video, and advertising surfaces. No blinded human preference was measured. A companion three-round latency audit reports Google browser wall time to the first visible organic result (p50/p95/p99 `215/407/414 ms`) beside OpenSonarX backend `search_ms` (`1.96/31.02/2400.04 ms`). Eight queries are text-identical; the tool case is a localized equivalent, and its later exact retry hit Google's unusual-traffic interstitial three times. The evaluator therefore keeps `market_latency_compared=false`: these are different surfaces and execution protocols, not a latency race. Personalization and a broad query distribution remain unmeasured; `market_superiority_proven` and `cash_thesis_tested` remain false.

The independent anti-overfit audit changes the overall status materially. Early sealed holdouts failed coverage repeatedly: 40/48 against visible Google organic 48/48, 21/40 in a current/high-impact split, and 13/48 under real keyless-provider rate limiting. Every inspected diagnostic was burned before the next category-level correction. Later credentialed runs restored coverage, then a new pooled evaluator captured raw Serper against the identical manifest, randomized and deduplicated both top-five slates without system identity, required complete candidate judgments, and scored pooled-IDCG nDCG plus paired bootstrap intervals. Under the same system-blind local `gpt-oss:20b` rubric, v10/v11 favored Serper. After query-relative source roles and a bounded per-provider candidate pool, the further-disjoint v12/v13 runs reached parity: acceptable top-one gaps were −2.1 and 0.0 percentage points; relevance nDCG gaps were +0.008 and +0.001; authority nDCG gaps were −0.007 and −0.024; severe top-one error was zero for both systems in both runs. In v12 Stage 2 raised acceptable OpenSonarX top-one count from 24 to 34; in v13, from 21 to 32. Those are meaningful diagnostics, not a general win. Every confidence interval included zero, the judge saw only URL/title/snippet, the judge was not external to implementation, each run had 48 English answer-seeking queries in one stratum, raw Serper is not the full Google product surface, and the declared gate requires at least 1,000 queries across eight strata plus two independent external blinded judges. Cold sources still lack generalized history and lineage; every `p_reliable` remained null and every cash contribution remained zero. See `docs/experiments/GENERALIZATION-HOLDOUT-2026-08-22.md` and the machine-readable `bench/fixtures/generalization-validation-summary-2026-08-22.json`.

Recalculate the exploratory latency artifact from its raw samples:

```bash
cargo run --manifest-path bench/Cargo.toml -- market-latency \
  --fixture bench/fixtures/google-vs-opensonarx-latency-exploratory-2026-08-22.json \
  --output table
```

The last coherent keyless `production-readiness` aggregate uses one corpus for the final cold, resident, and market artifacts plus its sustained keyless provider probe. Sixteen of seventeen search-core gates pass: artifact freshness/lineage, real-network execution, provider depth/capacity, cold and resident quality, live acquisition/crawl, multi-intent coverage, decision safety, cold and resident SLOs, market integrity, and limited candidate quality. Its only search-core failure is sustained provider redundancy (`1/2`). The later credentialed probe passes the observed availability gate (`2/2`) but is intentionally reported separately rather than relabelling the historical aggregate. Four broader research claims remain unvalidated: comparable market latency, broad market superiority, the predictive value of cash, and live calibrated uncertainty. Generalized structured lineage passes for four materially different units: poll measurement, factual relation, current office-holder claim, and market quote snapshot.

The live harness reads `SERPER_API_KEY` and `BRAVE_API_KEY`, as the production CLI does, and puts authenticated engines ahead of keyless fallbacks. Both variables were unset for the earlier 96/288/960-request corpus and its readiness aggregate. Serper was configured for the later provider and disjoint generalization captures described above; no credential is retained in fixtures or documentation.

## 5. Real-world pilot

### 5.1 Choose one observable vertical

The first pilot must use claims whose targets resolve from retained, machine-verifiable records. Every claim must pin:

- exact normalized proposition and direction;
- claim class, task, and time horizon;
- source identity and beneficial/funding cluster where known;
- commitment time, outcome deadline, and grace period;
- outcome source, schema, field, canonicalization, and archive policy;
- capital amount, maximum loss, lock time, predicate coverage, refunds, rebates, insurance, hedges, and funding provenance; and
- immutable evidence/model/policy versions available at prediction time.

The pilot must not begin with broad political truth, medical truth, investigative allegations, or any target requiring a jury to decide what “true” means. It should begin with a narrow event-bound class after jurisdiction-specific legal, payments, custody, tax, gambling, and consumer-protection review.

### 5.2 Sampling and logging

Before observing outcomes:

1. publish the target, cohort rules, exclusions, metrics, model variants, amount buckets, caps, and stop criteria;
2. hash and retain the pre-registration and source code;
3. randomly invite or audit a non-zero share of eligible claims and log the inclusion/offer propensity;
4. retain cash and no-cash claims from the same target population;
5. record withdrawals and unresolved outcomes rather than deleting them;
6. keep training and evaluation labels unavailable to feature construction; and
7. group claims by source, event, funder, and derivation cluster for evaluation.

A useful initial target is at least 5,000 resolved claims, 500 independently controlled sources, and 12 weeks spanning multiple outcome regimes. This is an operational starting point, not a universal power calculation. Final sample size must be determined from a pilot variance estimate and the pre-registered minimum effect.

### 5.3 Leakage-resistant split

Use a forward-time evaluation in which all evaluation outcomes occur after the training cutoff. Prevent the same source-event or funded cluster from leaking across folds where the intended deployment target requires unseen clusters. Report both:

- **known-source forward performance**, which tests regime stability; and
- **cold-source/cluster performance**, which tests generalization and wealth or identity effects.

Confidence intervals must use a block bootstrap over source and event clusters, not an IID claim bootstrap.

### 5.4 Required ablations

Evaluate the same frozen cohort with:

| ID | Model |
|---|---|
| B0 | claim-class and horizon base rate |
| B1 | typed source history only |
| B2 | source history + non-financial evidence, no cash |
| B3 | raw headline cash amount |
| B4 | effective exposure without the learned/capped likelihood |
| B5 | proposed joint/conditional evidence-plus-cash model against the same frozen B2 features; not implemented |
| B6 | declared self-confidence, without cash |
| B7 | popularity/domain-reputation baseline |

B3 and B7 are adversarial baselines, not proposed product features. They show whether the bounded design actually improves on naïve purchased-trust signals.

### 5.5 Preliminary go/no-go gates

Freeze exact gates before collecting the confirmatory cohort. A reasonable starting proposal is:

1. **Primary calibration:** B5 improves Brier score over B2 by at least 3% relatively, and the 95% block-bootstrap confidence interval for the improvement excludes zero.
2. **Proper-score consistency:** B5 improves log loss over B2; an average gain caused only by a few extreme errors fails review.
3. **Calibration safety:** overall ECE does not worsen by more than 0.01, and reliability diagrams do not show systematic overconfidence.
4. **Selective risk:** at matched coverage, B5 reduces error or achieves the same error with higher coverage.
5. **Wealth safety:** no pre-registered wealth, position-size, cold-source, funding, or language slice shows a material overconfidence regression; the nominal whale test remains bounded.
6. **Negative controls:** time-shuffled and uninformative cash produce an effect statistically compatible with zero; post-outcome cash produces exactly zero by eligibility.
7. **Manipulation safety:** detected self-funding, rebate, hedge, and correlated-position strategies cannot create a positive net confidence effect after discounts and cluster collapse.

If cash fails these gates, remove it from the estimator. The evidence, source-history, and two-stage architecture can still succeed without a financial feature.

## 6. Scale and latency gates

Measure the layers separately:

1. estimator arithmetic throughput and allocations;
2. feature lookup/cache hit rate;
3. top-K evidence join p50/p95/p99 latency;
4. end-to-end query p50/p95/p99 latency;
5. cache invalidation after outcomes, corrections, and dependency changes;
6. cost per evaluated claim and per query; and
7. degraded behavior under timeouts and partial graph availability.

MVP-1 clears a preliminary arithmetic gate of more than one million estimates/second on one development core. The next local serving gate is a 100-candidate cached Stage-2 decision within 10 ms p99. No Google-scale claim is warranted until distributed retrieval, coverage, freshness, evidence-join, tail-latency, and cost benchmarks pass independently.

## 7. Immediate execution sequence

1. Select the event-bound pilot class and obtain legal/custody approval for real loss.
2. Replace the implemented MVP-1 JSONL intake with the pilot's immutable split manifest and add Parquet ingestion if cohort size requires it.
3. Add source/event/funder cluster identifiers and logged invitation/audit propensity.
4. Run a non-financial shadow cohort to validate outcome ingestion and leakage controls.
5. Run a small real-cash safety cohort, inspect hedge/funding failures, and freeze confirmatory parameters.
6. Run the confirmatory forward cohort and publish every required ablation and slice, including failures.
7. Only then integrate B5 into a live Stage-2 search policy; Stage 1 remains unchanged.

The decisive result is not whether cash “sounds trustworthy.” It is whether eligible economic conviction improves future calibrated decisions after source history, evidence, selection, dependence, wealth, and manipulation are accounted for.
