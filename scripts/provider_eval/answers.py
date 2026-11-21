"""Ask every provider every prompt through the `ask` binary, resumably."""
import concurrent.futures as cf
import json
import subprocess
import threading
from pathlib import Path

from provider_eval.config import STOP_KINDS
from provider_eval.helpers import parse_spec, provider_of, read_jsonl
from provider_eval.prompts import ANSWER_NOTE


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
