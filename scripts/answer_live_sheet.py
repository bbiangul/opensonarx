#!/usr/bin/env python3
"""Blind grading sheet for an `oi-bench answer-live` report.

Every response either system emitted becomes one item: the request, the
dataset's reference answers, the response and the spans it cites. Items carry
an opaque key and are ordered by it, so the sheet says neither which system
wrote a response nor what status it had. The key-to-system map goes to a
separate file that is only read after grading.

When a cases file is given, each item also lists the false answers the
dataset records for that request.

A fifth argument restricts the sheet to some systems (`baseline`,
`pipeline`, or both separated by a comma); pass `-` as the cases file when
there is none.

Usage: answer_live_sheet.py <report.json> <sheet.jsonl> <key-map.jsonl> [cases.jsonl|-] [systems]
"""
import hashlib
import json
import sys


def main() -> None:
    report_path, sheet_path, map_path = sys.argv[1:4]
    false_answers = {}
    systems = set(sys.argv[5].split(",")) if len(sys.argv) > 5 else {"pipeline", "baseline"}
    if len(sys.argv) > 4 and sys.argv[4] != "-":
        for line in open(sys.argv[4]):
            if line.strip():
                case = json.loads(line)
                false_answers[case["id"]] = case.get("known_false_answers") or []
    report = json.load(open(report_path))
    stem = report_path.rsplit("/", 1)[-1].removesuffix(".json")
    items = []
    for result in report["results"]:
        if "search_error" in result:
            continue
        pipeline = result["pipeline"]
        emitted = []
        if pipeline["status"] in ("extractive", "partial_answer"):
            spans = [
                citation["excerpt"]
                for claim in pipeline["claims"]
                if claim["status"] == "corroborated"
                for citation in claim["citations"]
                if citation["role"] == "supports" and citation["excerpt"]
            ]
            emitted.append(("pipeline", pipeline["summary"], spans))
        baseline = result["baseline"]
        if baseline.get("answered") and baseline.get("answer", "").strip():
            emitted.append(("baseline", baseline["answer"], [q["quote"] for q in baseline["quotes"]]))
        for system, response, spans in emitted:
            if system not in systems:
                continue
            key = hashlib.sha256(f"{stem}#{result['id']}#{system}".encode()).hexdigest()[:16]
            items.append(
                (
                    key,
                    {
                        "key": key,
                        "request": result["query"],
                        "reference_answers": result.get("reference_answers") or [],
                        "known_false_answers": false_answers.get(result["id"], []),
                        "response": response,
                        "cited_spans": spans,
                    },
                    {"key": key, "report": stem, "id": result["id"], "system": system},
                )
            )
    items.sort(key=lambda item: item[0])
    with open(sheet_path, "w") as sheet, open(map_path, "w") as key_map:
        for _, item, origin in items:
            sheet.write(json.dumps(item, ensure_ascii=False) + "\n")
            key_map.write(json.dumps(origin) + "\n")
    print(f"wrote {len(items)} responses to grade to {sheet_path}")


if __name__ == "__main__":
    main()
