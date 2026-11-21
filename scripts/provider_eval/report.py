"""Render the markdown tables and the report from graded rows."""
import collections
import json

from provider_eval.config import SMALL_N
from provider_eval.stats import (
    beats_baseline,
    call_stats,
    score_of,
    self_preference,
    suggest_chains,
    tally,
    wilson,
)


def _pct(x):
    return "  -  " if x is None else f"{100 * x:4.0f}%"


def quality_table(counts):
    head = f"| task type | candidate | n | correct | partial | wrong | score | correct 95% CI | note |\n|---|---|---|---|---|---|---|---|---|"
    lines = [head]
    for (ttype, cand), c in sorted(counts.items()):
        n = c["n"]
        lo, hi = wilson(c["correct"], n)
        note = f"n<{SMALL_N}: low confidence" if n < SMALL_N else ""
        lines.append(f"| {ttype} | {cand} | {n} | {_pct(c['correct'] / n)} | {_pct(c['partial'] / n)} | "
                     f"{_pct(c['wrong'] / n)} | {score_of(c):.2f} | {100 * lo:.0f}-{100 * hi:.0f}% | {note} |")
    return "\n".join(lines)


def calls_table(stats):
    lines = ["| candidate | calls | ok | quota | other errors | wall mean s | wall median s |", "|---|---|---|---|---|---|---|"]
    fmt = lambda x: "-" if x is None else f"{x:.1f}"
    for cand, s in sorted(stats.items()):
        lines.append(f"| {cand} | {s['calls']} | {s['ok']} | {s['quota']} | {s['errors']} | "
                     f"{fmt(s['wall_mean'])} | {fmt(s['wall_median'])} |")
    return "\n".join(lines)


def beats_section(rows):
    wins = beats_baseline(rows)
    if not wins:
        return "No provider scored above Claude Sonnet on shared prompts."
    lines = []
    for (ttype, cand), v in wins:
        caveat = f" (only {v['n']} shared prompts: low confidence)" if v["n"] < SMALL_N else ""
        lines.append(f"- {ttype}: {cand} beats Sonnet by {v['delta']:+.2f} score over {v['n']} prompts{caveat}")
    return "\n".join(lines)


def self_pref_section(flags):
    if not flags:
        return "Not enough cross-graded answers (need a second grader with >= 5 shared answers) to test."
    return "\n".join(f"- {f['grader']}: scores its own provider {f['delta']:+.2f} vs the other grader over {f['n']} answers"
                     f"{'  **FLAGGED: possible self-preference**' if f['flagged'] else ''}" for f in flags)


def primary_grader(rows, wanted=None):
    if wanted:
        return wanted
    graders = collections.Counter(r["grader"] for r in rows)
    return graders.most_common(1)[0][0] if graders else None


def build_report(plan, answer_rows, grade_rows, grader=None):
    grader = primary_grader(grade_rows, grader)
    main = [r for r in grade_rows if r["grader"] == grader]
    counts = tally(main)
    chains = suggest_chains(counts)
    parts = [f"# Provider eval report\n\nGrader: {grader}. Sample: {len(plan['items'])} prompts "
             f"({sum(1 for i in plan['items'] if i['miss'])} known Sonnet misses, rest sampled). "
             "Score = correct 1, partial 0.5, wrong 0. The sample is enriched with Sonnet misses, "
             "so Sonnet's absolute numbers here are lower than its real-world rate.",
             "## Quality per task type\n\n" + (quality_table(counts) if counts else "No grades yet."),
             "## Where each provider beats Claude Sonnet (paired, same prompts)\n\n" + beats_section(main),
             "## Calls and latency\n\n" + calls_table(call_stats(answer_rows)),
             "## Grader self-preference\n\n" + self_pref_section(self_preference(grade_rows)),
             "## Quality-only chain suggestion (not a decision: add cost, pick the knee)\n\n"
             "Types with n >= 10 per candidate, best score first; apply with `JEV_CHAINS`.\n\n```json\n"
             + json.dumps(chains, indent=2) + "\n```"]
    return "\n\n".join(parts) + "\n"
