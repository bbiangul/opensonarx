# OpenSonarX — Python agent integration

Trust-ranked search infrastructure for AI agents. Your agent gets distilled,
citable markdown chunks ranked by **relevance and quality** — not by who paid
the most. Staked skin-in-the-game surfaces as a `trust_verdict` you can require
a floor on with `min_trust`; it never reorders results.

Zero dependencies — `opensonarx_client.py` uses only the Python standard library.

## Quick start

```bash
export OPENSONARX_API_KEY=...        # optional — anonymous tier works too
python search_example.py "how to cancel a useEffect subscription" trusted
```

```python
from opensonarx_client import OpenSonarX

oi = OpenSonarX(api_key="...")                  # or base_url="http://localhost:8080"
for r in oi.search("rust async traits", min_trust="trusted"):
    print(r["trust_verdict"], r["url"])
    print(r["content_md"])                      # distilled chunk, ready for the LLM
```

`min_trust` ∈ `unverified | neutral | trusted | highly_trusted`. It is a **filter
on provenance**, applied after ranking — relevance and quality always decide order.

## Use it as an agent tool

The client ships ready-made tool schemas for both providers, plus a dispatcher.

### Anthropic (Messages API)

```python
import anthropic
from opensonarx_client import ANTHROPIC_TOOL, dispatch_tool_call

client = anthropic.Anthropic()
msg = client.messages.create(
    model="claude-opus-4-8",
    max_tokens=1024,
    tools=[ANTHROPIC_TOOL],
    messages=[{"role": "user", "content": "How do I stream tokens with the Anthropic SDK?"}],
)

for block in msg.content:
    if block.type == "tool_use" and block.name == "opensonarx_search":
        results = dispatch_tool_call(block.input)   # -> list of {url, content_md, trust_verdict, ...}
        # send `results` back as a tool_result block in the next turn
```

### OpenAI (Chat Completions)

```python
from openai import OpenAI
from opensonarx_client import OPENAI_TOOL, dispatch_tool_call
import json

client = OpenAI()
resp = client.chat.completions.create(
    model="gpt-4o",
    tools=[OPENAI_TOOL],
    messages=[{"role": "user", "content": "What changed in React 19's use() hook?"}],
)

for call in resp.choices[0].message.tool_calls or []:
    if call.function.name == "opensonarx_search":
        results = dispatch_tool_call(json.loads(call.function.arguments))
        # append a role="tool" message with json.dumps(results) and continue the loop
```

The model can pass `min_trust` itself — e.g. it will ask for `"highly_trusted"`
when the user's question is correctness-critical.

## Files

| File | What |
|------|------|
| `opensonarx_client.py` | Client + `OPENAI_TOOL` / `ANTHROPIC_TOOL` schemas + `dispatch_tool_call` |
| `search_example.py`    | Minimal CLI demo |

> The hosted gateway lives at `https://api.opensonarx.com`. To run fully local,
> start a node (`oi network node`) and set `OPENSONARX_BASE_URL` to its address.
