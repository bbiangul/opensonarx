#!/usr/bin/env python3
"""Draw a hashed sample of TruthfulQA questions as `oi-bench answer-live` cases.

Questions in the "Indexical Error" categories are left out: their answer
depends on who is answering, where and when, so no retrieved page can settle
them. Every other category is eligible. The sample is the `count` eligible
questions with the smallest sha256 of "<label>:<row index>".

Usage: truthfulqa_cases.py <generation-validation.jsonl> <label> <count> <cases.jsonl>
"""
import hashlib
import json
import sys


def main() -> None:
    rows_path, label, count, out_path = sys.argv[1], sys.argv[2], int(sys.argv[3]), sys.argv[4]
    rows = [json.loads(line) for line in open(rows_path) if line.strip()]
    eligible = [row for row in rows if not row["category"].startswith("Indexical Error")]
    eligible.sort(key=lambda row: hashlib.sha256(f"{label}:{row['row_idx']}".encode()).hexdigest())
    with open(out_path, "w") as out:
        for row in eligible[:count]:
            answers = [row["best_answer"]] + [a for a in row["correct_answers"] if a != row["best_answer"]]
            out.write(
                json.dumps(
                    {
                        "id": f"truthfulqa:{row['row_idx']}",
                        "subject": row["category"],
                        "query": row["question"],
                        "expect": {"status": "extractive", "answer_contains_any": answers},
                        "known_false_answers": row["incorrect_answers"],
                        "dataset_type": row["type"],
                        "dataset_source": row["source"],
                    },
                    ensure_ascii=False,
                )
                + "\n"
            )
    print(f"{len(eligible)} eligible of {len(rows)}; wrote {min(count, len(eligible))} cases to {out_path}")


if __name__ == "__main__":
    main()
