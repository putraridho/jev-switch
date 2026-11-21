"""Load the graded lunagraph prompts the eval samples from."""
from pathlib import Path

from provider_eval.helpers import read_jsonl, task_type_of


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
