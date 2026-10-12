#!/usr/bin/env python3
"""OpenSonarX end to end, at small scale, with every piece checked.

One run starts real processes (three nodes, a dishonest node, a local Solana
validator), walks through eight acts, and ends each act with checks computed
from what the processes answered. Nothing is mocked and nothing is replayed:
pages are fetched from the web and claims are extracted by a hosted model
while the script runs, so numbers differ from run to run.

  Act 1  an agent searches and gets sources
  Act 2  the index is divided by subject, learned from its own content
  Act 3  two nodes each ingest only their subjects, reading pages for claims
  Act 4  a question is answered from stored claims, in milliseconds, across nodes
  Act 5  a replica with no model receives claims and checks every quote
  Act 6  a dishonest node offers a forged claim and the replica refuses it
  Act 7  an outside auditor re-fetches the pages and looks for every quote
  Act 8  a publisher bonds SOL behind a domain on Solana; the node shows it

Usage:
  export OPENROUTER_API_KEY=...        # model used at ingestion only
  scripts/demo.py                      # run everything, then stop
  scripts/demo.py --keep               # leave the nodes up for the live pages
  scripts/demo.py --pause              # wait for Enter between acts (recording)
  scripts/demo.py --skip-solana
  scripts/demo.py --rpc https://api.devnet.solana.com   # bond on devnet instead

Exit status is 0 only if every check passed.
"""
import argparse
import hashlib
import json
import os
import shutil
import signal
import statistics
import subprocess
import sys
import threading
import time
import urllib.error
import urllib.request
from pathlib import Path
from urllib.parse import quote, urlparse

ROOT = Path(__file__).resolve().parent.parent
DEMO = ROOT / "target" / "demo"
OI = ROOT / "target" / "release" / "oi"
BENCH = ROOT / "target" / "release" / "oi-bench"
MODEL = os.environ.get("OI_ANSWER_MODEL", "anthropic/claude-haiku-5.5")
SOLANA_BIN = Path.home() / ".local/share/solana/install/active_release/bin"

# Two questions on unrelated subjects, so that a division of the index by
# subject has something to divide. For each, a statement the page does not
# make, for the dishonest node to offer with a made-up quote.
QUESTIONS = [
    {
        "ask": "what is the largest desert in the world",
        "forged_claim": "The Sahara is the largest desert in the world, twice the size of Antarctica.",
        "forged_quote": "The Sahara is the largest desert on Earth, about twice the size of Antarctica.",
    },
    {
        "ask": "how does compound interest work",
        "forged_claim": "Compound interest is calculated only on the original principal and never on earlier interest.",
        "forged_quote": "Compound interest is always calculated on the original principal alone, never on interest earned before.",
    },
]

TTY = sys.stdout.isatty()
BOLD, DIM, GREEN, RED, YELLOW, CYAN, RESET = (
    ("\033[1m", "\033[2m", "\033[32m", "\033[31m", "\033[33m", "\033[36m", "\033[0m") if TTY else ("",) * 7
)

checks = []  # (act, statement, passed, evidence)
processes = []  # (name, Popen)
args = None


def say(text=""):
    print(text, flush=True)


def act(number, title):
    if args.pause and number > 1:
        input(f"{DIM}  (Enter for act {number}){RESET}")
    say()
    say(f"{BOLD}{CYAN}ACT {number}  {title}{RESET}")
    say(f"{CYAN}{'─' * (7 + len(title))}{RESET}")


def proved(number, statement, passed, evidence):
    checks.append((number, statement, bool(passed), evidence))
    mark = f"{GREEN}✔ PROVED    {RESET}" if passed else f"{RED}✘ NOT PROVED{RESET}"
    say(f"  {mark} {statement}")
    say(f"               {DIM}{evidence}{RESET}")


def clip(text, width=96):
    text = " ".join(str(text).split())
    return text if len(text) <= width else text[: width - 1] + "…"


def family(url):
    host = urlparse(url).netloc.lower()
    return host[4:] if host.startswith("www.") else host


def http(method, url, body=None, timeout=60):
    data = json.dumps(body).encode() if body is not None else None
    request = urllib.request.Request(url, data=data, method=method, headers={"content-type": "application/json"})
    with urllib.request.urlopen(request, timeout=timeout) as response:
        return json.loads(response.read())


def api(port):
    return f"http://127.0.0.1:{port}"


def start(name, command, env, log):
    handle = open(log, "w")
    process = subprocess.Popen(command, env=env, stdout=handle, stderr=subprocess.STDOUT, start_new_session=True)
    processes.append((name, process))
    return process


def signal_group(process, how):
    """Signal a process and its children. A group that is already gone, or
    holds only a finished process, is not an error."""
    try:
        os.killpg(process.pid, how)
    except OSError:
        pass


def stop(process, how=signal.SIGINT):
    if process.poll() is None:
        signal_group(process, how)
        try:
            process.wait(timeout=8)
        except subprocess.TimeoutExpired:
            signal_group(process, signal.SIGKILL)


def stop_all():
    # Ask every process to stop first, then wait for each.
    for _, process in processes:
        if process.poll() is None:
            signal_group(process, signal.SIGINT)
    for _, process in reversed(processes):
        stop(process)


def wait_for(what, condition, timeout, every=1.0):
    deadline = time.time() + timeout
    while time.time() < deadline:
        try:
            value = condition()
            if value:
                return value
        except (urllib.error.URLError, ConnectionError, OSError, KeyError, ValueError):
            pass
        time.sleep(every)
    raise TimeoutError(f"timed out waiting for {what}")


def node_env(data_dir, model):
    env = {k: v for k, v in os.environ.items() if not k.startswith(("OI_", "OPENROUTER", "OPENSONARX"))}
    env["OPENSONARX_DATA_DIR"] = str(data_dir)
    if model:
        env.update(
            OI_EXTRACT_CLAIMS="1",
            OI_ANSWER_MODEL=MODEL,
            OI_ANSWER_PROVIDER="openrouter",
            OPENROUTER_API_KEY=os.environ["OPENROUTER_API_KEY"],
            OI_NODE_LEARN="1",
        )
    return env


def start_node(name, p2p, http_port, routing, shard, model, bootstrap=None):
    data_dir = DEMO / name
    data_dir.mkdir(parents=True, exist_ok=True)
    command = [
        str(OI), "--open", "network", "node",
        "--port", str(p2p), "--http-port", str(http_port),
        "--routing", str(routing), "--shards", str(shard),
        "--min-replicas", "1", "--sync-secs", "5",
    ]
    if bootstrap:
        command += ["--bootstrap", f"/ip4/127.0.0.1/tcp/{bootstrap}"]
    process = start(name, command, node_env(data_dir, model), DEMO / f"{name}.log")
    wait_for(f"{name} to answer", lambda: http("GET", api(http_port) + "/health", timeout=3), 60)
    return process


def claims(port, question):
    return http("POST", api(port) + "/claims", {"query": question}, timeout=30)


def identities(answer):
    return [(m["url"], m["claim"]) for m in answer["matches"]]


def by_source(answer):
    """The sources holding a claim close to the question, in the order their
    nearest claim comes back, each with that claim."""
    sources = {}
    for match, detail in zip(answer["matches"], answer["details"]):
        if detail["about_question"] and match["source_family"] not in sources:
            sources[match["source_family"]] = (match, detail)
    return list(sources.values())


def show_answer(answer, indent="    "):
    families = answer["supporting_families"]
    if answer["corroborated"]:
        verdict = f"{GREEN}{families} independent sources hold claims close to this question{RESET}"
    elif families == 1:
        verdict = f"{YELLOW}one source holds a claim close to this question{RESET}"
    else:
        verdict = f"{DIM}nothing stored is close to this question{RESET}"
    say(f"{indent}{BOLD}{verdict}{BOLD}{RESET}  ·  {answer['lookup_ms']:.1f} ms  ·  "
        f"{answer['from_peers']} of {len(answer['matches'])} claims from other nodes")
    for match, detail in by_source(answer)[:4]:
        where = "this node" if detail["held_here"] else "peer …" + detail["held_by"][-6:]
        bond = f"  ◎{detail['bond_lamports'] / 1e9:g} bonded on chain" if detail["bond_lamports"] else ""
        say(f"{indent}  {BOLD}{match['source_family']}{RESET}{YELLOW}{bond}{RESET}  {DIM}held by {where}{RESET}")
        say(f"{indent}    {clip(match['claim'], 90)}")
        say(f"{indent}    {DIM}“{clip(match['quote'], 86)}”{RESET}")


# ── Acts ────────────────────────────────────────────────────────────────────


def act1_search():
    act(1, "An agent searches and gets sources")
    say("  The command-line client searches the open web, fetches the pages it finds and keeps them.")
    seed = DEMO / "seed"
    seed.mkdir(parents=True)
    urls = []
    enough = True
    for question in QUESTIONS:
        started = time.time()
        run = subprocess.run(
            [str(OI), "--open", "search", question["ask"], "-k", "6", "-o", "json"],
            env=node_env(seed, model=False), capture_output=True, text=True, timeout=180,
        )
        results = json.loads(run.stdout)
        results = results if isinstance(results, list) else results.get("results", [])
        families = []
        say(f"\n  {BOLD}“{question['ask']}”{RESET}  {DIM}({time.time() - started:.1f} s, pages fetched and indexed){RESET}")
        for result in results:
            families.append(family(result["url"]))
            say(f"    {result.get('rank', '·'):>2}  {family(result['url']):<28} {clip(result.get('title', ''), 54)}")
        question["urls"] = [result["url"] for result in results[:5]]
        urls += question["urls"]
        enough = enough and len(set(families)) >= 3
        question["families_found"] = len(set(families))
        time.sleep(3)
    proved(1, "each question returns sources from at least three independent sites", enough,
           ", ".join(f"{q['families_found']} sites" for q in QUESTIONS))
    return urls


def act2_routing():
    act(2, "The index is divided by subject, learned from its own content")
    say("  No fixed list of topics: the node clusters the pages it holds and writes a routing file.")
    say("  The file's version is a hash of its content and becomes part of every shard name.")
    routing = DEMO / "routing.json"
    run = subprocess.run(
        [str(OI), "--open", "network", "learn-routing", "--shards", "2", "--output", str(routing)],
        env=node_env(DEMO / "seed", model=False), capture_output=True, text=True, timeout=180,
    )
    lines = [line for line in run.stdout.splitlines() if "ort::logging" not in line]
    for line in lines:
        if not line.strip().startswith(("File:", "Every node")):
            say(f"    {line.strip()}")
    spec = json.loads(routing.read_text())
    sizes = [int(line.split(":")[1].split()[0]) for line in lines if line.strip().startswith("learned-")]
    proved(2, "a two-shard routing was learned from the stored pages, both shards non-empty",
           len(spec["centroids"]) == 2 and len(sizes) == 2 and min(sizes) > 0,
           f"version {spec['version']}, chunks per shard {sizes}")

    tampered = DEMO / "routing-tampered.json"
    spec["centroids"][0][0] += 0.01
    tampered.write_text(json.dumps(spec))
    refused = subprocess.run(
        [str(OI), "--open", "network", "node", "--port", "4399", "--routing", str(tampered)],
        env=node_env(DEMO / "seed", model=False), capture_output=True, text=True, timeout=60,
    )
    message = next((l for l in (refused.stdout + refused.stderr).splitlines() if "does not match" in l), "")
    proved(2, "a node refuses a routing file that was edited after it was written",
           refused.returncode != 0 and bool(message), clip(message.replace(str(DEMO) + "/", ""), 110) or "node started")
    return routing


def act3_ingest(routing, urls):
    act(3, "Two nodes each ingest only their subjects, reading pages once for claims")
    say("  Node A serves shard 0, node B serves shard 1. Both are given the same pages.")
    say("  Each keeps the chunks of its shard, a model lists what each page states, and a claim")
    say("  is stored only if its quote is found character by character in the page.")
    start_node("node-a", 4301, 8301, routing, 0, model=True)
    start_node("node-b", 4302, 8302, routing, 1, model=True, bootstrap=4301)
    learned = {}

    def learn(name, port):
        learned[name] = http("POST", api(port) + "/learn", {"urls": urls}, timeout=600)

    workers = [threading.Thread(target=learn, args=(n, p)) for n, p in (("A", 8301), ("B", 8302))]
    started = time.time()
    for worker in workers:
        worker.start()
    for worker in workers:
        worker.join()
    for name, port in (("A", 8301), ("B", 8302)):
        status = http("GET", api(port) + "/status")
        say(f"    node {name}: {status['pages']:>3} chunks kept, {status['claims']:>3} claims stored "
            f"{DIM}({learned[name]['seconds']:.0f} s of fetching and model reading){RESET}")
    a, b = (http("GET", api(port) + "/status") for port in (8301, 8302))
    proved(3, "both nodes extracted and stored claims from the pages of their own shard",
           a["claims"] > 0 and b["claims"] > 0,
           f"A {a['claims']} claims, B {b['claims']} claims, {len(urls)} pages offered to each, {time.time() - started:.0f} s")
    peers = wait_for("the nodes to see each other",
                     lambda: len(http("GET", api(8301) + "/status")["peers"]) >= 1
                     and len(http("GET", api(8302) + "/status")["peers"]) >= 1, 40)
    return bool(peers)


def act4_ask():
    act(4, "A question is answered from stored claims, in milliseconds, across nodes")
    say("  No model runs now. The node embeds the question, looks up the nearest stored claims,")
    say("  asks the peers whose shards match the subject, and counts independent sources.")
    say("  The index does not write an answer: it returns each source in its own words, with the")
    say("  quote. Whoever asked, a person or an agent, reads them.")
    same, disjoint, timings, crossed, backed = True, True, [], 0, 0
    for question in QUESTIONS:
        # Each node is asked each question once here, so every timing is a
        # first lookup: the question is embedded, not read from a cache.
        on_a, on_b = claims(8301, question["ask"]), claims(8302, question["ask"])
        timings += [on_a["lookup_ms"], on_b["lookup_ms"]]
        n = len(on_a["matches"])
        own_a, own_b = n - on_a["from_peers"], len(on_b["matches"]) - on_b["from_peers"]
        same = same and identities(on_a) == identities(on_b)
        disjoint = disjoint and own_a + own_b == n
        crossed += on_a["from_peers"] > 0 or on_b["from_peers"] > 0
        backed += on_a["corroborated"]
        question.update(answer=on_a, own_a=own_a, own_b=own_b)
        say(f"\n  {BOLD}“{question['ask']}”{RESET}  {DIM}asked to node A{RESET}")
        show_answer(on_a)
        say(f"    {DIM}of the {n} nearest claims, {own_a} are held by A and {own_b} by B{RESET}")
    proved(4, "both nodes give the same claims in the same order for every question", same,
           "node A and node B compared claim by claim")
    proved(4, "no returned claim is held twice: each node holds its own part of the index", disjoint,
           "; ".join(f"A {q['own_a']} + B {q['own_b']} = {len(q['answer']['matches'])}" for q in QUESTIONS))
    proved(4, "answers cross the network: a node returns claims that only its peer holds", crossed >= 1,
           f"{crossed} of {len(QUESTIONS)} questions needed the other node")
    median = statistics.median(timings)
    proved(4, "the lookup takes milliseconds", max(timings) < 100,
           f"median {median:.1f} ms, slowest {max(timings):.1f} ms over {len(timings)} first lookups, "
           "question embedding and network round included")
    proved(4, "for at least one question, two or more independent sources hold a close claim",
           backed >= 1,
           "; ".join(f"{q['answer']['supporting_families']} sources" for q in QUESTIONS))

    if not args.skip_answer:
        question = max(QUESTIONS, key=lambda q: q["answer"]["supporting_families"])
        say(f"\n  {DIM}For contrast, the same evidence computed at query time, with a model reading the pages:{RESET}")
        started = time.time()
        try:
            http("POST", api(8301) + "/answer", {"query": question["ask"], "limit": 6}, timeout=180)
            say(f"    query-time path: {BOLD}{time.time() - started:.1f} s{RESET}   stored-claims path: "
                f"{BOLD}{question['answer']['lookup_ms']:.1f} ms{RESET}")
        except Exception as error:  # the contrast is an illustration, not a check
            say(f"    {DIM}query-time path did not finish ({clip(error, 60)}){RESET}")


def act5_replica(routing):
    act(5, "A replica with no model receives claims and checks every quote")
    say("  Node C joins as a second holder of shard 0. It has no model and no API key.")
    say("  It pulls pages and claims from node A and keeps a claim only if it finds the quote")
    say("  in the page text it now holds.")
    start_node("node-c", 4303, 8303, routing, 0, model=False, bootstrap=4301)
    a = http("GET", api(8301) + "/status")

    def settled():
        first = http("GET", api(8303) + "/status")["claims"]
        time.sleep(6)
        return first > 0 and http("GET", api(8303) + "/status")["claims"] == first

    try:
        wait_for("node C to receive the claims", settled, 120, every=2)
    except TimeoutError:
        pass
    c = http("GET", api(8303) + "/status")
    say(f"    node A holds {a['pages']} chunks and {a['claims']} claims;  node C now holds {c['pages']} and "
        f"{c['claims']}, refused {c['claims_rejected_from_peers']}")
    proved(5, "a node with no model holds every claim after replication, each quote found in its own copy",
           not c["reads_pages_for_claims"] and c["claims"] == a["claims"] > 0
           and c["claims_rejected_from_peers"] == 0,
           f"model on C: none; C {c['claims']} of A's {a['claims']} claims, none refused")
    question = max(QUESTIONS, key=lambda q: q["own_a"])
    on_a, on_c = claims(8301, question["ask"]), claims(8303, question["ask"])
    head = lambda answer: identities(answer)[:1]
    proved(5, "the replica answers with the same closest claim as the node that read the pages",
           head(on_a) == head(on_c) and bool(head(on_a)),
           clip(on_c["matches"][0]["claim"], 100) if on_c["matches"] else "no claim")
    return question


def act6_forger(question):
    act(6, "A dishonest node offers a forged claim and the replica refuses it")
    say("  A modified node joins shard 0. It offers a real page with two claims: one sentence")
    say("  that is in the page, and one statement it made up, with a quote the page does not contain.")
    before = http("GET", api(8303) + "/status")
    shard = before["shards"][0]
    page = question["answer"]["matches"][0]["url"]
    forger = subprocess.run(
        [str(BENCH), "demo-forger", "--url", page, "--shard", shard,
         "--bootstrap", "/ip4/127.0.0.1/tcp/4303", "--port", "4390",
         "--forged-claim", question["forged_claim"], "--forged-quote", question["forged_quote"],
         "--seconds", "35"],
        capture_output=True, text=True, timeout=240,
    )
    lines = {l.split(":", 1)[0].strip(): l.split(":", 1)[1].strip() for l in forger.stdout.splitlines() if ":" in l}
    honest = lines.get("honest claim", "")
    say(f"    page          {page}")
    say(f"    honest claim  {clip(honest, 84)}")
    say(f"    {RED}forged claim  {clip(question['forged_claim'], 84)}{RESET}")
    say(f"    {RED}forged quote  “{clip(question['forged_quote'], 80)}”{RESET}")
    after = http("GET", api(8303) + "/status")
    refused = after["claims_rejected_from_peers"] - before["claims_rejected_from_peers"]
    kept = after["claims"] - before["claims"]
    say(f"    node C: {kept:+d} claim stored, {refused:+d} refused")
    holds = lambda text: any(m["claim"] == text for m in claims(8303, text)["matches"])
    proved(6, "the replica refused exactly the claim whose quote is not in the page",
           forger.returncode == 0 and refused == 1 and kept == 1 and not holds(question["forged_claim"]),
           f"refused from peers went {before['claims_rejected_from_peers']} → {after['claims_rejected_from_peers']}; "
           "the forged statement is not in node C")
    proved(6, "the same dishonest node's true claim was accepted",
           bool(honest) and holds(honest), "the honest sentence is in node C, found by its own text")
    say(f"    {DIM}What this check covers: a quote that is not in the page the replica received.")
    say(f"    A forged page is a different attack; the next act is the check against that.{RESET}")


def act7_audit():
    act(7, "An outside auditor re-fetches the pages and looks for every quote")
    say("  A separate process takes the claims the nodes returned, fetches each page itself from")
    say("  the web, and looks for each quote. As a control it also looks for quotes taken from a")
    say("  different page, which must not be found.")
    by_url = {}
    for question in QUESTIONS:
        for port in (8301, 8302):
            for match in claims(port, question["ask"])["matches"]:
                by_url.setdefault(match["url"], {})[match["claim"]] = match
    path = DEMO / "audit-claims.jsonl"
    with open(path, "w") as out:
        for url, records in by_url.items():
            rows = [{**m, "extractor": "node"} for m in records.values()]
            rows = [{k: r[k] for k in ("claim", "quote", "url", "source_family", "extractor")} for r in rows]
            out.write(json.dumps({"key": hashlib.sha256(url.encode()).hexdigest(), "records": rows}) + "\n")
    report = DEMO / "audit.json"
    subprocess.run([str(BENCH), "claim-audit", "--claims", str(path), "--urls", "50", "--output", str(report)],
                   capture_output=True, text=True, timeout=300)
    summary = json.loads(report.read_text())["summary"]
    honest, foreign = summary["honest_claims_located_in_page"], summary["foreign_quotes_located_in_page"]
    say(f"    pages re-fetched   {summary['urls_fetched']} of {summary['urls_audited']}")
    say(f"    honest quotes found again        {honest['numerator']} of {honest['denominator']}")
    say(f"    quotes from another page found   {foreign['numerator']} of {foreign['denominator']}")
    proved(7, "no quote taken from a different page is accepted",
           foreign["denominator"] > 0 and foreign["numerator"] == 0,
           f"{foreign['numerator']} of {foreign['denominator']} accepted")
    proved(7, "the nodes' own quotes are found again by an independent fetch",
           honest["denominator"] > 0 and honest["numerator"] >= 0.9 * honest["denominator"],
           f"{honest['numerator']} of {honest['denominator']} found; pages change, so this is not always all of them")


def act8_solana():
    act(8, "A publisher bonds SOL behind a domain on Solana; the node shows it")
    env = dict(os.environ)
    if SOLANA_BIN.exists():
        env["PATH"] = f"{SOLANA_BIN}:{env['PATH']}"
    local = args.rpc is None
    rpc = args.rpc or "http://127.0.0.1:8899"
    if local:
        say("  A local Solana validator runs the staking program; it is not a public cluster.")
        say("  The wallet and the program address are made for this run and hold only test SOL.")
        start("validator", ["solana-test-validator", "--reset", "--quiet", "--ledger", str(DEMO / "ledger")],
              env, DEMO / "validator.log")
        wait_for("the validator", lambda: subprocess.run(
            ["solana", "cluster-version", "--url", rpc], env=env, capture_output=True).returncode == 0, 60)
        # Nothing of the operator's is used on the local validator: a wallet
        # and a program keypair are generated inside the demo directory.
        wallet, program_keypair = DEMO / "wallet.json", DEMO / "program-keypair.json"
        for keypair in (wallet, program_keypair):
            subprocess.run(["solana-keygen", "new", "--no-bip39-passphrase", "--silent", "--force", "-o", str(keypair)],
                           env=env, capture_output=True, check=True)
        address = subprocess.run(["solana-keygen", "pubkey", str(wallet)], env=env, capture_output=True,
                                 text=True, check=True).stdout.strip()
        subprocess.run(["solana", "airdrop", "100", address, "--url", rpc], env=env, capture_output=True, timeout=60)
        env["WALLET"], env["PROGRAM_KEYPAIR"] = str(wallet), str(program_keypair)
    else:
        say(f"  Cluster: {rpc}  (your wallet and the program keypair in ~/.config/solana)")

    question = max(QUESTIONS, key=lambda q: q["answer"]["supporting_families"])
    before = claims(8301, question["ask"])
    domain = before["matches"][0]["source_family"]
    lamports = int(args.bond_sol * 1_000_000_000)
    say(f"  The publisher of {BOLD}{domain}{RESET} bonds ◎{lamports / 1e9:g} behind its domain.")
    env["OPENSONARX_DATA_DIR"] = str(DEMO / "node-a")
    run = subprocess.run([str(ROOT / "scripts" / "chain_bond_demo.sh"), rpc, domain, str(lamports)],
                         env=env, cwd=ROOT, capture_output=True, text=True, timeout=900)
    output = run.stdout + run.stderr
    staked_now = False
    cluster = next((name for name in ("devnet", "testnet") if name in rpc), None)
    for line in output.splitlines():
        line = line.strip()
        if line.lower().startswith(("program:", "signature:", "bonded", "stake account:", "lamports:")):
            say(f"    {clip(line, 110)}")
        staked_now = staked_now or line.startswith("bonded")
        # On a public cluster, where to look the transaction and account up.
        if cluster and line.startswith("signature:"):
            say(f"    {DIM}https://explorer.solana.com/tx/{line.split()[-1]}?cluster={cluster}{RESET}")
        if cluster and line.startswith("stake account:") and not getattr(act8_solana, "linked", False):
            act8_solana.linked = True
            say(f"    {DIM}https://explorer.solana.com/address/{line.split()[-1]}?cluster={cluster}{RESET}")
    if run.returncode == 0 and not staked_now:
        say(f"    {DIM}{domain} was already bonded on this cluster in an earlier run; the account is read back{RESET}")
    mirror = DEMO / "node-a" / "stakes.json"
    on_chain = 0
    if mirror.exists():
        on_chain = next((s["amount_lamports"] for s in json.loads(mirror.read_text()) if s["domain"] == domain), 0)
    if run.returncode != 0:
        (DEMO / "solana-act.log").write_text(output)
        say(f"    {RED}the Solana step failed; its output is in {(DEMO / 'solana-act.log').relative_to(ROOT)}{RESET}")
        for line in output.strip().splitlines()[-4:]:
            say(f"    {DIM}{clip(line, 110)}{RESET}")
    proved(8, "the bond was written to the chain and read back from the stake account"
           if staked_now else "the bond is on the chain and was read back from the stake account",
           run.returncode == 0 and on_chain > 0 and (on_chain == lamports or not staked_now),
           f"{on_chain} lamports read from the account of {domain}" + (" (local validator)" if local else f" ({rpc})"))
    lamports = on_chain or lamports

    def shown():
        bonds = {b["domain"]: b["lamports"] for b in http("GET", api(8301) + "/status")["bonds"]}
        return bonds.get(domain) == lamports

    try:
        wait_for("node A to show the bond", shown, 20)
        visible = True
    except TimeoutError:
        visible = False
    after = claims(8301, question["ask"])
    bonded = [d["bond_lamports"] for m, d in zip(after["matches"], after["details"]) if m["source_family"] == domain]
    proved(8, "the running node shows the bond next to that source, without a restart",
           visible and bonded and all(b == lamports for b in bonded),
           f"{len(bonded)} claims from {domain} now carry ◎{lamports / 1e9:g}")
    proved(8, "the bond changes nothing about which claims come back or their order",
           identities(before) == identities(after) and before["supporting_families"] == after["supporting_families"],
           "same claims, same order, same source count before and after")
    say()
    show_answer(after)


def main():
    global args
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--keep", action="store_true", help="leave the nodes running at the end")
    parser.add_argument("--pause", action="store_true", help="wait for Enter between acts")
    parser.add_argument("--no-build", action="store_true", help="do not run cargo build first")
    parser.add_argument("--skip-solana", action="store_true")
    parser.add_argument("--skip-answer", action="store_true", help="skip the query-time timing contrast")
    parser.add_argument("--rpc", help="Solana RPC to bond on instead of a local validator")
    parser.add_argument("--bond-sol", type=float, default=0.25, help="SOL bonded in act 8 (default 0.25)")
    args = parser.parse_args()

    if not os.environ.get("OPENROUTER_API_KEY"):
        sys.exit("OPENROUTER_API_KEY is not set. The model is used at ingestion, to read pages for claims.")
    path = os.environ["PATH"] + (f":{SOLANA_BIN}" if SOLANA_BIN.exists() else "")
    missing = [tool for tool in (["cargo"] + ([] if args.skip_solana else ["solana", "solana-keygen", "solana-test-validator"]))
               if not shutil.which(tool, path=path)]
    if missing:
        sys.exit(f"Not found on PATH: {', '.join(missing)}. The demo needs Rust (cargo) and, for act 8, the "
                 "Solana CLI (https://docs.anza.xyz/cli/install); or run with --skip-solana.")
    if not args.no_build:
        say(f"{DIM}building the node and the tools (first time takes a few minutes){RESET}")
        subprocess.run(["cargo", "build", "--release", "-q", "-p", "oi-cli"], cwd=ROOT, check=True)
        subprocess.run(["cargo", "build", "--release", "-q", "-p", "oi-bench", "--features", "onnx"], cwd=ROOT, check=True)
    subprocess.run(["pkill", "-f", f"{OI} --open network node"], capture_output=True)
    subprocess.run(["pkill", "-f", f"solana-test-validator.*{DEMO}"], capture_output=True)
    shutil.rmtree(DEMO, ignore_errors=True)
    DEMO.mkdir(parents=True)

    say(f"{BOLD}OpenSonarX, end to end at small scale{RESET}")
    say(f"{DIM}real processes, live pages, a hosted model at ingestion only ({MODEL}); logs in {DEMO.relative_to(ROOT)}{RESET}")
    started = time.time()
    try:
        urls = act1_search()
        routing = act2_routing()
        act3_ingest(routing, urls)
        act4_ask()
        question = act5_replica(routing)
        act6_forger(question)
        act7_audit()
        if not args.skip_solana:
            act8_solana()

        say()
        say(f"{BOLD}What this run proved{RESET}  {DIM}({time.time() - started:.0f} s){RESET}")
        for number, statement, passed, _ in checks:
            mark = f"{GREEN}✔{RESET}" if passed else f"{RED}✘{RESET}"
            say(f"  {mark} act {number}  {statement}")
        passed = sum(1 for check in checks if check[2])
        say(f"\n  {BOLD}{passed} of {len(checks)} checks passed{RESET}")
        say(f"\n  {DIM}Not shown here because it is not built: nodes on separate machines, rewards and")
        say(f"  penalties on chain for ingestion work, a large index of our own.{RESET}")
        (DEMO / "checks.json").write_text(json.dumps(
            [{"act": n, "statement": s, "passed": p, "evidence": e} for n, s, p, e in checks], indent=1))

        if args.keep:
            ask = quote(max(QUESTIONS, key=lambda q: q["answer"]["supporting_families"])["ask"])
            say(f"\n{BOLD}The nodes are still running. Live pages:{RESET}")
            for name, port in (("A (shard 0, reads pages)", 8301), ("B (shard 1, reads pages)", 8302), ("C (replica, no model)", 8303)):
                say(f"  node {name:<26} {api(port)}/demo?q={ask}")
            say(f"{DIM}Ctrl+C to stop everything.{RESET}")
            signal.pause()
    except KeyboardInterrupt:
        pass
    finally:
        stop_all()
    sys.exit(0 if checks and all(check[2] for check in checks) else 1)


if __name__ == "__main__":
    main()
