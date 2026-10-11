#!/usr/bin/env python3
"""What an agent does with an OpenSonarX evidence report.

OpenSonarX does not write the answer. It returns the sources and, per claim,
who states it, the exact quote, and who contradicts it. This script is the
other half: a minimal agent policy that turns those signals into what it is
willing to say.

    export OPENROUTER_API_KEY=...   # or use a local Ollama model
    export OI_ANSWER_PROVIDER=openrouter OI_ANSWER_MODEL=anthropic/claude-haiku-5.5
    python3 examples/python/agent_reads_evidence.py "what is the largest desert in the world"

The policy below is deliberately plain; a real agent would hand the same
report to its own model.
"""
import json
import subprocess
import sys
from urllib.parse import urlparse

OI = "./target/release/oi"


def evidence(query: str) -> dict:
    out = subprocess.run(
        [OI, "--open", "--output", "json", "search", query, "--limit", "6", "--answer"],
        capture_output=True,
        text=True,
        check=True,
    )
    return json.loads(out.stdout)


def host(url: str) -> str:
    return urlparse(url).hostname or url


def main() -> None:
    query = " ".join(sys.argv[1:]) or "what is the largest desert in the world"
    report = evidence(query)
    bonds = {host(r["url"]): r["stake_truth"] for r in report["results"] if r["stake_truth"]}

    print(f"Q: {query}\n")
    claims = [c for c in report["answer"]["claims"] if c["status"] != "off_request"]
    if not claims:
        print("AGENT: I found sources but nothing I can quote. Here they are:")
        for result in report["results"]:
            print(f"  - {result['url']}")
        return

    for claim in claims:
        lineage = claim.get("evidence_lineage") or {}
        supporters = [c for c in claim["citations"] if c["evidence_role"] == "supports"]
        opponents = [c for c in claim["citations"] if c["evidence_role"] == "contradicts"]
        if claim["status"] == "corroborated":
            families = lineage.get("reporting_source_families", len(supporters))
            print(f"AGENT: {claim['text']}")
            print(f"       Confirmed by {families} independent source(s).")
        elif claim["status"] == "contested":
            print(f"AGENT: Sources disagree. One reading: {claim['text']}")
        else:
            print(f"AGENT: One source says: {claim['text']}")
            print("       Nobody else confirmed it, so treat it as unverified.")
        for citation in supporters:
            bond = bonds.get(host(citation["url"]))
            bonded = f"  [bonded: {bond}]" if bond else ""
            print(f"         + {host(citation['url'])}{bonded}: \"{citation['evidence_excerpt']}\"")
        for citation in opponents:
            print(f"         - {host(citation['url'])}: \"{citation['evidence_excerpt']}\"")
        print()


if __name__ == "__main__":
    main()
