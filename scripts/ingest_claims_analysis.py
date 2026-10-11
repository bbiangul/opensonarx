#!/usr/bin/env python3
"""Analysis for docs/experiments/INGEST-CLAIMS-2026-10-11.md.

dev:  choose the two similarity thresholds on the development queries only,
      from a `claim-signal` output that lists the fetched claims.
test: score a `claim-signal` output produced with the frozen thresholds
      against the reader labels of the test queries.

Usage:
  ingest_claims_analysis.py dev  <signals.json> <labels.jsonl> [<labels.jsonl> ...]
  ingest_claims_analysis.py test <signals.json> <labels.jsonl> [<labels.jsonl> ...] [--pipeline <report.json> ...]
  ingest_claims_analysis.py judged <claim-judged.json> <labels.jsonl> [<labels.jsonl> ...]

A labels file is matched to a report by name: `answer-live-<x>-reader-labels.jsonl`
labels the queries of `answer-live-haiku-5.5-<x>.json`.
"""
import json
import random
import sys


def load_labels(paths):
    right = {}
    for path in paths:
        tag = path.rsplit("/", 1)[-1].removeprefix("answer-live-").removesuffix("-reader-labels.jsonl")
        for line in open(path):
            row = json.loads(line)
            if row["system"] != "baseline":
                continue
            ok = row["correct"] if "correct" in row else row["verdict"] == "C"
            right[(tag, row["id"])] = ok
    return right


def rows(signals, right):
    out = []
    for result in signals["results"]:
        tag = result["report"].removeprefix("answer-live-haiku-5.5-")
        key = (tag, result["id"])
        if key in right:
            out.append((result, right[key]))
    return out


def families(matches, min_query, min_claim, min_families=2):
    if not matches or matches[0]["query_similarity"] < min_query:
        return 0
    return len(
        {
            m["source_family"]
            for m in matches
            if m["query_similarity"] >= min_query and m["head_similarity"] >= min_claim
        }
    )


def rates(pairs):
    """pairs: (labelled, right). Returns with/without counts and error rates."""
    with_ = [r for l, r in pairs if l]
    without = [r for l, r in pairs if not l]
    err = lambda xs: (sum(not r for r in xs) / len(xs)) if xs else float("nan")
    return len(with_), sum(not r for r in with_), err(with_), len(without), sum(not r for r in without), err(without)


def quantile(values, q):
    values = sorted(values)
    return values[round((len(values) - 1) * q)]


def dev(signals_path, label_paths):
    signals = json.load(open(signals_path))
    data = rows(signals, load_labels(label_paths))
    heads = [r["matches"][0]["query_similarity"] for r, _ in data if r["matches"]]
    others = [m["head_similarity"] for r, _ in data for m in r["matches"][1:]]
    query_grid = [round(quantile(heads, q), 3) for q in (0.25, 0.5, 0.75)]
    claim_grid = [round(quantile(others, q), 3) for q in (0.25, 0.5, 0.75)]
    print(f"development answers: {len(data)}; right {sum(ok for _, ok in data)}")
    print(f"query-similarity candidates (quartiles of the head claim): {query_grid}")
    print(f"claim-similarity candidates (quartiles of the other fetched claims): {claim_grid}")
    best = None
    for q in query_grid:
        for c in claim_grid:
            pairs = [(families(r["matches"], q, c) >= 2, ok) for r, ok in data]
            n1, e1, r1, n0, e0, r0 = rates(pairs)
            coverage = n1 / len(pairs)
            eligible = 0.2 <= coverage <= 0.8
            diff = r0 - r1 if n1 and n0 else float("nan")
            print(
                f"  query>={q:.3f} claim>={c:.3f}: with {e1}/{n1} wrong ({r1:.1%}), without {e0}/{n0} ({r0:.1%}), "
                f"difference {diff:+.1%}, coverage {coverage:.0%}{'' if eligible else '  (outside 20-80% coverage)'}"
            )
            if eligible and (best is None or (diff, coverage) > (best[0], best[1])):
                best = (diff, coverage, q, c)
    if best is None:
        print("no combination has coverage between 20% and 80%")
    else:
        print(f"CHOSEN: --min-query-similarity {best[2]} --min-claim-similarity {best[3]}")


def test(signals_path, args):
    pipeline_reports = []
    if "--pipeline" in args:
        split = args.index("--pipeline")
        args, pipeline_reports = args[:split], args[split + 1 :]
    signals = json.load(open(signals_path))
    data = rows(signals, load_labels(args))
    pairs = [(r["corroborated"], ok) for r, ok in data]
    n1, e1, r1, n0, e0, r0 = rates(pairs)
    print(f"policy: {signals['policy']}")
    print(f"test answers: {len(pairs)}; right {sum(ok for _, ok in pairs)}")
    print(f"with label:    {e1}/{n1} wrong ({r1:.1%})")
    print(f"without label: {e0}/{n0} wrong ({r0:.1%})")
    random.seed(20261011)
    diffs = []
    for _ in range(10000):
        sample = [random.choice(pairs) for _ in pairs]
        s1, _, sr1, s0, _, sr0 = rates(sample)
        if s1 and s0:
            diffs.append(sr0 - sr1)
    diffs.sort()
    print(
        f"difference {r0 - r1:+.1%}, bootstrap 95% [{diffs[int(0.025 * len(diffs))]:+.1%}, {diffs[int(0.975 * len(diffs)) - 1]:+.1%}]"
    )
    print(f"label coverage: {n1}/{len(pairs)} answers ({n1 / len(pairs):.0%})")
    summary = signals["summary"]
    print(f"claims in the index: {summary['claims']}; embedder: {summary['embedder']}")
    for name in ("query_embed", "claim_lookup", "query_total"):
        s = summary[name]
        print(f"  {name}: p50 {s['p50_ms']:.2f} ms, p95 {s['p95_ms']:.2f} ms, max {s['max_ms']:.2f} ms (n={s['n']})")
    if pipeline_reports:
        query_time = {}
        for path in pipeline_reports:
            tag = path.rsplit("/", 1)[-1].removeprefix("answer-live-haiku-5.5-").removesuffix(".json")
            for result in json.load(open(path))["results"]:
                p = result.get("pipeline", {})
                query_time[(tag, result["id"])] = p.get("status") == "extractive" and (p.get("min_supporting_families") or 0) >= 2
        both = [(r["corroborated"], query_time.get((r["report"].removeprefix("answer-live-haiku-5.5-"), r["id"]), False)) for r, _ in data]
        table = {(a, b): sum(1 for x in both if x == (a, b)) for a in (True, False) for b in (True, False)}
        print(f"agreement with the query-time label (ingest, query-time): both {table[(True, True)]}, ingest only {table[(True, False)]}, query-time only {table[(False, True)]}, neither {table[(False, False)]}")


def interval(pairs):
    random.seed(20261011)
    diffs = []
    for _ in range(10000):
        sample = [random.choice(pairs) for _ in pairs]
        s1, _, sr1, s0, _, sr0 = rates(sample)
        if s1 and s0:
            diffs.append(sr0 - sr1)
    diffs.sort()
    return diffs[int(0.025 * len(diffs))], diffs[int(0.975 * len(diffs)) - 1]


def judged(signals_path, label_paths):
    """Score a `claim-judged` output: the judged label and, on the same
    answers, the embedding-similarity label it is compared with."""
    signals = json.load(open(signals_path))
    data = rows(signals, load_labels(label_paths))
    print(f"answers: {len(data)}; right {sum(ok for _, ok in data)}; judge {signals['summary']['judge']}")
    for name, field in (("judged same by the model", "judged_corroborated"), ("similar by embedding", "similar_corroborated")):
        pairs = [(r[field], ok) for r, ok in data]
        n1, e1, r1, n0, e0, r0 = rates(pairs)
        low, high = interval(pairs)
        print(f"{name}:")
        print(f"  with label:    {e1}/{n1} wrong ({r1:.1%})")
        print(f"  without label: {e0}/{n0} wrong ({r0:.1%})")
        print(f"  difference {r0 - r1:+.1%}, bootstrap 95% [{low:+.1%}, {high:+.1%}]; coverage {n1}/{len(pairs)} ({n1 / len(pairs):.0%})")
    table = {}
    for r, ok in data:
        key = (r["judged_corroborated"], r["similar_corroborated"])
        n, wrong = table.get(key, (0, 0))
        table[key] = (n + 1, wrong + (not ok))
    for key, label in (((True, True), "both"), ((True, False), "judged only"), ((False, True), "similar only"), ((False, False), "neither")):
        n, wrong = table.get(key, (0, 0))
        print(f"  {label}: {wrong}/{n} wrong")
    contested = [ok for r, ok in data if r["contested"]]
    print(f"contested (another family contradicts the head): {sum(not ok for ok in contested)}/{len(contested)} wrong")
    both = [ok for r, ok in data if r["contested"] and r["judged_corroborated"]]
    print(f"  of which also judged-corroborated: {sum(not ok for ok in both)}/{len(both)} wrong")


if __name__ == "__main__":
    mode, signals_path, rest = sys.argv[1], sys.argv[2], sys.argv[3:]
    {"dev": dev, "test": test, "judged": judged}[mode](signals_path, rest)
