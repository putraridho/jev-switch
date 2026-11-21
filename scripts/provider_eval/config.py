"""Constants shared by the provider eval: paths, providers, task map, verdict scoring."""
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
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
