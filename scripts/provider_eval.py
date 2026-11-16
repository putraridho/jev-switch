#!/usr/bin/env python3
"""Measure the provider routing table from data instead of guessing.

Replays graded prompts from the lunagraph training run on each provider, grades the
answers blind, and compares quality per task type. Subcommands, in order:

  plan    choose the sample and print call counts (no model calls)
  run     ask every provider every prompt (spends quota; needs --yes)
  grade   blind-grade the answers (spends quota; needs --yes)
  report  tables + report.md (no model calls)

Run with `python3 -I scripts/provider_eval.py ...`. Pure logic is importable and tested
in scripts/test_provider_eval.py.
"""
import argparse
import collections
import concurrent.futures as cf
import datetime
import hashlib
import json
import math
import os
import random
import re
import statistics
import subprocess
import sys
import threading
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
DATA_DIR = Path.home() / ".local/state/jev-router/train-lunagraph"
OUT_ROOT = Path.home() / ".local/state/jev-router/provider-eval"
ASK_BIN = REPO / "target/release/ask"
PROVIDERS = {"claude", "deepseek", "codex", "gemini", "grok"}
DEFAULT_GRADER = "claude:opus"  # the model the original run graded with

# APPROXIMATION: lunagraph categories were written for a Sonnet-vs-Opus study, not for
# this router, so the mapping to the router's task types is a judgment call. Task types
# with no lunagraph category (frontend, docs, current_info) are NOT measured by this data.
TASK_MAP = {
    "security": "security",
    "debug": "hard_debug",
    "review": "review",
    "edit_feature": "feature",
    "refactor": "feature",
    "design": "feature",
    "test": "feature",
    "edit_small": "simple_edit",
    "tooling": "simple_edit",
    "explain": "explain",
    "locate": "explain",
    "trace": "explain",
    "vague": "unclear",
    "followup": "unclear",
}

VERDICTS = ("correct", "partial", "wrong")
SCORE = {"correct": 1.0, "partial": 0.5, "wrong": 0.0}
STOP_KINDS = {"quota", "auth", "not_entitled"}  # provider unusable: stop, retry next run
SMALL_N = 10
SELF_PREF_MIN_N = 5
SELF_PREF_DELTA = 0.15
BASELINE_PREFIX = "baseline:"

# Wording reused from <data>/grade/BRIEF.md, the rubric of the original Opus grading.
RUBRIC = """\
Grade coding-assistant answers against the real code. The codebase (read-only snapshot) is your working directory; do not modify anything.

`expected` was written by another model reading the code: a strong hint, NOT gospel. Verify it; if it is wrong and an answer is right, the answer is correct. "[trap]" means the prompt has security-sounding words but is not security work.

The assistants ran read-only: for change requests they DESCRIBED the change. Grade the described change as if applied: right files, right approach, no bug introduced, pitfalls (tests, callers, types) handled.

For each answer open the relevant files and decide:
- "correct": a developer could rely on it; at most minor omissions.
- "partial": right direction but misses something material the developer would trip on.
- "wrong": factually wrong about the code, wrong cause, invented symbols, or a change that wouldn't work.
Be strict and concrete. Answers that refuse, stop without answering, or are an error message are "wrong"."""

ANSWER_NOTE = (
    "\n\n[Harness note: this is a read-only evaluation. Investigate with the read tools; if the request "
    "asks for changes, describe exactly what you would change (files, symbols, pitfalls) instead of "
    "editing. Keep the final answer under 400 words.]"
)


# ------------------------------------------------------------------ small helpers

def read_jsonl(path):
    """Rows of a JSON Lines file; a missing file or a broken line is skipped."""
    path = Path(path)
    if not path.exists():
        return []
    rows = []
    for line in path.read_text().splitlines():
        try:
            rows.append(json.loads(line))
        except ValueError:
            continue
    return rows


def parse_spec(spec):
    """'codex:gpt-6.1-sol' -> ('codex', 'gpt-6.1-sol')."""
    provider, sep, model = spec.partition(":")
    if not sep or not model or provider not in PROVIDERS:
        raise ValueError(f"bad provider spec {spec!r}: want <{'|'.join(sorted(PROVIDERS))}>:<model>")
    return provider, model


def provider_of(candidate):
    """Provider a candidate belongs to; baselines are Claude answers."""
    return "claude" if candidate.startswith(BASELINE_PREFIX) else candidate.split(":", 1)[0]


def stable_int(*parts):
    return int(hashlib.sha256(":".join(map(str, parts)).encode()).hexdigest()[:16], 16)


def task_type_of(category):
    return TASK_MAP.get(category)


# ------------------------------------------------------------------ data loading

def _verdicts(grade_dir, prefix):
    out = {}
    for f in sorted(Path(grade_dir).glob(f"{prefix}-*.out.jsonl")):
        out.update({r["id"]: r for r in read_jsonl(f) if "id" in r})
    return out


def _answers(runs_dir, model):
    out = {}
    for f in sorted(Path(runs_dir).glob(f"sample[0-9].{model}.jsonl")):
        out.update({r["id"]: r["answer"] for r in read_jsonl(f) if r.get("answer")})
    return out


def load_graded_items(data_dir):
    """Every Sonnet-answered, Opus-graded prompt of the main lunagraph run, with the
    original Sonnet (and, for misses, Opus) answers and verdicts attached."""
    data_dir = Path(data_dir)
    sonnet_v, opus_v = _verdicts(data_dir / "grade", "s"), _verdicts(data_dir / "grade", "o")
    sonnet_a, opus_a = _answers(data_dir / "runs", "sonnet"), _answers(data_dir / "runs", "opus")
    items = []
    for row in read_jsonl(data_dir / "runs/sample1.jsonl"):
        pid, ttype = row["id"], task_type_of(row.get("category", ""))
        if pid not in sonnet_v or pid not in sonnet_a or ttype is None:
            continue
        verdict = sonnet_v[pid]["verdict"]
        items.append({
            "id": pid, "category": row["category"], "task_type": ttype,
            "prompt": row["prompt"], "context": row.get("context", ""),
            "files": row.get("files", []), "expected": row.get("expected", ""),
            "sonnet_verdict": verdict, "miss": verdict != "correct",
            "opus_verdict": opus_v.get(pid, {}).get("verdict"),
            "baselines": {"sonnet": sonnet_a[pid], **({"opus": opus_a[pid]} if pid in opus_a else {})},
        })
    return items


# ------------------------------------------------------------------ sampling and plan

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


# ------------------------------------------------------------------ run (answers)

def answer_prompt(item):
    """The prompt a provider receives; same wording the original Sonnet run used."""
    text = f"(Earlier in this session: {item['context']})\n\n{item['prompt']}" if item.get("context") else item["prompt"]
    return text + ANSWER_NOTE


def snapshot_problem(path):
    """Why `path` cannot be used as the eval snapshot, or None. A git repo must be clean."""
    path = Path(path)
    if not path.is_dir():
        return f"snapshot {path} does not exist"
    if (path / ".git").exists():
        out = subprocess.run(["git", "-C", str(path), "status", "--porcelain"], capture_output=True, text=True)
        if out.stdout.strip():
            return f"snapshot {path} is a git repo with uncommitted changes"
    return None


def call_ask(ask_bin, spec, snapshot, prompt, timeout=3600):
    """One `ask` call; always returns the ask JSON shape, even if the binary misbehaves."""
    provider, model = parse_spec(spec)
    try:
        proc = subprocess.run([str(ask_bin), provider, model, str(snapshot)], input=prompt,
                              capture_output=True, text=True, timeout=timeout)
        return json.loads(proc.stdout.strip().splitlines()[-1])
    except (OSError, ValueError, IndexError, subprocess.TimeoutExpired) as e:
        return {"provider": provider, "model": model, "ok": False, "text": "", "usage": None,
                "error_kind": "other", "error": f"ask failed: {e!r}", "wall_ms": None}


def is_finished(row):
    return bool(row.get("ok")) or row.get("error_kind") not in STOP_KINDS


def latest_by_key(rows, key):
    return {key(r): r for r in rows}


def answer_key(row):
    return (row["prompt_id"], row["candidate"])


def pending_answers(plan, rows):
    done = {k for k, r in latest_by_key(rows, answer_key).items() if is_finished(r)}
    return [(it, spec) for it in plan["items"] for spec in plan["providers"] if (it["id"], spec) not in done]


class Appender:
    """Thread-safe JSONL appender."""

    def __init__(self, path):
        self.path, self.lock = Path(path), threading.Lock()

    def add(self, row):
        with self.lock, self.path.open("a") as f:
            f.write(json.dumps(row) + "\n")


def run_answers(plan, out_dir, ask_bin, snapshot, workers=2, runner=call_ask, log=print):
    """Ask every pending (prompt, provider); stops a provider at its first quota error."""
    out = Path(out_dir) / "answers.jsonl"
    todo, sink, stopped = pending_answers(plan, read_jsonl(out)), Appender(out), set()

    def one(job):
        item, spec = job
        provider = provider_of(spec)
        if provider in stopped:
            return None
        res = runner(ask_bin, spec, snapshot, answer_prompt(item))
        row = dict(res, prompt_id=item["id"], task_type=item["task_type"], candidate=spec)
        sink.add(row)
        if row.get("error_kind") in STOP_KINDS:
            if provider not in stopped:
                log(f"STOP {provider}: {row['error_kind']} on {item['id']} ({str(row.get('error'))[:120]}); "
                    "its remaining calls are skipped, re-run later to resume")
            stopped.add(provider)
        return row

    with cf.ThreadPoolExecutor(workers) as ex:
        rows = [r for r in ex.map(one, todo) if r is not None]
    log(f"run: {len(rows)} calls made, {len(todo) - len(rows)} skipped (provider stopped)")
    return rows


# ------------------------------------------------------------------ blind grading

LABELS = "ABCDEFGHIJKLMNOP"


def blind_order(candidates, seed, grader, prompt_id):
    """Shuffle candidates (reproducibly) and label them A, B, C..."""
    order = sorted(candidates)
    random.Random(stable_int(seed, grader, prompt_id)).shuffle(order)
    return list(zip(LABELS, order))


def parse_grades(text, labels):
    """{label: (verdict, why)} for the labels found; JSON lines first, then 'A: correct'."""
    found = {}
    for chunk in re.findall(r"\{[^{}]*\}", text or ""):
        try:
            obj = json.loads(chunk)
        except ValueError:
            continue
        label, verdict = str(obj.get("label", "")).strip().upper(), str(obj.get("verdict", "")).strip().lower()
        if label in labels and verdict in VERDICTS:
            found.setdefault(label, (verdict, str(obj.get("why", ""))[:300]))
    for label, verdict in re.findall(r"\b([A-Z])\b[\s:.)\-*]{1,6}(correct|partial|wrong)\b", text or "", re.I):
        if label in labels:
            found.setdefault(label, (verdict.lower(), ""))
    return found


def deblind(parsed, order, base):
    """Grade rows keyed by the real candidate; unparsed labels become 'unparsed'."""
    rows = []
    for label, candidate in order:
        verdict, why = parsed.get(label, ("unparsed", ""))
        rows.append(dict(base, candidate=candidate, label=label, verdict=verdict, why=why))
    return rows


def grading_prompt(item, order, answers):
    parts = [RUBRIC, "", f"Category: {item['category']}"]
    if item.get("context"):
        parts.append(f"Earlier in the session: {item['context']}")
    parts += [f"Prompt:\n{item['prompt']}", f"Files a correct answer must consult: {', '.join(item.get('files', []))}",
              f"Expected (hint): {item.get('expected', '')}", ""]
    parts += [f"=== Answer {label} ===\n{answers[cand]}\n" for label, cand in order]
    labels = ", ".join(label for label, _ in order)
    parts.append(f"Judge each answer ({labels}) independently; do not guess who wrote them. Reply with one JSON "
                 'object per line and nothing else: {"label":"A","verdict":"correct|partial|wrong","why":"<=40 words citing file:line"}')
    return "\n".join(parts)


def candidate_answers(item, answer_rows, baselines):
    """{candidate: answer text} for one prompt: ok provider answers plus chosen baselines."""
    out = {r["candidate"]: r["text"] for r in answer_rows if r["prompt_id"] == item["id"] and r.get("ok") and r.get("text")}
    for b in baselines:
        if b in item.get("baselines", {}):
            out[BASELINE_PREFIX + b] = item["baselines"][b]
    return out


def pick_cross(prompt_ids, n, seed):
    return set(sorted(prompt_ids, key=lambda p: stable_int(seed, "cross", p))[:n])


def grade_key(row):
    return (row["prompt_id"], row["grader"], row["candidate"])


def grading_jobs(plan, answer_rows, grades, grader, only=None, baselines=("sonnet",)):
    """(item, answers) for prompts this grader has not graded yet."""
    graded = {(r["prompt_id"], r["grader"]) for r in grades if r["verdict"] != "unparsed"}
    jobs = []
    for it in plan["items"]:
        if (it["id"], grader) in graded or (only is not None and it["id"] not in only):
            continue
        answers = candidate_answers(it, answer_rows, baselines)
        if len(answers) > 1 or any(not c.startswith(BASELINE_PREFIX) for c in answers):
            jobs.append((it, answers))
    return jobs


def run_grading(plan, out_dir, ask_bin, snapshot, grader, only=None, baselines=("sonnet",),
                workers=2, runner=call_ask, log=print):
    out = Path(out_dir) / "grades.jsonl"
    jobs = grading_jobs(plan, read_jsonl(Path(out_dir) / "answers.jsonl"), read_jsonl(out), grader, only, baselines)
    sink, stop = Appender(out), threading.Event()

    def one(job):
        item, answers = job
        if stop.is_set():
            return 0
        order = blind_order(answers, plan["seed"], grader, item["id"])
        res = runner(ask_bin, grader, snapshot, grading_prompt(item, order, answers))
        if res.get("error_kind") in STOP_KINDS:
            stop.set()
            log(f"STOP grader {grader}: {res['error_kind']}; re-run later to resume")
            return 0
        parsed = parse_grades(res.get("text", ""), {label for label, _ in order})
        base = {"prompt_id": item["id"], "task_type": item["task_type"], "grader": grader, "seed": plan["seed"]}
        for row in deblind(parsed, order, base):
            sink.add(row)
        return 1

    with cf.ThreadPoolExecutor(workers) as ex:
        done = sum(ex.map(one, jobs))
    log(f"grade[{grader}]: {done}/{len(jobs)} prompts graded")
    return done


# ------------------------------------------------------------------ aggregation

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


# ------------------------------------------------------------------ report

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


# ------------------------------------------------------------------ commands

def load_plan(out_dir):
    path = Path(out_dir) / "plan.json"
    if not path.exists():
        sys.exit(f"no plan at {path}; run `plan` first")
    return json.loads(path.read_text())


def require_snapshot(path):
    problem = snapshot_problem(path)
    if problem:
        sys.exit(problem)


def cmd_plan(a):
    providers = [s.strip() for s in a.providers.split(",") if s.strip()]
    for s in providers + [a.grader] + ([a.cross_grader] if a.cross_grader else []):
        parse_spec(s)
    types = a.types.split(",") if a.types else None
    snapshot = Path(a.snapshot or Path(a.data) / "lunagraph-main")
    plan = build_plan(load_graded_items(a.data), providers, a.per_type, types, a.seed, snapshot,
                      a.grader, a.cross_grader, a.cross_n)
    out = Path(a.out or OUT_ROOT / datetime.date.today().isoformat())
    out.mkdir(parents=True, exist_ok=True)
    (out / "plan.json").write_text(json.dumps(plan, indent=1))
    print(format_plan(plan), f"\n\nwrote {out / 'plan.json'}", sep="")


def _confirm(a, plan, spent):
    if not a.yes:
        print(format_plan(plan))
        print(f"\nDRY RUN: {spent}. Re-run with --yes to spend the quota.")
        return False
    return True


def cmd_run(a):
    plan, out = load_plan(a.out), Path(a.out)
    snapshot = a.snapshot or plan["snapshot"]
    todo = pending_answers(plan, read_jsonl(out / "answers.jsonl"))
    if not _confirm(a, plan, f"{len(todo)} answer calls pending"):
        return
    require_snapshot(snapshot)
    run_answers(plan, out, a.ask_bin, snapshot, a.workers)


def cmd_grade(a):
    plan, out = load_plan(a.out), Path(a.out)
    snapshot = a.snapshot or plan["snapshot"]
    grader, cross = a.grader or plan["grader"], a.cross_grader or plan["cross_grader"]
    baselines = tuple(a.baselines.split(","))
    answers, grades = read_jsonl(out / "answers.jsonl"), read_jsonl(out / "grades.jsonl")
    cross_ids = pick_cross([i["id"] for i in plan["items"]], a.cross_n if a.cross_n is not None else plan["cross_n"], plan["seed"]) if cross else set()
    n_main = len(grading_jobs(plan, answers, grades, grader, None, baselines))
    n_cross = len(grading_jobs(plan, answers, grades, cross, cross_ids, baselines)) if cross else 0
    if not _confirm(a, plan, f"{n_main} grader calls ({grader}) + {n_cross} cross-grader calls"):
        return
    require_snapshot(snapshot)
    run_grading(plan, out, a.ask_bin, snapshot, grader, None, baselines, a.workers)
    if cross:
        run_grading(plan, out, a.ask_bin, snapshot, cross, cross_ids, baselines, a.workers)


def cmd_report(a):
    plan, out = load_plan(a.out), Path(a.out)
    text = build_report(plan, read_jsonl(out / "answers.jsonl"), read_jsonl(out / "grades.jsonl"), a.grader)
    (out / "report.md").write_text(text)
    print(text, f"wrote {out / 'report.md'}", sep="")


def make_parser():
    p = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    sub = p.add_subparsers(dest="cmd", required=True)

    def add(name, fn, help_):
        s = sub.add_parser(name, help=help_)
        s.set_defaults(fn=fn)
        return s

    def common(s, spend=False, out_required=True):
        s.add_argument("--out", required=out_required)
        if spend:
            s.add_argument("--yes", action="store_true", help="spend quota (default: dry run)")
            s.add_argument("--snapshot", help="repo snapshot dir (default: the plan's)")
            s.add_argument("--ask-bin", default=str(ASK_BIN))
            s.add_argument("--workers", type=int, default=2)

    s = add("plan", cmd_plan, "choose the sample, print call counts (no model calls)")
    common(s, out_required=False)
    s.add_argument("--providers", required=True, help="comma list of provider:model")
    s.add_argument("--per-type", type=int, default=5)
    s.add_argument("--types", help="comma list of router task types (default: all mapped)")
    s.add_argument("--snapshot")
    s.add_argument("--data", default=str(DATA_DIR))
    s.add_argument("--seed", type=int, default=1)
    s.add_argument("--grader", default=DEFAULT_GRADER)
    s.add_argument("--cross-grader")
    s.add_argument("--cross-n", type=int, default=10)
    common(add("run", cmd_run, "ask each provider each prompt (spends quota)"), spend=True)
    s = add("grade", cmd_grade, "blind-grade the answers (spends quota)")
    common(s, spend=True)
    s.add_argument("--grader")
    s.add_argument("--cross-grader")
    s.add_argument("--cross-n", type=int)
    s.add_argument("--baselines", default="sonnet", help="comma list of baseline answers to include: sonnet,opus")
    s = add("report", cmd_report, "tables and report.md (no model calls)")
    common(s)
    s.add_argument("--grader", help="grader whose grades fill the tables (default: the most used)")
    return p


def main(argv=None):
    args = make_parser().parse_args(argv)
    args.fn(args)


if __name__ == "__main__":
    main()
