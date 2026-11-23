"""Summarise controlled comparisons from audit logs: no provider calls, logs only.

Each arm (for example the router and a pinned baseline) is one `audit.jsonl` from a run of
the same tasks on the same repository snapshots. Per arm: turns, accepted tasks, the accepted
share of the checked turns (with its interval), tokens per accepted task over every turn of
the arm (failed attempts and rework included, since they were spent), failures and rework.
A turn nobody checked is unverified and is never counted as accepted.
"""
import json
from pathlib import Path

from provider_eval.stats import wilson

# Checked turns an arm needs before its rate is more than a guess (as in jev-routing).
MIN_VERIFIED = 5
FAILED = ("failed", "no_route")


def read_turns(path):
    """The turn events of one audit.jsonl; lines that are not JSON or not turns are skipped."""
    turns = []
    for line in Path(path).read_text().splitlines():
        try:
            row = json.loads(line)
        except ValueError:
            continue
        if isinstance(row, dict) and row.get("event") == "turn":
            turns.append(row)
    return turns


def acceptance_of(turn):
    """accepted / rejected / unverified. Lines written before acceptance existed carry only the
    check's verdict, which still decides."""
    acceptance = (turn.get("quality") or {}).get("acceptance", "unverified")
    if acceptance == "unverified":
        return {"passed": "accepted", "failed": "rejected"}.get(turn.get("verdict"), "unverified")
    return "accepted" if acceptance == "accepted" else "rejected"


def summarise(turns):
    """The numbers of one arm."""
    out = dict.fromkeys(("turns", "accepted", "rejected", "unverified", "failed", "rework", "retries", "tokens"), 0)
    for t in turns:
        q = t.get("quality") or {}
        out["turns"] += 1
        out[acceptance_of(t)] += 1
        out["failed"] += t.get("outcome") in FAILED
        out["rework"] += q.get("rework", "none") != "none"
        out["retries"] += q.get("retries", 0)
        out["tokens"] += q.get("spent_tokens", 0)
    verified = out["accepted"] + out["rejected"]
    out["verified"] = verified
    out["rate"] = out["accepted"] / verified if verified else None
    out["interval"] = wilson(out["accepted"], verified) if verified else None
    out["tokens_per_accepted"] = out["tokens"] // out["accepted"] if out["accepted"] and out["tokens"] else None
    out["failure_rate"] = out["failed"] / out["turns"] if out["turns"] else None
    return out


def _pct(x):
    return "-" if x is None else f"{x * 100:.0f}%"


def format_arms(arms):
    """A markdown table of `{label: summary}`, one column per arm."""
    labels = list(arms)
    rows = [
        ("turns", lambda s: str(s["turns"])),
        ("accepted tasks", lambda s: str(s["accepted"])),
        ("checked turns (n)", lambda s: str(s["verified"])),
        ("accepted rate", lambda s: "-" if s["rate"] is None else
         f"{_pct(s['rate'])} [{_pct(s['interval'][0])}-{_pct(s['interval'][1])}]" +
         ("" if s["verified"] >= MIN_VERIFIED else " (too few)")),
        ("tokens per accepted task", lambda s: "-" if s["tokens_per_accepted"] is None else str(s["tokens_per_accepted"])),
        ("failure rate", lambda s: _pct(s["failure_rate"])),
        ("rework turns", lambda s: str(s["rework"])),
        ("retries (failovers)", lambda s: str(s["retries"])),
        ("unverified turns", lambda s: str(s["unverified"])),
    ]
    lines = ["| | " + " | ".join(labels) + " |", "|---|" + "---|" * len(labels)]
    lines += [f"| {name} | " + " | ".join(fmt(arms[a]) for a in labels) + " |" for name, fmt in rows]
    return "\n".join(lines)


def compare(arms):
    """One honest sentence per pair of arms with enough checked turns, else why not."""
    labels = list(arms)
    if len(labels) < 2:
        return ""
    base = labels[0]
    notes = []
    for other in labels[1:]:
        a, b = arms[base], arms[other]
        if min(a["verified"], b["verified"]) < MIN_VERIFIED:
            notes.append(f"{other} vs {base}: too few checked turns to compare (need {MIN_VERIFIED} per arm; run with /verify on).")
            continue
        (alo, ahi), (blo, bhi) = a["interval"], b["interval"]
        if blo > ahi:
            verdict = f"{other} accepts more of its work than {base}"
        elif bhi < alo:
            verdict = f"{base} accepts more of its work than {other}"
        else:
            verdict = "the accepted rates are not distinguishable"
        notes.append(f"{other} vs {base}: {verdict}.")
    return "\n".join(notes)


def build_report(arm_paths):
    """`arm_paths` is a list of (label, path); returns the report text."""
    arms = {label: summarise(read_turns(path)) for label, path in arm_paths}
    text = format_arms(arms)
    verdict = compare(arms)
    return text + ("\n\n" + verdict if verdict else "") + "\n"
