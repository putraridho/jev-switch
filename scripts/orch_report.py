#!/usr/bin/env python3
"""Does orchestrate mode save money? Reads the gateway logs and compares each
orchestrated task's actual cost with the same tokens priced as if Sonnet had done
all of it. A task is the typed prompt plus the follow-up turns (subagent hand-backs,
the planner's review) up to the user's next prompt.

    python3 -I scripts/orch_report.py [--since YYYY-MM-DD] [--state DIR]

Costs are in Sonnet-equivalent units, from assumed price ratios (below), not a bill.
The counterfactual keeps the tokens the orchestrated turn actually used, so it does
not know if Sonnet alone would have needed fewer or more; read the result as an
estimate, and watch the quality columns next to it.
"""
import argparse
import collections
import datetime
import json
import os
import statistics
import sys

# Relative price per token type, Anthropic's usual ratios (input = 1).
IN, CACHE_WRITE, CACHE_READ, OUT = 1.0, 1.25, 0.1, 5.0
# Relative price per model family, Sonnet = 1. Opus 5x is the README's figure;
# Haiku is a guess; Fable's price is unknown, so it is priced like Opus.
FAMILY = {"opus": 5.0, "fable": 5.0, "sonnet": 1.0, "haiku": 1 / 3}
MIN_TURNS = 10  # fewer orchestrated turns than this is an anecdote, not a result


def family(model):
    return next((f for f in FAMILY if f in (model or "")), "sonnet")


def tokens_cost(u):
    return (
        u.get("input_tokens", 0) * IN
        + u.get("cache_creation_input_tokens", 0) * CACHE_WRITE
        + u.get("cache_read_input_tokens", 0) * CACHE_READ
        + u.get("output_tokens", 0) * OUT
    )


def read_jsonl(path):
    try:
        with open(path) as f:
            for line in f:
                try:
                    yield json.loads(line)
                except ValueError:
                    pass
    except FileNotFoundError:
        return


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--state", default=os.path.expanduser("~/.local/state/jev-router"))
    ap.add_argument("--since", default="")
    a = ap.parse_args()
    since_ms = int(datetime.datetime.fromisoformat(a.since).timestamp() * 1000) if a.since else 0

    turns = [t for t in read_jsonl(f"{a.state}/turns.jsonl") if t.get("ts_ms", 0) >= since_ms]
    by_turn = collections.defaultdict(list)
    for r in list(read_jsonl(f"{a.state}/usage.jsonl.1")) + list(read_jsonl(f"{a.state}/usage.jsonl")):
        if r.get("usage") and r.get("prompt_id") and r.get("ts_ms", 0) >= since_ms:
            by_turn[(r["session"], r["prompt_id"])].append(r)

    def cost(rows):
        actual = sum(tokens_cost(r["usage"]) * FAMILY[family(r["model"])] for r in rows)
        as_sonnet = sum(tokens_cost(r["usage"]) for r in rows)
        return actual, as_sonnet

    # A task is a prompt the user typed plus the turns that follow it until their next
    # prompt: subagent hand-backs and the planner's review run as separate turns, so
    # counting only the first would hide most of an orchestrated task's cost. The
    # gateway logs a decision for typed prompts only, which tells the two apart.
    typed = {(g.get("session"), g.get("prompt_id")) for g in read_jsonl(f"{a.state}/gateway.jsonl")}
    tasks, current = [], None
    for t in sorted(turns, key=lambda t: t.get("ts_ms", 0)):
        if not t.get("prompt_id"):
            continue
        if (t["session"], t["prompt_id"]) in typed:
            current = {"turn": t, "turns": [t], "session": t["session"]}
            tasks.append(current)
        elif current and current["session"] == t["session"]:
            current["turns"].append(t)
    orch, other = [], []
    for k in tasks:
        rows = [r for t in k["turns"] for r in by_turn.get((t["session"], t["prompt_id"]), [])]
        if not rows:
            continue
        reason = (k["turn"].get("decision") or {}).get("reason")
        (orch if reason == "orchestrate" else other).append((k, rows))

    checks = list(read_jsonl(f"{a.state}/orch.jsonl"))
    checks = [c for c in checks if c.get("ts", 0) * 1000 >= since_ms]
    cutoff = checks[-1]["cutoff"] if checks else None
    print(f"orchestrate checks: {len(checks)}, triggered: {sum(1 for c in checks if c.get('trigger'))}"
          + (f" (cutoff {cutoff})" if cutoff else ""))
    if cutoff:
        near = [c for c in checks if not c.get("trigger") and (c.get("multi_part_p") or 0) >= cutoff - 0.15]
        print(f"  just under the cutoff (within 0.15): {len(near)}")
        for c in near[:5]:
            print(f"    p={c['multi_part_p']:.2f}  {c.get('prompt', '')[:80]!r}")

    print(f"\norchestrated tasks with usage: {len(orch)}")
    if not orch:
        print("nothing to compare yet.")
        return 0

    rows_out, tot_a, tot_s = [], 0.0, 0.0
    delegated = clean = 0
    for k, rows in orch:
        t = k["turn"]
        act, sonnet = cost(rows)
        main_cost = sum(tokens_cost(r["usage"]) * FAMILY[family(r["model"])] for r in rows if r["class"] == "main")
        tools = t.get("tools") or {}
        did_delegate = bool(tools.get("Agent") or tools.get("Task"))
        ended = k["turns"][-1].get("ended")
        delegated += did_delegate
        clean += ended == "end_turn"
        tot_a += act
        tot_s += sonnet
        rows_out.append((k, act, sonnet, main_cost, did_delegate, ended))

    print(f"\n{'task':18} {'turns':>5} {'actual':>12} {'as-Sonnet':>12} {'saving':>8} {'planner%':>9} deleg  ended")
    for k, act, sonnet, main_cost, d, ended in rows_out:
        t = k["turn"]
        print(f"{t['prompt_id'][:8]}…{t['session'][:6]:9} {len(k['turns']):5} {act:12,.0f} {sonnet:12,.0f} "
              f"{(1 - act / sonnet) if sonnet else 0:8.0%} {main_cost / act if act else 0:9.0%} {'yes' if d else 'NO ':5}  {ended}")

    print(f"\ntotal actual {tot_a:,.0f} vs as-Sonnet {tot_s:,.0f}: "
          f"{'saves' if tot_a < tot_s else 'COSTS'} {abs(1 - tot_a / tot_s):.0%} (Sonnet-equivalent units)")
    print(f"planner delegated (Agent call, first turn): {delegated}/{len(orch)}; last turn ended normally: {clean}/{len(orch)}")

    # Context: what comparable ordinary turns cost per request, same pricing.
    sizes = [cost(r)[0] for _, r in other if len(r) >= 20]
    if sizes:
        print(f"ordinary tasks with 20+ requests: {len(sizes)}, median cost {statistics.median(sizes):,.0f} "
              f"(orchestrated median {statistics.median(x[1] for x in rows_out):,.0f})")

    if len(orch) < MIN_TURNS:
        print(f"\nOnly {len(orch)} orchestrated tasks (< {MIN_TURNS}): too few to conclude anything.")
    print("\nCaveats: ratios are assumed, not billed; the as-Sonnet figure reuses the turn's own tokens;"
          " check the quality columns (delegated, ended) before trusting a saving.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
