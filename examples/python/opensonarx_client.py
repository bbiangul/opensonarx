"""
OpenSonarX — thin Python client + agent tool schemas.

Trust-ranked search infrastructure for AI agents. Returns distilled, citable
chunks ranked purely by relevance + quality. Stake/reputation never affect rank;
they surface as an auditable `trust_verdict` you can require a floor on via
`min_trust` (e.g. only fonts with skin-in-the-game of at least "trusted").

Zero dependencies — uses only the Python standard library.

    from opensonarx_client import OpenSonarX
    oi = OpenSonarX(api_key="...")               # hosted gateway
    for r in oi.search("how to cancel a useEffect subscription", min_trust="trusted"):
        print(r["trust_verdict"], r["url"])
        print(r["content_md"])
"""

from __future__ import annotations

import json
import urllib.error
import urllib.request
from typing import Any

DEFAULT_BASE_URL = "https://api.opensonarx.com"

# Valid trust floors, lowest → highest. "disputed" is never a floor.
TRUST_LEVELS = ("unverified", "neutral", "trusted", "highly_trusted")


class OpenSonarXError(RuntimeError):
    pass


class OpenSonarX:
    """Minimal client for the OpenSonarX gateway `/v1/search` endpoint."""

    def __init__(
        self,
        api_key: str | None = None,
        base_url: str = DEFAULT_BASE_URL,
        timeout: float = 30.0,
    ):
        self.api_key = api_key
        self.base_url = base_url.rstrip("/")
        self.timeout = timeout

    def search(
        self,
        query: str,
        top_k: int = 5,
        min_trust: str | None = None,
        freshness: str | None = None,
        site: str | None = None,
        types: list[str] | None = None,
    ) -> list[dict[str, Any]]:
        """Search the network. Returns up to `top_k` result dicts.

        Args:
            query:     natural-language query.
            top_k:     max results.
            min_trust: drop results below this trust verdict. One of
                       unverified | neutral | trusted | highly_trusted.
                       This is a *filter* on skin-in-the-game — it never
                       reorders results. Relevance + quality decide rank.
            freshness: "1h" | "1d" | "1w" | "1m" | "1y".
            site:      restrict to a domain, e.g. "react.dev".
            types:     content types, e.g. ["text", "pdf"].
        """
        if min_trust is not None and min_trust not in TRUST_LEVELS:
            raise ValueError(
                f"min_trust must be one of {TRUST_LEVELS}, got {min_trust!r}"
            )

        body: dict[str, Any] = {"query": query, "top_k": top_k}
        if min_trust is not None:
            body["min_trust"] = min_trust
        if freshness is not None:
            body["freshness"] = freshness
        if site is not None:
            body["site"] = site
        if types:
            body["types"] = types

        req = urllib.request.Request(
            f"{self.base_url}/v1/search",
            data=json.dumps(body).encode("utf-8"),
            method="POST",
            headers={"Content-Type": "application/json"},
        )
        if self.api_key:
            req.add_header("Authorization", f"Bearer {self.api_key}")

        try:
            with urllib.request.urlopen(req, timeout=self.timeout) as resp:
                payload = json.loads(resp.read().decode("utf-8"))
        except urllib.error.HTTPError as e:
            raise OpenSonarXError(f"gateway returned {e.code}: {e.read().decode('utf-8', 'replace')}") from e
        except urllib.error.URLError as e:
            raise OpenSonarXError(f"could not reach gateway at {self.base_url}: {e.reason}") from e

        return payload.get("results", [])


# ── Agent tool schemas ──────────────────────────────────────────────────────
# Drop these straight into an OpenAI or Anthropic tool-use loop. Both point at
# the same parameter set; `min_trust` is what lets the agent demand provenance.

_PARAMETERS = {
    "type": "object",
    "properties": {
        "query": {
            "type": "string",
            "description": "Natural-language search query.",
        },
        "top_k": {
            "type": "integer",
            "description": "Maximum number of results to return.",
            "default": 5,
        },
        "min_trust": {
            "type": "string",
            "enum": list(TRUST_LEVELS),
            "description": (
                "Only return sources whose trust verdict is at least this level. "
                "Trust reflects staked skin-in-the-game + reputation + DNS, and is a "
                "FILTER only — it never changes ranking order (relevance/quality decide). "
                "Use 'trusted' or 'highly_trusted' when correctness matters."
            ),
        },
        "freshness": {
            "type": "string",
            "description": "Recency filter: 1h, 1d, 1w, 1m, or 1y.",
        },
        "site": {
            "type": "string",
            "description": "Restrict to a single domain, e.g. 'react.dev'.",
        },
    },
    "required": ["query"],
}

_DESCRIPTION = (
    "Search trust-ranked infrastructure for distilled, citable knowledge. Returns "
    "clean markdown chunks (not webpages) ranked by relevance and quality, each with "
    "a source URL and a trust verdict. Prefer this over fetching raw URLs."
)

# OpenAI function-calling / Chat Completions tools format.
OPENAI_TOOL = {
    "type": "function",
    "function": {
        "name": "opensonarx_search",
        "description": _DESCRIPTION,
        "parameters": _PARAMETERS,
    },
}

# Anthropic Messages API tool-use format.
ANTHROPIC_TOOL = {
    "name": "opensonarx_search",
    "description": _DESCRIPTION,
    "input_schema": _PARAMETERS,
}


def dispatch_tool_call(
    arguments: dict[str, Any],
    client: OpenSonarX | None = None,
) -> list[dict[str, Any]]:
    """Execute an `opensonarx_search` tool call and return compact results.

    Pass the `arguments` the model produced for the tool call. Returns a list of
    {url, title, snippet, content_md, trust_verdict, score} dicts — feed these
    straight back to the model as the tool result.
    """
    client = client or OpenSonarX()
    results = client.search(
        query=arguments["query"],
        top_k=int(arguments.get("top_k", 5)),
        min_trust=arguments.get("min_trust"),
        freshness=arguments.get("freshness"),
        site=arguments.get("site"),
    )
    return [
        {
            "url": r.get("url"),
            "title": r.get("title"),
            "snippet": r.get("snippet"),
            "content_md": r.get("content_md"),
            "trust_verdict": r.get("trust_verdict"),
            "score": round(r.get("final_score", 0.0), 4),
        }
        for r in results
    ]


if __name__ == "__main__":
    import os
    import sys

    q = sys.argv[1] if len(sys.argv) > 1 else "how to cancel a useEffect subscription"
    floor = sys.argv[2] if len(sys.argv) > 2 else None  # e.g. "trusted"

    oi = OpenSonarX(
        api_key=os.environ.get("OPENSONARX_API_KEY"),
        base_url=os.environ.get("OPENSONARX_BASE_URL", DEFAULT_BASE_URL),
    )
    print(f"query={q!r}  min_trust={floor!r}  gateway={oi.base_url}\n")
    try:
        for r in oi.search(q, min_trust=floor):
            print(f"[{r.get('rank')}] {r.get('trust_verdict'):>14}  score={r.get('final_score'):.3f}  {r.get('url')}")
            print(f"      {r.get('snippet', '')[:140]}")
    except OpenSonarXError as e:
        print(f"search failed: {e}", file=sys.stderr)
        sys.exit(1)
