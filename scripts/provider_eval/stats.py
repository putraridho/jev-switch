"""Aggregate grades into scores, intervals, paired comparisons and chain suggestions."""
import collections
import math
import statistics

from provider_eval.answers import answer_key, latest_by_key
from provider_eval.config import BASELINE_PREFIX, SCORE, SELF_PREF_DELTA, SELF_PREF_MIN_N, SMALL_N, VERDICTS
from provider_eval.helpers import provider_of


def score_of(counts):
    n = counts["n"]
    return (counts["correct"] + 0.5 * counts["partial"]) / n if n else None


def wilson(k, n, z=1.96):
    """95% interval for a share; (0, 0) when n == 0."""
    if n == 0:
        return 0.0, 0.0
    p, d = k / n, 1 + z * z / n
    centre = (p + z * z / (2 * n)) / d
    half = z * math.sqrt(p * (1 - p) / n + z * z / (4 * n * n)) / d
    return max(0.0, centre - half), min(1.0, centre + half)


def tally(rows):
    """{(task_type, candidate): {n, correct, partial, wrong}}; unparsed rows are ignored."""
    out = collections.defaultdict(lambda: dict.fromkeys(("n",) + VERDICTS, 0))
    for r in rows:
        if r["verdict"] in VERDICTS:
            c = out[(r["task_type"], r["candidate"])]
            c["n"] += 1
            c[r["verdict"]] += 1
    return dict(out)


def paired_vs_baseline(rows, baseline=BASELINE_PREFIX + "sonnet"):
    """Per (task_type, candidate): prompts graded for both it and the baseline, and the mean
    score difference (candidate minus baseline)."""
    by_prompt = collections.defaultdict(dict)
    for r in rows:
        if r["verdict"] in VERDICTS:
            by_prompt[(r["task_type"], r["prompt_id"])][r["candidate"]] = SCORE[r["verdict"]]
    diffs = collections.defaultdict(list)
    for (ttype, _), scores in by_prompt.items():
        if baseline in scores:
            for cand, s in scores.items():
                if cand != baseline and not cand.startswith(BASELINE_PREFIX):
                    diffs[(ttype, cand)].append(s - scores[baseline])
    return {k: {"n": len(v), "delta": statistics.fmean(v)} for k, v in diffs.items()}


def beats_baseline(rows, baseline=BASELINE_PREFIX + "sonnet"):
    """Where a provider scores above the baseline on shared prompts, best first."""
    wins = [(k, v) for k, v in paired_vs_baseline(rows, baseline).items() if v["delta"] > 0]
    return sorted(wins, key=lambda kv: -kv[1]["delta"])


def self_preference(rows, min_n=SELF_PREF_MIN_N, threshold=SELF_PREF_DELTA):
    """Flag graders that score their own provider's answers above what the other graders
    give the same answers. `rows` may mix graders; needs >= 2 graders to say anything."""
    by_answer = collections.defaultdict(dict)  # (prompt, candidate) -> {grader: score}
    for r in rows:
        if r["verdict"] in SCORE:
            by_answer[(r["prompt_id"], r["candidate"])][r["grader"]] = SCORE[r["verdict"]]
    flags = []
    for grader in sorted({g for s in by_answer.values() for g in s}):
        own, other = [], []
        for (_, cand), scores in by_answer.items():
            others = [v for g, v in scores.items() if g != grader]
            if grader in scores and others and provider_of(cand) == provider_of(grader):
                own.append(scores[grader])
                other.append(statistics.fmean(others))
        if len(own) >= min_n:
            delta = statistics.fmean(own) - statistics.fmean(other)
            flags.append({"grader": grader, "n": len(own), "delta": delta, "flagged": delta >= threshold})
    return flags


def call_stats(answer_rows):
    """Per candidate: calls, ok, quota, other errors, wall seconds (mean, median of ok calls)."""
    by = collections.defaultdict(list)
    for r in latest_by_key(answer_rows, answer_key).values():
        by[r["candidate"]].append(r)
    out = {}
    for cand, rs in by.items():
        walls = [r["wall_ms"] / 1000 for r in rs if r.get("ok") and r.get("wall_ms") is not None]
        out[cand] = {"calls": len(rs), "ok": sum(1 for r in rs if r.get("ok")),
                     "quota": sum(1 for r in rs if r.get("error_kind") == "quota"),
                     "errors": sum(1 for r in rs if not r.get("ok") and r.get("error_kind") != "quota"),
                     "wall_mean": statistics.fmean(walls) if walls else None,
                     "wall_median": statistics.median(walls) if walls else None}
    return out


def suggest_chains(counts, min_n=SMALL_N):
    """Quality-only ordering per task type (providers only, types with n >= min_n for every
    listed candidate). A starting point for JEV_CHAINS, not a decision: cost is not in it."""
    by_type = collections.defaultdict(list)
    for (ttype, cand), c in counts.items():
        if c["n"] >= min_n and not cand.startswith(BASELINE_PREFIX):
            by_type[ttype].append((-score_of(c), cand))
    return {t: [c for _, c in sorted(v)] for t, v in by_type.items()}
