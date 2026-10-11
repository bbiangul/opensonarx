#!/usr/bin/env python3
"""Blind reading sheet for the claim-relation judge (INGEST-CLAIMS step 7).

Takes the pairs (head claim, candidate claim) the judge saw for the heads of
the given reports' queries, samples up to N judged `same` and N judged
otherwise in order of a hash of the pair, shuffles them by a second hash,
and writes:

  <out>-sheet.jsonl   {"n", "a", "b"}            what the reader sees
  <out>-key.jsonl     {"n", "relation", ...}     what the judge said

Usage:
  claim_relation_sheet.py sheet <judged.json> <relations.jsonl> <out-prefix> <per-group> <report-stem> [...]
  claim_relation_sheet.py score <out-prefix>-key.jsonl <reader-labels.jsonl>

Reader labels: {"n", "relation"} with relation in same | contradicts | different.
"""
import hashlib
import json
import sys


def digest(*parts):
    return hashlib.sha256("\n".join(parts).encode()).hexdigest()


def sheet(judged_path, relations_path, prefix, per_group, stems):
    heads = set()
    for result in json.load(open(judged_path))["results"]:
        if result["report"] in stems and result.get("head"):
            heads.add((result["head"]["url"], result["head"]["claim"]))
    pairs = {}
    for line in open(relations_path):
        row = json.loads(line)
        if "error" in row or (row["head"]["url"], row["head"]["claim"]) not in heads:
            continue
        for candidate in row["candidates"]:
            key = digest(row["head"]["url"], row["head"]["claim"], candidate["url"], candidate["claim"])
            pairs[key] = {
                "a": row["head"]["claim"],
                "b": candidate["claim"],
                "a_url": row["head"]["url"],
                "b_url": candidate["url"],
                "relation": candidate["relation"],
            }
    same = sorted(k for k, p in pairs.items() if p["relation"] == "same")[:per_group]
    other = sorted(k for k, p in pairs.items() if p["relation"] != "same")[:per_group]
    chosen = sorted(same + other, key=lambda k: digest("sheet", k))
    with open(f"{prefix}-sheet.jsonl", "w") as sheet_file, open(f"{prefix}-key.jsonl", "w") as key_file:
        for n, key in enumerate(chosen, 1):
            pair = pairs[key]
            sheet_file.write(json.dumps({"n": n, "a": pair["a"], "b": pair["b"]}, ensure_ascii=False) + "\n")
            key_file.write(json.dumps({"n": n, **pair}, ensure_ascii=False) + "\n")
    total_same = sum(p["relation"] == "same" for p in pairs.values())
    print(f"pairs for these heads: {len(pairs)} ({total_same} judged same); sheet: {len(same)} same + {len(other)} other")


def score(key_path, labels_path):
    key = {row["n"]: row for row in map(json.loads, open(key_path))}
    reader = {row["n"]: row["relation"] for row in map(json.loads, open(labels_path))}
    missing = sorted(set(key) - set(reader))
    if missing:
        sys.exit(f"reader labels missing for {missing}")
    table = {}
    for n, row in key.items():
        table[(row["relation"], reader[n])] = table.get((row["relation"], reader[n]), 0) + 1
    names = ["same", "contradicts", "different"]
    print("judge \\ reader   " + "  ".join(f"{name:>11}" for name in names))
    for judge in names:
        print(f"{judge:>14}   " + "  ".join(f"{table.get((judge, r), 0):>11}" for r in names))
    judged_same = [n for n, row in key.items() if row["relation"] == "same"]
    judged_other = [n for n, row in key.items() if row["relation"] != "same"]
    agree_same = sum(reader[n] == "same" for n in judged_same)
    missed = sum(reader[n] == "same" for n in judged_other)
    print(f"judged same, reader agrees: {agree_same}/{len(judged_same)}")
    print(f"judged not same, reader says same: {missed}/{len(judged_other)}")
    exact = sum(reader[n] == row["relation"] for n, row in key.items())
    print(f"three-way agreement: {exact}/{len(key)}")


if __name__ == "__main__":
    if sys.argv[1] == "sheet":
        sheet(sys.argv[2], sys.argv[3], sys.argv[4], int(sys.argv[5]), set(sys.argv[6:]))
    else:
        score(sys.argv[2], sys.argv[3])
