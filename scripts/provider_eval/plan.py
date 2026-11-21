"""Choose the sample, count model calls, build and format the plan."""
import collections
import datetime
import random

from provider_eval.helpers import provider_of


def select_sample(items, per_type, types=None, seed=1):
    """All known Sonnet misses, plus `per_type` other prompts per task type (seeded)."""
    wanted = set(types) if types else None
    by_type = collections.defaultdict(list)
    for it in sorted(items, key=lambda i: i["id"]):
        if wanted is None or it["task_type"] in wanted:
            by_type[it["task_type"]].append(it)
    chosen = []
    for ttype in sorted(by_type):
        rest = [i for i in by_type[ttype] if not i["miss"]]
        picked = random.Random(f"{seed}:{ttype}").sample(rest, min(per_type, len(rest)))
        chosen += [dict(i, stratum="miss") for i in by_type[ttype] if i["miss"]]
        chosen += [dict(i, stratum="sample") for i in picked]
    return chosen


def count_calls(n_prompts, n_providers, cross_n=0):
    """Model calls: one answer per (prompt, provider); one grader call per prompt
    (it grades every answer to that prompt), plus the cross-grade sample."""
    answers, grading, cross = n_prompts * n_providers, n_prompts, min(cross_n, n_prompts)
    return {"answers": answers, "grading": grading, "cross_grading": cross,
            "total": answers + grading + cross}


def build_plan(items, providers, per_type, types, seed, snapshot, grader, cross_grader, cross_n):
    selected = select_sample(items, per_type, types, seed)
    cross_n = cross_n if cross_grader else 0
    return {
        "created": datetime.datetime.now().isoformat(timespec="seconds"),
        "seed": seed, "snapshot": str(snapshot), "providers": providers,
        "grader": grader, "cross_grader": cross_grader, "cross_n": cross_n,
        "calls": count_calls(len(selected), len(providers), cross_n),
        "items": selected,
    }


def format_plan(plan):
    per_type = collections.defaultdict(collections.Counter)
    for it in plan["items"]:
        per_type[it["task_type"]][it["stratum"]] += 1
    lines = [f"providers: {', '.join(plan['providers'])}",
             f"grader: {plan['grader']}" + (f"  cross-grader: {plan['cross_grader']} (n={plan['cross_n']})"
                                             if plan["cross_grader"] else "  (no cross-grader)"),
             f"snapshot: {plan['snapshot']}", "", f"{'task type':<14}{'prompts':>8}{'misses':>8}{'sampled':>8}"]
    for ttype, c in sorted(per_type.items()):
        lines.append(f"{ttype:<14}{c['miss'] + c['sample']:>8}{c['miss']:>8}{c['sample']:>8}")
    c = plan["calls"]
    lines += ["", f"model calls: {c['answers']} answers + {c['grading']} grading + "
                  f"{c['cross_grading']} cross-grading = {c['total']} total",
              "(calls, not dollars: every call is a full agentic turn in the repo snapshot, so a",
              " prompt costs roughly a Sonnet 'claude -p' run; grader calls read several answers)"]
    if provider_of(plan["grader"]) == "claude" and not plan["cross_grader"]:
        lines.append("note: the grader and the Sonnet baseline are both Claude; add --cross-grader "
                     "to expose self-preference")
    return "\n".join(lines)
