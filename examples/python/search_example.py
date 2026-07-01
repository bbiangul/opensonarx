"""
Example: search OpenSonarX from Python (no dependencies, stdlib only).

Usage:
    export OPENSONARX_API_KEY=...           # optional; anonymous works too
    python search_example.py "rust async traits"
    python search_example.py "rust async traits" trusted   # require a trust floor

Point at a local node or self-hosted gateway with:
    export OPENSONARX_BASE_URL=http://localhost:8080
"""

import os
import sys

from opensonarx_client import OpenSonarX, OpenSonarXError


def main():
    query = sys.argv[1] if len(sys.argv) > 1 else "decentralized search for AI agents"
    min_trust = sys.argv[2] if len(sys.argv) > 2 else None

    oi = OpenSonarX(
        api_key=os.environ.get("OPENSONARX_API_KEY"),
        base_url=os.environ.get("OPENSONARX_BASE_URL", "https://api.opensonarx.com"),
    )

    print(f"Query: {query!r}   min_trust={min_trust!r}\n")
    try:
        results = oi.search(query, top_k=5, min_trust=min_trust)
    except OpenSonarXError as e:
        print(f"Search failed: {e}", file=sys.stderr)
        sys.exit(1)

    if not results:
        print("No results.")
        return

    for r in results:
        print(f"#{r.get('rank')}  [{r.get('trust_verdict')}]  {r.get('url')}")
        print(f"    {r.get('title')}")
        print(f"    {r.get('snippet', '')[:160]}")
        print()


if __name__ == "__main__":
    main()
