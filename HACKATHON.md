# OpenSonarX — Colosseum Fall 2026

**A search index for AI agents where every source arrives with accountability:
what independent sources corroborate or contradict it, and what its publisher
has bonded behind it on Solana.**

The agent writes the answer. We deliver the sources and the signals it needs
to decide which ones to trust.

## The insight

We spent the last week of the hackathon testing our own product against the
obvious alternative, on the live web: 480 queries, the protocol committed
before each run, and every response read and graded one by one. The grader
was an AI reviewer (Claude) working from a criterion fixed beforehand and
blind to the variable under test; there is no human second reader yet, and
we treat the numbers accordingly.

- **Agents already read well.** Given the same pages, a single model call
  answered as accurately as our full claim-and-citation pipeline, in half the
  time. On 240 live queries neither beat the other
  ([live head-to-head](docs/experiments/LIVE-HEAD-TO-HEAD-2026-10-10.md),
  [misinformation head-to-head](docs/experiments/MISINFORMATION-HEAD-TO-HEAD-2026-10-10.md)).
- **The errors come from the pages.** When an answer was wrong, the wrong
  sentence was on the page it was taken from. Checking that a quote exists
  does not help when the source itself is wrong.
- **Agreement between independent sources predicts correctness.** On a fresh
  sample of 240 live queries, answers that two or more independent source
  families supported were wrong 7.9% of the time; the rest were wrong 20.1%
  ([trust label](docs/experiments/TRUST-LABEL-2026-10-10.md)). The margin is
  narrow (bootstrap 95% lower bound of the difference: +1.0 point) and it
  comes from one AI reader, so we call it a first result, not a law.

- **The work can move to ingestion, and the signal survives.** We extracted
  each page's claims once, when the page entered the index, verified every
  quote, and computed corroboration at query time from the stored claims
  alone: one query embedding and one vector lookup, no model. Query-side
  cost: 3.6 ms at the median, against about 13 s. On the held-out test,
  answers carrying that label were wrong 8.2% of the time and the rest
  22.4% (bootstrap 95% of the difference: +4.1 to +24.3 points)
  ([claims at ingestion](docs/experiments/INGEST-CLAIMS-2026-10-11.md)).
  The index held exactly the pages those queries had retrieved, so this is
  the covered case; it says nothing yet about coverage.

So the missing piece in search for agents is not a better answer. It is a
trust signal on each source, computed before the question is asked, and
someone who is accountable for it. That is what the index, its nodes and the
Solana program are for.

## What runs today

```bash
cargo build --release --bin oi

# Sources for an agent, no key needed (DuckDuckGo + Brave HTML discovery,
# robots-aware fetch, clean text, local hybrid index that grows per query)
./target/release/oi --open search "how long does it take to drive from nyc to washington dc"

# The same search with the evidence report: which claims the sources make,
# the exact quote behind each, and where sources contradict each other.
# Reads pages with a model you choose (OpenRouter or a local Ollama model).
export OPENROUTER_API_KEY=...        # your key
OI_ANSWER_PROVIDER=openrouter OI_ANSWER_MODEL=anthropic/claude-haiku-5.5 \
  ./target/release/oi --open search "how long does it take to drive from nyc to washington dc" --answer
```

On that query the report shows two sources in conflict, each with its quote:
one page says 5 hours 43 minutes, another says about 4 hours. A plain
search-plus-model pipeline answered "5 hours 43 minutes" in our test. An agent
reading the evidence report sees the conflict instead.

`examples/python/agent_reads_evidence.py` is the other half: a minimal agent
that reads that report and decides what it is willing to say ("confirmed by
three independent sources", "sources disagree", "one source, unverified").

Every quote in the report is located character-for-character in the retained
page text by deterministic code before it can be shown. A model proposes;
code verifies.

## Solana

`programs/opensonarx-staking` is a native Solana program (no Anchor):

- bond SOL or the protocol token to a domain you publish (`Stake`, `StakeV2`),
  with a cooldown before withdrawal;
- open a dispute against a bonded domain with a bond of your own, commit and
  reveal jury votes, slash the losing side;
- reward accounting, governance proposals, sentinel registration, payment
  channels, a reputation root.

`tests/integration/solana_e2e.rs` runs 84 tests against the program inside
the Solana runtime (`solana-program-test`), including one that goes from a
stake transaction to a search result carrying that stake:

```bash
cargo test --manifest-path tests/integration/Cargo.toml --test solana_e2e
```

**Deployment status.** The program builds to SBF and has been deployed to a
local validator, where a real `Stake` transaction bonded SOL to a domain, the
stake account was read back from the chain, and the search showed the bond.
It is not on devnet yet: the faucet refused the deploy wallet on 11 October.
Details and the one-command reproduction are in
[docs/hackathon/DEVNET.md](docs/hackathon/DEVNET.md).

**What the bond does and does not do.** A bond never raises a result's rank
and never buys a probability. It is shown next to the source (the `BOND`
column) and it is what a successful dispute takes away. Whether money at
stake makes publishers more accurate is a hypothesis we have not tested yet:
there are no external stakers.

## What is not built

- Our own index is small. Discovery leans on third-party search and the
  index grows from the pages each query fetches.
- Ingestion-time claims live in the SDK (`oi_sdk::claims`) and were measured
  by feeding stored pages through it. They are not yet wired into the crawl
  pipeline, and the `--answer` report still uses the slower query-time path.
- Serving that signal from several nodes is not solved. We split the claims
  across three real nodes and queried one: the fan-out works and costs about
  2 ms on localhost, but the result differed from the single-node one on 16
  of 240 queries and the label lost its edge. The nodes' page search ranks
  by a per-node score and returns one result per URL; claims need a plain
  vector lookup with an absolute score, and the response has to carry the
  vectors. That is protocol work we have not done.
- Nodes checking each other's ingestion work on chain is design, not code.
  What makes it tractable is that the check is mechanical: a quote is in the
  page with that hash, or it is not.
- The uncertainty engine (`crates/oi-index/src/uncertainty.rs`) only emits a
  probability when it has resolved outcomes for a source. It has none yet, so
  every result is labelled `evidential_only`.
- No users, no revenue.

## Development history

This repository predates the hackathon. To be explicit:

- **Before the hackathon (February–August 2026):** the hybrid index
  (HNSW + BM25), crawler and distiller, P2P layer, CLI, the staking program
  and its tests, the uncertainty engine, whitepaper v2, and an earlier
  rule-based answer layer.
- **During the hackathon:** removal of the rule-based answer layer and its
  replacement with model-backed evidence traits verified by deterministic
  quote location; hosted model backend; the evaluation tooling
  (`oi-bench answer-eval`, `answer-live`, blind grading sheets); three live
  experiments registered before they ran (480 queries, 455 graded
  responses); the trust-label result; claim extraction at ingestion time
  and its registered test; the default-discovery fix; the on-chain bond
  client; this document and the devnet work.

`git log` has the dates. Several October 9 commits bundle work that had been
sitting uncommitted since August; their messages say "pending changes".
