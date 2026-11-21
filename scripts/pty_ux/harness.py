"""The harness: fakes, the pty driver, and the scenario registry and CLI helpers."""

import subprocess

from .fakes import *  # noqa: F401,F403
from .terminal import *  # noqa: F401,F403


# ----------------------------------------------------------------- scenarios

SCENARIOS = []


def scenario(fn):
    SCENARIOS.append(fn)
    return fn


def run_cli(world, args, env=None, stdin=None, timeout=30):
    return subprocess.run(
        [BIN, *args],
        cwd=world.repo,
        env=env if env is not None else world.env(),
        input=stdin,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        stdin=None if stdin is not None else subprocess.DEVNULL,
        text=True,
        timeout=timeout,
    )


def expect_no_escapes(label, text):
    if "\x1b" in text:
        shown = text.replace("\x1b", "<ESC>")
        raise Fail(f"{label} has escape bytes with colour off:\n{shown[:600]}")


def banner_line(p):
    """The first row on screen: the one-line session header."""
    rows = [r for r in p.vt.screen() if r.strip()]
    return rows[0] if rows else ""


def settings_rules(w):
    path = os.path.join(w.repo, ".claude", "settings.local.json")
    if not os.path.exists(path):
        return None
    with open(path) as f:
        return json.load(f).get("permissions", {}).get("allow", [])


def screen_text(vt):
    """The emulated screen, row by row."""
    return "\n".join("".join(row).rstrip() for row in vt.rows)


def cli_args(w):
    with open(os.path.join(w.pids, "args.txt")) as f:
        return f.read().split("\n")
