# OpenSonarX v2: A Web-Scale Search and Uncertainty Protocol

**Version 2.9 — DRAFT — 9 October 2026**

**Authors:** OpenSonarX Core Team

> Revision note: v2.9 integrates a typed structural checklist into answer construction
> and the returned trace, corrects generic discovery routing and adds origin-negation/context
> safeguards. Live diagnostic captures on 80 new implementer-selected queries exposed
> inadequate answer coverage and latency; they do not demonstrate superior truth quality.
> See `docs/experiments/EXECUTION-2026-10-09.md` for per-cycle results and unresolved gates.
> Protected P0 delivery, conditional cash and real payouts remain unimplemented/unvalidated.
> The subsequent execution cycle also fixed title/body denial overrides, origin synonym
> routing, evidence-refresh selection and provider-failure handling. The new 40-query
> diagnostic capture was interrupted at the provider level by exhausted Serper credits;
> it cannot confirm quality gains. See `docs/experiments/EXECUTION-CYCLE2-2026-10-09.md`.
> The subsequent local economics audit adds billable-attempt, pilot-volume,
> participant-cost, subsidy-farming, ambiguity and collateral-capacity stress tests.
> It aligns the legacy SDK burn default with the contract initializer (10%).
> These are illustrative scenarios, not measured costs or deployed settlement;
> slashing a bond does not currently indemnify the consumer. See
> `docs/experiments/ECONOMICS-REVIEW-2026-10-09.md` for assumptions and open gates.
> A subsequent local implementation adds integer price floors, participant-cost
> and coalition-budget checks, collateral allocation with cluster limits and
> replay rejection, and conserved dispute distributions. Legacy local/contract
> fee splits reject invalid percentages and avoid overflow. These are not yet
> authenticated, durable P0 delivery/billing or deployed deterministic settlement.
> See `docs/experiments/ECONOMICS-IMPLEMENTATION-2026-10-09.md`.
>
> Historical revision note: v2.8 disables the marginal cash probability update, bounds descriptive
> evidence mass, and adds an explicit critical-missing gate. It specifies a simpler publisher
> delivery profile, economically testable challenge rewards, cost sensitivity and quantitative
> validation gates. Only the local estimator safeguards are implementation changes. Protected
> delivery, cash/model activation, real payouts and market superiority remain unvalidated.
> Appendix I records the changes. No new market-search capture accompanies this revision.
>
> Previous revision: v2.7 clarifies the product as general search with a claim-level uncertainty layer
> and publisher-selected conditions of use. It specifies an open contextual evidence format,
> separates discovery from protected delivery, and distinguishes free access, attribution,
> paid use, and separately licensed training. It adds creator economics, multimodal and training
> boundaries, current competitive overlap, and a bounded validation pilot. These additions are
> design requirements, not newly implemented services or new empirical results. Appendix H records
> the changes. The August 2026 search results below remain historical.
>
> Previous revision: v2.6 added the first claim-scoped structured evidence join. Brazilian election-poll
> answers resolve the exact retained TSE/PesqEle registration, distinguish one underlying
> measurement from its editorial reports, and expose known independence separately from unresolved
> dependency uncertainty. It also records passing 96-request cold and 288-request resident no-mock
> matrices, a sustained provider-capacity gate, a real no-primary failover run, and pooled
> system-blind market diagnostics. Two further-disjoint 48-query runs reached parity with raw
> Serper after query-relative source roles and bounded per-provider candidate depth, but the general
> superiority gate remains false. The cash mechanism is unchanged: every live result still had zero
> cash effect because no eligible, outcome-calibrated claim position existed.
> Appendices F and G summarize the recent changes.

---

> *"Search broadly. Measure uncertainty. Preserve the web."*

---

## Abstract

OpenSonarX is a proposed general-purpose search service for people and AI agents. It finds relevant sources, exposes what supports or contradicts each material claim, and uses the experimental OpenUncertainty layer to explain when reliance is warranted or evidence is insufficient. Publishers can separately choose conditions for authorized use, including free distribution, attribution, and payment. The current implementation is a local prototype, not the complete protocol or a validated replacement for market search.

The web's trust signals are weakening. Generative systems have sharply reduced the cost of producing fluent, optimized, mutually derivative content, while a rapidly growing class of web readers is software. This creates an **AI-slop feedback loop**: low-evidence pages gain retrieval exposure, are summarized or cited by agents, become inputs to further pages and answers, and crowd out the original work that made the corpus useful. Not all AI-assisted content is low quality; the failure is source-less, stale, circular, or unsupported content receiving confidence it has not earned.

OpenSonarX v1 proposed stake-weighted search: publishers bond $TRUTH tokens on domains to vouch for quality. Two findings since then force a redesign. First, **capital must never touch relevance**: unit-tested adversarial constructions show the v1 ranking formula let bounded stake multipliers override relevance between closely-ranked results, and the protocol now pins stake-neutral ranking as an invariant (the surviving DNS multiplier has now been removed; a full with-registry/without-registry corpus ablation remains a committed Phase A deliverable — see §2 P4). Second, and more fundamentally: **incentives are only as sound as the verification predicate they settle against**. The one vector our adversarial analysis leaves fully open (Sybil feedback), and the subjective residues of the partially-closed ones (jury collusion, dispute timing), sit exactly where capital was placed at risk against a *subjective* judgment. Staking against a corruptible oracle is not Bitcoin; it is a prediction market with a bribable referee.

v2.8 retains three separated planes. The **retrieval plane** finds a broad, stake-independent candidate set. The **evidence plane** attaches provenance, deterministic checks, source history, freshness, citation dependencies, corroboration, contradiction, model assessments, and eligible economic-conviction records at claim level. The **decision plane** computes task-scoped uncertainty when calibration permits, then may re-rank, filter, cite, or abstain. Raw capital and popularity never buy relevance or confidence. Cash can affect a future probability only through a conditional or joint model that demonstrates incremental information beyond the same no-cash evidence baseline. The current marginal cash model has zero probability authority.

The central product output is not a universal truth label. It is a versioned **Uncertainty Estimate**: a probability and interval when a measurable calibration target exists, or an evidence state—support, conflict, and ignorance—when it does not. In both cases it preserves distinct uncertainty sources such as dependence, staleness, and distribution shift instead of hiding them in one prestige score. OpenSonarX aims to make general-purpose search competitive at web scale while reducing epistemic noise for agents and humans. That is a design goal to be benchmarked, not a claim that the current repository is already a Google-scale system.

Publishers may additionally choose free distribution, visible attribution, or paid licensed delivery, with training governed separately. A proposed open evidence-unit format connects contextual claims to retained originals, transformation records, corrections, and rights. Public catalogs may advertise protected resources without exposing their contents. This can support original information production and authorized dataset delivery without requiring a page visit, but payment does not certify accuracy, access control does not prevent every downstream copy, and the protocol cannot compel nonparticipants to pay. Improving training data is a separate research hypothesis, not a demonstrated prevention of model collapse. Search and evidence remain useful without a blockchain or native token; on-chain settlement is optional infrastructure for justified multi-party commitments.

---

## Table of Contents

1. [Motivation: The Trust Collapse](#1-motivation-the-trust-collapse)
2. [Design Principles](#2-design-principles)
3. [The Trust Stack](#3-the-trust-stack)
4. [The New Primitive](#4-the-new-primitive)
5. [Protocol Core](#5-protocol-core)
6. [The Publisher Protocol](#6-the-publisher-protocol)
7. [Web-Scale Search and Discovery](#7-web-scale-search-and-discovery)
8. [Cold Start and the Underwriting Market](#8-cold-start-and-the-underwriting-market)
9. [Economics](#9-economics)
10. [Security Analysis](#10-security-analysis)
11. [What This Protocol Refuses to Do](#11-what-this-protocol-refuses-to-do)
12. [Legal & Regulatory Posture](#12-legal--regulatory-posture)
13. [Migration from v1](#13-migration-from-v1)
14. [Roadmap](#14-roadmap)
15. [Conclusion](#15-conclusion)
16. [Related Work and References](#16-related-work-and-references)

---

## 1. Motivation: The Trust Collapse

### 1.1 Trust signals were always economics

The web never had a universal trust protocol; it had *proxies for costly effort*. A published page implied someone paid for hosting and took the time to write. Fluent, specific prose often implied a competent human author. An inbound link could reflect third-party editorial judgment, and PageRank extracted a useful signal from that link graph. A masthead's reputation summarized years of institutional behavior.

Many of these signals weakened in the same way: **the cost of producing the signal fell while the value attributed to it did not**. SEO industrialized link manufacturing, and generative models made plausible text, images, and publications much cheaper to produce. The signals still exist, but they carry less evidentiary weight. The problem is not a shortage of content; it is the increasing separation between observable presentation and the costly behavior readers once inferred from it.

### 1.2 The consumer and producer changed at the same moment

LLMs and autonomous agents are becoming an important class of web readers. They do not interact with advertising like humans do, they can consume at machine speed, and they can also generate the low-cost content that later enters retrieval corpora. If machine-mediated consumption grows, publishers need a payment path that does not depend entirely on human page views, and agents need machine-readable evidence that does not require a human review of every document.

### 1.3 The AI-slop feedback loop

The failure is recursive:

```
cheap unsupported production
  → more indexable volume
  → retrieval and citation exposure
  → summaries, rewrites, and synthetic derivatives
  → weaker attribution and publisher revenue
  → less investment in primary work
  → an even noisier corpus
```

Conventional spam detection attacks presentation patterns, which adversaries can imitate or change. OpenSonarX instead asks what evidence follows the content: Where did the claim originate? Which artifacts support it? Which checks are reproducible? Are the apparent sources independent or circular? How current is the evidence? What happened when this source made comparable claims before? AI assistance itself is not a negative signal; unsupported derivation and hidden dependency are.

### 1.4 What must be rebuilt

Three things, implemented as separate planes:

1. **Web-scale retrieval:** broad, fast discovery whose candidate generation cannot be purchased through stake or reputation.
2. **A calibrated uncertainty layer:** a claim- and task-specific estimate of what reliance is warranted, why, and how much remains unknown.
3. **An economic loop that can fund content creation:** payment for licensed retrieval, delivery, or citation flowing to identified rightsholders and contributors.

The planes share identifiers, but remain deliberately separate: ordinary payment is not evidence of accuracy; accountability is not a license; relevance is not trust; and uncertainty is not metaphysical truth. A deliberately risked claim position is handled as the separate, empirically calibrated conviction signal in §5.8.3.

Throughout this paper, **trust means warranted reliance under a named task and loss**, not prestige, popularity, or moral approval. Uncertainty is the machine-readable representation of how much reliance the available evidence warrants and how much the system still does not know. In that operational sense, trust is central to the product even though it is forbidden from purchasing or pruning base relevance.

### 1.5 Product definition and the creator's choice

For a person, OpenSonarX is a search service that returns relevant sources and, where supported, an answer with evidence and explicit limitations. An AI agent can use the same interface on the person's behalf. The mission remains **more reliable general search**, not a mandatory financial market attached to every question. A navigational query such as "Claude" should find the appropriate destination, not invent a truth dispute about it.

Machine-generated answers can use a creator's work without delivering the page visit that previously supported advertising or offers. An observational [Pew Research Center study](https://www.pewresearch.org/short-reads/2025/07/22/google-users-are-less-likely-to-click-on-links-when-an-ai-summary-appears-in-the-results/) of 900 U.S. adults' browsing in March 2025 found traditional-result clicks on 8% of visits with a Google AI summary versus 15% without, and summary-source clicks on 1% of visits. Query mix and the study's design prevent treating this as a causal estimate or a universal forecast of creator revenue.

Publishers do not all want the same return. A source may seek attribution, audience, authority, customer acquisition, direct revenue, or a combination. The proposed protocol makes that choice machine-readable rather than requiring every source to charge. Attribution does not guarantee a click or confer epistemic authority. Ordinary paid use can finance original work without a visit, but it creates revenue only where an authorized buyer values the service and accepts the terms (§6, §9).

### 1.6 Retrieval contamination and recursive training are different problems

Retrieval contamination occurs when unsupported derivatives look like independent evidence. Recursive training can also degrade models: [Shumailov et al. (2024)](https://www.nature.com/articles/s41586-024-07566-y) demonstrate model collapse under studied generational training conditions, including loss of less frequent information. This is not proof that all synthetic data are harmful or that frontier models inevitably collapse. [Gerstgrasser et al. (2024)](https://arxiv.org/abs/2404.01413) show that retaining original data while accumulating synthetic data avoids collapse in their experimental settings.

OpenSonarX addresses retrieval first by preserving original artifacts and representing derivation and unresolved ancestry. A separately licensed training export could preserve the same lineage, original coverage, diversity, and correction history. Such an export is a candidate tool for better data selection, not proof of correct training outcomes. Human origin is not proof of truth, AI assistance is not proof of error, and no detector is assumed to classify every item's origin reliably. §7.3 requires an independent training experiment before claiming a downstream model improvement.

---

## 2. Design Principles

### P1 — Incentives settle against verification predicates, or they are theater

Bitcoin's relevant lesson is not "incentives + token." Its consensus relies on a small set of deterministic validation rules plus a fork-choice rule that independent nodes can reproduce. Slashing conditions are only as trustworthy as the process that decides them. Wherever a protocol places capital at risk against a subjective judgment, that judgment becomes the attack surface.

Our v1 adversarial analysis ([docs/SECURITY-ECONOMICS.md](docs/SECURITY-ECONOMICS.md)) points in the same direction: objective mechanisms can be specified and tested, while Sybil feedback, jury collusion, and ambiguous disputes retain judgment-dependent attack surfaces. v2 therefore adopts a strict rule: **bonded capital moves only on a Grade-A deterministic predicate whose inputs, version, and trace are reproducible. Grade-B model checks and human review may add evidence, warnings, or type-scoped reputation annotations; they never slash, seize, or redistribute a bond. If no Grade-A predicate resolves a dispute, the protocol returns the relevant bonds and records the outcome as unresolved.**

### P2 — Decompose evidence before estimating uncertainty

"Trust" on the old web was a single conflated impression. v2 decomposes it into typed evidence with radically different semantics (§3). It then estimates uncertainty only for a specific claim, task, horizon, and policy. There is no consensus-level global identity score: an objective provenance fact is not silently averaged with subjective accuracy, and success in one commitment class cannot bootstrap authority in another. A combined decision score is allowed only after the components, target, calibration, and policy remain inspectable.

### P3 — Accountability, not truth

The protocol proves narrow facts: which key signed which bytes; where and within what interval a log recorded the commitment; what deterministic trace ran; and what licensed use produced a receipt. It does not prove global firstness or certify truth. For claims without a Grade-A predicate, it offers provenance, evidence, and type-scoped history only. A criterion outcome is a protocol event about a declared machine rule, not an adjudication of truth in any external sense (§12.3).

### P4 — Capital never touches relevance; uncertainty may guide decisions

The retrieval plane answers *"does this match the query?"* and produces a broad candidate set using stake-independent content features. The decision plane answers *"should this result be relied on for this task?"* and may re-rank the candidates using calibrated uncertainty, conflict, ignorance, freshness, diversity, and cost under an explicit client policy (§5.8–§5.9). Raw bond size, domain stake, payment volume, identity age, and popularity are forbidden inputs to relevance and cannot directly update uncertainty. The sole financial exception is the derived, claim-specific economic-conviction feature in §5.8.3: it requires genuine pre-outcome loss on an objective predicate and enters only through a held-out, capped calibration. This supersedes v1 whitepaper §12.2–12.3 (stake boost, reputation factor), which are retracted.

**Honest status of this invariant.** Unit tests (`crates/oi-index/tests/stake_ablation.rs`, `crates/oi-index/tests/signal_comparison.rs`, the `test_zero_stake_no_boost` / `test_stake_never_affects_score` / `test_reputation_never_affects_score` family, and a focused `HybridIndex` with-registry/without-registry test) pin that stake, reputation, and DNS verification do not enter the current scoring path and that non-zero registry enrichment leaves default-query order and scores unchanged. The retired v1 formula demonstrably allowed capital to override relevance between closely ranked documents; its surviving 1.05×/0.95× DNS multiplier has now been removed from `compute_score` and retained only as metadata. What is **not** yet established is the same ablation on representative benchmark corpora and a crate-boundary audit proving that no alternate retrieval path imports registry fields. Those remain Phase A gates (§13).

### P5 — Discovery stays open; confidence is earned

Any lawfully crawlable content can enter the unbonded discovery queue and, once indexed, ranks on retrieval quality rather than stake. Accountability labels are earned separately. Finite crawling capacity prevents a universal inclusion guarantee, so §7 specifies capacity reservations, transparent queues, and exposure-normalized exploration instead of pretending every URL can be indexed immediately.

### P6 — Zero-action observation; publisher cryptography is abstracted, not absent

Level 0 observation requires no protocol enrollment by the publisher. Site access controls, crawler policy, and law may still prohibit a fetch. Publisher-signed priority, licenses, commitments, and bonds require publisher authority, but plugins and familiar formats (DNS TXT records, `.well-known` JSON, feeds, and CMS integrations) should hide the cryptographic mechanics. Capabilities that create legal rights or financial exposure require explicit authorization (§6).

### P7 — Calibration, not confidence theater

A numeric confidence without a measurable target and held-out evaluation is another unearned trust signal. Every probabilistic output names its target, task, horizon, policy, evidence snapshot, model version, calibration set, and uncertainty interval. If there are too few comparable outcomes to support calibration, the protocol emits an evidential state with explicit ignorance instead of manufacturing a probability.

### P8 — Open protocol, explicit rights, portable evidence

The evidence exchange specification is open and versioned. Content availability and permissions are separate choices: an open-source client does not entitle its operator to every asset. Publishers may offer free or protected resources without buying ranking or trust. Internal storage formats may optimize cost and latency, but authorized consumers must have a documented export path that retains context, provenance, and rights references. A proprietary container, chain anchor, or license declaration cannot establish ownership of information about the world or prevent every downstream reuse.

---

## 3. The Trust Stack

Five layers, in descending order of verifiability. The protocol treats them with different mechanisms because they *are* different problems:

| Layer | Question | Verifiability | v2 mechanism |
|---|---|---|---|
| **Provenance** | Which identity committed to these bytes, when did a log include it, and what changes were observed? | **Objective** for signatures, hashes, inclusion proofs, and log consistency. Unsigned content receives operator attestations whose honesty and independence remain assumptions (§5.2). | Witness layer + publisher-signed commitments (§5.2). |
| **Fidelity** | Does the document faithfully represent its cited sources? Verbatim quotes? Numbers matching the dataset? Still matching upstream? | **Deterministically checkable** for a large class (exact-match after canonical normalization, snapshot equality, hash re-check); **model-checkable** (probabilistic) for semantic fidelity. | Instant-settlement commitments + challenge/audit game (§5.4–5.5), with slashing-grade economics on the deterministic subset. |
| **Accuracy** | Are the claims true? | **Sometimes reducible to a declared event predicate; often not protocol-decidable.** | Event-bound criterion outcomes where deterministic; evidence graphs and corrections otherwise (§5.6). |
| **Reliance/uncertainty** | Should this claim be used for task P over horizon H? | **Calibratable only against a declared, observable target.** Otherwise the output is evidential, not probabilistic. | OpenUncertainty engine: task-conditioned posterior or support/conflict/ignorance state (§5.8). |
| **Demand/exposure** | Was it retrieved or cited by paying agents? | **Measurable as protocol activity, not as usefulness or truth.** | Privacy-preserving receipt ledger (§9.1), discounted for correlated payers (§9.2). |

The old web's collapse is precisely the collapse of *proxies* for the top two layers. v2 replaces the proxies with the real thing, and attaches economics layer by layer only as far down as objectivity reaches.

---

## 4. The New Primitive

Cryptographic predicates have long been cheap to verify. What has changed is that automated extraction and model-assisted review make a wider range of checks economical to run at scale. v2 divides checks into **two grades, and the distinction is load-bearing**:

**Grade A — deterministic predicates.** Bit-reproducible by third parties given the same retained inputs and version: content-hash equality, signature validity, exact string match after canonical normalization, equality against a committed snapshot, or a rule over an archived signed record. Live DNS and scheduled fetches create time-specific observations; they become historically reproducible only when the exact response and verification material are retained. **Slashing-grade economics attach here and only here.**

**Grade B — model-checked predicates.** An LLM judging semantic fidelity: does this summary contradict its source? is this distillation faithful? Automated checks can make this review inexpensive enough to run far more broadly, but they are **probabilistic evidence, not objective predicates**: non-deterministic across runs, *correlated* across verifiers built on similar models (so N-of-M majority buys far less security than independence would suggest), and attackable through the very content being evaluated (prompt injection embedded in a committed document). The LLM-as-judge literature documents these failure modes. v2 uses Grade B checks as evidence and triage — never as the sole basis for a slash (§5.5).

In OpenSonarX v2, **the network's work is verification, audit, and evidence maintenance**. That work produces the claim-level graph from which uncertainty is estimated. Slashing decisions ride only on Grade-A checks; Grade-B results can affect a calibrated likelihood only under a published model and evaluation version, and never become consensus facts.

One consequence deserves emphasis: the two accountability paths in §5 cover different parts of the verifiability spectrum. Deterministic commitments can be bonded and settled. Novel claims without a machine-checkable criterion cannot be made objective by adding stake, time, corroborators, or a jury; they receive evidence and track-record annotations, not capital settlement.

---

## 5. Protocol Core

### 5.1 Identities and Keys

An identity is rooted in an Ed25519 keypair, optionally associated with:

- one or more **domains** via a DNS TXT binding observation (`oi-verify=<identity>`, checked via DoH at bind time and re-verified periodically; Grade-A status requires the retained authenticated material described in §5.4);
- a **wallet** for bonds and earnings — bound to the **identity keypair, never to the domain** (see below);
- a **typed record**: the full class-specific history of its commitments, challenges, audits, criterion outcomes, corrections, and paid exposure (§5.7).

**Key hierarchy and compromise.** Because an identity's history and payment authority are valuable, a bare hot key would make custody existential: one compromised CMS session could issue commitments or spend funds under the masthead. Therefore:

- An identity has a **cold root key** and any number of **delegated operational keys**, each carrying a bond ceiling and a scope (e.g., per-desk), revocable by the root at any time. Routine publication uses delegated keys; the root never needs to be hot.
- A **revocation record** signed by the root or a pre-registered recovery key freezes the affected operational key. Commitments issued before the revocation remain attributable to that key; commitments after the logged revocation time are invalid. A disputed compromise window is annotated and left unresolved unless a pre-declared Grade-A rule (for example, a key-expiry boundary) decides it. No jury decides whether an act was "stolen-key" or "institutional," and no identity-wide penalty follows from such a subjective classification.
- Track record is **non-transferable**. On unrecoverable root loss, the history is frozen (readable, never migrated) and the operator starts a new identity — the price of making matured identities unsellable. There is no reputation-transfer channel, by construction.

**Domain binding is a label, not a vault or license.** DNS shows that an identity controlled a domain at the verification time. It never moves funds or grants content rights: earnings accrue to an onboarded identity key under accepted terms. A binding change freezes future domain-authorized payments and license use during a cooldown; continuity of the historical identity record requires a signature from the prior key. Existing balances and records do not transfer with a lapsed domain.

Identities are cheap to create; class-specific history takes time and eligible work to accumulate. No identity receives protocol-wide authority. Sybils can still manufacture apparent activity, so raw denominators, payer concentration, sponsorship links, and correlation warnings remain visible rather than being compressed into a promise that an identity is globally mature.

### 5.2 The Witness Layer

The zero-publisher-action foundation. Protocol nodes already crawl; v2 can turn an authorized fetch into a signed observation while stating precisely what that observation does and does not establish.

1. A node fetches a URL, runs **canonical distillation** (§5.3), and computes `h = sha256(CDS_version || canonical_distillate)`.
2. It submits a witness record `(url, request_profile, cds_version, h, beacon, t_received, vantage, node_sig)` to an append-only Merkle transparency log. `beacon` is a recent public value that could not have been known before it was published; the signed tree head containing the record supplies a not-later-than bound. Together they bound when the **attestation** was created. They do not prove the origin server's response time or that every client received the same representation.
3. When **W** registered operators report the same variant within the configured window, the system emits a **quorum attestation**: *these operators state that they observed this representation, using this request profile, after beacon B and before tree head T*. The signatures prove who made the statements and make later alteration detectable; they do not turn operator statements into a cryptographic fact about the server.

**What witnessing proves — stated exactly.** Signature validity, record inclusion, and log consistency are cryptographic. Operator honesty and independence are not. "Distinct operators" rests on registration and infrastructure evidence, which are operational assumptions even if witnessing later becomes stake-backed. Interfaces therefore display the operator set, request profile, variant, and time bounds; they use "quorum attestation," not "proof that the URL served X."

**Log topology and availability.** One anchored Merkle root is insufficient: a log can equivocate between clients or withhold the artifact needed to reproduce a check. Phase A therefore requires multiple independently operated logs, signed tree heads, inclusion and consistency proofs, public monitors that gossip observed heads, and an availability policy for the exact response bytes or a licensed canonical artifact referenced by every economically live Grade-A predicate. A chain anchor proves that some root existed; monitors and retained artifacts make split views and unverifiable history detectable. Retention and serving remain subject to the license and erasure constraints in §12.

**Observation order is not authorship.** A witness record proves that a node signed an observation within bounded log time; it does not independently prove what the node saw. Crawl frequency is asymmetric: aggregators and well-resourced sites may be observed more often than small originators. Earliest-attested order inside a near-duplicate cluster is therefore informative metadata only, never evidence of authorship or a payment-routing rule.

**Anchored priority — the real thing.** Economic priority exists only where a publisher creates it: an **anchored priority commitment** is the content hash signed by the identity's key and submitted to the log at or before publication. Signature + beacon + tree head prove a narrow fact: *this identity committed to these bytes no later than T and not before B*. They do not prove authorship, originality, lawful ownership, or public availability. Any priority-based payment must therefore be explicitly licensed by the payer and scoped to exact or rule-defined derivative content; where those conditions are absent, each identity earns only from its own licensed use.

Claim-level priority across genuinely different texts asserting the same fact remains heuristic (assisted by `oi-facts` entity extraction), is labeled as heuristic, and never routes value — per P3.

### 5.3 Canonical Distillation and Variant-Aware Witnessing

The hard technical problem of the witness layer has two parts that earlier drafts conflated:

**Function determinism (achievable, required).** The **CDS (Canonical Distillation Spec)** is a deterministic, rule-based transform: fetched representation → canonical Markdown. No LLM in the canonical path — LLM distillation is non-deterministic and is used only for enrichment layers (summaries, extracted spans) that are never hashed. Every hash is tagged with its CDS version; nodes running the same version must produce byte-identical distillates *for the same input bytes*, pinned by golden-file conformance tests that are part of the spec. Version upgrades are coordinated with overlap windows; historical hashes remain verifiable under their recorded version.

**Input agreement (not generally achievable — and not assumed).** Two honest nodes can legitimately receive different content at the same URL: geo/CDN variation, A/B tests, consent walls, personalization, rendering differences, and time variance. Divergence is therefore data, not evidence that a node lied:

- Witnessing is **variant-aware**: records carry request and vantage metadata, and observations of the same URL cluster into contemporaneous variants. The interface says which operators attested to which variant; it does not assert a single true representation.
- Node dishonesty is established only by Grade-A evidence about the record itself: an invalid signature, a failed inclusion/consistency proof, or two incompatible signed statements under a request profile whose rules make them mutually exclusive. A later re-fetch, even from many vantages, cannot prove what a node saw historically; it can only create a new contemporaneous observation.
- Consequences for consumers of witnessing: exact-mirror freshness can settle only against a publisher-declared reference artifact and request profile with deterministic timing rules. Multi-vantage observations reduce operational error but do not make a semantic or historical inference slashable. Anchored priority commitments (§5.2) bind to publisher-signed bytes and avoid crawl-order ambiguity.

The cost of this honesty: there is no single canonical hash per URL, and any mechanism that assumed one (v2.0's quorum-resolved "one true distillate") is restated in variant terms. The existing distiller (`oi-crawl`) is the seed of CDS-1; making it deterministic, versioned, and conformance-tested is a Phase A gate.

### 5.4 Commitments

The core object of v2 is a typed **commitment** by an identity about specific content. A bond is permitted only when that type has a Grade-A settlement rule; evidence-only commitments carry no truth bond:

```
Commitment {
  id,
  identity,               // key that signed (root or delegated, §5.1)
  content_ref,            // (url, cds_version, variant_hash)
  type,                   // taxonomy below
  statement,              // typed parameters of the claim
  bond?,                  // present only for a Grade-A settlement rule
  settlement_regime,      // INSTANT | TEMPORAL | EVIDENCE_ONLY
  settlement_criteria,    // exact inputs, predicate version, deadline, transitions
  sealed_evidence[],      // salted commitments to undisclosed material (§5.6)
  anchored_at,            // beacon-sandwiched, anchored log position (§5.2)
  status,                 // OPEN | CHALLENGED | AUDITED_PASS | AUDITED_FAIL
                          // | CRITERION_MET | CRITERION_NOT_MET
                          // | CORROBORATED | CONTESTED | CORRECTED
                          // | EXPIRED_UNRESOLVED
}
```

**Initial commitment taxonomy.** New versions may be added prospectively through governance, but a type may be bonded only after its Grade-A inputs, trace, artifact-availability rule, and refund behavior pass review:

| Type | Statement | Regime | Checked by | Grade |
|---|---|---|---|---|
| `domain_binding` | "The authenticated DNS record for D bound K at observation T" | Instant metadata | Retained DNSSEC proof where available; otherwise multi-resolver operational evidence | A with reproducible DNSSEC input; otherwise attested, unbonded |
| `verbatim_quotes` | "Every quotation herein is verbatim from its cited source" | Instant | **Exact match after canonical normalization** (whitespace/encoding/typographic-quote folding; bracketed editorial insertions declared inline). Fuzzy similarity is used only to *locate* candidates and to flag for review — never to pass a mismatch. A dropped "not" or a changed digit is a mismatch, period. | A |
| `data_fidelity` | "Numbers herein match cited dataset/API at snapshot H" | Instant | Equality against the committed snapshot | A |
| `exact_mirror_freshness` | "Representation R will equal declared upstream snapshot rule U within Δ" | Instant (recurring) | Equality under a pinned request, canonicalization, and timing profile | A |
| `semantic_freshness` | "This page remains substantively current with source X" | Evidence only | Model-assisted semantic comparison | B — no slash |
| `anchored_priority` | "Identity K committed to bytes H between B and T" | Instant + priority | Signature, beacon, inclusion, and consistency proofs | A; makes no authorship claim |
| `faithful_distillation` | "This document faithfully represents source URL X at hash H" | Evidence only | Semantic fidelity check | B — no slash |
| `event_bound_assertion` | "Machine-observable event E will meet criterion C by T" | Temporal | Pinned external record + deterministic criterion (§5.6) | A |
| `open_reporting_assertion` | "Publisher reports claim C" | Temporal evidence | Anchoring, corroboration, correction history | No truth-settling grade |

Commitments enter the index alongside the content they cover; every result can expose the attached commitment types, statuses, traces, evidence, and the identity's relevant class-specific history—the machine-readable epistemic record of P2.

### 5.5 Settlement Regime I: Instant

Instant settlement exists only for a versioned Grade-A predicate with declared inputs and a reproducible trace.

1. **Optimistic display.** An open commitment is displayed as `OPEN`, never as passed merely because nobody challenged it.
2. **Random audit.** A capped verifier budget funds random and risk-weighted audits. The verifier publishes the predicate version, input references, artifact hashes, execution trace, and result. `AUDITED_PASS` and `AUDITED_FAIL` describe that execution; they are not generic truth labels.
3. **Challenge.** Anyone may post a challenge bond and identify the exact predicate and input that allegedly fail.
4. **Independent reproduction.** Multiple selected verifiers rerun the same deterministic predicate. A majority vote is not the source of validity: the trace is. Matching traces execute the pre-declared payout, refund, or slash rule. Divergent traces pause the commitment class, return the challenge and commitment bonds, and open a specification defect.
5. **Specification repair.** Human or model review may explain the defect and propose a new predicate version, but it cannot decide the old bond. Historical commitments remain bound to their original version unless all affected parties explicitly opt into a migration.

**Execution gate.** Reproducibility by honest observers is not enough if the settlement contract cannot distinguish a real trace from colluding verifiers' assertions. Each bonded type must therefore specify a deterministic execution environment and an enforceable verification path: direct contract execution or a proof for small predicates, or a content-addressed optimistic execution with a complete fraud-proof game and challenge window for larger predicates. Until that path is implemented and audited, the type may emit `AUDITED_*` evidence but cannot move capital.

Grade-B checks run in an evidence lane. Model-family diversity, repeated sampling, injection-resistant harnesses, and human review can improve the evidence, but none converts the result into Grade A. Suspected prompt injection is logged with the model assessment as a security signal and never automatically moves capital.

**Verifier economics, capped.** Verifiers are paid for reproducible work within a per-epoch budget. A challenge from the challenged party or an economically correlated cluster receives no reputation credit, and all payouts are included in the coalition anti-wash test of §9.2.

**Suppression rules.** Filing a challenge only annotates the affected commitment. Automatic suppression occurs after an `AUDITED_FAIL` when the client or serving policy has pre-declared that exact predicate as a safety or licensing gate. Suppression is commitment-scoped, not result-scoped, and a specification disagreement never suppresses unrelated content. Open-reporting assertions are never protocol-suppressed on the basis of a truth dispute.

### 5.6 Settlement Regime II: Temporal (Novel Claims)

Novel claims expose the boundary of the protocol: **time can reveal new evidence, but time does not make a subjective claim mechanically decidable.** v2.2 introduced—and v2.4 retains—the separation between two objects that v2.1 incorrectly combined.

#### 5.6.1 Event-bound assertions

An `event_bound_assertion` predicts or asserts a narrowly specified machine-observable event. Before bonding, the committer must pin:

- the exact external record or API, request profile, schema, and archival rule;
- a deterministic predicate over that record;
- a deadline and grace period;
- the payout, refund, and slash transition for `CRITERION_MET`, `CRITERION_NOT_MET`, and `EXPIRED_UNRESOLVED`; and
- whether the record is controlled by a party whose conduct is part of the disputed claim.

The status describes the criterion, not external truth. For example, `CRITERION_NOT_MET` means only that the declared record did not contain the declared value by the deadline. If the source is unavailable, changes schema, equivocates, or falls outside the pinned archival assumptions, the outcome is `EXPIRED_UNRESOLVED`: bonds and payer escrow are returned. A subject-controlled record can confirm that the subject published something; its silence or denial does not refute a broader journalistic allegation.

Only this class may carry settlement-contingent assertion escrow. `CRITERION_MET` releases it according to the pre-declared rule. `CRITERION_NOT_MET` returns it to payers and may slash the committer only when that exact failure transition was declared before publication and the enforceable Grade-A trace shows the predicate failed. The deterministic trace, not an interpretation, executes the transition. Lead time and specificity may affect rewards inside this class, but being "first and alone" is not a general reputation multiplier.

#### 5.6.2 Open reporting assertions

An `open_reporting_assertion` records that an identity reported a claim; it does not ask the protocol to decide whether the claim is true. At publication, the protocol can establish:

- an **anchored commitment** to the exact claim bytes within a bounded time window;
- optional **sealed-evidence commitments** `H(evidence ‖ salt)` using a fresh high-entropy salt and padded metadata; and
- the identity's prior record in this same assertion class.

A sealed envelope proves only that later-revealed bytes match an earlier commitment. It does not prove that the evidence is authentic or that it substantiates the claim. A publisher may reveal to the public, reveal under a separate legal confidentiality agreement, or never reveal. Non-reveal removes the envelope's evidentiary weight; it does not burn a bond, imply fabrication, or create a challenger bounty. A verification circle may publish a signed, typed assessment, but that assessment is Grade B and has no protocol power over capital. Confidentiality remedies, if any, arise from a separate contract and applicable law rather than subjective protocol slashing.

Other identities may attach corroboration, contradiction, or echo records. Novel evidence, funding relationships, timing, and historical co-occurrence help clients estimate independence, but the score is heuristic and can be evaded by patient Sybils. Corroboration therefore never changes an assertion to "true" and contradiction never changes it to "false." The available lifecycle is:

```
OPEN ──► CORROBORATED / CONTESTED ──► CORRECTED
  └──────────────────────────────────► EXPIRED_UNRESOLVED
```

These annotations are non-exclusive: a claim may be both corroborated and contested, and the full evidence graph remains visible. Corrections append a new version and link the changed spans; history is never silently rewritten. Clients may build editorial policies from the graph, but the protocol does not supply a consensus verdict.

Open reporting earns ordinary licensed retrieval or citation revenue under §9.1. It receives no truth-contingent assertion premium and no burn-on-expiry. This prevents the protocol from quietly deciding that unresolved journalism deserves no payment while also preventing ordinary demand from masquerading as evidence that the reporting is accurate. Any assertion-specific reward must be tied to a separate Grade-A event-bound commitment.

### 5.7 Typed Accountability Records, Not a Global Verdict

The protocol record is a vector of histories, not a scalar judgment about an identity:

```
AccountabilityRecord(identity) = {
  provenance,
  fidelity.verbatim,
  fidelity.data,
  freshness.exact,
  fidelity.semantic_evidence,
  temporal.event_bound,
  temporal.open_reporting,
  demand_exposure
}
```

Each field exposes raw counts and denominators: open commitments, audited passes and failures, challenged and unresolved cases, capital-time at risk for that exact class, corrections, event-bound criterion outcomes, paid exposure, and payer concentration. Grade-B evidence is stored with its model, prompt, version, confidence, and disagreement metadata; it never contributes to a consensus pass rate.

There is **no cross-class transfer**. Passing many quote checks does not make an identity authoritative on breaking news. High demand does not improve fidelity. A large bond does not improve any historical rate. Underwriting, client filters, and any reward multiplier are scoped to the same commitment class and cannot confer a global uplift. An eligible claim-specific bond may contribute the separate calibrated conviction likelihood in §5.8.3; it never rewrites the source's pass/fail history.

Clients may apply local, published policy profiles to the vector — for example, `--accountability-policy legal-citations-v1.json` might require domain control, a minimum number of recent verbatim audits, a maximum unresolved rate, and acceptable artifact retention. Different applications can legitimately produce different labels from the same facts. Those labels are client policy outputs, not protocol consensus. A policy label cannot enter base relevance; the claim-level uncertainty engine below may use the underlying typed evidence only through a declared, calibrated model.

### 5.8 OpenUncertainty: Claim-Level Calibrated Reliance

OpenUncertainty is the central algorithmic proposal of v2.7. It estimates whether relying on a claim is warranted **for a declared use**, rather than assigning an enduring trust score to a page, domain, or identity.

#### 5.8.1 Unit, target, and output modes

An estimate is defined over a claim `c`, task or use policy `p`, horizon `h`, and versioned evidence snapshot `G_t`:

```
U(c, p, h, G_t)
```

The target variable `Y(c,p,h)` must be stated before a probability is emitted. Examples include “the quoted span matches the cited snapshot,” “the declared event occurs by T,” or “independent review under rubric R finds the answer adequately supported.” Different tasks can produce different estimates for the same claim. A source can be dependable for sports scores over a five-minute horizon and unsuitable for medical advice over a five-year horizon.

The engine has three modes:

1. **`DETERMINISTIC`:** an applicable Grade-A predicate returns `PASS`, `FAIL`, or `UNRESOLVED`. The engine reports that exact result and scope; it does not inflate it into proof of the whole claim.
2. **`CALIBRATED`:** a measurable target, sufficient comparable outcomes, and held-out calibration data exist. The engine returns a probability of the declared target plus an uncertainty interval.
3. **`EVIDENTIAL_ONLY`:** the target is not observable, the sample is insufficient, or the evidence is out of distribution. The engine returns support, conflict, and ignorance without inventing a probability.

```
UncertaintyEstimate {
  claim_ref, task, horizon, policy_id, mode,
  p_reliable?, interval_90?,
  support, conflict, ignorance,
  provenance, deterministic_support,
  source_calibration, evidence_coverage,
  independence, freshness, model_disagreement,
  posterior_dispersion?, dependency_uncertainty,
  staleness_uncertainty, distribution_shift,
  evidence_root, model_version, calibration_set,
  generated_at
}
```

`p_reliable` means only `P(Y(c,p,h)=1 | G_t, model_version)`. It is not the probability that a page, person, or institution is “true.” `ignorance` is separately reported because lack of evidence must not be disguised as a confident 0.5.

The uncertainty output is therefore a vector, not `1 - p_reliable`:

```
U_vector = (
  posterior_dispersion,  // finite-data and parameter uncertainty, if probabilistic
  ignorance,             // missing eligible evidence or target coverage
  conflict,              // material evidence in opposing directions
  dependency_uncertainty,// unresolved common ancestry/correlation
  staleness_uncertainty, // horizon mismatch and evidence decay
  distribution_shift     // distance from validated calibration support
)
```

These components have different remedies and must not be collapsed at the protocol layer. More independent evidence may reduce ignorance and posterior dispersion; it does not resolve genuine conflict. A fresh primary record can reduce staleness; it does not repair a shifted task definition. A client policy may map the vector to expected loss for a named decision, but the original components remain visible.

In `EVIDENTIAL_ONLY` mode, the engine publishes a normalized descriptive opinion, not a posterior:

```
S = sum_i adequacy_i · max(direction_i, 0)
C = sum_i adequacy_i · max(-direction_i, 0)
K = policy-declared missing-evidence mass
M = min(S + C, 1)
S_bounded = M · S / (S + C)   // 0 when S + C = 0
C_bounded = M · C / (S + C)   // 0 when S + C = 0
sample_ignorance = N_min / (N + max(N_min, 1))
K_effective = K + max(M + K, 1) · sample_ignorance

support    = S_bounded / (M + K_effective)
conflict   = C_bounded / (M + K_effective)
ignorance  = K_effective / (M + K_effective)
```

`direction_i` records whether an item supports or conflicts with the exact claim under a versioned relation rubric; `adequacy_i` combines scope, availability, freshness, and estimated independence without asserting truth probability. `K` comes from a policy-declared evidence checklist and coverage model, not an arbitrary tuning knob chosen after seeing the claim. `N` is eligible typed source-history count; `N_min` is its declared minimum. Bounded observed mass prevents document volume alone from washing away a fixed missing requirement. Dependence clustering remains necessary: bounding does not discover unknown copies. If no eligible evidence or checklist item exists, the engine defines `(support, conflict, ignorance) = (0, 0, 1)`. The raw `S`, `C`, `K`, rubric, and graph remain available because the normalized tuple alone cannot show whether both support and conflict are abundant or both are absent.

`critical_missing` separately records an unmet mandatory task-checklist item, including a required inaccessible artifact. It prevents a complete reliance endorsement and forces `EVIDENTIAL_ONLY` with no probability, regardless of source history or capital. A deterministic check may still report its narrow predicate outcome, but the missing flag remains and cannot authorize a broader conclusion. The generic estimator implements this flag; the search adapter's heuristic masses do not yet implement the complete typed critical checklist. Existing answer-specific required-field gates remain separate. Filling essential fields, not adding pages, is the route out of this state.

#### 5.8.2 Typed source calibration

`s` is a versioned `SourceRef`, not merely a domain: it names the relevant artifact origin, publisher key, author/feed/dataset identifier where available, and binding interval. The graph retains all applicable levels and the model declares which one it uses. Syndicated copy does not inherit the origin's history unless provenance establishes the relationship; a changed domain binding does not transfer history; an unknown or reset identity returns to a broad prior rather than escaping a failure record with fresh confidence.

For each source `s` and claim class `k`, the engine maintains a hierarchical posterior over outcomes that were actually observable:

```
theta(s,k) ~ Beta(
  alpha0(k) + eligible_random_passes(s,k),
  beta0(k)  + eligible_random_failures(s,k)
)
```

The class prior shrinks sparse sources toward uncertainty rather than toward either distrust or institutional prestige. No estimate transfers automatically across classes. New sources receive wider intervals, not a low-trust penalty.

Only eligible randomized audits and randomly sampled well-defined event outcomes enter this conjugate Beta-Binomial posterior. Stratified designs declare their target population and aggregation before observation. Risk-triggered audits are valuable evidence but are not silently added as ordinary Bernoulli counts.

If risk-based observations are used to train a broader predictive model, the system logs selection probabilities and applies bounded inverse-propensity weighting or a pre-registered doubly robust estimator, with bootstrap or sandwich uncertainty appropriate to that estimator. This correction is valid only if the propensity is known, relevant cases have non-zero selection probability, and the outcome rubric is stable. Unlogged or publisher-selected successes cannot support a population reliability estimate. When positivity, logging, label-quality, or effective-sample-size diagnostics fail, the affected class falls back to `EVIDENTIAL_ONLY`. Raw bond size, payment, traffic, identity age, domain popularity, and number of unaudited claims never update `theta`. Economic conviction, when eligible, remains a separate likelihood feature and never rewrites source history.

#### 5.8.3 Economic conviction: cash at risk

Cash at risk is evidence of one narrow fact: **the committer accepts an economic loss if the exact declared predicate resolves against its position**. That is stronger evidence of the committer's conviction than a costless statement, but it is not truth by definition. Rich actors can be wrong, manipulate outcomes, insure or hedge the loss, borrow the stake from an interested party, or treat a loss as advertising spend. OpenUncertainty therefore asks an empirical question: *within a comparable claim class, task, horizon, and settlement regime, did this kind of pre-outcome exposure predict the declared target on unseen outcomes?*

Ordinary retrieval/citation payments, domain stake, wallet balance, locked capital unrelated to the claim, and cash posted after material outcome information are ineligible. A `ConvictionSignal` is eligible only when all of the following are recorded before the outcome:

1. it names the exact claim, direction, task, horizon, and versioned Grade-A or event-bound settlement predicate;
2. the amount is actually committed, cannot be withdrawn opportunistically, and has a non-zero maximum loss;
3. predicate coverage states what fraction of the claim the loss actually covers;
4. lock duration and settlement deadline are explicit;
5. known self-funding, rebates, side payments, insurance, and hedges are deducted through funding-independence and unhedged fractions; and
6. retained inputs make the capital outcome reproducible. If settlement becomes ambiguous, the bond is returned and the observation is not labeled a success.

The reference normalization converts headline cash into bounded effective exposure:

```
A = log(1 + min(bond_usd, reference_bond · cap) / reference_bond)
    / log(1 + cap)
T = min(lock_days / reference_lock_days, 1)

effective_exposure = A · max_loss_fraction · T · predicate_coverage
                       · funding_independence · unhedged_fraction
```

All factors are clamped to `[0,1]`. The logarithm and amount cap prevent a billionaire's nominal position from scaling linearly into confidence; the other factors make a fully hedged, refundable, briefly locked, self-funded, or weakly scoped position approach zero. Initial bucket thresholds are implementation hypotheses, not semantic definitions.

The former MVP estimated a marginal cash/no-cash odds difference within claim class/task/horizon. That quantity is an association, **not automatically a Bayes factor conditional on source history and evidence**. Stronger sources or easier claims may select cash; adding their marginal advantage to a source prior can double count competence and selection. This update is retracted from probability use in v2.8.

The proposed replacement freezes a no-cash model `m0(x)` and a joint or conditional model `m1(x,b)` for the same target. `x` contains the same pre-outcome source history, non-financial evidence, declared difficulty, task, horizon, policy and feature versions. If a residual update is used, its reference form is:

```
Delta_cash(x,b) = clip(
  logit m1(x,b) - logit m0(x),
  -lambda_cash, +lambda_cash
)
```

The deployed output would be `logistic(logit m0(x) + Delta_cash(x,b))`, not a residual applied to an unrelated prior. Cross-fitting, conditional comparability/positivity, immutable pre-outcome features, source/event/funder component splits and independent forward validation are required. Unobserved confounding or sparse support can invalidate a slice even after matching/propensity correction. Predictive increment is the activation question; causality is not presumed. Until those gates pass, `Delta_cash = 0`. The effect may eventually be positive, zero, or negative. The reserved `ln(3)` cap bounds future changes to 3× odds; a cap alone does not validate the model. No conditional cash model is implemented or activated in this revision.

Any future cash update must retain its conditioning set, baseline/model versions and lineage; it cannot be added again as independent evidence. Multiple positions controlled or financed by the same beneficial cluster are one correlated exposure, not independent corroboration. Cash never enters Stage-1 retrieval, never creates a global source score, and never upgrades an `EVIDENTIAL_ONLY` claim to `CALIBRATED` without an observable target and sufficient source/outcome data. Measure eligible-query share and incremental utility against using the same budget to acquire better primary evidence; objective extraction guarantees do not by themselves warrant world-truth inference.

#### 5.8.4 Evidence aggregation and dependence

For a calibratable target, the reference aggregation model starts from a task/class prior and adds versioned evidence likelihoods:

```
logit P(Y=1 | G_t) = logit pi(k,p,h) + sum_i w_i · log BF_i

w_i = availability_i · freshness_i · independence_i · calibration_i
```

`independence_i` and dependency uncertainty answer different questions and must not be collapsed. `independence_i` represents **known derivation**: when several articles report registration `BR-04496/2026`, the implementation treats them as one underlying measurement, selects one list-relative representative, and marks the other reports as derivatives rather than additional measurements. Dependency uncertainty represents **residual ignorance** about ancestry that has not been resolved. Resolving a relationship therefore cannot make a known copy independent. The current decision policy combines them as `dependence_risk = 1 - independence · (1 - dependency_uncertainty)` before applying the reliance penalty. This is an evidential policy feature, not a calibrated probability.

`BF_i` is an evidence-type likelihood ratio learned on held-out outcomes, never a hand-written badge weight presented as probability. Exact predicate results have scope-specific likelihoods; a valid signature strongly supports attribution but contributes no direct likelihood that the signed claim is factually correct. The proposed economic-conviction update from §5.8.3 must additionally condition on the same non-financial baseline; its current effect is zero. Grade-B model assessments enter only when their model family, prompt, attack harness, and calibration set are recorded. Without adequate calibration they remain evidential annotations.

Each underlying event may enter the likelihood exactly once. A source's observed track record, a model judgment about the source, and a citation derived from the same audited event are not three independent features. The feature registry records lineage and mutual-exclusion groups, while counterfactual ablations test for accidental double counting. More flexible aggregation models may replace the reference log-odds form only if they improve forward-time calibration and retain inspectable contribution and dependence traces.

The evidence graph links mirrors, quotations, datasets, syndicated articles, common model generations, funding relationships where known, and other derivation paths. Sources that repeat one underlying artifact do not count as independent confirmation. Dependence discounts are estimated by cluster and propagated through the graph; unresolved common ancestry widens the interval and raises ignorance. This is the principal defense against **citogenesis**—a claim appearing well corroborated only because many pages copied the same unsupported origin.

Posterior intervals are produced by sampling over source calibration, evidence likelihoods, dependency ambiguity, freshness, and detected distribution shift. Conflict, missing inputs, correlated witnesses, stale evidence, and out-of-distribution claims widen the interval. The engine must expose which factors moved the estimate and which missing evidence would most reduce uncertainty.

#### 5.8.5 From claims to documents and answers

OpenUncertainty does not create a domain-wide scalar. For query `q` and candidate document `d`, the system first identifies the answer-bearing claims `C(q,d)`. A policy may then compute a local reliance summary over those claims—for example, the minimum 90% posterior bound for a high-stakes answer, or a coverage-weighted posterior mean for exploratory reading. Unrelated history and non-answer-bearing claims cannot raise the result.

Answers preserve the claim graph: each material statement points to supporting and conflicting sources, derivation paths, freshness, and the estimate version. When evidence is insufficient, the system abstains or says what is unknown. A fluent synthesis cannot silently erase uncertainty present in its sources.

#### 5.8.6 Evaluation and falsifiability

The probabilistic mode is accepted only if it beats declared baselines on held-out and forward-time evaluation. Minimum reporting includes Brier score, log loss, expected calibration error, calibration by claim class and source-exposure bucket, selective-risk/coverage curves, interval coverage, and error under distribution shift. Cash experiments additionally compare no-cash, raw amount, effective exposure, source history, self-reported confidence, and evidence-plus-cash ablations; report results by wealth, funding cluster, hedge observability, position size, and source cold-start status; and bootstrap by source/event rather than treating correlated claims as independent. Retrieval evaluation separately reports recall, nDCG, source diversity, original-source recovery, and near-duplicate suppression.

Adversarial suites must include circular citation farms, copied primary sources, stale-but-consistent pages, coordinated Sybils, publisher-selected audits, prompt-injected documents, model-family correlation, new-source cold start, sudden source regime changes, whale positions, self-funded or rebated bonds, hidden hedges, outcome manipulation, and many correlated positions presented as independent conviction. A model that improves average ranking but becomes overconfident on these slices fails the gate.

**Honest status.** The repository contains an **MVP-1 reference implementation**, not a validated production algorithm. It implements the three output modes, Beta-Binomial typed source history, bounded descriptive evidence, a critical-missing safeguard, effective cash-exposure diagnostics, a 90% approximate interval, `EVIDENTIAL_ONLY` fallback, Stage-2 decision reranking, and Brier/log-loss/ECE metrics. V2.8 disables all probability updates from marginal cash buckets. The current numerical MVP estimate uses source history only; non-financial evidence controls fallback, interval widening and decision penalties but does not yet have learned likelihoods. Tests cover ineligible capital, amount saturation, diagnostic positive/zero/negative associations with zero probability authority, duplicate-volume resistance, critical missing despite strong history, sparse/OOD fallback, deterministic scope and dual ranks. The **historical, retired marginal-model** synthetic forward split improved Brier from `0.2175` to `0.1945` and log loss from `0.6275` to `0.5744`; its approximately 6.5 million in-process estimates/second excluded retrieval and evidence acquisition. Those results describe injected synthetic association, not the corrected model or cash's incremental value. The JSONL evaluator rejects temporal leakage, derives histories from training only and reports coverage and source/event/funder cluster-bootstrap intervals; its legacy `calibrated_cash` field now carries source-only estimates with an explicit disabled-model status. The conditional cash model, generalized claim graph, learned non-financial evidence likelihoods, propensity correction, dependency posterior, durable outcome cohort, outcome-integrity process, distributed join and confirmatory result remain unimplemented or unvalidated. Narrow poll lineage and factual claim signatures remain executable. These mechanics **do not show that real cash predicts real reliability**. Phase B starts in an outcome-rich vertical; general-purpose claims remain evidential until validation justifies more. Commands and historical/corrected-model distinctions are in `docs/OPENUNCERTAINTY-MVP1.md`; proposed activation and pilot gates are in `docs/experiments/PILOT-GATES-2026-10-08.md`.

**Live serving status (22 August 2026).** A production-path harness executes real Arctic ONNX embedding, persistent HNSW/BM25 retrieval, credential-free Bing/Brave-public-HTML/DuckDuckGo discovery, robots-aware crawling, distillation, foreground evidence hydration, intent-aware ranking, Stage-2 metadata, citation construction, structured TSE/PesqEle lookup, and abstention. It covers 24 semantic cases with four surface forms each: navigation, transaction, tools, current polls, current office holder, live price, factual relations, due diligence, definition, tax/health guidance, local discovery, and product recommendation. The latest complete empty-index matrix passed 96/96, grew from 0 to 146 documents, and measured 1.63 qps, p50 0.93 s, p95 5.09 s, and p99 5.94 s at concurrency four; foreground hydration p99 was 4.30 s and background-crawl p99 was 2.74 s. The last cold failure was a circular acquisition precondition in broad Brazilian presidential-poll search: scenario completion waited for an extractive answer even though the answer needed scenario completion to acquire two reporting families. Insufficient broad queries now make one bounded two-round pass and schedule enough distinct families for both slots. The public poll slate preserves exactly one Stage-2 representative for the resolved measurement, interleaves citations across emitted scenarios, and pushes same-family derivatives behind independent editorial families; isolated, concurrent-paraphrase, and complete cold runs passed without weakening corroboration or official-lineage requirements.

The first three-pass resident/revalidation run over that exact corpus passed 288/288, grew from 146 to 177 documents, and measured 12.35 qps, p50 3.2 ms, p95 1.24 s, and p99 2.40 s. Foreground hydration p99 was zero and background-crawl p99 was 1.70 s, so the coherent first resident cycle passes all declared quality, latency, crawl, and throughput gates. A subsequent no-warmup concurrency-16 soak passed 960/960, grew from 177 to 210 documents, and measured 26.95 qps, p50 2.9 ms, p95 0.57 s, p99 2.65 s, and background-crawl p99 1.01 s. This is useful single-process concurrency evidence, not a distributed or long-duration scale claim. A preceding diagnostic before temporal background-crawl deduplication passed quality and latency but reached only 8.71 qps while acquiring 64 pages. The flywheel now suppresses opportunistic recrawl of the same retained exact URL for five minutes while still acquiring new URLs; each crawl output batch enters the index under one mutation to reduce reader/writer contention. Thematic news remains a result slate and background acquisition when claim-by-claim synthesis is unsupported, rather than blocking the answer on foreground hydration. Due diligence uses one recent/contextual discovery matrix instead of a duplicate news retry. A 30-second process-local acquisition cache still reuses only successful policy-qualified answers, and factual incomplete-list abstention is reused only with recent multi-family evidence plus a relation-legible public audit trail. Provider failures, empty acquisition, stale evidence, and discovery snippets are never cached as positive claims.

The current path distinguishes entity navigation, direct actions, product comparison, factual/high-stakes answers, and exploratory retrieval. Navigational identity requires a canonical registrable origin. Transactional routing follows bounded first-party-declared action links and stores routes only as non-evidential metadata. Tool queries require functional surfaces and domain diversity. Live prices require a value, asset/unit identity, and a current timestamp; otherwise the system abstains. Local queries disclose that distance ranking is unavailable without user location. Recommendation queries disclose unresolved commercial/editorial independence and reject candidates that match only the task but not the product. Health and tax guidance prefer official authority and remain evidential rather than clinical/legal determinations. Poll answers join `BR-04496/2026`, preserve one underlying measurement and its editorial derivatives, and state that the registry verifies metadata rather than publisher-reported percentages or sampling quality. Pix answers keep a stable relation claim and add a claim-matching Banco Central source. Current office-holder claims form a third structured lineage class. Factual display remains relation-directional, and a language-selected bounded MediaWiki fallback supplies auditable leads without counting as provider redundancy or independent corroboration.

The last coherent keyless fail-closed aggregate passed 16 of 17 search-core gates and failed sustained provider redundancy (1/2). A later credentialed capacity probe passed both Bing and Serper at 30/30 attempts, removing that observed availability blocker for the tested configuration; the aggregate artifact has not been relabelled, and upstream index independence is not established. Four research claims remain unvalidated: comparable market latency, broad market superiority, real-cash predictive value, and live calibrated uncertainty. Generalized lineage passes for poll measurement, factual relation, current office-holder, and market-quote claims. Live estimates remain `EVIDENTIAL_ONLY` where reliance applies; every live `cash_log_bayes_factor` is `0`. Exact latency, provider failures, corrected defects, and reproduction commands are in `docs/experiments/LIVE-PRODUCTION-E2E-2026-08-21.md` and the generalization audit.

**Provider-health status (22 August 2026).** A separate no-mock probe executes three live query classes against raw adapters with explicit provider concurrency. The default keyless waterfall uses Bing as primary, a truthful public-HTML Brave adapter as secondary, and DuckDuckGo as a delayed tertiary hedge. Brave public HTML passed short serial and burst probes, but the sustained ten-round, concurrency-four gate exposed rate limiting: Bing passed 30/30 and Brave public HTML passed 12/30 before HTTP 429, so only one of two required providers was healthy. A real no-Bing chaos matrix using Brave public HTML and DuckDuckGo passed 17/24 canonical cases before both rate-limited; the remaining cases failed closed instead of manufacturing results. Entirely off-topic batches are discarded before blending and crawl. Public HTML is therefore useful bootstrap capacity, not a production SLA or independent sustained redundancy.

**Credentialed update (23 August 2026).** With the production Serper credential configured, the same sustained three-query-class, ten-round, concurrency-four probe passed Bing 30/30 and Serper 30/30. Bing measured p50 293 ms (max 450 ms); Serper measured p50 1.03 s (max 3.61 s). This is real two-adapter availability evidence, not proof that the underlying indexes, sources, or failure domains are independent.

The live harness consumes the same `SERPER_API_KEY` and `BRAVE_API_KEY` environment configuration as the production CLI. Neither credential was present in the earlier 96/288/960-request corpus and its 16/17 readiness aggregate, so those artifacts remain explicitly keyless. The later credentialed provider and generalization captures are separate frozen artifacts; they remove the observed two-adapter availability failure for that configuration but do not retroactively make the providers independent or the earlier aggregate a market-quality result.

**Dated market development regression (22 August 2026).** A versioned signed-out Google `pt-BR`/Brazil snapshot captures the top five for nine canonical queries spanning navigation, transaction, tool use, typo resolution, due diligence, current polls, factual origin, founder completeness, and generic news. A reproducible comparator applies the same declared URL/surface/diversity contracts to Google's slate and the no-mock OpenSonarX report. On the final coherent retained corpus, Google scored 74.1% and OpenSonarX 100.0%, with four OpenSonarX wins, five ties, and no losses. These cases repeatedly drove implementation changes and are therefore a **seen regression suite**, not confirmatory evidence. Pix places the claim-matching Banco Central page first, while incomplete founder enumeration still abstains. A companion three-round latency audit captured Google browser wall time to the first visible organic result (p50/p95/p99 `215/407/414 ms`) and OpenSonarX backend `search_ms` (`1.96/31.02/2400.04 ms`). It deliberately keeps `market_latency_compared=false`: the surfaces and execution protocols differ, only eight of nine query strings are identical, and the later exact tool-query retry was blocked by Google's unusual-traffic interstitial. The candidate fixture omits Google AI Overview, knowledge panels, maps, shopping, video, and ads; the sample is small and policy-authored, human judgments were not blinded, and personalization was controlled only by signed-out locale/country parameters. The machine reports therefore keep `market_superiority_proven=false` and `cash_thesis_tested=false`.

**Independent generalization audit (22–23 August 2026).** Query-only manifests were sealed before capture, reject fewer than 40 cases, normalized duplicates, every canonical/variant development query, prior-holdout query/ID reuse, prior-manifest hash drift, and the mock embedder; reports hash both manifest and release binary. The first 48-query external MS MARCO answer-seeking sample returned 40/48 non-empty OpenSonarX slates while Google showed 48/48; a current/high-impact split returned 21/40; and a disjoint keyless run fell to 13/48 under real rate limiting. Each inspected diagnostic was then burned. Credentialed iterative holdouts exposed and corrected category-level failures in provider sanity, intent routing, topic-specific authority registration, general Stage-2 applicability, and relevance/trust interaction. A retained but off-topic page once outranked a materially more relevant source because lower ignorance overwhelmed relevance; Stage 2 is now restricted to reranking within a top-relevance band and reports both ranks plus the guard signal. The later evaluator captures a raw market engine against the identical manifest, pools and canonicalizes both top-five slates, randomizes candidates without system identity, requires a complete judgment set bound to exact artifact hashes, and computes pooled-IDCG nDCG plus deterministic paired-bootstrap intervals. The claim gate is structurally unavailable below 1,000 queries, eight strata, and two independent external system-blind judges.

Under one system-blind local-model diagnostic using the same rubric and prompt protocol, v10/v11 still favored raw Serper: acceptable top-one gaps were −12.5 and −6.3 percentage points, relevance nDCG gaps −0.099 and −0.086, and query-relative-authority nDCG gaps −0.150 and −0.118. This exposed a real cold-source tie: most external candidates had identical evidential support/ignorance. Stage 2 now uses a query-relative source role—primary, edited reference, unclassified, or unverified UGC—only inside the existing relevance band and never as a probability. Stage 1 also preserves bounded top-k depth per configured discovery provider before the final top-k decision, instead of discarding roughly half of each provider's slate during interleaving. Explanatory `A vs B` queries no longer become live events, and short navigation is exempt from reliance only after a canonical host or public-authority path verifies the destination.

The further-disjoint v12/v13 runs both returned 48/48 non-empty slates, as did raw Serper, and repeated statistical parity. V12 measured acceptable top-one 70.8% versus 72.9%, relevance nDCG 0.841 versus 0.833, authority nDCG 0.728 versus 0.735, and zero severe top-one errors for both systems. V13 measured 66.7% versus 66.7%, 0.870 versus 0.869, 0.716 versus 0.740, and again zero severe errors. Every 95% interval included zero. Within OpenSonarX, Stage 2 raised acceptable top-one count from 24 to 34 in v12 and from 21 to 32 in v13; the v13 paired decomposition improved nine retrieval choices and harmed two. This is credible evidence that the current two-stage mechanism helps its own retrieval order and has become competitive with raw Serper in one English answer-seeking slice. It is not evidence of general superiority: each run has 48 queries in one stratum, the judge saw only URL/title/snippet, the judge was not independent of implementation, raw Serper is not the complete Google product surface, heterogeneous languages/intents and human judgments remain absent, and source history/lineage are still sparse. Every `p_reliable` remained null and every cash contribution remained zero. The full anti-overfit protocol and results are in `docs/experiments/GENERALIZATION-HOLDOUT-2026-08-22.md`.

### 5.9 Two-Stage Search: Relevance First, Risk-Aware Decisions

The architecture separates candidate retrieval from reliance-aware presentation.

**Stage 1 — broad retrieval.** Base relevance is stake- and trust-blind:

```
R(q,d) = w_s · normalized_vector_similarity(q,d)
       + w_b · normalized_BM25(q,d)
       + stake_blind_retrieval_features(q,d)
```

No bond, accountability component, policy label, payment amount, identity age, staking-registry field, or uncertainty output may prune or score the broad candidate generation path. This preserves recall, makes new sources discoverable, and prevents an incumbent trust graph from deciding what can be seen.

**Stage 2 — explicit decision policy.** After evidence is joined for the top candidate set, a client may produce a task-specific order:

```
D_p(q,d) = g_p(R, lower_bound_reliance, ignorance,
               conflict, freshness, source_diversity, licensed_cost)
```

For illustration, a conservative factual policy might use `D = R · lower_bound_reliance - lambda·ignorance - mu·conflict - nu·cost`; an exploratory policy might preserve high-uncertainty candidates with an exposure floor. `lower_bound_reliance` is available only in `CALIBRATED` mode. An evidential-only policy may act on support, conflict, and ignorance but cannot manufacture a probability, while a deterministic result affects only the predicate scope it actually resolves. This formula is not a protocol default. Each policy is versioned, inspectable, and evaluated against its intended loss function. Interfaces return both `retrieval_rank` and `decision_rank`, the applied policy, and the reasons for any movement. In the executable search policy, trust may reorder only candidates inside a declared top-relevance band; it cannot rescue a materially off-topic page merely because that page is retained or easier to audit.

This is the load-bearing distinction: **capital never buys relevance; evidence—including validated economic conviction—may inform reliance only through calibrated uncertainty.** There is no universal result order appropriate for every use. A medical evidence search, breaking-news monitor, shopping comparison, and open-ended web exploration have different error costs.

Pinned tests currently show that stake amount, reputation, and DNS verification do not enter `compute_score`; a focused non-zero `HybridIndex` regression also returns identical default-query order and scores with and without a registry. The former DNS-verification multiplier (`dns_mult`, 1.05×/0.95×) has been removed from the scoring path and the signal remains only as provenance metadata. A representative-corpus registry ablation and dependency-boundary audit are still Phase A gates before the repository can claim full Stage-1 invariant compliance. The legacy explicit `min_trust` compatibility filter remains a post-ranking caller option; it is not OpenUncertainty and must not be presented as calibrated reliance.

The narrow live path now makes the policy distinction operational. Navigational queries resolve and cache a likely first-party root but never call that routing signal truth. Transactional queries share canonical provider flights, reject malformed action-looking paths, preserve a conservative first-party root, and prefer an action route only when the matching root explicitly links it or a conventional route passes the bounded verifier. Current-event discovery may crawl an undated lead, but final serving requires a verifiable effective date inside the query window. Factual discovery may admit an entity page for hydration, but final extraction requires the entity and requested relation to be linked in the answer-bearing sentence or a bounded grammatical context; a government URL counts as primary only when it identifies the authority named by the claim. Due-diligence and thematic-news queries abstain where their required structured joins or event graph are absent. These are deterministic safety/coverage policies around an evidential MVP, not learned universal trust weights.

v1 whitepaper §12.2 (stake boost), §12.3 (reputation factor), and §12.6 (DNS multiplier) are retracted.

---

## 6. The Publisher Protocol

Low-friction adoption in four levels. The levels separate passive observation, identity control, content-use permission, and financial commitments; no one signal silently stands in for the others.

**Level 0 — Observed (publisher takes no protocol action).**
Subject to the crawler's lawful authority and configured policies, nodes may record fetch attestations, hashes, change history, metadata, and links. The Phase A target is enforceable handling of `robots.txt`, `noindex`, applicable TDM reservations, and deployment-specific allow/deny rules; the current repository has robots handling but does not yet implement that full policy surface. Level 0 does not create a license, a payable publisher balance, or permission to redistribute a canonical distillate. Any snippet or extracted-span serving must be enabled only under a documented jurisdictional policy or separate license (§12).

**Level 1 — Identity-bound (DNS control plus a signed identity manifest).**
`oi-verify=<identity>` is evidence that the identity currently controls DNS for the domain, subject to §5.1's binding-change rules. It enables a domain-binding label and publisher-signed anchored priority. DNS control is **not** evidence of legal identity, copyright ownership, authority over user-generated content, or consent to copy, train on, or serve content.

**Level 2 — Licensed and committed (signed manifest plus authorized keys).**
`/.well-known/opensonarx.json` declares OpenSonarX identity, operational keys, commitment scopes, and references to standard rights and provenance records. It does not invent a second licensing language:

```json
{
  "version": 2,
  "identity": "ed25519:...",
  "operational_keys": [ { "key": "ed25519:...", "bond_ceiling": "...", "scope": "newsdesk" } ],
  "default_commitments": ["verbatim_quotes", "exact_mirror_freshness:P7D"],
  "feeds": ["https://example.com/rss"],
  "license_ref": "https://example.com/rsl.xml",
  "license_standard": "RSL-1.0",
  "content_credentials": "c2pa:embedded-or-sidecar",
  "peac_wire": "interaction-record+jwt/0.2",
  "receipt_profile": "https://opensonarx.org/profiles/peac-receipts/v1",
  "evidence": [
    { "claim_ref": "...", "commitment": "H(evidence||salt)", "size_class": "M",
      "reveal_policy": "on_challenge" }
  ]
}
```

The manifest is signed by the identity key and must include a warranty that the signer has authority to bind the declared resources. Machine-readable permissions use the [Really Simple Licensing (RSL) 1.0 specification](https://rslstandard.org/rsl), including its URL-pattern, usage, attribution, payment, and license-server mechanisms, rather than a proprietary OpenSonarX license object. Deployments may additionally enforce negotiated contracts or jurisdictional requirements that RSL alone cannot settle. Permissions are resource-, use-, and time-scoped, revocable prospectively, and exclude user-generated or third-party material unless separately authorized.

Where media already carries [C2PA Content Credentials](https://spec.c2pa.org/specifications/specifications/2.4/specs/C2PA_Specification.html), OpenSonarX imports the manifest, ingredient relationships, signatures, and validation status into the evidence graph; it does not replace them. Invalid or absent C2PA provenance is not itself proof that content is false. Commitment and bond authorization remains separately scoped by operational key and ceiling. A CMS plugin can generate the records, but the publisher must make the legal and financial choices.

**Level 3 — Full participant.**
Runs a witness or monitor, posts supported commitment classes, and may sponsor newcomers within the type-scoped rules of §8.

**The adoption funnel.** Level 0 supplies observation and discovery, not retroactive accounts payable. Earnings begin only after an identified payee accepts the payment terms and the relevant use is licensed. The primary low-friction incentive is therefore control: publishers can attach their own signed priority records, specify permitted machine uses, receive usage receipts, and collect payment from the effective date forward.

### 6.1 Publisher-selected policies

Policy is scoped by resource, use, version, and effective interval. The proposed deployment offers the following product choices through standard licenses or explicit contracts, rather than inventing a competing rights vocabulary:

| Publisher choice | Consumer experience | Boundary |
|---|---|---|
| **Free distribution** | Authorized clients access permitted content without a content fee. | Hosting, indexing, and verification still require funding. Free content does not imply a free search service. |
| **Attribution-based distribution** | The answer visibly credits the source and preserves a functional reference under the applicable terms. | A citation does not promise traffic, commercial leads, or truth. Attribution compliance needs testing. |
| **Paid licensed access or use** | The client accepts the declared price and authorized-use scope before delivery or a separately contracted payable event. | The fee purchases service or permission, never relevance or a confidence increment. |
| **Separate training license** | A buyer obtains an authorized, versioned dataset delivery under distinct terms. | Consultation, indexing, and response use do not automatically permit training, retention, or redistribution. |

An authenticated zero-price license may be appropriate for a creator who wants attribution and usage records. It is not equivalent to publishing unrestricted plaintext. Policies must state whether citation, caching, transformations, answer excerpts, redistribution, and bulk export are allowed; permission is not inferred from silence or from access to a URL. An agent's declared purpose is an accountable request, not cryptographic proof of its later behavior. Policy changes apply prospectively according to the applicable terms, with cache revalidation and revocation rules rather than a promise to erase delivered copies.

### 6.2 Discovery, authorization, and protected delivery

The default for a protected resource is a **publisher-approved catalog entry**: useful metadata, availability, terms, and an authorization endpoint, with only an approved preview. A searcher can learn that the resource exists before deciding whether to obtain it. Fully private collections instead restrict discovery itself. Neither mode promises that another party cannot independently publish or discover the same information.

An unpaid client may receive the authorized catalog entry while receiving no restricted content. Restricted snippets, extracted claims, embeddings, cached answers, and query-match metadata must not become back doors around the delivery policy. Private indexing belongs inside an authorized security domain. Public retrieval can score authorized metadata without receiving the protected body. Access restrictions are rights and security boundaries, not financial admission criteria to the broad relevance index (§5.9).

Protected delivery may use authenticated resource endpoints or licensed decryption keys, including interoperable RSL extensions where appropriate. Operators must validate publisher authority, client authorization, budget, price, license scope, and retention before serving. Access failures and payment refusal are reported separately from insufficient factual evidence. A cheaper accessible source may be selected under a disclosed client budget, but the answer must expose material inaccessible evidence and avoid claiming complete coverage.

Public plaintext elsewhere, a compromised node, an authorized client's redistribution, or independently reported information can bypass protocol economics. Technical controls reduce unauthorized access at the controlled endpoint; contractual obligations, monitoring, and remedies govern later use where applicable. The protocol does not sell universal copy prevention. This policy gateway, protected index boundary, and licensed-delivery path are **specified, not implemented as a production service**.

---

## 7. Web-Scale Search and Discovery

Google-class scope requires more than a trust layer. It requires crawling, indexing, retrieval quality, latency, freshness, abuse handling, and unit economics at orders of magnitude beyond the current repository. OpenSonarX claims an architecture with that potential, not present parity. The path to scale is a latency-bounded pipeline:

1. **Rights-aware ingestion.** Sitemaps, feeds, query misses, links, publisher submissions, and scheduled recrawls enter partitioned crawl queues subject to access policy, origin budgets, change prediction, and abuse controls.
2. **Canonical processing.** CDS distillation, language and safety metadata, claim extraction, near-duplicate clustering, citation parsing, and derivation detection create documents plus a typed claim/evidence graph. AI-generated content is not categorically penalized; unsupported derivation, circular citation, and concealed source dependence are exposed.
3. **Distributed indexing.** Lexical, vector, entity, freshness, and link indexes are sharded independently of the economic registry. Index replication and serving can scale horizontally without running uncertainty inference across the whole corpus at query time.
4. **Broad candidate retrieval.** Multiple stake-blind retrievers and a learned relevance reranker produce a high-recall candidate set. Low-history and high-uncertainty sources are eligible here by construction (§5.9).
5. **Evidence join.** For only the top `K` candidates, the service loads versioned claim graphs, cached source calibration, provenance, conflicts, freshness, and license terms.
6. **Uncertainty inference.** OpenUncertainty evaluates the answer-bearing claims within a strict latency budget. A missing or expired cache returns `unknown` or widens the interval; it never silently reuses stale certainty.
7. **Policy decision and answer construction.** The client policy reranks, diversifies, cites, warns, or abstains. Results expose both relevance and decision order, and answers retain claim-level citations and counterevidence.

The current single-node implementation exercises this sequence on query misses. Equivalent natural formulations are rewritten into a small, neutral intent-family discovery set so concurrent users share provider cache flights without hard-coding a source, candidate, or expected answer. Provider calls are bounded by deadlines and circuit breakers. Selected URLs are deduplicated by source family, and a per-URL crawl single-flight lets one leader fetch/distill/index while followers wait for and reread the shared index. Discovery and serving are deliberately separate: a web snippet is a crawl lead, not retained evidence; claim-bearing answers are reconstructed only after successful foreground hydration and a fresh retrieval pass. Background crawl then grows the persistent corpus for later requests. This is an operational seed of the pipeline above, not a distributed crawler or proof of web-scale freshness.

Query-independent evidence features are precomputed and content-addressed by `evidence_root`, model version, and calibration set. New evidence, corrections, dependency changes, source-regime alarms, and freshness thresholds invalidate the affected claims. Query-specific inference runs only on the top candidate set. This keeps the expensive epistemic work bounded while preserving broad recall.

General search also requires **intent routing**. Factual, current, due-diligence, and informational/exploratory slates now invoke reliance assessment because ordering sources is itself a reliance decision; high-impact guidance uses a stronger authority policy. Navigational, transactional, and tool queries keep epistemic uncertainty `not_applicable` unless they make a separate claim: routing to a site or action is not proof that its contents are true. OpenUncertainty does not pretend that “best pizza,” a home page, or a scientific claim has the same target. Personalization, localization, malware, illegal content, adult safety, and conventional spam defenses remain separate serving features and gates with their own disclosure and appeal requirements. They cannot be mislabeled as epistemic certainty.

Nor does uncertainty replace relevance research. Query understanding, multilingual retrieval, link and entity analysis, freshness prediction, multimedia indexes, learned reranking, accessibility, and tail-latency engineering remain necessary to reach general-purpose quality. The proposed advantage is orthogonal: once relevant candidates exist, the system can distinguish evidence strength, dependency, conflict, and ignorance more honestly than a relevance-only rank.

The result slate should deliberately recover **primary artifacts and independent viewpoints**, not ten surface variations of the same source. Near-duplicate clustering collapses copies for display; the dependency graph discounts detected syndication; a diversity constraint preserves material counterevidence. Derivation detection is probabilistic and evadable—heavy paraphrase, translation, private model context, and omitted citations can hide common ancestry—so the system cannot promise to eliminate slop or prove source independence. Undetected lineage is represented as model risk and tested adversarially. These are decision features, not a right for the system to hide lawful disagreement.

v1's flywheel — query misses trigger crawl, distillation, and indexing, so some misses make later queries more local — remains part of discovery. Demand is a useful, cost-bearing scheduling signal, but it is neither Sybil-resistant nor sufficient: an attacker can fund queries, and a new source has no demand history. Discovery therefore uses explicit capacity allocation rather than a claim of unconditional coverage:

- **Baseline capacity reservation.** A fixed share of each crawl epoch is reserved for unbonded candidates selected under a published queue policy. The system publishes queue age, drop reasons, per-origin rate limits, and service objectives. This is a non-displacement guarantee for capacity, not a promise that every submitted URL will be fetched.
- **Demand lane.** Query misses and paid customer coverage requests schedule candidates subject to rate limits and abuse controls. Correlated request clusters are discounted.
- **Exploration lane.** A fixed share is sampled from new origins and low-exposure candidates. Demand and accountability statistics are normalized by eligible impressions or crawl opportunities so incumbents do not receive credit merely because they were already exposed.
- **Publisher-funded ingestion (scheduling only, capped).** A publisher or customer may pay the operational cost of a faster initial crawl or recrawl within a separately capped lane. The result is labeled as sponsored ingestion; payment cannot displace baseline/exploration capacity, guarantee inclusion, alter Stage-1 score, narrow uncertainty, or create accountability history. Per-origin and correlated-cluster caps limit queue capture.
- **Obligation lane.** Exact-mirror commitments and event predicates due for a scheduled check, using the pinned artifact, request profile, and retention rules of §5.3–§5.6.
- **Typed citation graph.** A citation can guide discovery and expose provenance relationships. Its weight is scoped to the relevant commitment class and is never treated as truth, independence, or a ranking boost purchased through stake.

Scale and quality are falsifiable. Every release reports indexed-document and unique-origin counts, crawl freshness, ingestion throughput, index cost, p50/p95/p99 query latency, evidence-join and inference latency, cost per query, candidate recall, nDCG, original-source recovery, near-duplicate suppression, result diversity, claim-evidence coverage, calibration error, and selective risk. “Google-level” remains a long-term performance target until externally reproducible benchmarks demonstrate comparable coverage and retrieval quality.

### 7.1 Open contextual evidence units

The proposed exchange unit is a **contextual claim with evidence**, not a certified fact. Pages remain valuable original artifacts. The protocol adds addressable representations so a client can retrieve the relevant portion without silently discarding qualifiers or source context. One unit may have several interpretations and evidence edges; extraction does not make those edges independent observations.

The following is a proposed semantic envelope, not a shipped schema or a change to the canonical CDS hash:

```
EvidenceUnit {
  schema_version, unit_id,
  assertion, language, entities,
  scope: { place?, population?, method?, valid_interval?, qualifiers },
  original_refs: [{ artifact_hash, representation_version, locator }],
  source_refs, published_at?, observed_at,
  transformations: [{ tool, version, input_refs, output_ref, limitations }],
  relations: [{ type, target_ref, detection_method, uncertainty }],
  evidence_state_ref?, uncertainty_estimate_ref?,
  correction_refs, supersedes_refs,
  rights_ref, policy_version, permitted_retention_ref
}
```

Relations distinguish extraction, quotation, syndication, translation, synthetic derivation, support, and contradiction. Known common origin discounts corroboration; unknown common ancestry remains uncertainty. Source-provided declarations and inferred lineage remain separately typed. Signatures establish a key's statement about an artifact, not authorship or world truth. Corrections invalidate affected answer caches and add a visible new version rather than rewriting historical evidence silently.

**Illustrative poll unit:** "Candidate A has 32%" is incomplete. A useful unit binds that hypothetical number to election, location, question/scenario, field dates, sampled population, methodology, sample size, margin of error, institute, and the exact cited snapshot. A TSE registration is a separate metadata record; it does not validate the percentage. Twenty reports of the same survey remain one underlying measurement. No illustrative value in this example is a new live result.

Canonical retained bytes and deterministic CDS representations stay distinct from probabilistic enrichment. Model-generated assertions, OCR, summaries, and semantic links record their input and transformation version; they never replace the canonical source hash or create Grade-A capital authority merely because a structured field exists. Retention and serving remain subject to authorization, rights, and deletion policy. A unit pointing to unavailable evidence exposes that gap.

### 7.2 Multimodal adapters and reuse economics

Adapters may ingest text, structured datasets, images, audio, and video where permitted. Text locators identify spans or pages; image locators identify a region in a versioned image; audio/video locators identify time ranges and tracks. Transcripts, OCR, captions, frame selections, and extracted assertions are derived representations with separate error modes. A video claim about execution must not lose a negation, speaker attribution, or a qualifier in another interval.

The exchange format remains open and exportable. An operator may use compact binary storage, columnar data, or modality-specific indexes internally, but benchmark and conformance tests must establish that optimization preserves relevant context and rights references. Clients can consume units through an API or SDK, with optional agent-tool integrations. Broad multimodal coverage and a portable evidence-unit implementation remain roadmap work, not established capabilities of the current search path.

Preprocessing and cached, versioned evidence can amortize work across authorized queries. The hypothesis is lower repeated extraction and smaller relevant payloads without worse answers. The countercosts are ingestion, extraction errors, storage, revalidation, authorization, corrections, and auditing. Measure cold and warm latency, p95/p99, full lifecycle cost, cache validity, tokens delivered, and factual support against both full-document and ordinary chunk-based retrieval. A shorter representation alone is not proof of lower total cost or greater reliability.

### 7.3 Separately licensed training exports

Training is an optional downstream use, not a prerequisite for search or a reason to centralize all publisher content. An export names a fixed snapshot, permitted uses, authorized rights sources, transformations, origin/dependency groups, exclusions, and corrections known at export time. Original evidence is retained or referenced only where licensed. Query histories, protected source identities, or private graphs are not exported merely because the service possesses them. Permission checks apply throughout a derived item's rights chain, and unsupported rights block that item's delivery.

The export should distinguish observations from extracted/model-generated interpretations, preserve less common and conflicting information, and avoid weighting many derivatives as many independent originals. Missing origin declarations remain unknown. Provider assertions of "human" or "original" require verification where material; the protocol cannot reliably detect every undisclosed synthetic or copied item.

A training-quality claim requires its own pre-registered experiment: comparable models, budgets, token counts, original-data retention, and selection baselines; independent test sets isolated by time and derivation family; factual-error, diversity, rare-case, contamination, and task-performance measurements across multiple generations where feasible. Compare raw permitted data, ordinary deduplication, and provenance-aware selection. A successful retrieval experiment does not establish a training benefit, and a training benefit does not prove that hallucinations or model collapse have been eliminated. This export and its evaluation are **proposed and unvalidated**.

---

## 8. Cold Start and the Underwriting Market

A useful accountability system must avoid turning history into an admission gate. Four properties make v2 navigable from zero:

**1. The unbonded lane is open from day one** (P5, §7). A new publisher may enter the same baseline and exploration queues as anyone else. Once indexed, its content competes in broad retrieval on relevance, not capital or history. Payment begins when identity, license, and payee requirements are satisfied; no universal indexing time or superiority to existing search is promised.

**2. Records begin with narrow, testable commitments.** New identities can accumulate audited results for quotes, data snapshots, or exact freshness without waiting for an adversarial challenger. Time-to-evidence depends on audit capacity and actual eligible commitments. The protocol publishes the expected audit latency and never promises a universal "trusted in weeks" label.

**3. Denominators and uncertainty stay visible.** Clients see how much and what kind of work was checked. A thousand trivial quote checks remain a thousand quote checks; they do not convert into authority on data, freshness, or reporting. A source with little history receives a broad interval and an exploration opportunity, not a permanent low-trust score. Audit allocation can use risk and licensed exposure, but the calibration model must correct for the resulting selection bias (§5.8).

**4. Underwriting: type-scoped sponsorship.** An identity may co-bond a newcomer's Grade-A commitments in a named class:

- The sponsor may earn a capped, time-bounded share of licensed revenue under an explicit contract.
- The co-bond can be slashed only by the same deterministic predicate that governs the underlying commitment. Sponsoring open reporting cannot put capital at risk for truth.
- Sponsorship does not transfer the sponsor's pass history or generate a consensus label. Clients may choose to display it as a separate signal.
- Positions, beneficial-owner disclosures, and correlated funding paths are public where legally permissible. Correlation controls limit obvious self-sponsorship, but do not prove independence against patient Sybils.

This market can fund audits and discovery for newcomers, but sponsorship does not narrow an uncertainty interval by itself. Only eligible evidence and later observable outcomes can do that. The market does not solve cold start by decree: usefulness, abuse resistance, and sponsor concentration must be measured in pilots. Its lending, securities, custody, tax, and consumer-protection characterization is a deployment gate (§12.4).

---

## 9. Economics

### 9.1 The usage ledger is the core loop

Agents or other customers may pay for useful retrieval, authorized content use, or licensed delivery. A protocol-compatible creator can also choose free or attribution-based distribution (§6.1). The proposed receipt schema distinguishes what happened:

- a **served-result receipt** says a result appeared in a paid response;
- a **citation receipt** says an answer referred to a specific content or claim identifier;
- a **licensed-delivery receipt** says licensed bytes or a distillate were delivered; and
- a **settlement receipt** records how the fee was allocated.

A licensed training export needs a distinct **dataset-delivery receipt** binding the authorized buyer, snapshot, scope, terms, and delivered items. It proves the recorded delivery and declarations, not that training actually occurred or complied thereafter. Zero-price authorization and attribution events may generate nonpayable records, but must never be represented as paid demand or automatically mint a reward. A displayed citation alone creates no debt outside accepted terms.

These events measure exposure or use, not truth or usefulness. A payout requires an identified payee, accepted terms, and a license covering the recorded use. Receipt aggregation must minimize query leakage: raw queries and per-user histories stay off public logs; deployments use batched settlement, minimum cohort sizes, short retention, and auditable aggregate proofs where practical.

Every payable receipt binds the content or claim identifier, license identifier, event type, price, channel, and unique nonce to payer authorization and a service signature. Settlement rejects replayed or conflicting nonces. The wire representation should be implemented as a versioned [PEAC](https://www.peacprotocol.org/)-compatible interaction receipt/profile, with OpenSonarX fields carried as declared extensions, rather than as another incompatible receipt protocol. PEAC interoperability does not make the event truthful or the payment lawful; OpenSonarX still validates payer authority, RSL or contractual rights, replay protection, privacy, and settlement rules. Where payer unlinkability is required, the design must use blinded authorization or another reviewed privacy mechanism rather than omitting payer consent.

**Attribution and escrow.** Only a receipt that explicitly cites an `event_bound_assertion` identifier creates assertion-specific escrow. Its payer-authorized contract follows §5.6's `CRITERION_MET`, `CRITERION_NOT_MET`, or `EXPIRED_UNRESOLVED` transitions; unresolved funds return to the payer rather than burn. A page-level or open-reporting receipt is ordinary licensed content revenue and is not retroactively burned, redistributed, or treated as evidence of accuracy. Anchored priority affects payment only where the payer's license and product terms explicitly define a derivative-use rule.

**Current implementation status.** The repository contains a creator-ledger prototype that records locally served results and assigns fixed credits, plus partial fee plumbing. It is not yet a paid, privacy-preserving, multi-party usage ledger and it does not implement the receipt taxonomy or assertion escrow above. Existing state-channel code is a starting point, but the PEAC profile/integration, multi-party allocations, refunds, replay protection, dispute evidence, and privacy-preserving aggregation require new implementation and tests.

**Initial serving profile P0 — specified, not implemented.** Charge content fees for authorized licensed delivery, not citation count or inferred causal usefulness. Before delivery, the payer accepts resource/version, payee allocation, price, scope and retention/cache-reuse window. Cache reuse within that window has no new content charge unless the accepted license expressly defines per-use pricing. Service fees and connector compensation remain separate; free/attribution receipts are nonpayable and never mint rewards. Multiple sources receive only the explicitly accepted delivery fees. Training always needs separate permission. Verify publisher authority and rights documentation; signatures, firstness and extraction do not establish ownership. Conflicting rights block delivery pending review. Reject replayed nonces and refund failed delivery. Full-source royalty attribution is deferred. P0 does not enforce all behavior after lawful delivery.

#### 9.1.1 Creator return without a page visit

A payable event can remunerate an authorized source even when the user reads only the generated answer. The payer may be a person, an agent provider, or an enterprise buyer. A deployment may bundle retrieval and licensed-use costs, or obtain separate authorization for a premium source. It publishes the event and allocation rule before charging, including when a citation, delivery, or reuse is payable. Node service fees, content payments, connector work, and verification fees are distinct cost items, with no assumed universal revenue split.

Attribution-based sources instead receive the agreed visible credit and reference. That can support recognition or discovery, but neither future traffic nor revenue is guaranteed. Original-source payment requires valid rights and agreed derivative-use terms; first crawl, earliest anchor, or extraction of a claim is not sufficient. A connector cannot acquire the publisher's rights simply by transforming its media. Higher prices, creator revenue, paid popularity, and fees paid to verifiers cannot improve evidence strength by themselves.

The economic hypothesis is that useful authorized consumption can help sustain original data, reporting, documentation, and other work that improves the evidence corpus. It requires buyer demand and acceptable creator return after all operating costs. The protocol cannot collect from Google or an unrelated model operator that has not contracted to use it. It does not promise to replace advertising income or universal creator compensation.

### 9.2 Coalition-safe settlement

Self-dealing must be evaluated at the attacker-controlled coalition, not one nominal creator account. For any action an attacker can manufacture, the required condition is:

```
attacker_fee + irreversible_external_cost
  > creator_payout + curator_payout + sponsor_payout
    + verifier_payout + witness_payout + recoverable_subsidy
```

The right side includes every role the same beneficial owner or correlated cluster can recover, including emissions. The present on-chain 85% payee / 5% sentinel / 10% burn split gives a 10% nominal loss to a coalition that pays the entire fee and recovers both listed payouts. That can satisfy the direct-fee inequality **only if no curator, sponsor, verifier, witness, emission, rebate, or downstream subsidy closes the margin**. The current implementation does not model that complete coalition. Phase B must redesign or validate settlement using payer-funded fees, delayed rewards, exposure caps, subsidy eligibility, and correlation discounts, then test the inequality under the cheapest modeled strategy.

Paid exposure is still gameable for strategic visibility. Payer diversity, account age, funding-path correlation, rate limits, and exposure normalization can reduce manipulation, but none makes demand Sybil-proof. Demand remains a separate field in §5.7 and cannot purchase a fidelity or reporting label.

### 9.3 What carries over from v1

| v1 mechanism | v2 role |
|---|---|
| Dispute bonds and graduated slashing | Reused only for versioned Grade-A predicates; payout rules require new coalition analysis |
| Commit-reveal jury and quorum | Not a capital-settlement layer in v2; may remain as non-binding governance input |
| Trusted-reporter bond discount | Optional, class-scoped challenge discount based only on relevant Grade-A history |
| Emission schedule (decaying, ~3y) | Not carried unchanged; any subsidy must satisfy §9.2 for the full attacker coalition |
| Curator caps and UGC passthrough = 0 | Defaults to preserve, subject to license and settlement redesign |
| Governance timelocks, deposits, and parameter floors | Reused where compatible; new commitment versions, client policies, and log governance require separate review |
| State channels and settlement fees | Engineering substrate only; the multi-party receipt and refund protocol is new work (§9.1) |

### 9.4 Token sequencing: predicate before economy

Bitcoin shipped a working verification predicate before its token was worth anything. v2 commits to the same ordering:

1. **No token is needed for Phase A** (witnessing + anchoring is cryptography plus an operator registry), and Phases B–C run on fiat-denominated bonds and payments (§14) — so each phase's economics exist *within* that phase.
2. A native token is optional. Phase D proceeds only if measurements show that it improves decentralization or security beyond fiat or stable-value payment rails, and only when there is organic demand. If it does not, the protocol remains tokenless.
3. External adversarial audit of any settlement program, plus the open items named in §10, precedes custody of material value. The v1 "before any token has value" checklist is an input to that review, not an automatic carry-over approval.

### 9.5 Useful participation before decentralization or token rewards

The first buyer should receive value from an operator-run service with replaceable discovery and storage suppliers. The team owns the evidence/uncertainty and decision interfaces; it need not first recruit a global publisher, verifier, or token network. Sources can contribute open data, and a bounded licensed pilot can begin with a few contracted publishers. Original-source coverage, workload reuse, provider portability, and customer retention are measured advantages, not presumed network effects.

| Participant | Contribution and proposed return | Economic test |
|---|---|---|
| Person or agent buyer | Pays for useful service within an authorized budget. | Better supported decisions and acceptable full cost/latency. |
| Publisher or dataset provider | Chooses free access, attribution, or contracted payment. | Useful exposure or net revenue without surrendering undeclared rights. |
| Retrieval, storage, and gateway operator | Provides crawling, indexing, retention, and authorized serving for service fees. | Verifiable useful work, sustainable costs, and a substitutable provider. |
| Adapter or integration developer | Provides reusable tooling under a service contract or accepted grant. | Delivery and maintenance outcomes, not self-generated usage. |
| Auditor or claim guarantor | Performs scoped checks or assumes an eligible loss for a separate fee/premium. | Investigation, capital, and failure costs fit the declared budget. |

Optional native-token grants or governance do not fund the system by assumption. Compare them with stable-value grants and nontransferable participation rights. Growth in use does not guarantee token appreciation. If blockchain improves multi-party escrow or settlement, batch it without exposing raw queries or forcing a wallet into ordinary search. It does not establish content truth, lawful ownership, purpose compliance, or model quality. Centralized arbitration remains centralized even if its payout runs on-chain.

---

### 9.6 Challenge incentives and full-cost pricing

The historical example's US$10 bounty gave a correct challenger only US$8 net before investigation; US$10 investigation made the action loss-making even when correct. The revised illustrative distribution for a US$100 author bond, US$10 challenge deposit and US$2 fee is:

| Destination (USD) | Proceeding | Not proceeding | Unresolved |
|---|---:|---:|---:|
| Author | 0 | 102 | 100 |
| Challenger | 40 | 0 | 10 |
| Verifier | 2 | 2 | 2 |
| Restricted reserve | 70 | 8 | 0 |
| **Total** | **112** | **112** | **112** |

The US$40 returns the US$10 deposit plus a US$30 bounty. Net payoff before investigation is +28/-12/-2. With precision `q` conditional on resolution, investigation cost `C` per attempt and unresolved rate `u`, `EV = (1-u)[30q - 10(1-q)] - 2 - C`. For `u=0,C=10`, break-even precision is 55%, a correct challenge nets US$18 and 90% precision gives US$14 expected gain. These are proposed values, not implemented payouts or evidence that independent challengers exist. Expensive investigation, ambiguous outcomes, collusion or reserve access can still break the design. The verifier's US$2 assumes a cheap check, not a subsidy for arbitrary investigative work. Reserve funds are not consumer insurance; any compensation policy needs separate funding, beneficiaries, limits and fraud tests.

Reprice guarantees at full detection instead of assuming unreported errors. The earlier 5,000 monthly US$100 commitments with two-day lock, 20% capital buffer, 10% annual opportunity cost, US$0.005 checks and 0.1% error require US$858.33/month before omitted administration/custody/tail costs at 100% detection. A US$1,030 premium adds a 20% cost markup, not demonstrated solvency. Correlated failures and longer locks require separate capital stress. No real guarantee is offered here.

For queries, the historical one-million/month scenario modeled US$0.0177 total cost and US$0.0023 slack at US$0.02 price. An **illustrative, unmeasured** incremental cost of US$0.003 for the added policy/extraction/maintenance path raises cost to US$0.0207: US$0.02 loses US$700/month; US$0.03 gives US$9,300/month before remaining omissions. Neither the price nor margin is validated. Measure content payments, node/connector work, ingestion, extraction, storage, correction, revalidation, audit, payment, support and fixed costs, including low startup volume. Successful payment arithmetic cannot substitute for paid repeat demand and creator net return. `scripts/economics_pilot.py` reproduces sensitivity and conservation tests; §14.1 supplies stop gates.

## 10. Security Analysis

Threat model: adversaries want to acquire misleading confidence, poison calibration, create circular corroboration, buy visibility, steal priority or payments, suppress competitors, manufacture receipts, corrupt witnesses or logs, exploit confidential evidence, exhaust the query-time evidence join, and capture governance. Status terms are evidence-based: **SHIPPED** means present and tested in the current repository; **PARTIAL** means some code exists but the stated property is incomplete; **SPECIFIED—UNIMPLEMENTED** means this paper defines a mechanism with no production proof; **OPEN** means the defense is not yet adequate.

| # | Attack | Defense | Status |
|---|---|---|---|
| 1 | Capital or identity status changes base relevance | Remove every registry-derived feature from Stage 1; retain stake/DNS ablations and dependency-boundary tests (§5.9) | **PARTIAL** — `dns_mult` has been removed and focused scorer/`HybridIndex` tests pass; the representative-corpus ablation and full dependency audit remain incomplete. |
| 2 | Cross-class, identity-reset, or syndication-based uncertainty laundering | Versioned multi-level `SourceRef`s, no global score, hierarchical class priors, claim/task conditioning, raw typed records, and no automatic transfer (§5.7–§5.8) | **SPECIFIED—UNIMPLEMENTED** — entity resolution, calibration, and adversarial tests are required. |
| 3 | Log equivocation or split views | Independent logs, signed tree heads, consistency proofs, monitor gossip (§5.2) | **SPECIFIED—UNIMPLEMENTED**. |
| 4 | Artifact withholding makes old checks unverifiable | Retention and availability rules for economically live Grade-A inputs (§5.2) | **OPEN** — legal erasure and durable reproducibility remain in tension. |
| 5 | Canonicalization or variant ambiguity | Versioned CDS, golden tests, request profiles, variant-aware observations (§5.3) | **PARTIAL** — a distiller exists; CDS-1 and its conformance suite do not. |
| 6 | Predicate ambiguity, verifier divergence, or fabricated traces | Deterministic execution/proof gate; divergence refunds and pauses the class (§5.5) | **SPECIFIED—UNIMPLEMENTED**. |
| 7 | Model error, correlated judges, prompt injection | Grade B has no capital authority; versioned model evidence is dependency-discounted and must pass calibration and attack tests before probabilistic use (§4/§5.8) | **SPECIFIED—UNIMPLEMENTED**. |
| 8 | Coalition wash-trading across payee, curator, sponsor, verifier, and witness roles | Full-coalition inequality (§9.2) | **OPEN** — the current split supplies a nominal 10% margin, but emissions and additional recoverable roles are not modeled or tested. |
| 9 | Paid-demand and receipt manipulation | Correlation discounts, exposure normalization, receipt taxonomy, privacy controls (§7/§9) | **SPECIFIED—UNIMPLEMENTED**; Sybil resistance is not claimed. |
| 10 | Priority theft or authorship overclaim | Publisher-signed anchored priority; labels state only commitment time; payment requires an explicit derivative-use rule (§5.2) | **SPECIFIED—UNIMPLEMENTED**. |
| 11 | Crawl-queue flooding or capital capture | Capacity reservations, rate limits, cluster caps, published queue metrics (§7) | **SPECIFIED—UNIMPLEMENTED**. |
| 12 | Domain lapse, key compromise, or insider misuse | Key-bound funds, scoped operational keys, revocation, binding cooldown (§5.1) | **SPECIFIED—UNIMPLEMENTED**. |
| 13 | Corroboration Sybils, hidden derivation, and citogenesis | Evidence graph, echo/translation/paraphrase detection, correlation metadata, interval widening; neither repeated citations nor repeated positions become independent evidence (§5.6–§5.8) | **OPEN as an inference problem** — common ancestry and common funding can remain undetected. |
| 14 | Sealed-evidence source identification | Fresh salts, padded metadata, optional non-reveal, separate confidentiality contracts (§5.6) | **PARTIAL** — the design reduces hash-linkage risk but cannot prevent watermarking, endpoint compromise, or legal process. |
| 15 | Sponsorship collusion | Type-scoped co-bonds, public positions, cluster caps, no transferred pass history (§8) | **SPECIFIED—UNIMPLEMENTED**; independence cannot be proven. |
| 16 | Query or readership privacy leakage | Off-public-log raw data, batching, cohort thresholds, limited retention (§9.1) | **OPEN** pending a concrete privacy and threat model. |
| 17 | Governance capture or malicious predicate upgrade | Version pinning, prospective upgrades, timelocks, monitors, client choice (§5.5/§9.3) | **PARTIAL** — v1 controls exist, but v2 governance is not implemented or audited. |
| 18 | Challenge, audit, or log-submission budget exhaustion | Bonds, per-origin limits, bounded queues, verifier epoch caps, and non-displaceable maintenance capacity | **SPECIFIED—UNIMPLEMENTED**; parameter griefing remains open. |
| 19 | Forged or replayed usage receipts | Payer authorization, service signature, license binding, unique nonces, and replay checks (§9.1) | **SPECIFIED—UNIMPLEMENTED**. |
| 20 | Calibration poisoning or publisher-selected easy audits | Logged audit/position propensity, bounded inverse-propensity correction, held-out forward evaluation, random-audit capacity, and no publisher-selected cash successes (§5.8) | **OPEN** — estimators and manipulation bounds are research work. |
| 21 | Outcome-target gaming or label leakage | Pre-registered task targets, time-split evaluation, immutable calibration-set hashes, and independent evaluation operators (§5.8) | **SPECIFIED—UNIMPLEMENTED**. |
| 22 | Overconfidence under sparse evidence or regime change | Hierarchical shrinkage, explicit ignorance, posterior intervals, OOD detection, and source-regime alarms (§5.8) | **OPEN** — safe thresholds require domain pilots. |
| 23 | Uncertainty policy becomes incumbent censorship | Stake-blind broad retrieval, exploration floors, public dual ranks, policy choice, and counterevidence display (§5.9/§7) | **SPECIFIED—UNIMPLEMENTED**; policy governance remains open. |
| 24 | Evidence-join or inference latency denial of service | Top-K bounded inference, versioned caches, per-query budgets, and `unknown` fallback (§7) | **SPECIFIED—UNIMPLEMENTED**. |
| 25 | Wealth, hidden hedging, self-funding, or many correlated positions buy confidence | Claim-specific eligibility; log amount saturation; loss/time/scope/funding/unhedged factors; cluster collapse; held-out wealth slices; learned effect allowed to be zero/negative; absolute likelihood cap (§5.8.3) | **PARTIAL** — normalization, cap, calibration, and whale tests exist in MVP-1; hedge/funding attribution and real-world manipulation bounds are open. |
| 26 | Protected content leaks through snippets, embeddings, caches, or catalog probing | Authorized previews, isolated indexes, scope-aware caches, metadata minimization, and adversarial extraction tests (§6.2) | **SPECIFIED—UNIMPLEMENTED**; downstream plaintext leakage remains open. |
| 27 | Synthetic origin forgery or poisoned evidence units contaminate answers or exports | Distinct source declarations and inferred lineage, preserved originals, transformation records, family-disjoint evaluation, and no automatic truth promotion (§7.1–§7.3) | **OPEN** — universal origin detection and complete lineage are not claimed. |
| 28 | Zero-price activity farms grants or paid-exposure statistics | Typed nonpayable records, no automatic rewards, payer-authorized payable events, and full-coalition analysis (§9.1–§9.2) | **SPECIFIED—UNIMPLEMENTED**. |

Protocol invariants to be implemented and continuously tested:

1. **Deterministic capital authority:** no slash, seizure, or redistribution without a versioned Grade-A predicate and matching reproducible traces.
2. **Unresolved conservation:** when the predicate or required artifact is unavailable or ambiguous, commitment bonds and payer escrow return to their owners; verifier work may be paid only from a capped budget that cannot be self-farmed.
3. **Coalition-negative manufactured usage:** for the cheapest modeled attacker cluster, total recoverable payout and subsidy is strictly less than the fee plus irreversible external cost (§9.2).
4. **No cross-class transfer:** activity in one accountability field cannot raise another field or a consensus global label; probabilistic evidence transfer requires an explicit, validated hierarchical model.
5. **Retrieval separation:** no financial, identity, accountability, or uncertainty signal enters Stage-1 relevance or candidate pruning.
6. **Calibrated reliance:** a numeric uncertainty output requires a named observable target, versioned evidence/model/calibration set, held-out metrics, and an interval; otherwise the mode is `EVIDENTIAL_ONLY`.
7. **Unknown is preserved:** missing, stale, dependent, or out-of-distribution evidence cannot be silently converted into confidence.
8. **Economic conviction is empirical, not purchased truth:** only eligible claim-specific exposure may enter uncertainty, only through a held-out class-scoped likelihood with an absolute cap; insufficient calibration means zero effect.

These are target properties, not claims about the current implementation. Phase gates require executable tests, red-team strategies, and measured parameter margins for each one.

---

## 11. What This Protocol Refuses to Do

Stated as product surface, because the refusals are load-bearing (P3):

- **It does not adjudicate contested truth.** Political claims, live scientific disputes, matters of interpretation: no bond market, no jury verdict, no truth label. Such content receives provenance, priority, track record, and commitment status — nothing more.
- **It does not present observation records as authorship proofs.** Anchored signatures prove that an identity committed to bytes within a time interval; witness records prove that operators signed observations; claim clustering is heuristic. None alone proves authorship, originality, ownership, or truth.
- **It does not promise Sybil-proof or collusion-proof judgment.** It publishes assumptions, raw denominators, correlation heuristics, and modeled attacker costs. Where no defensible bound exists, §10 says `OPEN`.
- **It does not let capital or reputation buy relevance.** Stage 1 remains broad and stake-blind. Stage 2 may use claim-level uncertainty under an explicit policy because that is the product's purpose; it must show both ranks and preserve exploration rather than smuggle a universal trust order into retrieval (§5.8–§5.9).
- **It does not emit a probability merely because users want one.** A numeric estimate requires an observable target and demonstrated calibration. Otherwise the answer is support, conflict, and ignorance—not confidence theater.
- **It does not turn ordinary payment into evidence.** Open reporting may earn licensed retrieval revenue, but that payment does not improve an accuracy field. A distinct claim-specific position may contribute only the empirically learned, capped conviction likelihood in §5.8.3; it is not inferred from demand or usage revenue. Only explicitly cited event-bound claims use assertion escrow (§5.6/§9.1).
- **It does not infer consent from DNS or silence.** Passive observation, identity control, content-use permission, and financial participation are separate states (§6).
- **It does not pretend criterion outcomes are legal or journalistic adjudications.** `CRITERION_MET` and `CRITERION_NOT_MET` describe execution of a declared machine rule, not external truth (§12.3).
- **It does not promise exclusive discovery or universal copy prevention.** Protected delivery applies to controlled endpoints and authorized operators; independent reporting, public copies, and downstream leakage remain possible (§6.2).
- **It does not certify every structured unit as a fact.** Contextual claims and multimodal extractions preserve their original support, transformation errors, and uncertainty (§7.1–§7.2).
- **It does not claim to eliminate hallucinations or prevent model collapse.** Retrieval quality and training-data quality require separate experiments (§1.6, §7.3).

---

## 12. Legal & Regulatory Posture

This section states design requirements and launch gates, not legal conclusions or advice. Copyright, database rights, privacy, defamation, intermediary liability, payments, tax, and financial regulation vary by jurisdiction and product behavior. Each deployment needs a documented rights basis, data-role analysis, and advice from qualified counsel before launch.

### 12.1 Copyright and text-and-data-mining

Crawler permission, text-and-data-mining authority, indexing, snippet display, model training, retrieval, and redistribution are different uses and require a per-jurisdiction rights matrix. Technical signals such as `robots.txt`, `noindex`, and `noai` must be honored where promised, but they do not have one universal legal effect. In the EU, the DSM Directive's Article 4 exception is conditional on lawful access and the absence of an appropriate express reservation; it does not by itself authorize serving a full work to customers.

Level 0 therefore stores and serves only what the deployment's documented legal basis permits. DNS control does not grant content rights. Level 2's signed manifest binds identity and commitments while its RSL record or separately negotiated contract expresses machine-use permission; operators must still validate scope and authority, especially for syndication, licensed wire copy, comments, and other third-party material. Whether a URL, hash, snippet, or distillate is protected or regulated is fact- and jurisdiction-specific; this paper makes no categorical exemption claim.

Extraction into a contextual claim, transcription, embedding, or compact internal format does not automatically create a new permission to distribute or train. Rights and restrictions on original and derivative representations must be resolved separately. The protocol does not assert exclusive ownership of independently observable facts or claim that a purpose flag universally binds third parties. License validity, attribution requirements, permitted transformations, and enforcement after authorized delivery require deployment-specific legal analysis. Access security and legal permission remain different controls.

### 12.2 Data protection

URLs, signatures, receipts, relationship graphs, and even salted hashes can be personal data when linkable to a person. Before production, each deployment must identify controllers, processors, joint-controller relationships, purposes, lawful bases, retention periods, data-subject request handling, international transfers, security controls, and whether a data-protection impact assessment is required.

Tombstoning is a serving-layer control, not cryptographic erasure. An append-only redaction record can stop protocol stores from serving an artifact and encryption-key destruction can make retained ciphertext inaccessible, but neither can erase copies held by independent log operators or users. On-chain anchors contain only aggregate roots to reduce exposure, yet a root can continue to prove inclusion if another party retained the leaf and proof. Retention and replication policy must therefore be designed together with the log topology in §5.2; the paper does not claim that append-only transparency and every erasure request are automatically compatible.

### 12.3 Publishers, defamation, and discovery

A signed, timestamped assertion can increase evidentiary and discovery exposure. Phase C therefore requires design-partner counsel to review labels, contracts, source handling, and jurisdictional risks before real newsroom use:

- **Criterion labels are narrow:** interfaces say which machine criterion was met or not met and link its trace. They do not publish a protocol finding that a person lied or a story is false.
- **Entity structure is not an automatic shield:** a separate contracting or bonding entity may change risk allocation, but liability, privilege, agency, capitalization, and veil-piercing depend on facts and law. The protocol promises no isolation.
- **Sealed evidence remains sensitive:** salt reduces dictionary matching but cannot defeat document watermarking, endpoint compromise, inference, or compulsory legal process. Non-reveal is always available inside the protocol and has no truth penalty; retention location and access control require counsel and source-protection expertise.
- **Corrections are appendable and visible:** the product must make correction operationally easy without claiming that the protocol determines the legal effect of a correction.

### 12.4 Financial characterization

Labels such as "sponsorship" or "revenue share" do not determine regulatory treatment. Bonds, pooled rewards, transferable instruments, custody, refunds, and yield-like payments may trigger securities, lending, gambling, money-transmission, consumer-protection, sanctions, tax, or accounting obligations. Level 0 creates no unclaimed publisher balance. For Levels 1–3, funds move only after payee onboarding and accepted terms, with custody and dormant-property treatment defined before receipt.

Phase B requires payment and custody analysis for every launch market. Phase C adds media, defamation, and source-protection review. A native token, if ever proposed, requires a separate necessity case, governance analysis, independent smart-contract audit, and jurisdiction-by-jurisdiction launch decision. The tokenless design remains the default if those gates are not met.

---

## 13. Migration from v1

What exists in the codebase today, and what becomes of it:

| Component (today) | v2 disposition |
|---|---|
| Stake-amount/reputation exclusion tests (`stake_ablation.rs`, `signal_comparison.rs`, `test_zero_stake_no_boost` family) | Seed for the Stage-1 retrieval-separation invariant (§5.9). Stake, reputation, DNS-neutrality, and a focused non-zero `HybridIndex` registry ablation pass; a representative-corpus ablation and dependency audit remain Phase A work. |
| Former `dns_mult` in `compute_score` (v1 §12.6) | **Removed from the scoring path**; DNS proof remains provenance metadata. It can become a scoped uncertainty feature only if empirically validated for a declared target (§5.8–§5.9, §10 #1). |
| Domain staking + DNS TXT verification | `domain_binding` metadata + Level 1 binding; wallet re-bound to keypair; freeze/continuity rules on binding change (§5.1). Existing domain stake does not automatically become a `ConvictionSignal`: it lacks claim scope, direction, horizon, and outcome-specific loss. |
| OpenUncertainty MVP-1 (`crates/oi-index/src/uncertainty.rs`) | Executable seed for typed source calibration, eligible economic conviction, intervals, evidential fallback, dual ranks, and calibration metrics. It is not evidence that the real-world predictive thesis holds (§5.8). |
| Verdict tiers + `compute_verdict` conjunctive gates | Retired as protocol consensus. May remain temporarily as a legacy API while clients migrate to typed records and `UncertaintyEstimate` (§5.7–§5.8). A legacy verdict must never be relabeled as a calibrated probability. |
| Feedback stub (open Sybil vector) | **Deleted, not fixed** — votes are no longer an input |
| Dispute bonds, commit-reveal jury, graduated slashing | Bonds and deterministic payout plumbing are reusable for Grade-A predicates. Jury outcomes become non-binding governance evidence and never slash (§5.5). |
| Jury reward epoch caps | Reused as a design constraint for capped verifier-work budgets; multi-role farming analysis is still required (§9.2). |
| Sentinel crawl daemon | Seed for exact-mirror checks and event-bound criterion watching; later re-fetches do not prove historical witness dishonesty (§5.3, §5.6). |
| Distiller (`oi-crawl`) | Seed of CDS-1; deterministic + versioned + conformance-tested — Phase A gate (§5.3) |
| SimHash dedup | Seed for echo detection, variant clustering, and dependency discounting (§5.3, §5.8). It is not a security boundary for priority or proof of independent evidence. |
| `oi-facts` (GLiNER) | Seed for heuristic claim extraction, query-specific claim selection, and evidence-graph navigation (§5.8); never a truth or payment predicate. |
| Creator ledger | Existing served-result/fixed-credit prototype; extend to the typed, paid, privacy-preserving receipts and event-bound escrow of §9.1. |
| Replication integrity checks | Preserve as a base; identity-key chunk signatures and the artifact-availability policy remain specified work. |
| State channels + settlement burn | Starting substrate only. Multi-party allocation, refunds, replay protection, coalition safety, and privacy are new work (§9). |
| Robots.txt handling in the fetcher | Preserve and test; add `noindex`, applicable TDM-reservation, and jurisdictional policy handling as Phase A work. The paper does not claim those additions are already shipped (§6/§12.1). |
| v1 whitepaper §12.2, §12.3, §12.6 | **Retracted**; the §12.6 DNS multiplier has now been removed from `compute_score`. |

The migration preserves useful retrieval and cryptographic components but is not a configuration-only upgrade. OpenUncertainty now has an isolated estimator MVP-1 and a narrow live two-stage answered-search integration. The generalized claim/evidence graph, production calibration pipeline, distributed evidence join, typed protocol records, transparency-log monitoring, deterministic predicate traces, standards-based rights integration, privacy-preserving receipts, refunds, and coalition-safe settlement remain new work. The roadmap distinguishes an implemented mechanism from an empirically validated product.

---

## 14. Roadmap

Sequencing follows P1 and §9.4: ship retrieval separation and reproducible evidence predicates before capital can affect uncertainty or settle. A phase gate is met only by code, conformance tests, an operational threat model, empirical results, and—where applicable—legal approval for the launch market.

| Phase | Deliverable | Economics in-phase |
|---|---|---|
| **A — Retrieval and provenance foundation** | Verify the shipped `dns_mult` removal with with/without-registry corpus ablations; establish stake-blind multi-retriever baselines; specify CDS-1; implement variant-aware attestations, claim extraction, near-duplicate clusters, citation dependencies, independent logs, signed-tree-head gossip, artifact availability, and anchored priority. Publish the rights-policy matrix and Level 0/1 privacy review. | No token or truth bond. Funded as search/provenance infrastructure; viability is an experiment, not assumed. |
| **B — OpenUncertainty vertical** | In one outcome-rich domain, extend MVP-1 with typed Grade-A evidence, random audits/positions with logged propensities, hierarchical source calibration, dependency-aware aggregation, intervals, `EVIDENTIAL_ONLY` fallback, explanations, dual ranks, and the adversarial/forward-time evaluation suite. Compare base rate, source history, self-reported confidence, raw cash, effective cash, evidence-only, evidence-plus-cash, popularity, domain reputation, and uncalibrated-LLM baselines. | Fiat may fund licensed access and audits. Claim-specific Grade-A/event-bound cash at risk is an experimental estimator feature under §5.8.3; ordinary payment, demand, domain stake, and raw amount remain excluded. Stop if cash adds no out-of-sample calibration/selective-risk value or creates unacceptable wealth/manipulation slices. |
| **C — General search and publisher economy** | Expand claim classes and languages; deploy the latency-bounded top-K evidence join, cache invalidation, source-diverse answer construction, open-reporting graphs, corrections, RSL/C2PA integration, PEAC-compatible receipts, and a publisher payment pilot. Scale gates cover index size, freshness, recall, tail latency, cost/query, original-source recovery, uncertainty coverage, and abuse. | Fiat or stable-value usage payments; Grade-A bonds only after execution and coalition-safety audits. Gate on copyright, privacy, media, payment, and source-protection review. |
| **D — Optional federation and decentralization** | Evaluate whether additional index/log operators, portable evidence graphs, permissionless verification, or a native asset materially improve coverage, resilience, or neutrality. If a token is proposed, publish the necessity case, governance model, economic simulations, audits, and jurisdiction decisions first. | Tokenless rails remain valid. No launch merely to complete the roadmap. |

Each phase must publish success and stop criteria before evaluation. Core gates include calibration and interval coverage by slice, selective risk, original-source recovery, retrieval recall/nDCG, crawler coverage and abuse, p95/p99 latency, cost per indexed document and query, publisher permission conversion, privacy incidents, coalition attack margin, and customer retention. Failure to beat the declared baselines or meet safety margins triggers redesign or termination rather than automatic progression.

### 14.1 Bounded publisher and evidence-unit pilot

The recommended next scope is **text and structured data**, an operator-run API, a small set of authorized publishers, and agents performing real tasks. Entry examples include protocol documentation and objectively checkable execution records, with separate election-poll cases testing measurement/report lineage. These are initial evaluation domains, not a retreat from general search. New query families, languages, and sources must enter sealed holdouts so a polished narrow demonstration cannot substitute for generalization.

Start with free, attribution-based, and paid licensed resources. Exercise catalog discovery, budget authorization, delivery, context-preserving units, citations, corrections, and typed receipts without token incentives. A payment pilot additionally requires rights, custody, privacy, and security approvals. Multimodal coverage and training exports follow only after this path is useful and sustainable.

| Gate | Evidence required before advancement |
|---|---|
| **Consumer quality** | Blind full-answer assessment against market search and agent-search baselines. Claim support, original recovery, contradictions, selective risk, and coverage, with family/time-disjoint holdouts. |
| **Unit fidelity** | Compare full documents and ordinary chunks with contextual units. Measure lost qualifiers, wrong entities/dates, extraction errors, corrections, and original-span recovery. |
| **Policy and security** | Free and paid workflows, visible attribution, training-use denial, revocation, budget limits, protected-cache isolation, replay/refund behavior, and unauthorized extraction attempts. A purpose-denial test does not prove downstream use enforcement. |
| **Economics and reuse** | Cold/warm p50/p95/p99, lifecycle costs including ingestion and audits, creator net return, cost per useful answer, repeat paid use, and substitutable-provider failure tests. |
| **Financial conviction** | A separate prospective evidence-only/evidence-plus-cash experiment with eligible outcomes and manipulation slices. Successful content payment is not evidence that cash improves truth estimation. |
| **Training extension** | Separately authorized snapshot export and controlled model experiments under §7.3. No training-quality claim based only on a search demonstration. |

Publish dataset/manifest hashes, exact evaluation scope, declared budgets, exclusions, confidence intervals, and failure cases. General-superiority claims still require the pre-existing broad independent benchmark gate (§5.8–§5.9), rather than a new hand-picked pilot score. Stop or narrow a feature when better evidence, lawful serving, lifecycle cost, buyer demand, creator return, or protection against leakage cannot meet predeclared requirements.

**Execution order and proposed quantitative gates.** First deliver an operator-run API over text and structured data, free/attribution sources, and a real agent workflow; then test P0 protected delivery after rights/security approvals. No token, global participant network, training export or broad multimodal support is needed for first customer value. Name implementation, independent evaluation, rights/security and economics owners before starting; those roles are not assigned by this paper. Recruit for a four-week bounded service pilot, with an inspected 240-query variance/cost set followed by a sealed >=1,000-query/eight-stratum manifest and two external blinded full-answer reviewers. Freeze power, policies and thresholds before capture. The proposed quality target is >=20% relative reduction in material errors at matched answer coverage, positive paired cluster-bootstrap 95% lower bound, and coverage no more than five percentage points below the declared strongest baseline. Navigation/transaction success is measured separately. No-answer output is not a correct answer.

Proposed latency ceilings: search-only cold p95 <=2s/p99 <=5s, full evidence-bearing answers p95 <=5s/p99 <=10s, measured at comparable end-to-end endpoints with load and failures. Require >=99% required-locator/context fidelity with no materially reversed negation/entity/date/scope in the release suite, no protected-delivery/budget violations in the declared security tests, positive challenger EV using measured investigation distributions, positive full-cost contribution and >=30% modeled operating margin at the agreed pilot price, and at least two of three recruited teams independently choosing paid reuse for two weekly cycles. These are targets, not results; insufficient precision is inconclusive. Keep content payments distinct from the separate conditional-cash gate (>=3% relative Brier gain with positive bootstrap lower bound, log-loss/calibration/interval/selective-risk and manipulation checks). The complete sampling, responsibilities, P0 terms, stress model and stop rules are in `docs/experiments/PILOT-GATES-2026-10-08.md`.

**Operational trust boundary.** The current operator configures providers, labels, policy, model releases, access and availability. Publish versioned changes, bind answers to their versions, and retain correction/appeal traces. Blockchain settlement does not decentralize these choices. Real financial deployment additionally requires deterministic execution, independent security, legal/custody and coalition-incentive validation; arithmetic and local tests do not grant those approvals.

---

## 15. Conclusion

The web does not lack content; it lacks a scalable way to distinguish relevance from warranted reliance. Without that distinction, synthetic volume, circular citations, and fluent unsupported summaries can feed one another until the retrieval corpus loses both signal and the economic capacity to create new primary work.

v1 answered with domain stake. v2.7 keeps the useful intuition—costly commitments can reveal conviction and create accountability—but rejects capital as a shortcut to truth or relevance. It proposes a different core: retrieve broadly, build a claim-level evidence graph, estimate uncertainty only against a declared target, expose conflict and ignorance, then let an explicit risk policy decide whether to rerank, cite, warn, or abstain. Crawl order is not authorship. A signature is not factual accuracy. Corroboration without independence is not confirmation. Ordinary payment is not evidence. Claim-specific cash at risk is evidence of conviction whose predictive value must be learned. Missing evidence is not confidence.

This architecture has the potential to support a general-purpose search engine with less epistemic noise than relevance-only ranking because it attacks the feedback loop at its structural weak point: unsupported derivatives cannot gain confidence merely by multiplying, while primary and genuinely independent evidence remain visible. The same separation protects discovery: new sources can enter broad retrieval before they have history, and uncertainty is allowed to change reliance only through a visible client policy.

The novelty claim is deliberately narrow. Transparency logs, content credentials, machine-readable licensing, interaction receipts, Bayesian calibration, prediction-market signals, and hybrid retrieval already exist. OpenSonarX's proposed contribution is their integration into a **web-scale, claim-conditioned, dependency-aware uncertainty layer with two-stage search**, deterministic-only capital authority, a bounded outcome-learned economic-conviction feature, and a publisher payment path whose ordinary payments never buy confidence. Whether that combination is new enough for publication and strong enough for production depends on a systematic prior-art review and the Phase B benchmarks—not rhetoric.

The current repository now proves that a narrow real search/crawl/evidence/abstention path can operate end to end and remain correct across the tested concurrent query families. It still does not contain the completed estimator, a complete dependency graph, or a Google-scale system. OpenSonarX succeeds only if forward-time and adversarial results show better calibration, selective risk, original-source recovery, diversity, and user outcomes at acceptable latency and cost, while preserving open discovery and lawful publisher participation. Those are falsifiable claims. Until they pass, “Google-level with more trust” is the ambition; OpenUncertainty is the hypothesis.

The publisher extension serves that same mission: authorized, contextual information can support better answers while the creator chooses attribution or payment. Open exchange and reusable evidence may give outside participants a productive role without making the initial product depend on a token network. Training exports and multimodal units are optional later uses with separate permissions and quality gates. V2.8 changes local estimator safeguards and the specification/validation plan; August search benchmarks remain historical and the financial layer remains inactive.

---

## 16. Related Work and References

OpenSonarX combines established ideas; it does not claim to have invented transparency logs, content provenance, licensing, receipts, Bayesian calibration, selective prediction, or hybrid retrieval. The following are design inputs and overlap that any novelty claim must address:

- **Transparency and artifact logs:** [RFC 9162, Certificate Transparency Version 2.0](https://www.rfc-editor.org/rfc/rfc9162.html) defines Merkle-tree inclusion and consistency mechanisms. [Sigstore Rekor](https://docs.sigstore.dev/logging/overview/) applies transparency logging to signed artifacts. OpenSonarX uses these patterns for observable evidence history; a log proves inclusion and consistency, not truth.
- **Content provenance:** [C2PA 2.4](https://spec.c2pa.org/specifications/specifications/2.4/specs/C2PA_Specification.html) defines signed manifests, claims, assertions, and ingredient relationships. OpenSonarX imports these records when available. Its additional research question is whether provenance and derivation can improve a calibrated, claim/task-specific reliance estimate; it does not replace C2PA.
- **Machine-readable rights and payment:** [RSL 1.0](https://rslstandard.org/rsl) is a published recommendation for expressing AI/search use, attribution, reporting, and compensation and for discovering or acquiring licenses. [Cloudflare Pay Per Crawl](https://developers.cloudflare.com/ai-crawl-control/features/pay-per-crawl/what-is-pay-per-crawl/) is an operational example of paid crawler access. OpenSonarX consumes these rights rather than defining a competing license vocabulary.
- **Prediction markets, money, and belief elicitation:** Wolfers and Zitzewitz, [*Prediction Markets*](https://www.nber.org/papers/w10504), survey markets as information-aggregation mechanisms; this is prior art for using financially exposed forecasts, not for OpenSonarX's search integration. Crucially, Servan-Schreiber et al., [*Prediction Markets: Does Money Matter?*](https://gsbpreserve.stanford.edu/view/60889/prediction-markets-does-money-matter), found play-money markets performed as well as real-money markets in their comparison, so “real money automatically means better prediction” is not an acceptable premise. Armantier and Treich, [*Eliciting Beliefs: Proper Scoring Rules, Incentives, Stakes and Hedging*](https://www.tse-fr.eu/articles/eliciting-beliefs-proper-scoring-rules-incentives-stakes-and-hedging), show theoretically and experimentally that incentives, event stakes, risk aversion, and hedging can distort reported beliefs. These results motivate outcome calibration, hedge/funding discounts, negative controls, and the permission for cash's learned effect to be zero or negative (§5.8.3).
- **Proper scoring and forecast evaluation:** Gneiting and Raftery, [*Strictly Proper Scoring Rules, Prediction, and Estimation*](https://stat.uw.edu/research/tech-reports/strictly-proper-scoring-rules-prediction-and-estimation-revised), develops the basis for evaluating probabilistic forecasts with proper scores. OpenUncertainty uses Brier score and log loss as primary predictive metrics and reports calibration separately; no single metric is treated as proof of usefulness.
- **Portable interaction receipts:** [PEAC](https://www.peacprotocol.org/) defines portable signed interaction records and HTTP purpose/receipt conventions. It is pre-1.0 and must be version-pinned. OpenSonarX proposes a compatible profile for licensed retrieval, citation, and settlement events, with additional privacy and coalition constraints; receipt syntax itself is not the novelty.
- **Closest licensing/provenance overlap:** Baskaran, Pherwani, and Krishnan, [*Aegon: Auditable AI Content Access with Ledger-Bound Tokens and Hardware-Attested Mobile Receipts*](https://arxiv.org/abs/2604.06693), combines licensing tokens, a Certificate-Transparency-style ledger, and signed provenance events across chunking, embedding, retrieval, and citation. This substantially overlaps OpenSonarX's rights/log/receipt plane. OpenSonarX's claimed differentiation is the search-facing uncertainty algorithm, dependency-aware corroboration, dual ranking, deterministic-only capital rule, and open-discovery constraints—not the existence of an audit ledger.
- **Generative-search verifiability:** Liu, Zhang, and Liang, [*Evaluating Verifiability in Generative Search Engines*](https://arxiv.org/abs/2304.09848), formalizes citation recall and precision and shows why fluent, cited answers can still be unsupported. These metrics belong in the Phase B/C evaluation suite.
- **Selective prediction and calibrated fact-checking:** Geifman and El-Yaniv, [*Selective Classification for Deep Neural Networks*](https://arxiv.org/abs/1705.08500), provides the risk/coverage framing for abstention. Yang, [*Calibrated Selective Fact-Checking via Evidence Chain Evaluation*](https://arxiv.org/abs/2607.18240), is especially close to OpenUncertainty's evidence/abstention goal and reports that selective behavior can help even when aggregate calibration does not beat a retrieval baseline. OpenSonarX must compare directly rather than assume superiority.
- **Model judgment limits:** Zheng et al., [*Judging LLM-as-a-Judge with MT-Bench and Chatbot Arena*](https://arxiv.org/abs/2306.05685), documents biases relevant to Grade-B evaluation. Model outputs remain versioned evidence, never deterministic capital authority.
- **Crawler and mining controls:** [RFC 9309](https://www.rfc-editor.org/rfc/rfc9309.html) standardizes the Robots Exclusion Protocol. [EU Directive 2019/790](https://eur-lex.europa.eu/eli/dir/2019/790/oj), especially Articles 3–4, is one input to jurisdiction-specific text-and-data-mining analysis; it is not a global serving license.
- **Internal implementation evidence:** [docs/SECURITY-ECONOMICS.md](docs/SECURITY-ECONOMICS.md), the ranking regression tests cited in P4, and the migration inventory in §13 define the local baseline that Phase A must verify.

### 16.1 Current product and protocol overlap

The October 2026 review identified concrete alternatives. The following are descriptions from primary documentation, not independent benchmarks, audited economic results, or claims that every roadmap feature is live:

| Reference | Documented overlap | Implication for OpenSonarX |
|---|---|---|
| [Parallel Research Basis](https://docs.parallel.ai/task-api/guides/access-research-basis) | Task outputs with field-level citations, excerpts, reasoning, and confidence levels. | Answers with evidence and confidence are already a product category. Compare actual user outcomes. |
| [Cleanlab TLM](https://help.cleanlab.ai/tlm/tutorials/tlm/) | Response trustworthiness scoring, including assessment of supplied responses. | Numeric uncertainty and response triage alone are insufficient differentiation. |
| [OriginTrail DKG](https://docs.origintrail.io/how-dkg-works/agents-and-trust) | Agent knowledge sharing, anchored records, and scoped trust states. | Provenance and an agent knowledge graph are substantial architectural overlap. |
| [Factland](https://docs.factland.org/) | Claims, evidence, staking, and community-jury verification. | Financial conviction predates this proposal. The deterministic-only capital policy differs from jury settlement. Public documentation does not prove successful cash-based accuracy. |
| [UMA](https://docs.uma.xyz/protocol-overview/how-does-umas-oracle-work) | Bonded optimistic proposals and disputes, escalated to token-holder DVM voting. | Reuse the incentive lessons without pretending that voting becomes a deterministic truth predicate. |
| [Desearch 2.0](https://www.desearch.ai/whitepaper) | A technical proposal for a maintained collection, reusable crawl/embed work, and versioned indexes. | Reusable search work and external contributors are not an exclusive novelty claim. |
| [NewsGuard AI](https://www.newsguardtech.com/press/newsguard-launches-first-ai-chatbot-built-to-deliver-trusted-journalism-only-from-reliable-news-websites/) | Announced curated-source AI answers and publisher revenue sharing. | Source quality and creator payment already overlap. An announcement is not an audited payout result. |
| [Cloudflare Pay Per Use](https://blog.cloudflare.com/pay-per-use/) | A beta for contracted content-use compensation with buyer-reported usage. | Content remuneration without a click is not unique. Reporting and enforcement remain important boundaries. |

### 16.2 Scientific context for the extension

- **Recursive data and preservation:** Shumailov et al., [*AI models collapse when trained on recursively generated data*](https://www.nature.com/articles/s41586-024-07566-y), and Gerstgrasser et al., [*Is Model Collapse Inevitable?*](https://arxiv.org/abs/2404.01413), motivate explicit training workflows, retained originals, and conditional claims rather than inevitability (§1.6, §7.3).
- **Copy-aware truth discovery:** Dong, Berti-Equille, and Srivastava, [*Integrating Conflicting Data: The Role of Source Dependence*](https://www.vldb.org/pvldb/vol2/vldb09-pvldb47.pdf), is prior art for discounting dependent sources. OpenSonarX must establish the value of its task-conditioned implementation, not claim to invent copy awareness.
- **Evidence-grounded evaluation:** [AVeriTeC](https://arxiv.org/abs/2305.13117) offers real-world claims and web evidence as one evaluation input. Dataset availability, rights, time leakage, and domain mismatch must be checked before use. It cannot replace live multilingual general-search evaluation.

The prior-art conclusion is therefore conditional: individual components are not new, and the scientific and product references create meaningful overlap. The potentially publishable contribution is the integrated estimator and search architecture specified in §5.8–§7. A serious novelty claim requires a systematic literature and patent search plus experimental evidence that dependency-aware, propensity-corrected uncertainty improves calibration and selective risk at search scale. Publisher policy, structured media, training export, or blockchain alone does not supply that evidence. Benchmark against Parallel and a strong retrieval-plus-response-assessment baseline, and separately ablate evidence units, dependency handling, rights overhead, and eligible cash.

---

## Appendix A — New and Changed Protocol Parameters (vs v1 Appendix A)

| Parameter | Default (proposed) | Notes |
|---|---|---|
| `attestation_threshold W` | 3 | Number of registered operators required for a quorum-attestation label; does not prove independence or server behavior |
| `beacon_max_age` | 10 min | Freshness bound on the not-earlier-than beacon (§5.2) |
| `anchor_cadence` | ≤ 1 h | Mandatory log-root anchoring interval (§5.2) |
| `independent_log_min` | 2 | Minimum logs accepting economically live commitments; independence criteria remain a Phase A governance decision |
| `monitor_gossip_interval` | ≤ 10 min | Maximum target interval for exchanging signed tree heads and detecting split views |
| `artifact_retention_policy` | class + jurisdiction specific | Must make Grade-A checks reproducible or force `EXPIRED_UNRESOLVED`; no universal retention default |
| `cds_version` | 1 | Canonical Distillation Spec; consensus-versioned |
| `uncertainty_model_version` | deployment-pinned | Hashes estimator code, feature schema, priors, and inference configuration (§5.8) |
| `uncertainty_policy_version` | client-pinned | Declares task target, horizon, loss function, aggregation, and decision behavior (§5.8–§5.9) |
| `calibration_min_n` | class- and risk-specific | Minimum eligible observable outcomes before `CALIBRATED` mode; sample size alone does not waive slice or OOD gates |
| `credible_interval_level` | 90% | Default posterior interval displayed with `p_reliable`; high-stakes policies may require a wider interval |
| `source_prior_by_class` | learned, published | Hierarchical prior derived only from eligible training data; no protocol-wide source prior |
| `reference_bond_usd` | USD 1,000 in MVP-1 | Claim-class normalization amount for economic conviction; production values must be pre-registered and purchasing-power reviewed (§5.8.3) |
| `bond_cap_multiple` | 100 × reference in MVP-1 | Headline amount saturates at this level before loss/time/scope/funding/hedge discounts (§5.8.3) |
| `reference_lock_days` | 30 days in MVP-1 | Duration at which the capital-time component reaches full weight; class/horizon-specific in production |
| `min_cash_bucket_outcomes` | 20 in MVP-1 | Minimum eligible outcomes in both the exposure bucket and no-cash baseline; sufficient only for mechanism tests, not a production launch gate |
| `max_abs_cash_log_bayes_factor` | `ln(3)` reserved for a future validated conditional model | Current marginal contribution is zero. A future validated change is capped to 3× odds (§5.8.3). |
| `propensity_weight_cap` | pilot-calibrated | Caps inverse-propensity audit weights to control variance and manipulation |
| `dependency_discount_floor` | 0 | Minimum effective weight for evidence with unresolved shared ancestry; zero permits complete duplicate collapse |
| `freshness_half_life` | class/horizon-specific | Decay learned or validated against forward outcomes; never one global freshness constant |
| `ood_threshold` | model- and class-specific | Below this adequacy threshold the engine widens intervals or switches to `EVIDENTIAL_ONLY` |
| `claim_coverage_min` | policy-specific | Minimum proportion of answer-bearing claims with eligible evidence before a synthesized answer may be emitted |
| `decision_top_k` | latency-calibrated | Maximum candidate set receiving query-time evidence join and uncertainty inference (§7) |
| `uncertainty_latency_budget` | deployment-specific | On expiry, return `unknown`/base retrieval rather than stale or fabricated confidence |
| `evidence_cache_ttl` | evidence-type-specific | Upper bound only; new evidence, correction, dependency, freshness, or regime events invalidate earlier |
| `exploration_result_floor` | policy-specific | Minimum exposure opportunity for new/high-uncertainty relevant sources, subject to safety law and client intent |
| `audit_hazard_rate` | type-dependent | Random-audit probability per commitment-epoch (§5.5) |
| `challenge_bond_ratio_instant` | 0.5 × commitment bond | Instant regime, challenge rung |
| `verify_replication_n` | 3 | Independent reproduction target for Grade-A traces; agreement is checked by trace equality, not majority authority |
| `verifier_rewards_epoch_cap` | per-epoch budget | Restored from v1's jury cap; covers audits + checks (§5.5) |
| `event_window` | commitment-declared | Deadline and grace period for an event-bound predicate; source-specific bounds required |
| `envelope_salt` | ≥ 128-bit, per envelope | `H(evidence ‖ salt)`; size padded to fixed classes (§5.6) |
| `receipt_cohort_min` | deployment-calibrated | Minimum aggregation cohort before public settlement statistics; privacy analysis required |
| `raw_receipt_retention` | shortest operational period | Raw queries and user-level histories are not public protocol data (§9.1) |
| `coalition_safety_margin` | > 1.0 | `(fee + irreversible cost) / recoverable coalition payout`; must exceed 1 under the cheapest modeled attack (§9.2) |
| `baseline_capacity_share` | pilot-calibrated | Non-displaceable crawl capacity for unbonded candidates (§7) |
| `exploration_capacity_share` | pilot-calibrated | Capacity for new and low-exposure origins (§7) |
| `sponsored_ingestion_share_cap` | ≤ 20% per origin and cluster | Maximum separately budgeted crawl share purchasable by one correlated cluster; cannot displace baseline/exploration capacity (§7) |
| `underwriter_share_cap` | 30% | Of underwritten identity's usage revenue |
| `sponsorship_term_max` | deployment-calibrated | Maximum duration of a type-scoped sponsorship (§8) |
| `delegated_key_bond_ceiling` | per-key | Cap on commitments a delegated key may bond (§5.1) |
| `binding_change_cooldown` | 30 days | Freeze + challenge window on DNS-binding changes (§5.1) |
| `forbidden_retrieval_inputs` | all financial/identity/accountability/uncertainty fields | Enforced at Stage 1 by schema boundaries, dependency tests, and corpus ablation (§5.9) |
| `forbidden_uncertainty_inputs` | raw/unscoped bond, domain stake, payment, traffic, identity age, popularity, unaudited self-reports | Enforced by feature schema and counterfactual ablations. Only the derived, eligible, held-out-calibrated `ConvictionSignal` is the financial exception (§5.8.3) |

Defaults are hypotheses until Phase A/B measurements validate them. No v1 economic parameter carries over automatically; §9.3 lists reusable mechanisms and required re-review.

---

## Appendix B — Historical Revision Notes: v2.0 → v2.1

v2.1 made several important corrections to v2.0: it replaced crawl-order authorship claims with publisher-signed anchored commitments, added salts and padding to sealed-evidence commitments, disclosed the `dns_mult` ranking violation, introduced key hierarchy and domain-binding continuity, made witnessing variant-aware, and added legal and implementation-status sections.

It also introduced mechanisms that later revisions reject: Grade-B and apex slashing, a global verdict score and tiers, subjective TRUE/FALSE settlement for reporting, burn-on-expiry, protocol confidentiality penalties, and Level 0 accrued balances. Appendix C documents the v2.2 correction, Appendix D the v2.3 uncertainty architecture, and Appendix E the v2.4 executable conviction experiment. This historical note preserves lineage without restating obsolete rules as live parameters.

---

## Appendix C — Revision Notes: v2.1 → v2.2

1. **Deterministic-only capital:** removed Grade-B slash ceilings, subjective apex slashing, confidentiality slashing, fabrication judgments, and any other capital outcome not backed by a reproducible predicate (§4/§5.5).
2. **Temporal split:** replaced the mixed TRUE/FALSE ladder with event-bound assertions (`CRITERION_*`) and open reporting evidence (`CORROBORATED`, `CONTESTED`, `CORRECTED`, `EXPIRED_UNRESOLVED`) (§5.6).
3. **Typed accountability:** removed the global `V = Ê^α·Ŝ^β·Û^γ` score and consensus tiers; clients now apply local policies to non-transferable, class-specific histories (§5.7).
4. **Precise provenance:** renamed witnessed facts as operator attestations, narrowed anchored priority to commitment time, removed historical-liar inference from later re-fetches, and added log consistency, gossip, and artifact-availability requirements (§5.2–§5.3).
5. **Permission separation:** DNS proves control but grants no content rights; Level 2 carries a signed, scoped license; Level 0 creates no unclaimed balance (§6/§12).
6. **Finite discovery:** replaced universal indexing and Sybil-resistant-demand claims with capacity reservations, transparent queues, rate limits, and exposure-normalized exploration (§7).
7. **Coalition economics:** replaced the creator-only burn rule with full-cluster accounting, disclosed that the current 85/5/10 split does not prove wash resistance, and specified typed private receipts and refund behavior (§9).
8. **Implementation honesty:** standardized `SHIPPED`, `PARTIAL`, `SPECIFIED—UNIMPLEMENTED`, and `OPEN`; corrected creator-ledger, robots-policy, state-channel, and migration claims (§10/§13).
9. **Legal posture:** converted categorical claims into jurisdiction-specific design requirements and launch gates, including the limits of DNS consent, tombstoning, separate entities, and technical crawler signals (§12).
10. **Evidence base and gates:** added external references, measurable phase exit criteria, and an optional—not inevitable—token phase (§14/§16).

---

## Appendix D — Revision Notes: v2.2 → v2.3

1. **Uncertainty became the core algorithm:** added OpenUncertainty, with claim/task/horizon conditioning, deterministic, calibrated, and evidential-only modes, explicit ignorance, posterior intervals, and falsifiable calibration requirements (§5.8).
2. **Trust was separated from reputation:** typed source history can inform only comparable observable targets through a hierarchical model; capital, demand, popularity, and unaudited self-reports are forbidden confidence inputs (§5.7–§5.8).
3. **Dependence became first-class:** added citation/derivation graphs, duplicate collapse, citogenesis defenses, and interval widening for unresolved common ancestry (§5.8).
4. **Search became two-stage:** broad retrieval remains stake- and uncertainty-blind; a declared client risk policy may rerank only the top candidate set using calibrated claim-level reliance, conflict, ignorance, freshness, diversity, and licensed cost (§5.9).
5. **Web-scale path became explicit:** added a distributed ingestion/index/retrieval/evidence-join/inference pipeline, cache invalidation, latency fallbacks, open-source exploration, and concrete coverage, quality, cost, and tail-latency metrics (§7).
6. **Cold start was corrected:** new sources receive wide uncertainty intervals and exploration opportunities, not an inherited low-trust penalty; sponsorship never purchases posterior confidence (§8).
7. **Standards replaced proprietary overlap:** Level 2 references RSL for machine-readable licensing, imports C2PA records, and profiles PEAC for signed interactions and receipts (§6/§9).
8. **Prior art was strengthened:** added Aegon, selective fact-checking, generative-search citation evaluation, RSL, PEAC, and Pay Per Crawl; narrowed the novelty claim to the integrated dependency-aware uncertainty/search architecture (§16).
9. **Security and roadmap were rebuilt:** added calibration poisoning, label leakage, OOD overconfidence, uncertainty censorship, inference DoS, vertical calibration gates, and a measurable path from narrow-domain validation to general web search (§10/§14).
10. **Ambition and status were separated:** the paper now states Google-class, lower-noise search as a falsifiable long-term target while explicitly labeling the current algorithm and scale as unimplemented research hypotheses.

---

## Appendix E — Revision Notes: v2.3 → v2.4

1. **Economic conviction was distinguished from payment and purchased trust:** ordinary usage revenue, raw stake, and wealth remain excluded; only claim-specific, pre-outcome, genuinely losable exposure can become an estimator feature (§2 P4/§5.8.3).
2. **Cash became falsifiable:** its likelihood is learned against comparable no-cash outcomes, may be positive/zero/negative, becomes zero under insufficient calibration, and is capped at an MVP-1 odds effect of 3× (§5.8.3).
3. **Wealth and evasion defenses became explicit:** log amount saturation, loss/time/scope/funding/hedge factors, beneficial-cluster collapse, wealth-slice evaluation, and whale/self-funding/hedge attacks were added (§5.8.3/§10).
4. **The algorithm became executable:** added a Rust MVP with deterministic/calibrated/evidential-only modes, typed source history, intervals, Stage-2 reranking, calibration metrics, and focused unit tests (`crates/oi-index/src/uncertainty.rs`).
5. **Forward evaluation tooling was added:** the controlled synthetic test demonstrates signal recovery, negative-control behavior, whale resistance, and multi-million-estimate/second arithmetic; the real-cohort JSONL path rejects temporal leakage and reports cluster-bootstrap uncertainty while explicitly refusing to label the thesis validated without real evidence (`bench/src/uncertainty_bench.rs`; `docs/OPENUNCERTAINTY-MVP1.md`).
6. **Stage-1 cleanup progressed:** removed the surviving DNS multiplier from `compute_score`, made DNS verification relevance-neutral in tests, and retained the non-zero corpus and dependency audits as Phase A gates (§5.9/§13).

---

## Appendix F — Revision Notes: v2.4 → v2.5

1. **The narrow search architecture became end to end:** the live harness now exercises real ONNX embedding, persistent hybrid retrieval, web discovery, robots-aware crawling, distillation, foreground hydration, evidence-aware decision metadata, citation construction, and abstention without mocks.
2. **Intent now controls work, not truth:** navigation, transaction, current events, factual relations, due diligence, and exploration use different coverage and answer policies while preserving the original query as the identity/ranking scope. Neutral canonical rewrites let equivalent surface forms share discovery work.
3. **Discovery and serving were separated:** undated or snippet-only leads may be crawled, but current-event serving requires an effective date and factual serving requires an answer-bearing relation. External metadata never becomes retained claim evidence merely because a search engine returned it.
4. **Concurrency defects became regression tests:** provider cache fills and URL crawls use single-flight; followers reread the shared index; malformed transactional paths are rejected in favor of a conservative first-party root; claim-aware public-authority matching prevents an unrelated government page from suppressing the actual primary source.
5. **The factual relation gate became directional:** an entity named in a title cannot borrow an arbitrary creation/founding sentence from the body. Bounded grammatical context is allowed only for compatible anaphors or creator→generic-entity constructions, preventing “content created by Claude” from answering “who created Anthropic?”.
6. **Live scale evidence was recorded without overclaiming:** at the v2.5 boundary, two independent empty-index 64-request matrices passed at concurrency 32; a 192-request resident/revalidation matrix also passed. That revision covered only 16 semantic contracts, not broad web recall, sustained distributed load, cost, or market superiority.
7. **Cash remained an explicit null result:** every live query had zero eligible cash likelihood and stayed evidential where appropriate. v2.5 validates plumbing and conservative behavior; it does not validate the economic-conviction thesis.

---

## Appendix G — Revision Notes: v2.5 → v2.6

1. **A narrow claim graph became executable:** Brazilian poll pages are joined to the exact TSE/PesqEle registration retained in the answer-bearing citations. The answer exposes one underlying measurement, its editorial reporting families, and the official metadata record.
2. **Known dependence was separated from unresolved dependence:** `independence` discounts known derivatives even when their ancestry is resolved; `dependency_uncertainty` represents remaining ignorance. Repeated coverage of one poll can no longer masquerade as repeated measurements.
3. **Registry authority was scoped:** PesqEle verifies registration and methodology metadata. It does not verify the percentages copied by publishers, the realized sampling error, or absence of methodological bias; the answer says so in machine-readable lineage and human-readable warnings.
4. **The join was narrowed to the actual answer:** registrations are extracted only from pages cited for the provisional claim, preventing older retrieved polls from contaminating the answer with unrelated official records.
5. **Freshness and latency paths were corrected:** exact same-page recrawls refresh metadata without duplicating documents, stale retained sources are revalidated, fresh local poll evidence skips redundant hydration, and PesqEle records use a validated 24-hour persistent cache. A measured cross-process registry lookup fell from approximately 2.18 s cold to 3 ms cached.
6. **Factual discovery became bounded and multilingual:** relation/entity rewrites coalesce provider work across paraphrases and search both Portuguese and English canonical forms, eliminating an order-dependent cold failure for `who created Pix?` without relaxing the answer gate.
7. **No-mock validation was broadened and kept fail-closed:** the suite expanded from 17 to 24 semantic contracts (96 surface-form requests). The final empty-index matrix passed 96/96 with p99 5.94 s. Its first coherent three-pass resident/revalidation matrix passed 288/288 at 12.35 qps, p95 1.24 s, and p99 2.40 s. A separate nine-query Google election-poll candidate-set replay improved declared policy nDCG@5 from 0.8955 to 0.9985 and reduced hard-risk@3 from 18.52% to 3.70%, while honestly exposing a diversity regression from 2.44 to 2.22 independent clusters@3.
8. **Cash remains unvalidated:** every live contribution is still zero. v2.6 validates a richer evidence path, not the economic-conviction thesis, Google-scale coverage, or universal superiority.
9. **Navigational identity was hardened against resident-index poisoning:** an entity token in an arbitrary subdomain no longer impersonates the canonical origin, same-domain social/login/service routes do not replace the root destination, and persisted navigation metadata is rechecked under the current policy before reuse.
10. **Poll scenario coverage became explicit:** broad poll queries make one bounded, observable expansion for missing first- or second-round evidence even before a claim becomes extractive. The crawl slate reserves two reporting families per round plus one failure-margin slot. Added pages must still pass the normal two-family and lineage gates; otherwise the answer remains partial and says so.
11. **Tool intent gained a market-observed contract and its own route:** Portuguese and English word-order variants of “Python online compiler” share one discovery flight, reject the language homepage as the requested destination, require a functional tool surface plus top-three domain diversity, and do not treat a keyword-shaped hostname as authority. They are classified as non-claim-bearing `tool`, not entity navigation, and return a slate rather than an official-destination assertion.
12. **Provider health became semantic and capacity-aware:** valid URLs no longer suffice when a malformed or poisoned SERP is wholly unrelated to the query. A truthful Brave public-HTML adapter adds useful keyless coverage and passed short probes, but the sustained three-class, ten-round, concurrency-four gate found Bing healthy at 30/30 and Brave public HTML at only 12/30. The 1/2 redundancy gate still fails, and the serving path drops poisoned batches before crawl.
13. **Transactional routing follows first-party declarations:** the Claude homepage's published login link is parsed and target-checked, equivalent action queries share one probe, canonical URLs beat plan variants, and the cached route remains non-evidential metadata.
14. **A multi-intent Google snapshot gained reproducible quality and exploratory-latency evaluators:** nine canonical cases scored 74.1% for the dated Google slate and 100.0% for the final coherent-corpus OpenSonarX report under shared declared contracts, with four wins and five ties. A separate three-round raw-sample fixture records Google browser p50/p95/p99 of 215/407/414 ms and OpenSonarX backend `search_ms` of 1.96/31.02/2400.04 ms, while fail-closing `market_latency_compared=false` because the surfaces are not comparable. AI Overview, maps, shopping, knowledge panels, video, and ads remain outside the fixture comparator.
15. **Readiness became fail-closed and decomposed:** timestamped provider, cold, resident, and market artifacts feed one aggregate evaluator. The latest coherent run passes 16/17 search-core gates and fails only sustained provider redundancy. Four research claims remain explicitly unvalidated; generalized structured lineage passes across poll measurement, factual relation, current office-holder, and market-quote claims.
16. **Factual citation count no longer implies independence:** compatible factual answers share a stable claim signature across paraphrases and report editorial-family count separately from `known_independent_observations`. Until a derivation graph resolves syndication, quotation, and shared upstream sourcing, the factual lineage stays `unknown_editorial_derivation`.
17. **Poll scenario identity became evidence-audited:** an unqualified poll answer cannot silently inherit an explicitly alternate slate from the same page. The observed `39–32` alternate was replaced by the `39–33` primary scenario only after bounded discovery produced two qualifying editorial families and the normal registry-lineage gate; section boundaries prevent cross-slate percentage pairs.
18. **Successful acquisition and fresh evidence gained bounded reuse:** equivalent requests share a 30-second process-local answer acquisition entry only for policy-qualified outcomes. Fresh local serving requires intent-specific evidence snapshots; empty/provider-failure outcomes are not cached. Opportunistic background crawling suppresses an exact retained URL for five minutes while foreground claim revalidation keeps its stricter policy. Unsupported thematic news does not block on hydration, due diligence no longer repeats the same discovery matrix, incomplete factual completion respects its hydration budget, and crawl batches are ingested atomically without weakening primary-source or poll repair.
19. **The live PesqEle join became resilient to transient JSF emptiness:** one fresh-session retry is allowed, context selection favors maximal shared terms, and institute editorial discovery no longer depends on the registry listing succeeding. When retained pages already identify the institute, broad registry enumeration is removed from the latency-critical discovery path while the exact registration remains mandatory at the final join. The official record remains metadata evidence, never validation of publisher percentages.
20. **Factual abstention now preserves only relation-legible leads:** title/snippet and retained-body predicates are separated, subject/object direction is enforced, `Founders Fund` and unrelated creator-profile collisions are rejected, final decision ranks are normalized while retrieval ranks are preserved, and a bounded MediaWiki reference fallback never counts as web-provider redundancy or independent corroboration.
21. **Public-HTML bootstrap capacity is not confused with a contract:** Bing, Brave public HTML, and DuckDuckGo are staged rather than fired simultaneously. Brave's public adapter uses a truthful OpenSonarX identifier, remains distinct from the authenticated API, and is concurrency-bounded; its sustained 429 behavior remains a red production gate.
22. **Primary citations are promoted only after claim-aware matching:** a cited official URL is re-read by exact URL and placed first only when the claim itself identifies the matching authority. This made the Banco Central Pix source visible at rank one without introducing a generic government-domain boost.
23. **Definition reference fallback no longer depends on an empty slate:** a shallow local exploratory result can coexist with bounded reference leads until the requested depth is met; the reference family still cannot count as independent corroboration.
24. **Real failover is tested without a primary provider:** the no-Bing chaos run used only Brave public HTML and DuckDuckGo and passed 17/24 canonical contracts before rate limiting. Safe abstention worked; independent provider availability did not.
25. **Credentialed availability and disjoint generalization were measured separately:** Bing and Serper each passed 30/30 sustained attempts. After burning every inspected diagnostic holdout, v8 reached 48/48; v9 exposed a 45/48 compact-intent regression; and v10 returned to 48/48. Later v10–v13 captures add pooled system-blind judgments rather than treating coverage as quality.
26. **Trust reranking gained a relevance invariant:** Stage 2 now exposes retrieval and decision rank and may reorder only within a declared top-relevance band. A retained or easier-to-audit page can no longer outrank a materially more relevant candidate merely because both lack calibrated evidence.
27. **Blind comparison became fail-closed:** raw market and OpenSonarX captures bind to one sealed manifest; pooling canonicalizes and randomizes candidates without system identity; judgments must be complete and hash-bound; scoring uses a common pooled IDCG and paired bootstrap intervals. The superiority claim requires 1,000 queries, eight strata, and two independent external blinded judges, so the current local-model diagnostic cannot pass it.
28. **Cold-source roles became query-relative:** primary sources, edited references, unclassified sources, and unverified UGC can break otherwise-identical evidential ties only inside the relevance band. The role is not a universal domain label, not `p_reliable`, and not a financial signal. V12/v13 Stage 2 raised acceptable top-one counts from 24→34 and 21→32 respectively.
29. **Candidate depth is preserved before trust:** a bounded internal pool retains top-k depth per configured provider before the final top-k decision. V10/v11 lost clearly to raw Serper; the further-disjoint v12/v13 runs repeated parity on acceptable top-one and relevance nDCG while remaining slightly behind on authority nDCG. All confidence intervals included zero, so this is competitiveness evidence in one narrow stratum, not general superiority.
30. **Ambiguous routing no longer disables reliance:** explanatory `A vs B` forms are not live events, and a short query receives navigation's `not_applicable` uncertainty only when a canonical entity host or public-authority path verifies the destination. Otherwise the answer still refuses an official-site claim while the candidate slate receives evidential reliance assessment.

---

## Appendix H — Revision Notes: v2.6 → v2.7

1. **The product and mission are explicit:** general search for people and agents, with claim-level evidence and warranted reliance as the core. Publisher economics and training delivery extend the product without replacing that mission (§1.5).
2. **The creator chooses the return:** free access, visible attribution, paid licensed use, and separate training terms. Citation does not guarantee traffic or create a debt without accepted terms (§6.1, §9.1.1).
3. **Discovery and delivery are separated:** approved catalogs may expose protected resources' existence and terms. Private discovery remains optional, while restricted bodies, extracts, embeddings, and caches need authorization boundaries (§6.2).
4. **Exclusive-discovery promises are rejected:** neither encryption, blockchain, nor licensing prevents every external copy, independently reported observation, or unauthorized downstream use (§6.2, §11, §12.1).
5. **An open evidence-unit envelope is specified:** contextual claims preserve originals, locators, transformations, dependencies, corrections, and rights. They are not certified facts or a proprietary replacement for the web (§7.1).
6. **Multimodal and efficiency claims stay conditional:** adapters can preserve page spans, image regions, and audiovisual time ranges. Extraction remains fallible, and reuse must beat full-document and ordinary-chunk baselines on total cost and quality (§7.2).
7. **Training is a separate licensed research extension:** original-data preservation and dependency-aware selection may help, but recursive-training outcomes depend on the workflow. No prevention of model collapse or universal hallucination cure is claimed (§1.6, §7.3).
8. **Receipts distinguish zero-price and dataset delivery:** neither free attribution nor dataset delivery proves paid demand, actual training, or downstream compliance (§9.1). Source compensation remains separate from bonded conviction.
9. **Leverage precedes token dependencies:** an operator-run useful service can use replaceable suppliers, open standards, and outside connectors. Optional blockchain settlement and native-token rewards require distinct necessity and safety cases (§9.5).
10. **Security and rollout gates expand:** protected-cache leakage, origin forgery, and zero-price reward farming enter the threat model. A bounded text/data publisher pilot precedes broad media and training exports (§10, §14.1).
11. **Competitive overlap is updated:** Parallel, Cleanlab, OriginTrail, Factland, UMA, Desearch, NewsGuard, and Cloudflare constrain novelty claims. The comparison is documentary, not a new performance benchmark (§16).
12. **Implementation evidence is unchanged:** this revision adds specifications and evaluation requirements. It does not deploy payments, a token, protected delivery, multimodal evidence units, or a training marketplace. August search results remain historical and all recorded live cash effects remain zero.

---

## Appendix I — Revision Notes: v2.7 → v2.8

1. **Marginal cash is retracted from probabilities:** diagnostic association cannot be summed onto source history. A future conditional/joint model uses the same no-cash features and independent activation gates (§5.8.3). The current contribution is zero even with dense bucket data.
2. **Evidence volume cannot erase a fixed gap:** bounded descriptive mass and a generic critical-missing gate have local regression tests. Full critical-checklist integration in search is still pending (§5.8.1).
3. **Economic roles have testable choices:** P0 initially pays for authorized delivery with explicit cache and rights rules, not inferred causal credit or manufactured citations (§9.1). Protected serving remains specified.
4. **Challenge rewards include investigation:** US$30 proposed bounty gives US$18 net after illustrative US$10 investigation, with precision and unresolved-rate sensitivity. Conservation is necessary, not sufficient (§9.6).
5. **Pricing includes stress rather than attractive omissions:** full detection reprices the guarantee, and incremental query-cost scenarios invalidate the old US$0.02 margin where appropriate. Numbers remain assumptions and do not establish demand, solvency or audited payouts (§9.6).
6. **The first product has a finite scope:** text/data operator-run API first, protected P0 next, with training, broad multimodal and token dependencies deferred (§14.1).
7. **Activation is measurable:** full-answer matched-coverage quality, fidelity, cold latency, actual paid reuse, complete cost and conditional cash receive separate pre-registration/stop gates. Roles must be filled before the pilot; missing evidence produces an inconclusive result, not a success (§14.1).
8. **Operator trust stays visible:** model, label, policy and access decisions remain operator-controlled. Public settlement does not decentralize them (§14.1).
9. **Historical evidence is preserved:** August search figures and the retired synthetic result remain historical; no new market-quality claim or financial deployment accompanies this revision.
