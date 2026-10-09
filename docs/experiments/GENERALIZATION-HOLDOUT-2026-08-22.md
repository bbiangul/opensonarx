# Generalization holdouts — 2026-08-22/23

**Verdict: fail for the mission claim.** The current repository does not yet
demonstrate that OpenSonarX returns more reliable results than Google in general.
The earlier 24-case production suite and nine-case Google comparison are useful
development regressions, but they are seen, policy-authored cases. They cannot be
used as confirmatory evidence after repeatedly driving implementation changes.

This report deliberately separates three questions that were previously too
easy to blur:

1. Does the production path return a useful result slate for independent queries?
2. Are the returned results and answers more relevant and trustworthy than the
   market baseline?
3. Does money add calibrated predictive information after evidence, source
   history, dependence, and selection effects are controlled?

The first question passes in the latest credentialed 48-query answer-seeking
slices, after failing in several earlier slices. That is a narrow coverage
result, not a general retrieval verdict. The second now has a reproducible
pooled, system-blind *diagnostic*: v10/v11 lost to raw Serper, while the two
further-disjoint v12/v13 runs reached statistical parity on relevance and
acceptable top-one rate and remained slightly behind on authority nDCG. The
judge was one local model seeing only URL/title/snippet, not two independent
external assessors, and every run had only 48 queries in one stratum. The
general-superiority gate therefore remains false. The third question was not
exercised because every live cash contribution remained zero.

## Protocol designed against overfitting

`oi-bench generalization-capture` accepts a query-only manifest and refuses:

- fewer than 40 cases;
- duplicate IDs or normalized queries;
- overlap with every canonical or variant query in the development suite;
- reuse of a query from any declared prior holdout, or a hash mismatch in that
  frozen prior manifest;
- the mock embedder.

The capture contains no relevance or trust labels. It hashes the sealed manifest
and evaluated release binary, starts with an isolated empty index, and records the
real discovery, crawl, retrieval, answer, uncertainty, lineage, latency, and
failure traces. `generalization-compare` then verifies identical query IDs/text
and manifest hashes before reporting only objective coverage and slate-diversity
observations. It hard-codes relevance and trustworthiness as unjudged and keeps
the superiority verdict false.

The newer fail-closed path adds four separate steps. A live market capture is
bound to the same manifest hash; OpenSonarX and market top-five slates are
canonicalized, pooled, deduplicated, randomized, and written without system
identity; a judge must return a complete judgment for every immutable pooled
candidate; and only then does a private key recover system/rank membership for
paired scoring. The scorer uses pooled-IDCG nDCG, deterministic paired
bootstrap intervals, exact artifact hashes, and rejects partial, duplicate, or
unblinded judgments. The pre-registered claim gate requires at least 1,000
queries across eight strata and two independent external blinded judges. A
local Ollama judgment is diagnostic and is structurally ineligible to pass it.

Once a holdout has been inspected to diagnose a failure, it is burned: fixes may
be regression-tested against it, but confirmation must use another disjoint,
sealed manifest and a frozen binary. The manifests record a deterministic sample
recipe and source hashes. This is stronger than picking a fresh-looking list by
hand after seeing the results.

The first answer-seeking samples come from the
[official MS MARCO dataset](https://www.microsoft.com/en-us/research/publication/ms-marco-human-generated-machine-reading-comprehension-dataset/),
which uses anonymized queries from a commercial search engine. This is useful
external traffic, but still only one English answer-seeking stratum. Future
confirmation must add heterogeneous
[BEIR tasks](https://github.com/beir-cellar/beir),
[Natural Questions](https://research.google/pubs/natural-questions-a-benchmark-for-question-answering-research/),
Portuguese traffic, navigation, local, commerce, current events, and
adversarial/dependence slices.

## Results

| Frozen run | Scope | OpenSonarX non-empty | Market observation | Interpretation |
|---|---|---:|---|---|
| MS MARCO holdout v1 | 48 independent English answer-seeking queries | 40/48 (83.3%) | Google 48/48 (100%) | OpenSonarX missed eight queries Google covered. |
| Current/high-impact holdout v1 | 25 current Google Trends BR + 15 independently sampled health/legal/finance candidates | 21/40 (52.5%) | Google capture blocked after the earlier batch | Coverage failed; the Google interstitial is a protocol limitation, not a Google quality loss. |
| MS MARCO holdout v2, post-fix frozen binary | 48 new queries excluding both earlier manifests | 13/48 (27.1%) | No new Google capture | Real keyless-provider saturation dominated the E2E result; this is a service-availability failure, not an isolated ranking estimate. |
| MS MARCO v3/v4/v5, credentialed iterative holdouts | 48 new queries per run | 39/48, 42/48, then 48/48 | Raw Serper had 48/48 candidate slates in the comparable later captures | Each inspected run was burned before the next fix; v5 closed coverage but did not judge trust. |
| High-impact v2/v3 | 48 new health/legal/finance candidates per run | 45/48, then 47/48 | Raw Serper 48/48 on v2 | Exposed a topic-wrong health authority registry and incomplete reliance routing; neither run is a quality win. |
| MS MARCO v6/v7, trust-path diagnostics | 48 new queries per run | 45/48, then 44/48 | Raw Serper 48/48 on v7 | v6 exposed trust rescuing an off-topic retained page; v7 verified the relevance guard but exposed intent/coverage gaps. |
| MS MARCO v8 | 48 further disjoint English answer-seeking queries | 48/48 (100%) | Raw Serper 48/48 | First narrow credentialed coverage pass; later code changes burned it as confirmation. |
| MS MARCO v9, post-relevance-guard diagnostic | 48 further disjoint queries | 45/48 (93.8%) | Raw Serper 48/48 | Compact informational queries were still being misrouted as navigation; the failure is retained rather than hidden. |
| MS MARCO v10, pre-blind diagnostic | 48 further disjoint English answer-seeking queries | 48/48 (100%) | Raw Serper 48/48 | Coverage and Stage-2 execution passed, but the later system-blind score clearly favored Serper. |
| MS MARCO v11, source-role diagnostic | 48 further-disjoint queries | 47/48 (97.9%) | Raw Serper 48/48 | Blind local-model score still favored Serper: acceptable top-one 58.3% vs 64.6%, relevance nDCG 0.758 vs 0.844, authority nDCG 0.619 vs 0.737. A conceptual `A vs B definition` query was misrouted as a live event. |
| MS MARCO v12, wider candidate pool | 48 further-disjoint queries | 48/48 (100%) | Raw Serper 48/48 | First blind diagnostic parity: acceptable top-one 70.8% vs 72.9%, relevance nDCG 0.841 vs 0.833, authority nDCG 0.728 vs 0.735; every 95% interval included zero. |
| **MS MARCO v13, ambiguous-navigation reliance confirmation** | **48 further-disjoint queries** | **48/48 (100%)** | **Raw Serper 48/48** | **Parity repeated: acceptable top-one 66.7% vs 66.7%, relevance nDCG 0.870 vs 0.869, authority nDCG 0.716 vs 0.740; every 95% interval included zero and superiority remained false.** |

With authenticated Serper configured beside Bing, the sustained three-query,
ten-round, concurrency-four availability probe passed both engines: Bing 30/30
with p50 293 ms and Serper 30/30 with p50 1.03 s. This removes the observed
two-provider availability blocker for that configuration. It does not establish
that their upstream indexes or ranking signals are independent.

V10–v13 all used empty indexes, real Arctic embedding, live Serper+Bing,
crawling, concurrency four, and frozen release binaries. V12 widened the
bounded internal discovery pool so two configured providers could each
contribute top-five depth before the final top-five decision; the displayed
limit did not change. V12 returned 48/48 with backend p50/p95/p99
1.02/2.35/4.52 seconds versus raw Serper 1.00/1.73/3.01 seconds. V13 returned
48/48 with 1.05/3.21/9.43 seconds versus raw Serper 0.99/1.73/1.94 seconds. The
v13 tail came from two factual verification workloads: three to four search
passes and foreground hydration of three and seven pages, not from every
ordinary trust rerank.

The source-role correction distinguishes query-relative primary sources,
edited references, unclassified sources, and unverified UGC only inside the
existing relevance band. It is a reliance-ordering prior, not a truth
probability. In v12 the raw retrieval order had 24 acceptable top-one results;
the final decision had 34, with seven directly comparable failures corrected
and none harmed. In v13 the corresponding counts were 21 and 32, with nine
corrected and two harmed. Edited-reference top-one results were acceptable in
8/8 v12 cases and 16/18 v13 cases; unclassified sources remained the largest
error bucket. V13 also routed an initially navigational but unverified short
query through `EVIDENTIAL_ONLY`, while verified destinations such as a matching
first-party host remain `not_applicable`. Every `p_reliable` stayed null and
every cash contribution stayed zero.

Unblinded diagnostic inspection makes the limitation concrete. The v10 slate
put a dermatology blog above the American Cancer Society and PubMed for an
untreated-melanoma survival query; a Facebook group first for safe chicken
temperature; Quizlet first for a congressional-organization question; and one
Reddit result for “volume of earth,” which was still treated as navigation.
Raw Serper also exposed weak sources in several cases. The later blind scores
show that the wider pool plus query-relative role prior can materially improve
OpenSonarX's own retrieval order and reach parity in two disjoint samples. They
also show why the claim must remain narrow: cold-source history, generalized
claim evidence, lineage, independent human judgments, heterogeneous strata,
and statistical power are still absent.

For MS MARCO v1, Google averaged 4.92 visible organic results and 4.77 distinct
domains in the top five. OpenSonarX averaged 3.17 results and 2.88 domains. Google
was the only side with a non-empty slate on eight cases; OpenSonarX was the only
side on none. The top host matched on five cases. Google displayed an AI Overview
on 26/48 pages, but presence was not counted as correctness.

The measured OpenSonarX backend p50/p95/p99 was
1.45/2.59/2.75 seconds. Google's browser-to-visible-organic wall time was
0.45/0.60/0.88 seconds. These numbers are reported side by side for operational
context but are not comparable endpoints, so no latency winner is declared.

The current/high-impact split was particularly revealing:

| Stratum | Non-empty | Intent behavior before diagnosis |
|---|---:|---|
| Current Google Trends BR | 16/25 | 16/25 were classified as navigation; only 1/25 as current events |
| Health/legal/finance candidates | 5/15 | 1/15 was incorrectly allowed to behave as navigation; no result received applicable uncertainty metadata |

Observed failure classes included head-to-head events treated as website
destinations, cross-query current-event contamination, short medical terms
presented as canonical destinations, third-party phone/routing pages, irrelevant
top results, and safe but very frequent empty slates.

The post-fix disjoint MS MARCO v2 run used the real release binary, concurrency
four, crawling, and an empty index. It returned only 13/48 non-empty slates,
grew from 0 to 88 documents, and measured backend p50/p95/p99 of
0.61/2.66/3.37 seconds. Brave public HTML returned HTTP 429; DuckDuckGo returned
HTTP 403 and a timeout; the existing sustained-provider gate had already shown
only Bing meeting its declared capacity requirement. The system failed closed,
which is preferable to fabrication, but reliable general search also requires
availability and recall. Abstention alone cannot satisfy the mission.

## General corrections made after diagnosis

The fixes were expressed as category rules and tested with synthetic names not
present in any holdout:

- a short `A x B`, `A vs B`, or `A versus B` query is a current event, and
  current evidence must visibly cover both named sides;
- an unstructured `game/jogo/match/partida` query must preserve its named
  subject, preventing a fresh article from another query from winning merely
  because it is recent;
- health/legal/financial guidance is claim-bearing even when short, so Stage 2
  emits evidential uncertainty instead of routing it as a website;
- a short ambiguous query is only labelled a destination when the top result
  has a verifiable entity-host match or a registered public-authority path.
- a topic-specific authority registry cannot stand in for an entire domain;
  the dengue entrypoints are no longer injected into unrelated health queries;
- every informational/exploratory slate now receives evidential Stage 2, while
  navigation, transactions, and tool routing remain outside epistemic ranking;
- Stage 2 may reorder only within a top-relevance band, so retained/auditable
  content cannot overcome a materially stronger topical candidate;
- provider sanity ignores question scaffolding and generic words such as
  “benefits”, and product/navigation fallback may preserve an ambiguous result
  without calling it the official destination.
- structurally non-answering evidence (wrong entity, future-results preview,
  self-declaration, event page, derivative, or topic hub) cannot define the
  relevance band that blocks an answer-bearing alternative;
- compact definition forms and biomedical risk/viral vocabulary are routed as
  information rather than websites. This category fix moved the next disjoint
  run from v9's 45/48 back to v10's 48/48; v9 remains a burned failure.
- query-relative source roles break otherwise-identical cold-source uncertainty
  ties inside the relevance band, without assigning a probability or using
  stake/cash;
- the discovery pool preserves bounded top-k depth per configured provider
  before the final top-k decision, rather than discarding half of each
  provider's slate during interleaving;
- explanatory comparisons such as `A vs B definition` are not live events;
- a short query is exempt from reliance assessment only when a canonical host
  or public-authority path verifies the destination. Ambiguous short slates
  remain informational even if the lexical classifier initially says
  navigation.

These changes remove unsafe behavior classes. They do not repair missing web
coverage, prove relevance gains, or constitute a post-fix market win.

## What must happen before a superiority claim

A confirmatory program must be precommitted before capture and must include:

- at least 1,000 disjoint queries across answer, navigation, local, current,
  commerce, high-impact, multilingual, and adversarial strata;
- pooled and randomized Google/OpenSonarX result slates with blinded human
  judgments for relevance, answer correctness, query-relative authority,
  provenance, independence, freshness, and unsupported-claim severity;
- nDCG/recall, selective risk versus coverage, catastrophic-error rate, source
  diversity/dependence, original-source recovery, and confidence intervals by
  stratum—not a single hand-weighted score;
- identical locale, device, personalization, time window, and comparable latency
  endpoints; blocked vendor captures are missing data, never losses;
- a frozen binary and no tuning until the scored report is archived;
- a separate prospective cash/no-cash ablation on resolvable claims, reporting
  Brier/log loss/ECE and wealth/funder/position-size/manipulation slices.

Until those gates pass, the correct product statement is: the architecture has
promising safety primitives, demonstrated fail-closed behavior, and repeatedly
closed coverage on disjoint credentialed answer-seeking slices after visible
intermediate failures, but it has not
demonstrated more trustworthy general-purpose results than Google. Two
further-disjoint local-model diagnostics now show parity with raw Serper in one
English answer-seeking stratum, which is meaningful progress but not the claim.
The next bottleneck is independent blinded judgment across heterogeneous
strata, plus real source history, claim lineage, and prospective outcomes.

## Reproduction

```bash
cargo build --release -p oi-bench --features onnx

HOLDOUT_DIR="$(mktemp -d /tmp/opensonarx-generalization.XXXXXX)"
target/release/oi-bench generalization-capture \
  --manifest bench/fixtures/generalization-msmarco-holdout-v1.json \
  --data-dir "$HOLDOUT_DIR" --passes 1 --concurrency 4 --top-k 5 \
  --crawl true --output json --report-path /tmp/opensonarx-holdout.json

target/release/oi-bench generalization-compare \
  --manifest bench/fixtures/generalization-msmarco-holdout-v1.json \
  --opensonarx-report /tmp/opensonarx-holdout.json \
  --google-report bench/fixtures/google-generalization-msmarco-capture-v1.json \
  --output table

# Blind pooled market diagnostic (SERPER_API_KEY must already be exported).
target/release/oi-bench generalization-market-capture \
  --manifest "$MANIFEST" --engine serper --top-k 5 --concurrency 4 \
  --report-path /tmp/market.json

target/release/oi-bench generalization-blind-pool \
  --manifest "$MANIFEST" --opensonarx-report /tmp/open.json \
  --market-report /tmp/market.json --top-k 5 \
  --blinding-nonce "$BLIND_NONCE" --packet-path /tmp/public-packet.json \
  --key-path /tmp/private-key.json

# Send only the public packet to independent judges. A local-model diagnostic
# is supported but is explicitly ineligible for the superiority gate.
target/release/oi-bench generalization-blind-ollama-judge \
  --packet /tmp/public-packet.json --model gpt-oss:20b \
  --judgments-path /tmp/local-diagnostic.json

target/release/oi-bench generalization-blind-score \
  --packet /tmp/public-packet.json --key /tmp/private-key.json \
  --judgments /tmp/local-diagnostic.json --bootstrap-iterations 10000
```

Machine-readable frozen summary:
`bench/fixtures/generalization-validation-summary-2026-08-22.json`.
