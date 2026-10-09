# Live production-path E2E validation — 2026-08-21/22

**Status:** a real end-to-end milestone passed across multiple query intents; useful and materially safer than the earlier executable, but not yet production-ready or evidence that OpenSonarX is a Google-scale replacement.

## 1. What actually ran

The runs used the release binaries, the real `arctic-embed-s` ONNX embedder, a persistent HNSW/BM25 index, live credential-free Bing and DuckDuckGo discovery, real robots-aware fetch and extraction, content distillation, deduplication, intent routing, Stage-2 OpenUncertainty metadata, and the CLI answer path. No mock embedder, synthetic SERP, or canned answer was accepted by this benchmark.

```text
query
  -> classify intent
  -> embed with Arctic Embed S
  -> retrieve 50-100 local candidates for a 5-result request
  -> test intent-specific local coverage
  -> if weak: neutral intent-family rewrites + live external discovery
       -> provider cache/single-flight + deadline/circuit breaker
  -> common-scale relevance + intent reranking (stake-neutral)
  -> treat external metadata as discovery leads, never retained claim evidence
  -> crawl selected source families through per-URL single-flight
  -> distill, index, persist, then retrieve the shared index again
  -> claim-specific discovery when retained evidence is incomplete
  -> enforce final date / entity–relation / primary-authority gates
  -> retained-evidence extraction
  -> Stage-2 decision metadata
  -> destination, extractive claims with citations, or explicit abstention
```

Cash did not affect any rank or answer in these runs. There were no eligible, calibrated, claim-specific positions, so every `cash_log_bayes_factor` was exactly zero and no `p_reliable` was fabricated. Outputs correctly remained `EVIDENTIAL_ONLY` for factual/current/due-diligence cases and `not_applicable` for navigation/action routing.

## 2. Test suite

The executable suite now contains 24 canonical cases. Each case also has four
natural surface forms, producing a 96-request paraphrase/concurrency suite:

| Class | Cases |
|---|---|
| Navigational / ambiguous | Claude, ChatGPT, Perplexity, Gama Fund, typo `gama found`, Nubank, Receita Federal, Java, Python |
| Transactional | Gama Fund application, Claude login |
| Tool | Python online compiler |
| Due diligence | “gama fund é legítimo” |
| Current structured fact | Brazilian presidential poll, current president, Bitcoin price |
| Factual | Anthropic founders, Pix creator |
| Current thematic | latest AI news |
| Exploratory / high stakes | quantum-computing definition, tax guidance, dengue guidance |
| Local / recommendation | nearby Japanese restaurants, programming notebook recommendation |

Variants include word-order changes (`login claude`), explicit destination
phrasing (`site oficial ...`), additional typos (`gama fnd`), relation
paraphrases (`quem inventou o Pix?`, `qual instituição criou o Pix?`), founder
paraphrases (`quem criou a Anthropic?`), and current-poll formulations
(`última pesquisa para presidente do brasil`). They reuse the case's semantic
contract; they are not string-only assertions.

The gates check expected answer mode, top destination constraints, forbidden noise, decision metadata on every top-five result, absence of fabricated probabilities, zero uncalibrated cash effect, citation count, and structured poll-claim shape. These are smoke/contract gates, not a complete human relevance evaluation.

## 3. Measured results

### Historical v149 claim-lineage snapshot — 22 August 2026

After the TSE/PesqEle join, measurement-level lineage, recrawl-freshness fix,
persistent registry cache, multilingual factual-flight normalization, URL
canonicalization, answer-aware primary expansion, canonical-origin hardening,
and bounded poll-scenario completion, the latest empty-index run produced:

| Run | Quality | Documents | Wall | Throughput | Search p50 | Search p95 | Search p99 | Hydration p95 / p99 |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| cold v44, concurrency 16 | 64/64 | 0→123 | 17.02 s | 3.8 qps | 1.43 s | 4.97 s | 7.39 s | 2.84 s / 4.53 s |
| resident v44, 192 requests, concurrency 24 | 192/192 | 123→123 | 11.56 s | 16.6 qps | 4.2 ms | 1.01 s | 2.81 s | 586 ms / 1.24 s |
| cold v45, concurrency 16 | 64/64 | 0→123 | 19.53 s | 3.3 qps | 867 ms | 6.80 s | 7.90 s | 3.42 s / 4.53 s |
| resident v45, 192 requests, concurrency 24 | 192/192 | 123→123 | 11.34 s | 16.9 qps | 5.3 ms | 896 ms | 2.77 s | 564 ms / 1.21 s |
| cold v46, concurrency 16 | 64/64 | 0→124 | 19.13 s | 3.3 qps | 896 ms | 7.75 s | 8.15 s | 4.53 s / 4.73 s |
| cold v47, concurrency 16 | 63/64 | 0→115 | 20.08 s | 3.2 qps | 736 ms | 7.00 s | 7.50 s | 4.23 s / 4.96 s |
| cold v48, concurrency 16 | 64/64 | 0→118 | 16.80 s | 3.8 qps | 745 ms | 6.60 s | 7.61 s | 3.71 s / 4.99 s |
| resident v48, 192 requests, concurrency 16 | 192/192 | 118→118 | 14.52 s | 13.2 qps | 4.8 ms | 1.05 s | 2.49 s | 619 ms / 1.24 s |
| cold v52, concurrency 17 | 68/68 | 0→126 | 18.39 s | 3.7 qps | 807 ms | 6.77 s | 7.57 s | 4.01 s / 4.70 s |
| resident v52, 204 requests, concurrency 17 | 204/204 | 126→126 | 12.12 s | 16.8 qps | 4.2 ms | 1.04 s | 2.76 s | 592 ms / 1.08 s |
| cold v55, concurrency 17 | 68/68 | 0→134 | 20.22 s | 3.4 qps | 822 ms | 9.57 s | 9.62 s | 4.57 s / 4.71 s |
| resident v55, 204 requests, concurrency 17 | 204/204 | 134→134 | 11.98 s | 17.0 qps | 3.2 ms | 1.08 s | 2.65 s | 614 ms / 1.11 s |
| cold v65, concurrency 17 | 68/68 | 0→121 | 16.30 s | 4.2 qps | 631 ms | 6.68 s | 6.87 s | 3.56 s / 4.07 s |
| resident v65, 204 requests, concurrency 17 | 204/204 | 121→121 | 14.33 s | 14.2 qps | 8.0 ms | 1.16 s | 2.44 s | 0 ms / 1.18 s |
| cold v67, concurrency 17 | 68/68 | 0→122 | 17.41 s | 3.9 qps | 744 ms | 7.14 s | 7.20 s | 3.96 s / 4.08 s |
| resident v67, 204 requests, concurrency 17 | 204/204 | 122→122 | 12.74 s | 16.0 qps | 6.4 ms | 1.22 s | 2.63 s | 0 ms / 657 ms |
| cold v113, concurrency 4 | 64/68 | live empty index | — | — | — | — | — | — |
| cold v115, concurrency 4 | 60/68 | live empty index | — | — | — | — | — | — |
| cold v118, concurrency 4 | 68/68 | 0→131 | 38.46 s | 1.8 qps | 731 ms | 6.57 s | 7.06 s | 2.83 s / 4.07 s |
| resident v119, 204 requests, concurrency 4 | 204/204 | 131→131 | 9.17 s | 22.2 qps | 1.9 ms | 1.43 s | 2.31 s | 0 ms / 0 ms |
| cold v143b, concurrency 1 diagnostic | 68/68 | 0→96 | 36.80 s | 1.8 qps | 3.5 ms | 2.22 s | 8.29 s | 742 ms / 5.95 s |
| cold v145, concurrency 1 diagnostic | 67/68 | live empty index | — | — | — | — | 14.02 s | — / 5.78 s |
| cold v148, concurrency 1 diagnostic | 68/68 | 0→92 | 37.78 s | 1.8 qps | 3.7 ms | 2.25 s | 5.56 s | 544 ms / 4.07 s |
| **cold v149, concurrency 4** | **68/68** | **0→84** | **34.02 s** | **2.0 qps** | **556 ms** | **5.44 s** | **5.44 s** | **1.45 s / 3.70 s** |
| **resident v149, 204 requests, concurrency 4** | **204/204** | **84→89** | **8.02 s** | **25.4 qps** | **1.3 ms** | **15.5 ms** | **3.82 s** | **0 ms / <1 ms** |

The passing cold runs span p95 4.97–9.57 seconds while p99 remains in the same
broad seconds-level range. That variance is part of the result: the
credential-free live provider/crawl path is not a deterministic latency
service and must not be represented by the fastest run alone.

The final v149 pair is the coherent readiness input: same release binary, same
data directory, concurrency four from the first cold request onward, 68/68 from
empty state followed by 204/204 resident. The aggregate evaluator passed 15/16
search-core gates and failed only independent provider redundancy. The v113
and v115 failures are equally important. v113 lost all four poll forms
after DuckDuckGo returned 403, PesqEle returned an empty transient response,
and Bing did not recover enough matching coverage. v115 lost the four tool
forms and four poll forms under a different provider burst. A focused
production-path tool rerun then passed 4/4 in 352 ms p99, confirming volatility
rather than a relaxed contract. The architecture can complete the mission path,
but one-provider discovery cannot yet do so reliably.

The later serial diagnostics found two different defects that the v118 success
did not reveal. v143b passed 68/68 but exceeded the five-second foreground
hydration gate on Pix. v145 then passed only 67/68: a Skilljar profile whose
snippet named an Anthropic co-founder survived as the top lead even though its
predicate was about creating Skilljar, while the poll path reached 14.02 s and
Pix opened an unnecessary third-family crawl. The directional creator-first
gate now requires the requested entity to be the direct object (with only a
bounded `of` connector), final factual abstention reruns Stage 2 and normalizes
display ranks, and factual completion requests only the missing answer-bearing
families. Focused Anthropic, Pix, and poll reruns passed 4/4 before v148 restored
68/68 under a serial diagnostic and v149 passed the actual concurrent profile.

Resident latency improved after equivalent requests began sharing successful
answer acquisition for 30 seconds and policy-qualified fresh evidence gained a
five-minute local snapshot path. Cache keys preserve intent semantics: `created`
and `founded` are not merged, and generic current-event queries retain their
exact identity. Positive cache entries are limited to complete/qualified
outcomes or explicit evidence-bounded abstentions; empty/provider-failure
acquisition is never cached. The fresh path requires recent crawls plus two
source families for polls or three for generic current news. It is therefore a
bounded evidence reuse mechanism, not a confidence shortcut.

The cold p99 hydration gate was brought below five seconds without weakening
poll or primary-source repair. When an incomplete factual enumeration has
already spent four seconds hydrating, the system may still discover completion
candidates but skips a second crawl and applies the normal fail-closed answer
gate. After the stricter display correction, a focused empty-index Anthropic
run passed 4/4 with p99 search 6.18 s and p99 hydration 2.60 s while correctly
remaining `insufficient_evidence`. A focused Pix run passed 4/4 with p99 search
6.97 s and hydration 4.35 s, and the final v149 full run lowered aggregate cold
hydration p99 to 3.70 s.

The v67 run also closes a due-diligence concurrency race. The first v66 cold
run passed 65/68: only one of four equivalent Gama Fund risk phrasings received
the independent articles before keyless-provider capacity was exhausted.
Due-diligence discovery now canonicalizes all risk phrasings into one neutral
entity flight and one bounded adverse-evidence flight; the original wording
still owns filtering and abstention. A fresh focused run passed 4/4, and the
full v67 matrix restored 68/68.

The v65 run strengthened correctness rather than merely changing timing.
Provider batches now need a topically relevant result, so parseable but
unrelated URLs are discarded before blending and crawl. Transactional routing
also follows action links declared by an entity-matching first-party root. All
four cold Claude login forms reached the canonical `https://claude.ai/login`
in 286–299 ms; a persisted route answered the focused resident query in 10.3
ms. The route remains discovery metadata and contributes no factual trust.

The first v46 resident run passed only 180/192 (93.8%): after crawling,
`python.codeutility.io` outranked `python.org` because the first subdomain label
was treated as the entity host. A later diagnostic exposed the same class in
`social.perplexity.ai`, an internal Nubank SSO route, and a Receita service
subdomain. Canonical destination identity now uses the registrable-origin
label, requires the origin rather than an arbitrary subdomain for the exact
signal, re-evaluates cached navigation metadata, and does not synthesize brand
roots inside government namespaces. The unchanged polluted corpus then passed
192/192, so the correction did not depend on deleting bad documents.

The v47 cold run passed 63/64. Under concurrency, `última pesquisa para
presidente do brasil` could finish with a corroborated second-round claim before
another request retained the first-round page. The stronger gate rejected the
apparently complete answer. Broad poll queries now open one bounded expansion
for the missing round, recorded as `poll_scenario_expansion`; an isolated cold
four-variant run passed 4/4, and v48 restored 64/64. If expansion still cannot
corroborate the missing round, the response keeps the supported claim and says
that scenario coverage is partial.

The suite then added a held-out tool-intent class based on a live, unsigned
Portuguese-localized Google observation for `python online compiler`. Google
ranked OnlineGDB, Programiz, Online-Python, and OneCompiler in positions 1–4;
Python.org appeared ninth, correctly treating the request as a runnable tool
rather than language-homepage navigation. OpenSonarX's first cold attempt put
OneCompiler first and the other three in its top four for the canonical form,
but passed only 2/4 variants: Portuguese word order returned no result and a
“browser compiler” variant narrowed to a keyword-shaped site. Tool variants now
share the neutral `python online compiler` discovery flight while the original
query still owns ranking. The contract accepts no handpicked domain: top 1 must
expose a Python compiler/editor/IDE surface, Python.org is forbidden at top 1,
and the top three must contain three domains. The intent is now represented as
`tool`, rather than overloading entity navigation, and the answer returns a tool
slate instead of asserting an official destination. The v54 targeted cold run
passed 4/4; its exact resident corpus passed 12/12 from local pages with p50
0.4 ms and p99 8.8 ms. The subsequent v55 full matrix passed 68/68 cold and
204/204 resident. This is one localized SERP
observation, not a Google API experiment or statistical market comparison.

The cold run immediately before v39 scored 63/64: `who created Pix?` could
finish before a concurrent Portuguese paraphrase had retained the primary
evidence. The query already passed in isolation, proving an order-dependent
coverage race rather than an impossible query. Each origin-relation request
now issues the same bounded Portuguese and English canonical rewrites; variants
coalesce into four shared provider flights per entity/relation. The repeated
empty-index run then passed 64/64 without weakening the factual serving gate.

Two later matrices were also retained as failures instead of being averaged
away. After URL canonicalization, v40 returned a correct Pix claim but failed
the contract because it lacked a claim-matching `bcb.gov.br` citation. “Fonte
oficial” was only a provider hint and could disappear under concurrency.
Answer-aware expansion now searches the normalized entity–answer pair and,
when that answer names a likely public authority, one bounded `site:gov.br`
variant. The authority URL must still identify the institution in the claim and
the fetched page must still entail it. A four-variant Pix run passed 4/4 while
reducing wall time from 6.81 s with four expansion queries to 3.45 s with two;
v44 then restored the complete 64/64 matrix.

The resident diagnostic showed the intended split. Covered navigation, Pix,
and current-poll queries generally completed in single- or double-digit
milliseconds. Due diligence and an incomplete Anthropic-founder claim still
opened live discovery and remained `insufficient_evidence`; their seconds-level
latency is the cost of refusing to turn an incomplete local corpus into a
confident answer.

Focused v55 traces make that split concrete. `claude` used one local pass,
returned `intent=navigational`, `answer=destination`, and
`https://claude.com/` at top 1 in 11.5 ms. `python online compiler` returned
`intent=tool` and `answer=tool_results`; its focused cold four-form run passed
4/4, then 12/12 resident requests used only local pages. `gama fund é legítimo`
used local plus Bing, made two search passes, spent 3.50 s in search, and still
returned `insufficient_evidence` instead of converting launch coverage into a
legitimacy judgment. `pesquisa eleitoral presidente brasil hoje` used local
evidence plus the TSE/PesqEle join, returned `intent=current_events` and an
extractive answer in 17.2 ms from the resident corpus.

### Earlier hardened snapshot — retained for comparison

An earlier hardened snapshot was tested from two unrelated empty directories with 64 natural
query forms and 32 simultaneous callers. Both runs executed live discovery and
foreground crawling; neither reused an earlier corpus:

| Run | Quality | Documents | Wall | Throughput | Search p50 | Search p95 | Search p99 | Hydration p95 / p99 |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| cold v28, concurrency 32 | 64/64 | 0→147 | 14.44 s | 4.4 qps | 1.97 s | 6.32 s | 6.37 s | 2.95 s / 4.28 s |
| cold v29, concurrency 32 | 64/64 | 0→156 | 14.13 s | 4.5 qps | 1.48 s | 6.48 s | 6.54 s | 2.73 s / 4.08 s |

The v29 corpus was then reused in one resident process. One unmeasured warm-up
pass was followed by three measured passes (192 requests) at concurrency 32:

```text
quality=192/192; documents=156→160; wall=10.97s; throughput=17.5qps
search p50=10.3ms; p95=1212.2ms; p99=2191.0ms
answer hydration p95=774.6ms; p99=912.2ms
background crawl p99=946.7ms
```

Only four documents were added in that resident run, so the result mostly
measures indexed serving with the current-event and factual revalidation that
still occurs by policy. It is not a pure in-memory cache microbenchmark.

The first cold concurrency-32 attempt was intentionally retained as a failure:
it scored 53/64 (82.8%). Equivalent transaction phrasings exhausted provider
capacity independently, an engine-synthesized `https://claude.ai/login-Claude`
path survived routing, and concurrently crawled Claude support content polluted
the Anthropic founder slate. Canonical action-family provider flights,
conservative same-host roots, and suspicious-action-path rejection raised the
next cold run to 63/64. That remaining failure showed a subject/object inversion:
“content created by Claude” was treated as a possible answer to “who created
Anthropic?”. The extractor was made directional, and two cold matrices passed.

A subsequent result-slate audit found that the answer abstained correctly but
could still display a page whose title named Anthropic while its body described
something else being created. The final serving gate was aligned with the same
directional extractor. The first cold run under that stricter gate reached 63/64
because the noun form `fundadores` did not share the canonical `quem fundou`
provider flight under concurrency. Normalizing `fundador/fundadores` and
`founder/founders` fixed the recall race without weakening the gate. Targeted
Anthropic/Pix variants passed 8/8, followed by the independent v28 and v29
64/64 matrices above.

The current SDK library suite contains 303 tests (295 passed and eight
live/manual tests ignored by default), the index crate contains 174 passing
library/integration tests, and the benchmark crate contains 73 passing tests;
zero failed in the recorded verification. The benchmark itself rejects a mock
embedder and quality-gates every request.

DuckDuckGo returned repeated HTTP 403 responses in every final matrix and its
circuit breaker opened. Credential-free Bing plus retained coverage carried the
final passing suite. Passing in that degraded condition demonstrates useful
resilience, while the nearby v113/v115 failures expose the production blocker:
two configured wrappers are not two healthy, independent discovery providers.

The raw-adapter health probe covers entity navigation, tool discovery, and a
current Brazilian query. It originally counted any non-empty HTTP result set
as healthy; a follow-up Anthropic probe showed why parseable but unrelated URLs
cannot count as availability. Health now requires at least one relevant surface
result, at least 24 attempts per engine, and provider concurrency no lower than
the cold E2E concurrency. A truthful keyless Brave public-HTML adapter passed
9/9 serial attempts and a short concurrency-four burst, but the sustained
three-class, ten-round, concurrency-four probe told the production story: Bing
passed 30/30 and Brave public HTML passed 12/30 before repeated HTTP 429s. The
two-provider gate exited non-zero with 1/2. A no-Bing E2E chaos run then used
Brave public HTML and DuckDuckGo without mocks; it passed 17/24 canonical cases
before both rate-limited. Masking a crawler as Chrome remains rejected as a
remedy. The supported authenticated Brave API is a separate adapter, as is
Serper; neither `BRAVE_API_KEY` nor `SERPER_API_KEY` was present in the recorded
run. The exact sustained probe and no-primary matrix must be repeated after a
contracted independent provider is configured.

### Seen multi-intent Google development regression

Nine signed-out Google `pt-BR`, Brazil-localized top-five result sets were
captured on 22 August for the canonical live queries: Claude navigation,
Claude login, Python online compiler, the `gama found` typo in the user's Gama
Fund context, Gama due diligence, the current Brazilian poll, Pix origin,
Anthropic founders, and current AI news. The fixture and evaluator are checked
into `bench/fixtures/google-multi-intent-br-2026-08-22.json` and
`bench/src/market_compare_bench.rs`.

The comparator applies explicit URL, functional-surface, forbidden-result,
freshness, primary-source, self-declaration, and domain-diversity contracts to
both slates. On the final coherent no-mock corpus, Google scored 74.1% and
OpenSonarX 100.0%: four OpenSonarX wins, five ties, and no losses. Pix placed the
claim-matching Banco Central page first, and Anthropic founder completeness
remained an explicit `insufficient_evidence` answer with relation-legible leads.
Earlier reports ranged from 87.0% to 100.0%, illustrating discovery and corpus
volatility rather than a stable quality gain. These nine cases drove repeated
implementation changes and are now classified only as a seen regression suite;
they are not confirmatory evidence of market superiority. The candidate comparator itself
does not compare latency. A later exploratory audit captured Google browser
wall time and OpenSonarX backend time under explicitly different protocols;
therefore it is reported separately and remains non-comparable. The contracts
are policy-authored, the sample is small and non-blinded, and personalization
was controlled only by the signed-out/locale parameters. Consequently the report sets
`market_superiority_proven=false` and `cash_thesis_tested=false`.

### Historical healthy-provider run

The final healthy cold-index iteration before the founder-completeness hardening produced:

- 15/15 contract gates under the then-current suite;
- documents: 0 -> 100;
- search p50: 859.6 ms;
- search p95: 7,543.1 ms.

A two-pass run in one resident process produced 30/30 under those gates, p50 12.1 ms and p95 2,817.3 ms. Most second-pass covered queries were approximately 0.7-10.7 ms; evidence-expansion queries remained in the seconds range.

The later founder audit found that one of those 15 passes was semantically too weak: two sources naming Dario and Daniela Amodei were treated as a complete founder list even though retained pages said “a group of colleagues” or “other former members.” The gate and implementation were therefore changed. Historical 15/15 must not be read as 15 perfect answers.

### Historical hardening under an external DNS outage

The first run with the hardened gates and 142 retained documents exposed an
11.4-second p95. Profiling found that the largest part was not useful network
work: the weak-coverage path embedded query-relevant spans for up to 50 full
documents before merging, even for intents whose policies did not consume
those spans. It also waited on platform DNS behavior beyond the intended HTTP
budget.

After adding a hard orchestration deadline, bounded in-flight protection, and
intent-specific enrichment, an intermediate run improved latency but produced
only 14/15: `perplexity` still ranked Wikipedia above an official homepage that
was absent from the old index. That was a real availability failure, not a gate
to waive.

The failure exposed two persistence/coverage defects. JavaScript-only official
homepages could disappear across processes when no text was extracted, and a
retained first-party subdomain could not recover its canonical entity root. The
code at that point began persisting verified navigational destinations in a metadata-only
cache and can derive a conservative canonical root from an exact-name
first-party subdomain (`social.perplexity.ai` -> `https://perplexity.ai/`). These
records can route navigation but are never factual evidence.

After that 15-case run passed, the literal screenshot query `gama found` found
another uncovered defect: it was classified as navigational but an event page
on Luma ranked first and the official Gama Fund site ranked fifth. The fix is
generic, bounded one-edit entity/host matching for destination routing. It
requires alphabetic labels of at least five characters with the same initial;
numeric lookalikes, short names, and compound domains are rejected. The signal
never becomes factual evidence or a trust assertion.

An expanded two-pass milestone reused the same 142-document directory, still while
DuckDuckGo DNS resolution failed. It did not add a manual Perplexity seed,
rewrite the old index, hydrate a page, or relax an answer gate:

- 32/32 decisions passed (16/16 in each pass);
- documents: 142 -> 142;
- aggregate search p50: 17.0 ms;
- aggregate search p95: 813.3 ms;
- first-pass range: 17.0-1,765.4 ms;
- second-pass range: 0.6-13.1 ms;
- `perplexity` routed to `https://perplexity.ai/` with source
  `navigation_derived+local`.
- `gama found` routed to `https://gamafund.com/`; Luma moved to second place.

The later concurrency audit found that even these numbers hid avoidable local
work. Concurrent cache misses independently recomputed the same embedding
batches, and the local fast path re-embedded up to 50 title/snippet candidates
even though HNSW had already computed the query/chunk cosine. The fixes were:

- single-flight cache fills with a second cache check after waiting;
- preservation of the absolute HNSW cosine separately from the index's
  query-local fusion normalization;
- external-only snippet embedding during local/web merge;
- no semantic sentence enrichment for intents whose answer policies inspect
  URL/title/snippet or the retained body directly;
- a two-second external orchestration deadline;
- bounded provider concurrency (four healthy calls), rather than either
  unbounded workers or a provider-wide one-request bottleneck.

After those changes, the canonical two-pass suite produced 32/32 with p50
2.5 ms, p95 15.9 ms, p99 18.7 ms, and 139.3 ms total wall time. DuckDuckGo DNS
was still failing, but failure resolution was fast in this particular final
run; these numbers must not be substituted for cold live-discovery/crawl
latency.

At that historical stage the environment had no configured Serper, Brave,
Bing, Google, or SearXNG provider. DuckDuckGo HTML was therefore a single point
of failure for a genuinely uncovered query; retained coverage and conservative
navigation derivation made that particular suite resilient, but did not create
fresh web coverage during the outage. The final snapshot above adds the
credential-free Bing adapter, while DuckDuckGo remains operationally degraded.

### Historical retained-only paraphrase and concurrency audit

The benchmark uses one resident production client shared by synchronized
workers. Every request executes intent routing, real Arctic ONNX query
embedding, HNSW/BM25 retrieval, intent reranking, decision metadata, and answer
construction. It rejects a mock embedder and applies the same quality contract
to every paraphrase.

| Concurrency | Requests | Quality | Wall | Throughput | p50 | p95 | p99 |
|---:|---:|---:|---:|---:|---:|---:|---:|
| 1 | 64 | 64/64 | 391.8 ms | 163.3 qps | 3.6 ms | 18.0 ms | 30.2 ms |
| 2 | 64 | 64/64 | 267.4 ms | 239.3 qps | 5.0 ms | 19.4 ms | 29.6 ms |
| 8 | 64 | 64/64 | 204.9 ms | 312.3 qps | 14.8 ms | 36.3 ms | 43.3 ms |
| 16 | 64 | 64/64 | 202.3 ms | 316.4 qps | 27.6 ms | 56.6 ms | 75.4 ms |

Concurrency 2 minimized tail latency on this machine; concurrency 8 delivered
almost all available throughput with materially lower p99 than 16. A separate
same-string hot-cache run exceeded 1,000 qps, but that is a cache-contention
measurement, not a credible estimate for diverse production traffic.

The paraphrase suite initially passed only 59/64. The five failures were kept
as regressions and fixed: a third uncorroborated poll percentage leaked into a
two-value claim; singular `última` missed current-event classification;
`quem criou a Anthropic?` could invert creator direction and evade founder-list
completeness; and two Pix creator paraphrases failed relation/type
normalization. The final 64/64 therefore reflects stricter behavior, not relaxed
gates.

### Focused repeatability

| Case | Independent cold runs | Observed search latency | Result |
|---|---:|---:|---|
| Brazil election poll, cold discovery/crawl | 3/3 | 4.1-9.6 s | repeated two-candidate percentage pairs with independent editorial-family citations |
| Pix creator, cold discovery/crawl | 3/3 | 3.1-7.8 s | Banco Central attribution corroborated across retained families |
| Brazil election poll, retained outage path before load hardening | 2 | 33.8 ms / 7.8 ms | same extractive quality gate passed |
| Pix creator, retained outage path before load hardening | 2 | 1.77 s / 7.6 ms | same extractive quality gate passed |
| Anthropic founders, retained outage path before load hardening | 2 | 45.7 ms / 13.1 ms | explicit abstention because complete compatible lists were absent |
| Perplexity navigation, retained outage path before load hardening | 2 | 813.3 ms / 0.7 ms | canonical official root derived from retained first-party subdomain |
| `gama found` typo navigation, retained outage path before load hardening | 2 | 20.2 ms / 0.7 ms | official Gama Fund root beat the semantically related Luma event |
| Historical retained-only 64-query suite, concurrency 8 | 1 | p95 36.3 ms / p99 43.3 ms | 64/64 with one shared real client, no fresh provider coverage |
| Final hardened empty-index suite, concurrency 32 | 2 | p95 6.32 s / 6.48 s | 64/64 twice with live discovery, crawl, index, evidence, and answer |
| Final resident/revalidation suite, concurrency 32 | 192 requests | p95 1.21 s / p99 2.19 s | 192/192; corpus still grew 156→160 |

Those retained-only numbers remain useful as a lower-bound microbenchmark, but
they are not the current production-path result. The hardened snapshot performs
live revalidation where policy requires it; its final cold p95 was 6.32–6.48
seconds and its resident p95 was 1.21 seconds on this machine.

## 4. Representative current outputs

### `claude`

```text
intent=navigational
answer_status=destination
Destino mais provável: Claude — https://claude.com/
uncertainty=not_applicable
```

The system treats this as a destination request, not as a request for news or a factual biography.

### `gama fund inscrição`

```text
intent=transactional
answer_status=destination
top destination=https://gamafund.com/
```

The official application surface leads instead of a news article that merely contains the word “inscrição.” Equivalent `inscrição/aplicar/inscrever` forms now share the same neutral provider workload. If a provider invents a compound action slug, the router rejects it and preserves a same-host first-party root rather than presenting an unverified deep action URL.

### `gama found`

```text
intent=navigational
answer_status=destination
Destino mais provável: Gama Fund — https://gamafund.com/
rank 1=https://gamafund.com/
rank 2=https://luma.com/y9wg2ujd
```

This exact typo from the screenshot originally failed. A bounded destination-only
entity/host edit match now recovers the canonical site without treating spelling
similarity as evidence or allowing arbitrary fuzzy domains.

### `gama fund é legítimo`

```text
intent=due_diligence
answer_status=insufficient_evidence
Os resultados recuperados não bastam para concluir legitimidade.
Declarações da própria entidade, páginas de inscrição e cobertura sobre sua
existência não substituem verificação jurídica, operacional e independente.
```

This is the desired trust behavior today: the system refuses to turn visibility, partner names, SEO prominence, or a domain stake into a legitimacy certificate. Production due diligence still needs registry, legal-entity, domain-control, partner-confirmation, sanction, complaint, and operational checks.

### `pesquisa eleitoral presidente brasil hoje`

The hardened runs produced separately labeled, corroborated scenarios rather
than blending their percentages:

```text
segundo turno: Sai Datafolha: Lula tem 47% e Flávio 43% no segundo turno.
primeiro turno: Pesquisa Datafolha: Lula tem 39% e Flávio Bolsonaro 33% no 1º turno.
```

Each emitted scenario required the same percentage pair and matching candidate
identities in at least two retained source families. Pages announcing a future
poll, records older than the explicit seven-day serving window, undated
external snippets, rejection/indecision percentages, sister domains, and
mismatched candidate pairs do not count as corroboration.

The audit found a subtler ambiguity on a page carrying more than one
first-round slate. Its explicitly alternate scenario could yield `39–32`, while
the unqualified primary scenario was `39–33`. The answer now detects that the
same protagonists occur in several same-page slates and performs one exact,
traceable primary-scenario discovery pass. It replaced `39–32` with `39–33`
only after O Globo and R7 supplied two qualifying source families and the normal
official-lineage gate passed. Percentage windows are also forbidden from
crossing Markdown section boundaries, so the last number of one slate cannot be
paired with the first number of another.

The latest path no longer treats those editorial families as independent
measurements. It extracts the exact registration only from pages cited for the
provisional claim and joins `BR-04496/2026` through the live public PesqEle JSF
flow. An empty JSF form/list response gets exactly one retry in a fresh session,
and institute selection uses maximal shared context. The institute-specific
editorial fallback is issued even if official listing discovery is transiently
empty, so the public registry improves lineage without becoming the sole web
recall path. The retained record reports registration on 15 August, disclosure on 21
August, 2,058 interviews, fieldwork from 18–20 August, statistician Renata Nunes
Cesar (`7249A`), declared cost of R$307,641.60, Folha and Globo as contracting
entities, a two-percentage-point margin, and 95% confidence.

The claim lineage exposes three different layers:

1. one underlying Datafolha measurement;
2. one official TSE registration record describing that measurement; and
3. multiple editorial families transmitting the reported percentages.

One retained article is the list-relative measurement representative; other
articles carrying the same registration are `editorial_report_of` derivatives
with zero measurement independence. This is separate from residual
`dependency_uncertainty`, which remains nonzero when ancestry is incomplete.
PesqEle verifies the registration and methodology metadata. It does **not**
verify the publishers' percentages, realized sampling accuracy, or absence of
methodological bias, and both the machine-readable lineage and answer warning
state that boundary.

### `quem criou o Pix?`

The system returns an extractive Banco Central attribution only after compatible statements from at least two retained source families, and the final contract requires a claim-matching `bcb.gov.br` citation. A generic `.gov` page no longer satisfies that requirement unless its authority matches the institution named in the claim.

### `quem fundou a Anthropic?`

```text
intent=factual
answer_status=insufficient_evidence
As fontes retidas confirmam parte dos fundadores, mas também indicam outros
cofundadores sem fornecer duas listas completas e compatíveis. Para não
apresentar uma lista parcial como completa, a resposta foi suspensa.
```

This deliberately scores as a pass. Current public sources disagree or vary in how they enumerate the founding group; naming only Dario and Daniela as the complete answer would be fluent but misleading. The implementation now requires two near-identical, sufficiently rich enumerations when retained evidence signals omitted cofounders. The same contract applies to `quem criou a Anthropic?`; a sentence about something created *by Anthropic or Claude* cannot be inverted into a sentence about who created Anthropic.

### `últimas notícias de inteligência artificial hoje`

The system abstains from a single prose answer. A thematic news query requires event clustering, per-claim corroboration, source-derivation collapse, and correction handling. Reusing the poll extractor or summarizing unrelated headlines would be unsafe.

## 5. Market comparison

### Reproducible Google candidate-set replay

A dated, signed-out `pt-BR` Google snapshot contains nine election-search
queries and 45 judged candidates. Replaying the unchanged Google candidate set
through Stage 2 produced:

| Metric | Google order | Risk-aware order |
|---|---:|---:|
| Policy utility nDCG@5 | 0.8955 | 0.9985 |
| Mean relevance@3 | 0.7667 | 0.9000 |
| Metadata completeness@3 | 0.7222 | 0.8144 |
| Traceable result rate@3 | 0.8519 | 0.9630 |
| Primary/official hit@3 | 0.5556 | 0.6667 |
| Hard-risk rate@3 | 0.1852 | 0.0370 |
| Independent evidence clusters@3 | 2.44 | 2.22 |

The declared policy reduced hard temporal/geographic/not-a-poll risk by 80%,
but also reduced list-level independent clusters. This supports the narrow
reranking hypothesis and exposes a real diversity defect. It is not an
independent-retrieval comparison: both orders use Google's captured candidates,
and the judgments/policy were authored for this experiment.

### Live observational comparison

The comparison was observational, not a controlled API parity benchmark. “Public
search” below means the web-search surface available to this evaluation; it is
not a captured, reproducible, personalized Google SERP.

- Public search found the official [Gama Fund](https://gamafund.com/) page, but also mixed the program with the unrelated [Gamma Fund](https://www.gammafund.io/) homonym. OpenSonarX's entity/host routing kept the requested destination first and excluded the homonym from the diligence namespace.
- For Claude, public search exposed official [Claude help](https://support.claude.com/en/articles/8114491-get-started-with-claude) and the [Claude login](https://claude.ai/login?from=logout); OpenSonarX similarly separates platform navigation from factual/news intent.
- For Pix, the [Banco Central FAQ](https://bcb.gov.br/estabilidadefinanceira/pix/perguntasfrequentes) and [Banco do Brasil](https://www.bb.com.br/site/pra-voce/pix/) support the institutional attribution returned by OpenSonarX.
- For the current Datafolha poll, exact public queries recovered a [GZH report](https://gauchazh.clicrbs.com.br/politica/eleicoes/noticia/2026/08/pesquisa-datafolha-lula-tem-47-das-intencoes-de-voto-no-segundo-turno-e-flavio-bolsonaro-43-cmt3gw5jq00jy01jn4ne2s3bb.html) with registration, sample, and margin metadata. Broader queries also exposed announcement pages and unrelated/stale results, illustrating why time/scope gates matter.
- For Anthropic's founders, the official [company page](https://www.anthropic.com/company?type=company) did not expose a complete founder list in the search surface, while aggregators varied in enumeration. OpenSonarX abstained instead of presenting a visible pair as the complete set.

No claim is made that this run beat Google, ChatGPT, Perplexity, or Claude overall. We did not have identical localized candidate pools, personalization controls, answer logs for every vendor, human-blinded judgments, or enough queries for statistical power. What is supported is narrower: the current architecture can produce correct destination routing, cautious structured extraction, auditable citations, and abstention where mainstream answer systems are often tempted to complete missing information.

## 6. Defects found and corrected during the run

1. Poll extraction was accidentally available to generic current-news queries.
2. External snippets could be treated as current evidence without a successful crawl.
3. Candidate identities were previously unioned across a sentence, allowing mismatched pairs to corroborate.
4. Rejection, undecided, blank/null, and glued duplicate percentages could enter a vote pair.
5. Same-day precise timestamps lost to coarse midnight dates encoded in URLs.
6. Distillation could hide the main poll scenario from extraction.
7. Globo sister domains could be counted as independent publishers.
8. Factual creation wording missed Portuguese variants used by Pix coverage.
9. Shallow hydration often stopped before a second answer-bearing source family.
10. Exact official destinations such as Python could lose to similarly named tutorial domains.
11. Retained deep first-party pages did not satisfy navigational coverage, causing unnecessary external calls.
12. Anthropic's visibly partial founder list passed as complete.
13. JavaScript-only navigational destinations were not persisted independently from crawled evidence.
14. Query-relevant span enrichment ran across dozens of candidates for intents that did not consume it, producing 10-14 second local tails.
15. HTTP phase timeouts alone did not bound the serving request when the platform resolver degraded; orchestration now has its own deadline and per-provider in-flight guard.
16. A retained first-party subdomain could not recover the canonical root for an exact-name navigational query; conservative root derivation now covers ordinary and common country-code second-level domains without turning the derived record into evidence.
17. The exact typo `gama found` let a semantically related event outrank `gamafund.com`; bounded one-edit entity/host matching now affects destination routing only and has negative guards for numeric, short, and compound labels.
18. A corroborated two-value poll signature could emit a third, uncorroborated percentage from the source sentence; emitted claim text is now bounded to the supported pair.
19. Singular `última pesquisa para presidente` was classified as exploratory; current/poll normalization now covers that natural formulation.
20. `quem criou a Anthropic?` could select the inverse relation “Anthropic, criadora do Claude” and bypass founder completeness; relation direction and context-bound partial-list detection now prevent both errors.
21. `quem inventou o Pix?` and `qual instituição criou o Pix?` failed despite compatible retained evidence; creator/inventor equivalence and answer-type-word removal now preserve the entity relation.
22. Concurrent embedding cache misses produced duplicate ONNX batches; fill single-flight and double-checking now coalesce them.
23. Local candidates were re-embedded solely to undo query-local score normalization; the index now preserves raw HNSW cosine and only external snippets require blend-time embedding.
24. A provider-wide one-request guard protected outages but discarded healthy concurrent discovery; the breaker now permits a bounded healthy pool and retains a single half-open probe.
25. The live harness reported no p99, throughput, concurrency, or warm-up distinction; all are now explicit and quality-gated.
26. Concurrent requests could crawl the same URL independently and a follower could miss content ingested by the leader; URL-level crawl single-flight and an unconditional post-hydration refresh now close both races.
27. Any government domain could previously suppress primary-source expansion; public-authority URLs now have to match the institution named in the extracted claim.
28. Current-event discovery either discarded useful undated leads too early or served undated snippets too late; discovery may crawl them, but final answered serving requires an effective date inside the explicit window.
29. Spell correction leaked into intent/entity identity and navigation cache keys, so corpus vocabulary could redirect a valid brand query; correction is now retrieval-only and original query text owns routing and scope.
30. Concurrent transaction paraphrases generated separate provider workloads; action families now use neutral canonical discovery queries and shared cache flights.
31. A synthesized `claude.ai/login-Claude` route passed action-word matching; compound action paths are rejected and a verified same-host root remains available.
32. A factual page that mentioned the entity in its title and an unrelated creation relation in its body could invert subject and object; answer extraction now requires a sentence-level or bounded grammatical entity–relation link.
33. Answer hydration and background crawl were reported as one number, hiding where the synchronous tail was paid; the benchmark now reports both separately plus the compatibility total.
34. `quem são os fundadores da Anthropic?` did not share discovery with `quem fundou a Anthropic?`; founder noun forms now normalize to the same neutral provider flight, eliminating a cold concurrency recall race.
35. Several editorial families reporting the same poll looked like several independent measurements; exact TSE registration lineage now collapses them into one measurement representative plus editorial derivatives.
36. TSE/PesqEle had no production join; the SDK now executes the real JSF GET/AJAX/detail flow with bounded timeouts, request single-flight, structured parsing, and crawl-time prefetch.
37. Broad retrieval could attach registrations from older polls that were not used in the answer; registry IDs now come only from pages cited for the provisional claim.
38. Every new process could repay a multi-second PesqEle lookup; validated positive records now use a 24-hour atomic persistent cache. The measured cross-process lookup changed from approximately 2,178 ms cold to 3 ms cached.
39. Exact same-URL/content recrawls were deduplicated without advancing `crawled_at`, creating an endless revalidation loop; exact recrawl now refreshes metadata without adding a document, while cross-source duplicates cannot refresh another source.
40. A stale retained current source could be displaced entirely by external leads instead of being revalidated; foreground hydration now rechecks up to two stale retained source families before filling remaining slots from discovery.
41. A fresh, extractive local poll answer still hydrated external pages redundantly; an answer-qualified freshness gate now skips foreground hydration and exposes `fresh_local_evidence_hit` in the trace.
42. Resolving a derivation relationship could accidentally erase its penalty by setting dependency uncertainty low; known `independence` and unresolved `dependency_uncertainty` are now separate inputs and combine into one dependence risk.
43. Canonical factual rewrites reduced duplicate provider work but made `who created Pix?` depend on a concurrent Portuguese query; every origin relation now uses bounded Portuguese and English rewrites that share provider flights across variants.
44. The BM25-disabled timing test measured lock scheduling as BM25 search time and failed nondeterministically; `bm25_ms` is now exactly zero when lexical search is disabled, while later lock use remains measured in its actual stage.
45. Provider URLs could retain `utm_*`, `msockid`, and equivalent tracking identifiers, so a cold `nubank` query displayed a Bing-specific URL and equivalent provider results could evade deduplication; HTTP(S) URLs are now canonicalized and known tracking parameters removed before dedup/crawl. A cold four-variant run returned `https://nubank.com.br/` in all four cases.
46. A correct Pix answer could pass corroboration but miss the required BCB citation under load; primary expansion now uses the extracted entity–answer pair plus one authority-scoped query, then requires claim-matching authority and fetched entailment. Reducing four redundant confirmation queries to two preserved 4/4 quality and cut the targeted wall time from 6.81 s to 3.45 s.
47. An editorial report that omitted the TSE ID remained apparently independent even when it repeated the registered institute, candidates, and exact percentage assignments; it can now become `inferred_editorial_report_of` only when exactly one verified unit matches the institute in title/snippet and a candidate-aware percentage fingerprint. The inference is exposed, receives `independence = 0`, and never validates the percentages.
48. Institute prefixes made `primeiro turno: ...` and `Datafolha — primeiro turno: ...` look like different scenario keys, allowing two unlabeled first-round variants to consume both answer slots; scenario parsing now owns the emission key, preserving explicitly named slates while collapsing presentation-only prefixes. The live answer returned one first-round and one second-round claim.
49. The poll benchmark accepted any two distinct scenario labels and did not verify result-level lineage cardinality; it now requires first- and second-round coverage, exactly one representative for each answered registered unit, and zero independence for its derivatives.
50. Navigational host identity used the first hostname label, allowing `python.codeutility.io` to impersonate `python.org` after entering the resident index. Identity now uses the registrable-domain label and rejects entity-named subdomains.
51. The intent scorer, navigation cache, and root derivation could still promote same-domain social, login, or government-service subdomains. Exact destination signals now require the canonical registrable origin, cached metadata is re-evaluated under current policy, and government namespaces are resolved through the public-authority path instead of a synthesized brand root.
52. A broad poll query could emit only one round when concurrent cold crawls completed in a different order. Missing-round coverage now triggers one bounded, traceable scenario expansion; if the second scenario remains uncorroborated, the answer explicitly reports partial coverage.
53. Short online-tool queries were treated like brand navigation: Portuguese word order missed English candidates, equivalent forms created separate provider work, and a keyword-shaped hostname could receive an exact-entity boost. Tool variants now share multilingual scope and one canonical discovery flight, disable brand-host guessing/authority, reject the language homepage as a substitute, and quality-gate functional surface plus result diversity.
54. Even after ranking was corrected, online-tool queries were still labeled `navigational` and summarized as a single probable destination. `tool` is now an explicit non-claim-bearing intent: it keeps functional surface and multi-domain resident-coverage gates, returns a tool slate, and never implies that the top result is an official entity homepage.
55. Provider redundancy was inferred from configured adapter count rather than measured health. `engine-probe` now checks multiple live query classes and rounds per raw adapter, reports semantic relevance and latency, and exits non-zero unless a configured minimum passes every attempt. Early 3×3 probes showed DuckDuckGo at 7/9 and later only Bing at 9/9. The current gate additionally requires at least 24 attempts per engine at production concurrency; Bing passed 30/30 and Brave public HTML 12/30. Every version fails the required two-provider gate.
56. The no-mock E2E harness ignored production `SERPER_API_KEY` and `BRAVE_API_KEY` configuration, so adding credentials would not actually change the measured path. It now consumes the same environment variables as the production CLI; the recorded run remains explicitly keyless because both were unset.
57. A valid HTTP URL was treated as provider health even when every parsed result was unrelated to the query. The raw probe now reports `relevant_result_count`, and serving discards an entirely off-topic provider batch before it can be blended or crawled. The first stronger six-adapter run had zero fully healthy providers rather than one; the latest has only Bing healthy, still below the required two.
58. `claude login` accepted a first-party root with embedded authentication text but did not lead to the direct action route exposed by Google. Transaction routing now parses bounded first-party root links, accepts only entity-matching conventional action destinations, probes the target, persists it as non-evidential metadata, and coalesces equivalent concurrent queries. Four cold variants reached `https://claude.ai/login`.
59. A Cloudflare challenge returned HTTP 200 at the real login target, so status plus a guessed path could neither prove nor safely reject the route. The verifier distinguishes an action URL explicitly declared by the first-party root from a guessed route; guessed 2xx paths still require an action-bearing title, while declared/protected routes retain their provenance marker.
60. Concurrent declared login variants could race among `?plan=free`, `?plan=pro`, `?plan=max`, and the canonical route. Action links are now ordered by canonicality and deduplicated by host+path before probing, so a plain login query cannot accidentally inherit a plan-specific URL.
61. Equivalent due-diligence risk phrasings still issued separate literal searches, and a v66 cold burst left three of four Gama queries empty while one recovered independent coverage. They now share a neutral entity request and a bounded adverse-evidence request; a focused cold run passed 4/4 and v67 passed 68/68.
62. Provider health, E2E correctness, latency, and market comparison could each be inspected but there was no fail-closed aggregate verdict. Probe, live, and market artifacts now carry timestamps and persist complete JSON; `production-readiness` verifies artifact lineage/freshness, provider redundancy and capacity, cold/resident profiles, acquisition, intent/answer coverage, decision safety, SLOs, and limited market contracts separately from research claims. Historical artifacts exposed multiple search-core failures; the final coherent artifact passes 16/17 search-core gates and fails only sustained provider redundancy, while four research claims remain unvalidated.
63. Factual corroboration counted citation families but exposed no claim-level lineage, so consumers could mistake five agreeing pages for five independent observations. Compatible factual wording now receives one stable `factual_relation_claim` signature across all four Pix paraphrases, plus reporting-family count, `independence_status=unknown_editorial_derivation`, no known independent-observation count, and explicit unresolved syndication/quotation/upstream-source paths. A release run passed 4/4 with the same claim signature and the primary BCB citation.
64. A broad poll answer could select an explicitly alternate same-page slate (`39–32`) as the unqualified topline while the primary slate was `39–33`. The answer now audits shared protagonists across multiple slates, performs one bounded primary-scenario discovery pass, and still requires ordinary two-family corroboration plus lineage before replacement.
65. Poll percentage extraction could cross a Markdown heading and pair the last percentage in one section with the first in the next. Scenario windows now stop at section boundaries.
66. Answer-level single-flight made followers wait but then allowed them to repeat the full acquisition recursively. Successful policy-qualified acquisitions now use a 30-second process-local cache keyed by canonical work plus caller filters/top-k; failures and empty acquisition are never cached.
67. Fresh retained evidence still paid redundant discovery/hydration. A five-minute snapshot path now requires recent crawls and intent-specific source-family coverage; polls need two families and generic current news needs three. Evidence-bounded founder abstention may reuse two recently crawled families without becoming a positive factual answer.
68. An incomplete factual enumeration could spend more than four seconds on initial hydration and then open a second crawl only to abstain. Completion discovery remains visible, but the second crawl is omitted after the budget is exhausted; poll and primary-source repair are exempt from this optimization.
69. PesqEle could transiently return an empty JSF form/list and an arbitrary noisy headline term could select the wrong company. The client retries exactly once with a fresh session, uses maximal context overlap, and keeps institute editorial discovery independent from official-list success.
70. Factual abstention could still display a predicate collision: a Skilljar profile named an Anthropic co-founder but described creating Skilljar. Creator-first matching now requires the requested entity to be the direct object, with only a bounded `of` connector, and regression tests also reject Claude/content and `Founders Fund` collisions.
71. A fetched page body could make a vague result auditable while its title/snippet remained misleading as the top visible search result. Factual discovery and public-display relation predicates are now distinct; relation-legible public surfaces stably precede body-only leads without promoting either to evidence.
72. Restored factual fallback leads retained stale decision ranks from earlier slates. The combined abstention slate now clears and reapplies Stage 2, preserves the original `retrieval_rank`, and emits contiguous final `decision_rank` values.
73. The encyclopedia fallback was either absent or at risk of masquerading as a second provider. A bounded MediaWiki Action API reference query now supplies relation leads only for sufficiently specific factual entities; it remains one source family, external snippets remain non-evidence, and it never contributes to the general provider-redundancy gate.
74. Pix opened unnecessary second-pass crawl families, while a poll query blocked on broad TSE enumeration even after retained articles had identified the institute. Factual completion now targets only missing answer-bearing families and rejects ambiguous short-entity encyclopedia expansion; poll discovery uses the institute-specific path first and still requires the exact registration at final join. Trace fields separate provider, registration, and scenario discovery latency.
75. Several nominally keyless adapters were no longer viable services: Stract's search endpoint is gone, Yep's supported API is authenticated, Mojeek's API is credentialed, and DuckDuckGo rejected truthful automated identification. The readiness gate remains red instead of counting dead adapters or browser impersonation as redundancy.
76. Recommendation routing ran after generic transaction detection, so comparison phrases could be treated as direct purchase intent. Recommendation detection now runs first, direct buying and comparison are separated, and the final slate requires a product-compatible candidate instead of accepting pages that match only the intended use.
77. Short definitions, implicit local phrasing, current-office variants, official-registry wording, and current-price aliases exposed intent/equivalence gaps. Routing and canonical acquisition now preserve these distinct contracts while coalescing genuine Portuguese/English paraphrases.
78. Office-holder extraction could panic under concurrent name matching and accept a competing same-family result too close to the selected answer. Bounds-safe extraction, same-family margins, current-office/re-election semantics, and a poll-exclusion guard now fail closed.
79. Pix primary-source repair selected the first lexical relation token (`linha`) rather than the strongest authority-shaped answer. It now searches the candidate set for a public-authority relation before generating claim-specific and `site:gov.br` queries; the fresh trace queried `"pix" "banco central"` and retained the Banco Central page.
80. Poll answer construction could discard a registration ID that Stage 2 had validly inferred by institute plus candidate/percentage fingerprint. Answer construction now preserves only verified inferred lineage, keeps the derivative at zero independence, and never converts inference into validation of percentages.
81. The MediaWiki fallback always queried English Wikipedia. Strong Portuguese queries now use `pt.wikipedia.org`; four empty-index quantum-definition variants passed after the fix. Local forms also gained a shared location-neutral canonical flight and passed 4/4 fresh while still warning that no distance order is possible without location.
82. A fresh notebook-recommendation run failed 0/4 after the routing fix because DuckDuckGo returned 403 and Bing returned retailer categories or generic pages without the required programming-laptop surface. A direct 3×3 Bing probe also passed 0/9 semantically for this topic. The system's empty abstention is correct; the recall failure remains unresolved.
83. A keyless Brave public-HTML adapter passed short serial and burst probes but collapsed under a sustained 30-attempt, concurrency-four probe, passing only 12/30 before HTTP 429. Provider readiness now requires at least 24 attempts per engine at no less than the cold E2E concurrency, preventing a short burst from being called production redundancy.
84. The default keyless path launched Brave and DuckDuckGo together after a shallow Bing batch and waited for both even when Brave had already returned a deep topical slate. Discovery is now a staged waterfall: Bing primary, Brave secondary, and DuckDuckGo after a bounded hedge only when the secondary slate is insufficient.
85. A no-Bing chaos run previously required source changes to choose engines. `live-search --engines` now runs the production client against an explicit real-adapter set; Brave public HTML plus DuckDuckGo passed only 17/24 canonical cases under rate limiting.
86. A shallow retained encyclopedia result suppressed the bounded reference fallback and caused the quantum definition to return depth one. Exploratory definition queries now merge reference leads until a minimum depth is met, while treating the reference as one family rather than independent corroboration.
87. A correct Pix answer cited the Banco Central but could leave it below the displayed top five. Exact cited authority evidence is now promoted only when the claim itself matches the authority, placing the BCB page first without a generic government-domain boost.
88. A cold broad poll could not become extractive because scenario completion required an extractive answer. Insufficient Brazilian presidential queries now acquire both first- and second-round candidates once; the crawl slate balances two public reporting families per round plus one failure margin, while fetched content and PesqEle still decide the answer.
89. The earlier cold matrix passed 96/96 and both earlier resident matrices passed 288/288. One acquisition-heavy resident run missed p95 by 130 ms, which exposed revalidation-tail variability rather than a quality failure.
90. Initial poll hydration could spend five slots on one scenario and later primary-slate audit could schedule a second complete five-page batch. The first batch now uses the round-balanced scheduler; a primary-slate audit uses three diverse failure-margin URLs while genuine missing-round completion keeps five reserved slots.
91. Citation-oriented poll display reordering initially improved top-three domain diversity but accidentally removed the one Stage-2 measurement representative. The final ordering first preserves every answered unit's representative, then interleaves claim citations and partitions same-family derivatives. The live target passed with exactly one `BR-04496/2026` representative and three top-three domains.
92. Extractive factual answers could collapse the public search slate to only two cited pages. Strict relation-legible discovery leads now fill unused display slots after answer construction, so their snippets cannot enter claims. This placed Banco Central first for Pix while preserving five useful results. Founder discovery can combine names extracted from separate retained biographies, but incomplete enumeration still abstains.
93. Generic current-news queries paid foreground hydration even though the product deliberately abstains from thematic synthesis; due diligence could execute an initial web matrix and then repeat it as a news retry. Generic news now returns the uncertainty-labelled result slate and moves crawling to the background. Due diligence uses one recent neutral/adverse/contextual matrix. Isolated real tests measured 1.84 s and 1.14 s search latency respectively, both at 100% quality.
94. Crawl batches ingested each page under a separate index write lock, and equivalent queries repeatedly recrawled the same retained URL. Outputs from one batch now enter under one mutation, and opportunistic background crawling suppresses exact retained URLs for five minutes while foreground claim freshness stays unchanged. On the first coherent resident cycle this reduced growth from an observed 64 pages to 31 and improved throughput from 8.71 to 12.35 qps.
95. The final current-code chain passed cold 96/96 and first resident/revalidation 288/288. The resident path met every declared serving gate at p95 1.24 s, p99 2.40 s, 12.35 qps, and background-crawl p99 1.70 s. The readiness verdict still fails only independent provider redundancy in the search core and leaves four research claims unvalidated.
96. A no-warmup resident soak increased concurrency from 4 to 16 and passed 960/960. It served 26.95 qps with p95 0.57 s, p99 2.65 s, background-crawl p99 1.01 s, and corpus growth from 177 to 210 documents. This demonstrates useful single-process concurrency stability, not distributed, cost, abuse-resistance, or Google-scale capacity.
97. A three-round signed-out Google browser audit measured wall time to the first visible organic heading across the nine market intents. It reached 27/27 ready pages with p50/p95/p99 of 215/407/414 ms. The same 27 current-code OpenSonarX samples passed quality with backend `search_ms` p50/p95/p99 of 1.96/31.02/2400.04 ms; the p99 was the first live due-diligence acquisition, followed by 0.04 and 0.06 ms resident hits. Eight query strings were identical. The tool query used a Portuguese localized equivalent, and a later three-attempt exact retry hit Google's unusual-traffic interstitial every time. A versioned evaluator recomputes the metrics and refuses a comparability claim because browser-to-DOM and backend-only latency are different surfaces.

Regression tests were added for these failure modes.

### Final systematic snapshot — 22 August 2026

| Artifact | Result | Interpretation |
|---|---:|---|
| Final cold v311, 96 requests, concurrency 4 | **96/96**; 0→146 docs; 1.63 qps; p50 0.93 s; p95 5.09 s; p99 5.94 s; hydration p99 4.30 s; background p99 2.74 s | Real embedder, live discovery, crawl, evidence, Stage 2, PesqEle/market joins, and answers; all cold contracts pass despite public-provider rate limits. |
| Poll representative/diversity repair v303b | **1/1**; search 1.35 s; exactly one measurement representative; three top-three domains | Scenario citation interleaving no longer removes the underlying-unit representative; normal registry and dependence gates remain mandatory. |
| Final coherent resident/revalidation v312 | **288/288**; 146→177 docs; 12.35 qps; p50 3.2 ms; p95 1.24 s; p99 2.40 s; background p99 1.70 s | Same current-code cold corpus; passes every declared resident quality, latency, crawl, and throughput gate on the first resident cycle. |
| Resident soak v315, concurrency 16 | **960/960**; 177→210 docs; 26.95 qps; p50 2.9 ms; p95 0.57 s; p99 2.65 s; background p99 1.01 s | No warmup or mocks; useful single-process concurrency evidence, not a distributed-scale claim. |
| Sustained provider v263, 3 classes × 10 rounds, concurrency 4 | Bing 30/30; Brave public HTML 12/30; 1/2 healthy | Short Brave probes were green, but sustained 429s correctly fail production redundancy. |
| No-Bing failover chaos v261, 24 canonical cases | 17/24 (70.8%) | Brave public HTML plus DuckDuckGo both rate-limited; safe abstention worked, recall did not. |
| Seen market regression v313, 9 cases | Google 74.1%; OpenSonarX 100.0%; 4 wins / 5 ties / 0 losses | Same retained corpus; policy pass 100%; cases drove implementation and cannot support confirmation. |
| Independent MS MARCO holdout v317, 48 cases | OpenSonarX 40/48 non-empty; Google 48/48 | Objective coverage only; relevance/trust unjudged; general superiority false. |
| Independent current/high-impact holdout v319, 40 cases | OpenSonarX 21/40 non-empty | Google follow-up blocked; missing baseline data is not a Google loss. |
| Disjoint post-fix holdout v321, 48 cases | OpenSonarX 13/48 non-empty | Brave 429 and DuckDuckGo 403/timeout expose E2E availability failure; not a clean ranking estimate. |
| Exploratory latency v316, 9 intents × 3 rounds | Google browser→organic p50/p95/p99 215/407/414 ms; OpenSonarX backend p50/p95/p99 1.96/31.02/2400.04 ms | 8/9 exact strings, 27/27 Google organic-ready, 27/27 OpenSonarX quality-pass; `market_latency_compared=false` because surfaces and execution protocols differ. |
| Readiness v314 | search core **FAIL**; research claims **FAIL** | Sixteen of seventeen search-core gates pass; only sustained provider redundancy fails. Four research claims remain unvalidated. |

The exact search-core readiness failure is sustained provider redundancy
(`1/2`). Cold quality, resident quality, acquisition, safety, declared latency,
throughput, market integrity, and limited candidate quality now pass on one
coherent artifact chain. An exploratory browser-versus-backend latency audit is
now versioned, but comparable Google latency, broad superiority,
real-cash predictive value, and live calibrated probabilities remain
unvalidated. Generalized lineage passes for four underlying-unit types:
`poll_measurement`, `factual_relation_claim`, `current_office_holder_claim`,
and `market_quote_snapshot`.

The same-day visual Google audit covered additional surfaces excluded from the
fixture comparator. Google gave excellent direct navigation, maps/local and a
structured Bitcoin quote. It also synthesized quantum and health answers, but
its visible Pix answer cited weak editorial/social sources before the official
Banco Central result, its poll slate did not collapse multiple reports to one
registered measurement, and product results did not expose affiliate/editorial
dependence. OpenSonarX was stronger on explicit lineage, primary-authority
repair, warnings, and abstention, but weaker on live price feeds, geolocation,
shopping coverage, provider redundancy, and synthesis. This is an observed
product-behavior comparison, not a blinded benchmark.

## 7. Remaining production blockers

1. **Provider redundancy:** configure at least two healthy independent production discovery providers with bounded per-provider timeouts, health scoring, and hedged requests. The sustained concurrency-four gate found Bing at 30/30 and Brave public HTML at 12/30 before 429s; the no-Bing chaos matrix passed only 17/24 after Brave and DuckDuckGo rate-limited. Public HTML is bootstrap capacity, not a production SLA. Add an authenticated contracted provider and rerun the same 30-attempt capacity gate.
2. **Latency and throughput:** the complete cold matrix reached p95 5.09 s and p99 5.94 s, inside the cold ceiling. The first coherent resident/revalidation matrix passed at 12.35 qps, p95 1.24 s, p99 2.40 s, and background-crawl p99 1.70 s. This closes the declared single-process serving gate, but it is not a long soak, distributed-capacity, cost, or Google-scale result. Provider, crawl, evidence hydration, and index mutation still require longer mixed-load tests.
3. **Freshness and coverage:** 24 semantic contracts and 96 surface forms are not web-scale recall. The fresh notebook failure, lack of geolocated local ranking, and price abstention without a jointly validated structured quote show concrete vertical gaps. Build a pooled, time-stamped gold set across thousands of head, torso, tail, multilingual, local, adversarial, shopping, and breaking queries.
4. **General source dependency graph:** poll registration supplies one narrow resolved measurement lineage, and factual relations now expose a normalized claim signature with unknown derivation. The small publisher-family seed map must still become a versioned ownership, syndication, quotation, funding, and underlying-event graph across claim classes before factual source families count as independent observations.
5. **Additional primary structured joins:** the TSE/PesqEle poll join is live. Due diligence still needs legal-entity, domain-control, sanctions, complaints, and operational registries; other verticals need their own authority-scoped adapters.
6. **Thematic synthesis:** generic current news requires event clustering and claim-by-claim evidence, not one extractive sentence.
7. **Corrections and temporal versions:** preserve what was known at query time and propagate source corrections/retractions.
8. **Human evaluation:** blind assessors must score answer correctness, completeness, citation entailment, source quality, noise, usefulness, and abstention quality against market baselines.
9. **Cash thesis:** still unvalidated in the live system. Domain stake is not claim-specific conviction; no current result received a cash confidence benefit. A prospective, legally reviewed, outcome-resolved cohort and evidence-only versus evidence-plus-cash ablation remain mandatory.
10. **Scale:** 96 cold requests plus two 288-request resident matrices at concurrency four validate useful single-process race safety, not Google-level indexing, distributed serving, sustained load, cost, abuse resistance, or broad recall.

## 8. Reproduction

```bash
cargo build --release -p oi-cli -p oi-bench --features onnx

# One real query with the full trace
OPENSONARX_DATA_DIR=/tmp/opensonarx-live \
  target/release/oi search "pesquisa eleitoral presidente brasil hoje" \
  --answer --explain --limit 5 --output table

# Entire 24-case production path
cargo run --release --manifest-path bench/Cargo.toml --features onnx -- \
  live-search --data-dir /tmp/opensonarx-live --passes 2 \
  --top-k 5 --answer --output table

# True cold matrix: create an isolated empty directory exactly once
OPENSONARX_E2E_DIR="$(mktemp -d /tmp/opensonarx-cold.XXXXXX)"
target/release/oi-bench live-search \
  --data-dir "$OPENSONARX_E2E_DIR" --passes 1 --concurrency 4 \
  --query-variants --top-k 5 --answer --summary-only --output table \
  --report-path /tmp/opensonarx-cold-report.json

# Reuse that exact populated directory for resident serving + revalidation
target/release/oi-bench live-search \
  --data-dir "$OPENSONARX_E2E_DIR" --passes 3 \
  --concurrency 4 --query-variants --top-k 5 --answer \
  --summary-only --output table \
  --report-path /tmp/opensonarx-resident-report.json

# Reproduce the dated Google election-poll candidate-set replay
target/release/oi-bench election-polls \
  --dataset bench/fixtures/brazil-election-polls-google-2026-08-21.json \
  --output table

# Probe sustained raw-provider capacity independently from search quality.
# The JSON is written even when the redundancy gate exits non-zero.
target/release/oi-bench engine-probe \
  --engines bing,brave-html --rounds 10 --concurrency 4 --output table \
  --report-path /tmp/opensonarx-provider-report.json

# Real no-primary failover/chaos: this is the production client with Bing
# removed, not a mock provider.
FAILOVER_DIR="$(mktemp -d /tmp/opensonarx-failover.XXXXXX)"
target/release/oi-bench live-search \
  --data-dir "$FAILOVER_DIR" --engines brave-html,duckduckgo \
  --passes 1 --concurrency 4 --top-k 5 --answer --summary-only \
  --report-path /tmp/opensonarx-failover-report.json

# Compare the canonical cases in that exact no-mock report with the dated
# signed-out Google candidate fixture; comparable Google latency is intentionally absent
target/release/oi-bench market-compare \
  --live-report /tmp/opensonarx-resident-report.json --output table \
  --report-path /tmp/opensonarx-market-report.json

# Recalculate the fail-closed exploratory browser-vs-backend latency audit
target/release/oi-bench market-latency \
  --fixture bench/fixtures/google-vs-opensonarx-latency-exploratory-2026-08-22.json \
  --output table \
  --report-path /tmp/opensonarx-market-latency-report.json

# Fail closed across discovery, cold/resident E2E, SLOs, candidate quality,
# and the still-unvalidated research claims.
target/release/oi-bench production-readiness \
  --provider-report /tmp/opensonarx-provider-report.json \
  --cold-report /tmp/opensonarx-cold-report.json \
  --resident-report /tmp/opensonarx-resident-report.json \
  --market-report /tmp/opensonarx-market-report.json \
  --report-path /tmp/opensonarx-readiness-report.json --output table

# With production credentials, include them in both the E2E path and probe
# export SERPER_API_KEY=...  # or BRAVE_API_KEY=...
# target/release/oi-bench engine-probe \
#   --engines serper,bing --rounds 10 --concurrency 4

# One case while iterating
target/release/oi-bench live-search \
  --data-dir /tmp/opensonarx-live --case factual_anthropic_founders \
  --top-k 5 --answer --output json
```

The benchmark refuses a mock embedder. Keep the data directory isolated for a true cold run and reuse it for a warm/flywheel run. Capture provider health alongside latency; otherwise a fast local result and a timed-out web fallback are incorrectly averaged as the same operating condition.

## 9. Bottom line

The thesis is now executable end to end for a narrow but meaningful slice. The system demonstrably behaves differently by intent, learns from live discovery, preserves evidence, emits Stage-2 uncertainty metadata, and can refuse unsupported conclusions. That is real progress toward a trust-oriented search engine.

It is not yet proof of a new universally superior uncertainty algorithm, proof that cash improves truth estimation, or proof of Google-scale search. The next decisive work is provider-resilient continuous evaluation, extending the narrow poll lineage into generalized dependency-aware evidence graphs and additional primary registries, blinded market comparison, and the prospective cash ablation defined in the MVP protocol.
