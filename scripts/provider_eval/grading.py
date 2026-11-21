"""Blind grading: shuffle and label the answers, ask the grader, de-blind the verdicts."""
import concurrent.futures as cf
import json
import random
import re
import threading
from pathlib import Path

from provider_eval.answers import Appender, call_ask
from provider_eval.config import BASELINE_PREFIX, STOP_KINDS, VERDICTS
from provider_eval.helpers import read_jsonl, stable_int
from provider_eval.prompts import RUBRIC


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
