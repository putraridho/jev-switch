"""Small pure helpers: JSONL reading, provider specs, stable hashing."""
import hashlib
import json
from pathlib import Path

from provider_eval.config import BASELINE_PREFIX, PROVIDERS, TASK_MAP


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
