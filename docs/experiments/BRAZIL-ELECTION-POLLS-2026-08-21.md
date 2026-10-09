# Brazil Election-Poll Search Experiment — 2026-08-21

**Status:** first executable Google candidate-set comparison; Stage-2 hypothesis supported, end-to-end search and cash theses not yet validated

**Capture:** 2026-08-21, Google Search, `hl=pt-BR`, `gl=br`, `pws=0`, signed out. News/video carousels and advertisements were excluded from the replay candidate set. Google results are volatile, so this document is a dated snapshot rather than a timeless statement about Google.

## 1. What this experiment tests

This is a nine-query, 45-result replay covering:

1. broad current presidential polling;
2. the latest completed national poll;
3. a national second-round query;
4. methodology, margin of error, sample, and TSE registration;
5. official TSE navigation;
6. the ambiguous question “who is winning?”;
7. a São Paulo governor query;
8. a historical 2022 poll snapshot; and
9. the deterministic official 2022 first-round outcome.

The executable fixture retains the observed Google order and adds auditable labels for query relevance, source class, primary/official provenance, temporal state, geography, TSE-metadata completeness, dependency cluster, staleness, distribution shift, and hard-risk flags. OpenUncertainty then runs in `EVIDENTIAL_ONLY` mode and applies its Stage-2 decision policy.

This experiment does **not** use OpenIndex retrieval. It reranks candidates retrieved by Google. It therefore tests whether the uncertainty layer can improve a fixed candidate set, not whether OpenIndex has Google-comparable crawl coverage or recall.

## 2. Ground-truth policy

Brazil has unusually useful structured ground truth for this vertical. Under the current consolidated TSE rules, an election poll must be registered in PesqEle up to five full days before disclosure. The registration includes the sponsor, payer and resource origin, methodology, field period, sample plan and weighting, confidence level, margin of error, control system, questionnaire, responsible statistician, geography, and office. The poll-identification number must appear in publication.

For disclosure, the mandatory visible fields are:

- fieldwork period;
- margin of error;
- confidence level;
- number of interviews;
- pollster and, when applicable, sponsor; and
- TSE registration number.

Registration is provenance, not a TSE truth certificate. The court explicitly states that it performs no prior control over the results and acts when formally prompted. Sources: [TSE 2026 registration guidance](https://www.tse.jus.br/comunicacao/noticias/2026/Janeiro/eleicoes-2026-pesquisas-eleitorais-devem-ser-registradas-a-partir-desta-quinta-1o), [consolidated Resolution 23.600/2019](https://www.tse.jus.br/legislacao/compilada/res/2019/resolucao-no-23-600-de-12-de-dezembro-de-2019), [2026 amendment Resolution 23.747](https://www.tse.jus.br/legislacao/compilada/res/2026/resolucao-no-23-747-de-26-de-fevereiro-de-2026), and the [daily TSE open dataset](https://dadosabertos.tse.jus.br/dataset/pesquisas-eleitorais-2026).

The benchmark keeps three targets separate:

| Target | Question | Resolution mode |
|---|---|---|
| Report fidelity | Did the page faithfully report the named poll, scenario, population, and metadata? | Deterministic comparison with the retained primary report/registry record |
| Poll estimate | What did the sampled population report during the field period? | Statistical estimate with sampling and non-sampling uncertainty |
| Election forecast | How well did a pre-election estimate predict a later vote share or outcome? | Historical, horizon-conditioned calibration only after the official outcome |

A page can accurately report a poll that later misses the election result. Conflating those targets would manufacture false labels.

## 3. Concrete Google behavior observed

### 3.1 Primary-source recovery was inconsistent

For `pesquisa eleitoral presidente Brasil 2026`, Google placed a current primary Nexus report fourth, behind two news pages and Wikipedia. The primary report states the sample (`2,003`), telephone mode, field dates (`14–16 August 2026`), 27-unit national coverage, ±2-point margin, 95% confidence, and TSE registration `BR-03317/2026`: [BTG/Nexus primary report](https://www.nexus.fsb.com.br/estudos-divulgados/pesquisa-btg-nexus-de-intencao-de-votos-para-presidente-do-brasil-17-de-agosto-de-2026/).

The risk-aware replay promoted that primary report to first while retaining secondary summaries for exploration.

### 3.2 “Latest result” included a poll with no result yet

For broad/current queries, Google placed this page prominently: [“A nova pesquisa ... com 6 mil entrevistados”](https://www.cartacapital.com.br/politica/a-nova-pesquisa-de-intencao-de-voto-para-presidente-com-6-mil-entrevistados/). The page was an announcement: collection had not ended and publication was scheduled for 26 August. It contained a registration number and planned method but no poll result.

The replay flags `future_results_absent`, preserves the page for a future-poll intent, and excludes it from a “latest completed result” answer.

### 3.3 A national second-round query returned a DF poll first

For `pesquisa eleitoral 2026 segundo turno Lula Flávio Bolsonaro`, Google's first organic result was [“Real Time/DF: Flávio tem 44% ...; Lula, 39%”](https://www.cnnbrasil.com.br/eleicoes/real-time-df-flavio-tem-44-das-intencoes-de-voto-no-2o-turno-lula-39/). The article clearly states that its 1,600 respondents were in the Federal District and gives registration `BR-05423/2026`.

The document itself is well attributed; the failure is query-document scope. The replay marks `scope_mismatch` and moves national results above it. The DF result remains available when explicitly labeled or when the query asks for the Federal District.

### 3.4 Google performed well on explicit official queries

Google ranked TSE guidance first and Resolution 23.747 third for the methodology query. It also ranked the TSE open-data results first for the deterministic 2022 outcome query. The risk-aware policy kept those official results at the top rather than reranking merely for novelty.

The official 2022 first-round control resolves to 57,259,504 votes (48.43% valid votes) for Lula and 51,072,345 (43.20%) for Bolsonaro: [TSE totalization](https://www.tse.jus.br/comunicacao/noticias/2022/Outubro/100-das-secoes-totalizadas-confira-como-ficou-o-quadro-eleitoral-apos-o-1o-turno).

## 4. Results

Run from the repository root:

```bash
cargo run --release -p oi-bench -- election-polls \
  --dataset bench/fixtures/brazil-election-polls-google-2026-08-21.json \
  --output table
```

| Metric | Google order | OpenUncertainty Stage-2 | Change |
|---|---:|---:|---:|
| Policy-utility nDCG@5 | 0.8955 | 0.9985 | +11.5% relative |
| Mean intent relevance@3 | 0.7667 | 0.8926 | +16.4% relative |
| TSE-metadata completeness@3 | 0.7222 | 0.8085 | +12.0% relative |
| Traceable-result rate@3 | 85.19% | 100.00% | +14.81 points |
| Primary/official source hit@3 | 55.56% | 77.78% | +22.22 points |
| Hard temporal/geographic/not-a-poll risk@3 | 18.52% | 3.70% | **−80.0% relative** |
| Independent evidence clusters@3 | 2.44 | 2.22 | **−9.1% relative** |

All 45 estimates were evidential-only. Cash coverage was `0%`, every cash log Bayes factor was exactly zero, and the ranking therefore could not be purchased in this test.

### Interpretation

The positive result is meaningful but narrow: given Google's candidates and the benchmark's declared policy, claim-level evidence, provenance, time, and geography removed four-fifths of the observed hard-risk exposure in the top three and materially improved ordering.

The diversity regression is also meaningful. The current `DecisionPolicy` scores candidates individually. It can penalize dependency uncertainty, but it does not yet perform list-level cluster diversification or explicit near-duplicate/source-event collapse. A production election-search policy needs a constrained selector or dependency-aware MMR pass after scoring. This benchmark should keep the regression visible until that primitive exists; it should not tune labels to hide it.

## 5. What passed and what did not

| Claim | Result |
|---|---|
| Stage-2 can correct the two observed temporal/geographic failures | **Passed in this captured candidate set** |
| Stage-2 improves the declared trust/relevance utility | **Passed in this captured candidate set** |
| Official/navigational queries remain stable | **Passed** |
| Cash increases real-world reliability | **Not tested** — no eligible prospective commitments |
| OpenIndex retrieval matches Google coverage/recall | **Not tested** — Google supplied the candidates |
| Result-list dependency diversity improves | **Failed** — 2.44 to 2.22 clusters@3 |
| Current polls correctly forecast the 2026 election | **Unresolved and not an appropriate current label** |
| Google-scale latency, freshness, crawl resilience, and cost | **Not tested** |

## 6. Acceptance gates for the vertical

The next pre-registered run should use at least two independent adjudicators plus deterministic validators and freeze these initial gates before evaluation:

| Gate | Initial target |
|---|---:|
| Independent-crawler recall@20 against a pooled Google/Bing/TSE/pollster gold set | ≥ 0.90 |
| Primary or official source hit@3 | ≥ 0.90 |
| Traceable result rate@3 | ≥ 0.95 |
| Hard time/scope/scenario risk@3 | ≤ 0.03 |
| Duplicate underlying-poll exposure@5 | ≤ 0.20 unless the query explicitly requests coverage of that poll |
| Independent evidence clusters@3 | no worse than retrieval baseline |
| Report-fidelity numeric-field accuracy | ≥ 0.995 on deterministic fields |
| Registration-ID precision | 1.000 when one is displayed |
| Cached Stage-2 latency, 100 candidates | p99 < 10 ms |
| End-to-end search latency | separately budgeted and reported at p50/p95/p99 |

The first replay reaches traceability but misses the primary-source, hard-risk, and diversity gates. Its hard-risk rate is 3.70%, just above the proposed 3% threshold.

## 7. End-to-end execution plan

### Track A — live search quality, no cash

1. Ingest the TSE polling dataset daily, preserving extraction time, registration history, questionnaires, payer/sponsor records, invoices, and corrections.
2. Crawl primary institute reports and licensed news sources. Normalize poll identity by registration number rather than URL or headline.
3. Parse office, geography, field dates, population, sample size, mode, margin, confidence, sponsor, first/second-round scenario, candidate set, stimulated/spontaneous form, valid-vote/total-response denominator, undecided/blank/null, and publication time.
4. Deterministically compare every quoted number and label with the retained primary report where access permits.
5. Index primary and secondary documents independently, then compare pooled recall and ranking against dated Google SERPs.
6. Add list-level dependency collapse and measure both original-source recovery and viewpoint/source diversity.

### Track B — historical forecast calibration

Build time-frozen cohorts from past elections. Score vote-share error, interval coverage, bias, house effects, horizon, geography, sample mode, and scenario stability. Do not use one binary “right/wrong” label for an entire poll. Evaluate report fidelity separately from forecast accuracy.

### Track C — prospective cash ablation

Cash can be tested only prospectively. An eligible commitment must bind one exact predicate, be posted before the outcome, specify maximum loss and settlement evidence, and expose hedge, refund, rebate, beneficial-owner, and funder relationships. Randomized invitation/audit propensities and a comparable no-cash cohort must be retained.

For this vertical, legally and statistically safer initial predicates are deterministic report-fidelity claims, such as whether a publication will reproduce the registered sample size, field dates, scenario, and candidate percentages within a declared tolerance. A claim that a candidate “will win” is a different event-bound forecast class and must not share calibration with report fidelity. Real-money operation requires jurisdiction-specific review of election, gambling/prediction-market, payments, custody, tax, consumer-protection, and campaign-finance rules.

The confirmatory cash gate remains the general MVP gate: evidence-plus-cash must improve held-out Brier score over the identical evidence-only model by at least 3% relatively, with a cluster-bootstrap 95% interval excluding zero, no material calibration or wealth/funding-slice regression, and negative controls compatible with zero.

## 8. Immediate engineering gaps exposed

1. **Independent retrieval corpus:** without it, this remains a Google reranker demonstration.
2. **List-level dependency diversification:** the individual scorer does not prevent several pages derived from one poll from occupying the viewport.
3. **Poll entity and scenario schema:** URL-level indexing is insufficient.
4. **Deterministic report-fidelity validator:** registration and article numbers must be joined field by field.
5. **Correction/version tracking:** both poll reports and news pages can change after publication.
6. **Outcome and horizon model:** report fidelity, poll estimates, and election forecasts require different targets.
7. **Continuous SERP capture:** one Brazilian location and one timestamp do not measure volatility, localization, or personalization.
8. **Blind adjudication:** the first labels were policy-authored and must be replaced or confirmed by blinded, double-coded judgments.
9. **Prospective cash cohort:** historical documents cannot validate the conviction signal retroactively.

## 9. Bottom line

This run supports the narrower proposition that OpenUncertainty can make election-poll search safer and more useful after retrieval. It does not yet support the stronger propositions that OpenIndex retrieves the web as well as Google, that the full system works at scale, or that cash adds predictive trust. The observed failures and the diversity regression give the next implementation work a concrete target instead of a narrative target.
