# Gama Fund end-to-end search trace — 2026-08-21

**Status:** live execution of the current CLI/SDK path; not a simulation of the whitepaper architecture

> Historical baseline: this trace was captured before the intent, answer, and
> OpenUncertainty integration described in the later
> [live production-path report](LIVE-PRODUCTION-E2E-2026-08-21.md). Statements
> below about missing routing or Stage 2 describe that earlier executable, not
> the current one.

## Scope

This run tests the currently wired path for four queries:

1. `gama fund`
2. `gama fund inscrição`
3. `gama fund é legítimo`
4. `claude` as a navigational control

The commands used the existing local index, the MiniLM-compatible ONNX embedding path, the default DuckDuckGo fallback, Brazilian locale hints, and crawl-on-query. The run intentionally reports what the current product does, including missing layers.

## Current executable path

```text
query
  -> optional vocabulary spell correction
  -> query embedding
  -> local HybridIndex query at requested top_k
  -> external DuckDuckGo query when local coverage is insufficient
  -> title/snippet embedding and common-scale semantic rescoring
  -> URL deduplication and a three-results-per-domain cap
  -> final top_k result list
  -> queue up to three external URLs
  -> robots check, fetch, distill, quality gate, embed, deduplicate, ingest, persist
```

The normal CLI/SDK path currently stops at a ranked result list. It does not call a query-intent router, claim extraction, an evidence/dependency graph, `OpenUncertainty::estimate`, `DecisionPolicy`, or an answer synthesizer. The OpenUncertainty implementation exists and its nine focused tests pass, but it is not wired into `oi search`.

## Environment observed

After the live runs, `oi index info` reported:

- 59,121 documents;
- 57,756 from `msmarco.example` (97.7% of the index);
- HNSW, BM25, and metadata counts consistent;
- 9 documents missing titles;
- 42,758 orphan dedup hashes.

The overwhelming MS MARCO share materially affects short entity queries: generic English passages can become local candidates before a newly crawled Brazilian entity.

## Result 1 — `gama fund`, first captured result list

| Rank | Result | Source | Final score |
|---:|---|---|---:|
| 1 | GAMA Asset Management (`gama-am.ch`) | DuckDuckGo | 0.6664 |
| 2 | Our funds — GAMA Asset Management | DuckDuckGo | 0.6595 |
| 3 | Fundos — Gama Investimentos | DuckDuckGo | 0.6173 |
| 4 | Gama Fund official (`gamafund.com`) | DuckDuckGo | 0.6043 |
| 5 | Generic “fund of funds” MS MARCO passage | Local | 0.5888 |

Observed behavior:

- the exact official entity was present but ranked fourth;
- two Swiss homonyms outranked it;
- one irrelevant generic passage entered the top five;
- no navigational intent or exact-entity policy was applied;
- up to three external results were queued for learning, and two pages were indexed in this captured run.

## Result 2 — `gama fund inscrição`

| Rank | Result | Source | Final score |
|---:|---|---|---:|
| 1 | Startups.com.br article about the extended application deadline | DuckDuckGo | 0.6002 |
| 2 | Irrelevant BlackRock municipal fund passage | Local MS MARCO | 0.5827 |
| 3 | Gama Fund official | DuckDuckGo | 0.5731 |
| 4 | GenAI Lab Sessions: Gama Fund | DuckDuckGo | 0.5674 |
| 5 | Another irrelevant BlackRock fund passage | Local MS MARCO | 0.5593 |

Observed behavior:

- the news article correctly matched the word `inscrição`, but the system did not recognize the query as transactional;
- the official application destination did not lead;
- two irrelevant local finance passages survived;
- three surfaced pages were successfully learned.

## Result 3 — `gama fund é legítimo`

| Rank | Result | Source | Final score |
|---:|---|---|---:|
| 1 | Luma event page about Gama Fund | Local | 0.7465 |
| 2 | Startups.com.br article | Local | 0.7222 |
| 3 | Gama Investimentos homonym | DuckDuckGo | 0.6875 |
| 4 | Status Invest homonym | DuckDuckGo | 0.6313 |
| 5 | Gama Fund official | DuckDuckGo | 0.5894 |

Observed behavior:

- crawl-on-query made previously surfaced pages locally searchable;
- the system interpreted lexical/semantic similarity but did not execute due diligence;
- it did not check official partner confirmation, legal identity, domain binding, source independence, or conflicts;
- it produced no `DETERMINISTIC`, `CALIBRATED`, or `EVIDENTIAL_ONLY` result;
- the official site fell to fifth place.

## Result 4 — `claude` navigational control

| Rank | Result | Source | Final score |
|---:|---|---|---:|
| 1 | `claude.com` | DuckDuckGo | 0.7054 |
| 2 | Georges Claude passage | Local MS MARCO | 0.6996 |
| 3 | Another Georges Claude passage | Local MS MARCO | 0.6944 |
| 4 | Georges Claude biographical passage | Local MS MARCO | 0.6805 |
| 5 | `claude.ai` sign-in | DuckDuckGo | 0.6354 |

The desired destination happened to rank first, but three irrelevant biographical passages occupied positions two through four. This is a relevance success at rank one, not evidence that navigational intent routing exists.

## Crawl and second-query behavior

A direct one-page spider crawl of `https://gamafund.com` succeeded:

- one URL discovered;
- one page crawled;
- zero failures;
- zero robots exclusions;
- one page indexed.

The distilled local document correctly contains the official program schedule, co-investment offer, Google/DeepMind benefits, target company profile, and FAQ headings.

With a site-constrained local-only query, this document was returned first. With an unconstrained local-only query at `top_k=20`, the local order was:

1. Luma event page;
2. Startups.com.br article;
3. Gama Fund official;
4. Graticule Asset Management Asia homonym;
5. Status Invest homonym.

At `top_k=5`, the local HNSW/BM25 candidate set contained only generic MS MARCO fund passages. This exposes a candidate-depth coupling: the requested output size is also used as the local retrieval candidate depth, so a correct entity can disappear before common-scale rescoring. Increasing the output limit to 20 made the official page available, but it still did not receive a navigational/entity boost.

After learning, the repeated blended `gama fund` query still placed external homonyms above the official domain. Learning improved coverage, but not intent correctness.

## What passed

- Release build of `oi-cli` completed.
- Live external retrieval completed.
- Local HNSW/BM25 retrieval completed.
- Common-scale semantic rescoring completed.
- URL/domain deduplication path completed.
- Crawl-on-query fetched and indexed pages.
- Direct crawl of the official site succeeded.
- The index remained structurally consistent.
- All nine focused `oi-index` OpenUncertainty tests passed.
- The external-engine URL deduplication test passed.

## What is missing from the live path

1. **Intent routing:** no distinction among navigation, transaction, news, due diligence, comparison, or factual verification.
2. **Entity resolution:** exact official entity/domain relationships do not receive an explicit navigational treatment.
3. **Candidate over-retrieval:** local candidate depth is tied to output `top_k`; correct entities can be pruned before reranking.
4. **Corpus isolation:** an evaluation corpus representing 97.7% of the index competes with live web entities without a production-corpus boundary.
5. **Claim extraction:** no decomposition of fund size, partners, deadlines, eligibility, or benefits.
6. **Evidence graph:** no distinction among self-description, partner confirmation, independent reporting, and copied press material.
7. **OpenUncertainty integration:** the estimator exists only as an isolated module/benchmark for this path.
8. **Decision policy and dual ranks:** no `retrieval_rank` versus `decision_rank` in normal search.
9. **Answer construction:** the CLI returns links and snippets, not the evidence-aware answer described in the whitepaper.
10. **Trace contract:** verbose logs expose implementation activity but there is no compact structured decision trace for a user or evaluator.
11. **Legacy economic wording:** every result set printed `external results — not staked` and suggested staking a domain “to vouch for quality.” That message contradicts v2.4: domain stake is not an eligible claim-specific conviction signal and cannot imply quality, relevance, or calibrated reliance.

## Minimum next E2E slice

A defensible next implementation should add, in this order:

1. a versioned intent/entity router with navigational and transactional policies;
2. independent retrieval depth (for example, retrieve 50–100 candidates before returning five);
3. an evaluation/production corpus boundary;
4. exact-domain and official-entity features that remain stake-neutral;
5. claim/evidence extraction for only the rerank candidate set;
6. `EVIDENTIAL_ONLY` integration first, without cash;
7. dual ranks plus a structured explanation trace;
8. answer construction that distinguishes official declarations from independent confirmation.

Only after this slice works should the cash-calibrated path be connected to event-bound queries. `gama fund` itself has no eligible cash-at-risk signal: the advertised fund size is a content claim, not a protocol position against an objective outcome.
