#!/usr/bin/env python3
"""Does Jev's task_type judgment agree with the labelled lunagraph categories?

Reads the JSONL written by `ROUTE_MODE=provider ROUTE_JSON=1 route prompts.jsonl`
(one object per prompt: id, category, prompt, task, task_confidence, security_p,
very_hard_p, continues_p, multi_part_p, model; `"failed": true` rows when Jev gave no
answer) and prints:

  - overall agreement between the mapped category and the predicted task type
  - a confusion matrix (rows = mapped category, columns = Jev's type)
  - per-type precision and recall
  - the task_confidence distribution (share below 0.5 and 0.7)
  - how often the routing decision would differ from "stay on Claude Sonnet"
  - up to 3 example misses per confusion pair

    python3 -I scripts/task_type_accuracy.py out.jsonl [--pairs N] [--examples N]

CAVEAT: the lunagraph categories were written for a Sonnet-vs-Opus study, not for this
router. TASK_MAP (single definition, in provider_eval/config.py) is a judgment call, and
several categories are genuinely ambiguous (refactor and test as "feature", locate and
trace as "explain", followup as "unclear"). Low agreement is therefore a prompt to
read the example misses, NOT proof that Jev is wrong; the examples show which side the
mismatch is on. Categories with no mapping (and the types frontend, docs and
current_info, which no category maps to) are not measured.
"""
import argparse
import collections
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from provider_eval.config import TASK_MAP  # noqa: E402  (the one definition of category -> type)

STAY_MODEL = "claude-sonnet-5-5"  # "no change" for a user already on Claude Sonnet
CONF_CUTS = (0.5, 0.7)


def load_rows(path):
    """Rows of the route output; blank and broken lines are skipped (and counted by the caller
    as missing, since the file is expected to hold one row per prompt)."""
    rows = []
    for line in Path(path).read_text().splitlines():
        line = line.strip()
        if not line:
            continue
        try:
            rows.append(json.loads(line))
        except ValueError:
            continue
    return rows


def analyze(rows, task_map=None, examples=3):
    """All numbers of the report from the route rows. Pure: no printing, no IO."""
    task_map = TASK_MAP if task_map is None else task_map
    failed = [r for r in rows if r.get("failed")]
    answered = [r for r in rows if not r.get("failed") and r.get("task")]
    mapped = [r for r in answered if r.get("category") in task_map]
    unmapped = len(answered) - len(mapped)

    confusion = collections.Counter((task_map[r["category"]], r["task"]) for r in mapped)
    agree = sum(n for (want, got), n in confusion.items() if want == got)

    per_type = {}
    for t in sorted({w for w, _ in confusion} | {g for _, g in confusion}):
        tp = confusion.get((t, t), 0)
        actual = sum(n for (w, _), n in confusion.items() if w == t)
        predicted = sum(n for (_, g), n in confusion.items() if g == t)
        per_type[t] = {
            "actual": actual,
            "predicted": predicted,
            "tp": tp,
            "precision": tp / predicted if predicted else None,
            "recall": tp / actual if actual else None,
        }

    confs = [float(r["task_confidence"]) for r in answered if r.get("task_confidence") is not None]
    conf = {f"below_{c}": (sum(1 for x in confs if x < c) / len(confs) if confs else None) for c in CONF_CUTS}
    conf["n"] = len(confs)

    models = collections.Counter(r.get("model", "none") for r in answered)
    differ = sum(n for m, n in models.items() if m != STAY_MODEL)

    miss_examples = collections.defaultdict(list)
    for r in mapped:
        want = task_map[r["category"]]
        if want != r["task"] and len(miss_examples[(want, r["task"])]) < examples:
            miss_examples[(want, r["task"])].append(r)

    return {
        "rows": len(rows),
        "failed": len(failed),
        "answered": len(answered),
        "mapped": len(mapped),
        "unmapped": unmapped,
        "agree": agree,
        "agreement": agree / len(mapped) if mapped else None,
        "confusion": confusion,
        "per_type": per_type,
        "confidence": conf,
        "models": models,
        "differ": differ,
        "differ_share": differ / len(answered) if answered else None,
        "miss_examples": miss_examples,
    }


def pct(x):
    return "n/a" if x is None else f"{100 * x:.1f}%"


def _matrix(confusion, types):
    w = max([len(t) for t in types] + [8])
    lines = ["  " + "mapped \\ Jev".ljust(w) + " " + " ".join(t[:w].rjust(w) for t in types) + " " + "total".rjust(6)]
    for want in types:
        counts = [confusion.get((want, got), 0) for got in types]
        if not sum(counts):
            continue
        lines.append("  " + want.ljust(w) + " " + " ".join((str(c) if c else ".").rjust(w) for c in counts) + " " + str(sum(counts)).rjust(6))
    return lines


def _clip(text, n=170):
    text = " ".join(str(text).split())
    return text if len(text) <= n else text[: n - 3] + "..."


def render(res, pairs=12):
    """The report as text. `pairs` limits how many confusion pairs show examples
    (most frequent first); 0 shows all."""
    out = ["TASK TYPE ACCURACY", "=" * 18]
    out.append(
        f"{res['rows']} prompts: {res['answered']} answered, {res['failed']} failed (Jev gave no answer), "
        f"{res['unmapped']} answered with a category TASK_MAP does not map (not measured)"
    )
    out.append("")
    out.append("NOTE: the category -> type mapping is approximate (see TASK_MAP in provider_eval/config.py).")
    out.append("Low agreement is a prompt to read the examples below, not proof of a Jev error.")
    out.append("")
    out.append(f"Agreement (mapped category == Jev's type): {res['agree']}/{res['mapped']} = {pct(res['agreement'])}")
    out.append("")

    types = sorted(res["per_type"])
    out.append("Confusion matrix (rows: what the category maps to, columns: what Jev said)")
    out += _matrix(res["confusion"], types)
    out.append("")

    out.append("Per type (precision = of Jev's calls of this type how many the mapping agrees with; recall = of the mapped ones how many Jev found)")
    out.append("  " + "type".ljust(12) + "mapped".rjust(8) + "jev".rjust(8) + "precision".rjust(11) + "recall".rjust(9))
    for t in types:
        s = res["per_type"][t]
        out.append("  " + t.ljust(12) + str(s["actual"]).rjust(8) + str(s["predicted"]).rjust(8) + pct(s["precision"]).rjust(11) + pct(s["recall"]).rjust(9))
    out.append("")

    c = res["confidence"]
    out.append(f"task_confidence (n={c['n']}): below 0.5: {pct(c['below_0.5'])}   below 0.7: {pct(c['below_0.7'])}")
    out.append("")

    out.append(
        f"Routing decision (Claude + Codex Plus, first turn): differs from staying on {STAY_MODEL} for "
        f"{res['differ']}/{res['answered']} = {pct(res['differ_share'])} of answered prompts"
    )
    for m, n in res["models"].most_common():
        out.append(f"  {m:<22}{n:>6}")
    out.append("")

    examples = res["miss_examples"]
    order = sorted(examples, key=lambda k: (-res["confusion"][k], k))
    shown = order if pairs == 0 else order[:pairs]
    out.append(f"Example misses ({len(shown)} of {len(order)} confusion pairs, most frequent first)")
    for want, got in shown:
        out.append(f"* mapped {want} -> Jev {got}  ({res['confusion'][(want, got)]} prompts)")
        for r in examples[(want, got)]:
            out.append(
                f"    [{r.get('id')}] category={r.get('category')} conf={float(r.get('task_confidence', 0)):.2f} "
                f"sec={float(r.get('security_p', 0)):.2f}  {_clip(r.get('prompt', ''))}"
            )
    return "\n".join(out)


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("jsonl", help="output of ROUTE_MODE=provider ROUTE_JSON=1 route ...")
    ap.add_argument("--examples", type=int, default=3, help="example misses per confusion pair (default 3)")
    ap.add_argument("--pairs", type=int, default=12, help="confusion pairs to show examples for, most frequent first; 0 = all (default 12)")
    args = ap.parse_args(argv)
    rows = load_rows(args.jsonl)
    if not rows:
        sys.exit(f"no rows in {args.jsonl}")
    print(render(analyze(rows, examples=args.examples), pairs=args.pairs))


if __name__ == "__main__":
    main()
