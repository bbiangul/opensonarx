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

## See all of it run

One command starts three nodes, a dishonest node and (unless pointed at
devnet) a local Solana validator, walks through eight acts and ends each
with checks computed from what the processes and the chain answered. Pages are fetched and read while it runs;
nothing is replayed.

```bash
export OPENROUTER_API_KEY=...     # the model is used at ingestion only
scripts/demo.py --keep            # --keep leaves the live pages up
```

It needs Rust, Python 3, an [OpenRouter](https://openrouter.ai) key and, for
the acts on chain, the [Solana CLI](https://docs.anza.xyz/cli/install) (or
pass `--skip-solana`, which leaves out the bonds and the slash). The first run compiles everything and downloads a small
embedding model; after that a run takes about two minutes. On the local
validator the script makes its own wallet and program address, so it uses
nothing of yours.

| Act | What happens | What is checked |
|---|---|---|
| 1 | An agent searches the open web | sources from three or more independent sites |
| 2 | The index is divided by subject, learned from its own pages | a versioned routing file; an edited file is refused |
| 3 | Two nodes ingest only their own subjects and read each page once for claims | both hold claims, each of its own shard |
| 4 | A question is answered from stored claims | same answer from both nodes, no claim held twice, claims fetched from the other node, milliseconds |
| 5 | A replica with no model joins and requires a bond | nothing copied from a node until its operator locks a bond on chain; then every claim, each quote found in its own copy of the page |
| 6 | A bonded operator's node offers a forged claim | the replica refuses exactly that claim, keeps the honest one, and records the evidence with the operator's wallet |
| 7 | An outside process re-fetches the pages, and the bond is slashed | every honest quote found again; no quote from another page accepted; the forged quote absent from the live page; a slash signed by anyone but the authority rejected; the dishonest operator's bond at zero on chain and the honest one untouched; the replica no longer copies from that operator |
| 8 | A publisher bonds SOL behind a domain | read back from the chain; the running node shows it; the returned claims and their order do not change |

A full run on devnet, 25 of 25 checks:
[docs/hackathon/DEMO-RUN.txt](docs/hackathon/DEMO-RUN.txt). Add
`--rpc https://api.devnet.solana.com` to use devnet with your own funded
wallet; without it the acts on chain use a local validator.
While the nodes are up, each serves a live page at `/demo` showing what it
holds, its peers, what it refused, the bonds it knows, and what each source
states about a question.

**The money loop, and where it stops.** A node's network identity is its
operator's Solana wallet key, so a peer reads the wallet from the peer id
it is already talking to. A node that requires bonds copies only from
operators whose bond account is in good standing. When it refuses a claim
it keeps the claim, the quote, the page and the operator's wallet. In the
demo an auditor repeats the check from the web and the bond is slashed on
chain, after which the replica stops copying from that operator. What is
decided on chain is custody and the slash itself. What is not: the slash
is sent by the protocol authority, and the program takes its word. One
failed quote would not be enough in production, because pages change (about
1 in 100 honest quotes fails a re-fetch a day later, by our measurement).
The bond is in a test token.

What the demo does not show, because it is not built: nodes on separate
machines, a slash that anyone can trigger and the chain itself checks,
payment to nodes for ingestion work, and a judgment of whether two sources
state the same fact (the count is of sources with a claim close to the
question; see "What is not built").

Building the demo found five defects in the network layer that only show
with three or more real nodes, all fixed with tests: a node's identity taken
from whoever forwarded its gossip, shard lists that never reached late
joiners, a replica dropping honest content through a crawl-time filter,
routing that ignored its own ranking, and heartbeats erasing shard lists.

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

**Deployment status.** The program is on Solana devnet at
[`5psb7XSg7ZkzPXuD9GirjYRyz3Nh5CbXEQLg84vpuE9c`](https://explorer.solana.com/address/5psb7XSg7ZkzPXuD9GirjYRyz3Nh5CbXEQLg84vpuE9c?cluster=devnet),
with the protocol initialized. The demo has run end to end against it: two
node operators locked bonds, one was slashed after its node was caught
offering a forged claim, and a publisher's 0.25 SOL bond on a domain was
read back from the chain and shown by the running node. Signatures and the
one-command reproduction are in
[docs/hackathon/DEVNET.md](docs/hackathon/DEVNET.md). It is devnet, with
test SOL; nothing is on mainnet.

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
- Claims are served by the network layer but not by a production network.
  A node now answers a claims request with a plain vector lookup and the
  entry node merges by absolute similarity. Split across three real nodes on
  localhost the signal costs 4.7 ms at the median and still separates right
  from wrong (6.7% against 23.1%). Claims are saved with the index and
  replicated with their pages, and a replica keeps a claim only if it finds
  the quote in its own copy of the page. Two nodes run as separate processes
  on one machine converge on their own: one learned pages and extracted 109
  claims, the other, with no model, held the same 109 about a minute later
  and answered the same queries identically. No two nodes have run on
  different machines, and extraction is not yet part of the crawler.
- Dividing the index by subject lets a query ask few nodes. With ten
  subject shards and claims kept with their page, as a node keeps them,
  asking three gave the same label as an exhaustive scan on 231 of 240
  queries; dividing by URL hash needs all ten. Centroids learned from the
  stored pages do better (236 of 240, and 227 asking a single shard). A
  node can now learn such centroids from its index and route with them
  (`oi network learn-routing`, `oi network node --routing`); two node
  processes sharing one routing file each kept only their shards and
  answered twelve queries identically, pulling from each other what they
  did not hold. Who publishes that file for a whole network is not solved.
  Reading this code we also found the node was not following its router's
  ranking when choosing which shards to ask, and fixed it.
- The label counts sources with a similar claim, not sources judged to
  agree. We tried replacing the similarity cut with a model's judgment made
  at ingestion (same fact, contradicts, different). It did not help: on the
  test answers the judged label was wrong 3 of 31 times against 28 of 146
  without it, an interval that includes zero, at half the coverage of the
  current label. It is in the code behind a trait and not in the served
  path.
- Nodes are checked and can lose a bond, but not yet without trust in an
  authority. A replica refuses a claim whose quote is not in the page, keeps
  the evidence, and the operator's bond can be slashed on chain (see the
  demo); the slash is sent by the protocol authority, which the program
  does not second-guess, and nobody is paid for honest work yet.
  What makes a trustless version tractable is that the check is mechanical: a quote is in the
  page, or it is not. We measured it with a second node fetching 300 pages
  itself a day later: none of 3,056 quotes taken from a different page was
  accepted, and 99.0% of the honest claims were found again (95% interval
  98.3% to 99.6%). That is enough to reject fabricated quotes and not
  enough to slash on a single failed claim: pages change, and 2% of them
  could not be fetched at all.
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
