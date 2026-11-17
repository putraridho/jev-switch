#!/usr/bin/env python3
"""Hermetic end-to-end UX test of `jev-code` over a real pseudo-terminal.

Run:   cargo build --bin code && python3 -I scripts/test_pty_ux.py [scenario-substring ...]

Nothing real is touched and no quota is spent:
  * the model CLI is a FAKE `claude` shell script speaking Claude's stream-json,
  * Jev is a FAKE local HTTP server (JEV_TYPESAFE_URL) that answers every question,
  * HOME and XDG_STATE_HOME are temp directories, PATH holds only the fake `claude`
    and the few system tools the program needs (git, sh, sleep, ...), so no real
    provider CLI can ever be started.

Each scenario is independent, has its own timeout, and says what it expected when it fails.
"""

import fcntl
import json
import os
import pty
import re
import select
import shutil
import signal
import struct
import subprocess
import sys
import tempfile
import termios
import threading
import time
import traceback
from http.server import BaseHTTPRequestHandler, HTTPServer

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BIN = os.path.join(ROOT, "target", "debug", "code")
SCENARIO_TIMEOUT = 90
SPINNER = "⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏"
ANSI = re.compile(r"\x1b\[[0-9;?]*[ -/]*[@-~]|\x1b[78]")
ANSWER = "FAKE ANSWER hello"


class Fail(Exception):
    pass


# ---------------------------------------------------------------- the fake Jev


class FakeJev(BaseHTTPRequestHandler):
    """Answers every question: choices with `explain`, yes/no questions with 0.1."""

    def do_POST(self):
        size = int(self.headers.get("content-length", "0"))
        request = json.loads(self.rfile.read(size) or b"{}")
        answers = {}
        for key, question in (request.get("questions") or {}).items():
            if question.get("type") == "choice":
                answers[key] = {"choice": "explain", "confidence": 0.9, "probabilities": {"explain": 0.9}}
            else:
                answers[key] = {"noul": 0.1}
        body = json.dumps({"answers": answers}).encode()
        self.send_response(200)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *args):
        pass


def start_jev():
    server = HTTPServer(("127.0.0.1", 0), FakeJev)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server, f"http://127.0.0.1:{server.server_port}/v1/systemone"


# --------------------------------------------------------------- the fake claude

FAKE_CLAUDE = r"""#!/bin/sh
# Fake `claude`: `auth status` for the availability probe, stream-json for a turn.
if [ "$1" = "auth" ]; then
  echo '{"loggedIn": true, "planTier": "pro"}'
  exit 0
fi
prompt=$(cat)
last=$(printf '%s' "$prompt" | tail -n 1)
echo $$ > "$FAKE_PIDS/cli.pid"
emit() { printf '%s\n' "$1"; }
emit '{"type":"system","subtype":"init","session_id":"sess-1"}'
case "$last" in
  *stall*)
    sleep 300 &
    echo $! > "$FAKE_PIDS/child.pid"
    emit '{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t0","name":"Bash","input":{"command":"sleep 300"}}]}}'
    sleep 120
    ;;
  *slow*)
    emit '{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t1","name":"Read","input":{"file_path":"a.txt"}}]}}'
    sleep 1.5
    emit '{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t2","name":"Bash","input":{"command":"cargo test"}}]}}'
    sleep 1.5
    ;;
  *)
    emit '{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t1","name":"Read","input":{"file_path":"a.txt"}}]}}'
    sleep 0.2
    ;;
esac
case "$last" in
  *fail*)
    emit '{"type":"result","subtype":"error","is_error":true,"result":"segmentation fault"}'
    exit 0
    ;;
  *edit*) echo hi > new.txt ;;
esac
emit '{"type":"assistant","message":{"content":[{"type":"text","text":"FAKE ANSWER hello"}]}}'
emit '{"type":"result","subtype":"success","is_error":false,"result":"FAKE ANSWER hello","session_id":"sess-1","usage":{"input_tokens":10,"cache_read_input_tokens":5,"cache_creation_input_tokens":0,"output_tokens":3}}'
"""

TOOLS = ["git", "sh", "bash", "sleep", "cat", "tail", "head", "env", "printf", "echo", "mkdir", "rm", "uname", "id", "tr", "sed", "grep", "date"]


class World:
    """One hermetic machine: home, state dir, git repo, PATH and a fake claude."""

    def __init__(self, jev_url, name="w"):
        self.tmp = tempfile.mkdtemp(prefix=f"jevux-{name}-")
        self.home = os.path.join(self.tmp, "home")
        self.state = os.path.join(self.tmp, "state")
        self.repo = os.path.join(self.tmp, "repo")
        self.bin = os.path.join(self.tmp, "bin")
        self.pids = os.path.join(self.tmp, "pids")
        for d in (self.home, self.state, self.repo, self.bin, self.pids):
            os.makedirs(d)
        self.jev_url = jev_url
        self.claude = os.path.join(self.bin, "claude")
        with open(self.claude, "w") as f:
            f.write(FAKE_CLAUDE)
        os.chmod(self.claude, 0o755)
        for tool in TOOLS:
            found = shutil.which(tool)
            if found and not os.path.exists(os.path.join(self.bin, tool)):
                os.symlink(found, os.path.join(self.bin, tool))
        self._git("init", "-q")
        with open(os.path.join(self.repo, "a.txt"), "w") as f:
            f.write("one\n")
        self._git("add", ".")
        self._git("commit", "-qm", "init")
        # The first-run hint is its own scenario; everywhere else it is already "shown".
        self.mark_first_run_done()

    def _git(self, *args):
        subprocess.run(
            ["git", "-c", "user.email=t@t", "-c", "user.name=t", *args],
            cwd=self.repo,
            check=True,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )

    @property
    def jev_state(self):
        return os.path.join(self.state, "jev-router")

    def mark_first_run_done(self):
        os.makedirs(self.jev_state, exist_ok=True)
        with open(os.path.join(self.jev_state, "first-run-done"), "w") as f:
            f.write("shown\n")

    def env(self, **extra):
        env = {
            "PATH": self.bin,
            "HOME": self.home,
            "XDG_STATE_HOME": self.state,
            "XDG_DATA_HOME": os.path.join(self.tmp, "data"),
            "TERM": "xterm-256color",
            "COLUMNS": "100",
            "LANG": "C.UTF-8",
            "JEV_API_KEY": "test",
            "JEV_TYPESAFE_URL": self.jev_url,
            "CLAUDE_BIN": self.claude,
            "FAKE_PIDS": self.pids,
            "JEV_TIMEOUT_MS": "3000",
        }
        env.update({k: v for k, v in extra.items() if v is not None})
        for k, v in extra.items():
            if v is None:
                env.pop(k, None)
        return env

    def pid_file(self, name):
        path = os.path.join(self.pids, name)
        deadline = time.time() + 10
        while time.time() < deadline:
            try:
                with open(path) as f:
                    text = f.read().strip()
                    if text:
                        return int(text)
            except FileNotFoundError:
                pass
            time.sleep(0.05)
        raise Fail(f"the fake CLI never wrote {name}")

    def cleanup(self):
        shutil.rmtree(self.tmp, ignore_errors=True)


def alive(pid):
    try:
        with open(f"/proc/{pid}/stat") as f:
            state = f.read().rsplit(")", 1)[1].split()[0]
    except FileNotFoundError:
        return False
    return state != "Z"


def wait_dead(pid, what, timeout=5):
    deadline = time.time() + timeout
    while time.time() < deadline:
        if not alive(pid):
            return
        time.sleep(0.05)
    raise Fail(f"{what} (pid {pid}) is still running after the turn was stopped")


# --------------------------------------------------------------------- the pty


CSI = re.compile(r"\x1b\[([0-9;?]*)([ -/]*[@-~])")


def clean(text):
    """What a terminal would show: escape sequences removed, and the few cursor moves the
    line editor and the status line use (`\\r`, erase-line, cursor right/left) applied, so a
    redrawn line replaces what was on it."""
    lines, line, col, i = [], [], 0, 0
    while i < len(text):
        c = text[i]
        if c == "\x1b":
            m = CSI.match(text, i)
            if m:
                params, final = m.group(1), m.group(2)
                n = int(params) if params.isdigit() else 1
                if final == "C":
                    col += n
                elif final == "D":
                    col = max(0, col - n)
                elif final == "K":
                    if params in ("", "0"):
                        del line[col:]
                    elif params == "2":
                        line = []
                i = m.end()
                continue
            i += 2 if text[i + 1 : i + 2] in ("7", "8") else 1
            continue
        if c == "\n":
            lines.append("".join(line))
            line, col = [], 0
        elif c == "\r":
            col = 0
        else:
            if col < len(line):
                line[col] = c
            else:
                line.extend(" " * (col - len(line)))
                line.append(c)
            col += 1
        i += 1
    lines.append("".join(line))
    return "\n".join(lines)


class Pty:
    def __init__(self, world, args, name, env=None, cwd=None):
        self.name = name
        self.raw = b""
        self.counts = {}  # how many times each needle had been seen when expect() last matched it
        self.exit = None
        env = env if env is not None else world.env()
        pid, fd = pty.fork()
        if pid == 0:
            try:
                fcntl.ioctl(0, termios.TIOCSWINSZ, struct.pack("HHHH", 30, 100, 0, 0))
                os.chdir(cwd or world.repo)
                os.execve(BIN, [BIN, *args], env)
            finally:
                os._exit(127)
        self.pid, self.fd = pid, fd
        self.eof = False

    # -- reading
    def pump(self, timeout=0.1):
        if self.eof:
            time.sleep(min(timeout, 0.05))
            return
        ready, _, _ = select.select([self.fd], [], [], timeout)
        if ready:
            try:
                chunk = os.read(self.fd, 65536)
            except OSError:
                chunk = b""
            if chunk:
                self.raw += chunk
            else:
                self.eof = True

    def visible(self):
        return clean(self.raw.decode("utf-8", "replace"))

    def expect(self, needle, timeout=10, what=None):
        deadline = time.time() + timeout
        while True:
            text = self.visible()
            seen = text.count(needle)
            if seen > self.counts.get(needle, 0):
                self.counts[needle] = seen
                return text
            if time.time() >= deadline or (self.eof and self.poll() is not None):
                raise Fail(f"{what or 'expected'} {needle!r} within {timeout}s, got:\n{self.tail()}")
            self.pump(0.1)

    def settle(self, seconds=0.4):
        end = time.time() + seconds
        while time.time() < end:
            self.pump(0.05)

    def tail(self, n=1500):
        return "    | " + self.visible()[-n:].replace("\n", "\n    | ")

    # -- writing and process state
    def send(self, text):
        os.write(self.fd, text.encode())

    def poll(self):
        if self.exit is None:
            pid, status = os.waitpid(self.pid, os.WNOHANG)
            if pid:
                self.exit = os.waitstatus_to_exitcode(status)
        return self.exit

    def wait_exit(self, timeout=10):
        deadline = time.time() + timeout
        while time.time() < deadline:
            self.pump(0.05)
            code = self.poll()
            if code is not None:
                self.settle(0.1)
                return code
        raise Fail(f"the process did not exit within {timeout}s, output:\n{self.tail()}")

    def still_running(self):
        return self.poll() is None

    def close(self):
        if self.poll() is None:
            try:
                os.kill(self.pid, signal.SIGKILL)
                os.waitpid(self.pid, 0)
            except (ProcessLookupError, ChildProcessError):
                pass
        try:
            os.close(self.fd)
        except OSError:
            pass


def prompt_ready(p, timeout=15):
    p.expect("› ", timeout, "the prompt")


def ask(p, text, answer=ANSWER, timeout=20):
    """Send a message, wait for its answer and for the next prompt."""
    p.send(text + "\r")
    p.expect(answer, timeout, "the answer")
    p.expect("› ", timeout, "the next prompt")


def at_empty_prompt(p):
    return p.visible().rstrip("\n").split("\n")[-1].strip() == "›"


def quit_with_ctrl_d(p):
    # Ctrl-D sent while the editor is not reading yet (a turn just ended) is lost, so wait for the prompt.
    deadline = time.time() + 10
    while not at_empty_prompt(p):
        if time.time() > deadline:
            raise Fail(f"no empty prompt to send Ctrl-D at:\n{p.tail()}")
        p.pump(0.1)
    p.settle(0.2)
    p.send("\x04")
    code = p.wait_exit(10)
    if code != 0:
        raise Fail(f"Ctrl-D should exit with 0, got {code}:\n{p.tail()}")


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


@scenario
def banner_is_compact(w):
    p = Pty(w, [w.repo], "banner")
    try:
        prompt_ready(p)
        before = p.visible().split("› ")[0]
        lines = [l for l in before.split("\n") if l.strip()]
        if len(lines) > 3:
            raise Fail(f"banner should be at most 3 lines, got {len(lines)}:\n{p.tail()}")
        if not any("jev-code" in l and "routing on" in l for l in lines):
            raise Fail(f"first banner line should say `jev-code · <dir> · routing on`:\n{p.tail()}")
        if not any("claude" in l and "●" in l for l in lines):
            raise Fail(f"second banner line should show provider glyphs (● claude ...):\n{p.tail()}")
        if "checking providers" in before.replace("checking providers...", "", 1):
            raise Fail("the progress notice must be erased, not left as a banner line")
        quit_with_ctrl_d(p)
    finally:
        p.close()


@scenario
def a_typed_prompt_runs_a_turn_with_route_tools_answer_and_summary(w):
    p = Pty(w, [w.repo], "turn")
    try:
        prompt_ready(p)
        p.send("explain the code quick\r")
        p.expect("→ claude", 15, "the route line")
        p.expect("⏺ Read a.txt", 15, "the tool line")
        p.expect(ANSWER, 15, "the answer")
        p.expect("✓ claude", 15, "the turn summary")
        summary = p.expect("› ", 15, "the next prompt")
        line = [l for l in summary.split("\n") if "✓ claude" in l][-1]
        for part in ("in 15", "(cached 5)", "out 3"):
            if part not in line:
                raise Fail(f"summary line should contain {part!r}: {line!r}")
        if not re.search(r"· \d+(ms|s)\b", line):
            raise Fail(f"summary line should have the elapsed time: {line!r}")
        quit_with_ctrl_d(p)
    finally:
        p.close()


@scenario
def status_line_shows_during_a_slow_turn_and_is_erased_after(w):
    p = Pty(w, [w.repo], "status")
    try:
        prompt_ready(p)
        start = len(p.raw)
        p.send("slow work please\r")
        deadline = time.time() + 15
        while not any(g in p.raw[start:].decode("utf-8", "replace") for g in SPINNER):
            if time.time() > deadline:
                raise Fail(f"no spinner glyph appeared during a slow turn:\n{p.tail()}")
            p.pump(0.1)
        p.expect("✓ claude", 20, "the end of the slow turn")
        p.expect("› ", 10, "the next prompt")
        turn = p.raw[start:].decode("utf-8", "replace")
        frames = sum(turn.count(g) for g in SPINNER)
        if frames < 5:
            raise Fail(f"the status line should be redrawn about every 100 ms (saw {frames} frames in a 3 s turn)")
        last = max(turn.rfind(g) for g in SPINNER)
        if "\x1b[2K" not in turn[last:]:
            raise Fail("the status line was not erased (no \\x1b[2K after the last spinner frame)")
        if turn.rfind("\x1b[?25h") < turn.find("\x1b[?25l"):
            raise Fail("the cursor was hidden for the status line and never shown again")
        # The line carries the model and what is running.
        if "Bash cargo test" not in turn:
            raise Fail("the tool call should show in the status line / tool lines")
        quit_with_ctrl_d(p)
    finally:
        p.close()


@scenario
def ctrl_c_stops_the_turn_not_the_program_and_leaves_no_process_behind(w):
    p = Pty(w, [w.repo], "ctrl-c-turn")
    try:
        prompt_ready(p)
        p.send("please stall forever\r")
        p.expect("⏺ Bash", 15, "the tool line of the stalled turn")
        cli, child = w.pid_file("cli.pid"), w.pid_file("child.pid")
        if not (alive(cli) and alive(child)):
            raise Fail("the fake CLI and its child should be running before Ctrl-C")
        p.send("\x03")
        p.expect("stopped", 10, "the `stopped` summary after Ctrl-C")
        p.expect("› ", 10, "the prompt after the stopped turn")
        if not p.still_running():
            raise Fail(f"Ctrl-C during a turn must not exit the program (exit {p.exit}):\n{p.tail()}")
        wait_dead(cli, "the fake CLI")
        wait_dead(child, "the fake CLI's child process")
        ask(p, "explain quick")  # the next prompt works
        quit_with_ctrl_d(p)
    finally:
        p.close()


@scenario
def ctrl_c_at_the_prompt_warns_and_a_second_one_exits(w):
    p = Pty(w, [w.repo], "ctrl-c-idle")
    try:
        prompt_ready(p)
        p.send("half typed")
        p.send("\x03")
        p.expect("press Ctrl-C again", 5, "the exit hint")
        p.expect("› ", 5, "a fresh prompt after the first Ctrl-C")
        if not p.still_running():
            raise Fail("the first Ctrl-C at the prompt must not exit")
        p.send("\x03")
        code = p.wait_exit(10)
        if code != 0:
            raise Fail(f"the second Ctrl-C should exit cleanly (0), got {code}")
    finally:
        p.close()


@scenario
def ctrl_c_twice_far_apart_does_not_exit(w):
    p = Pty(w, [w.repo], "ctrl-c-slow")
    try:
        prompt_ready(p)
        p.send("\x03")
        p.expect("press Ctrl-C again", 5, "the exit hint")
        p.expect("› ", 5, "a fresh prompt")
        time.sleep(2.4)
        p.send("\x03")
        p.expect("press Ctrl-C again", 5, "the hint again, because the window had passed")
        if not p.still_running():
            raise Fail("two Ctrl-Cs more than 2 s apart must not exit")
        quit_with_ctrl_d(p)
    finally:
        p.close()


@scenario
def ctrl_d_exits_cleanly_and_saves_the_session(w):
    p = Pty(w, [w.repo], "ctrl-d")
    try:
        prompt_ready(p)
        ask(p, "explain quick")
        quit_with_ctrl_d(p)
        sessions = os.path.join(w.jev_state, "sessions")
        if not (os.path.isdir(sessions) and os.listdir(sessions)):
            raise Fail("the session should have been saved under <state>/sessions")
    finally:
        p.close()


@scenario
def help_lists_every_command_and_the_keys(w):
    p = Pty(w, [w.repo], "help")
    try:
        prompt_ready(p)
        p.send("/help\r")
        p.expect("Keys", 10, "the keys section of /help")
        p.expect("› ", 5, "the prompt")
        shown = p.visible()
        for name in ["/model", "/provider", "/orchestrate", "/new", "/resume", "/diff", "/undo", "/quit", "/help", "/status", "/providers", "/doctor"]:
            if name not in shown:
                raise Fail(f"/help should list {name}:\n{p.tail(2500)}")
        for key in ["Ctrl-C", "Ctrl-D", "Alt-Enter", "Tab"]:
            if key not in shown:
                raise Fail(f"/help should list the key {key}")
        quit_with_ctrl_d(p)
    finally:
        p.close()


@scenario
def tab_completes_a_command(w):
    p = Pty(w, [w.repo], "tab")
    try:
        prompt_ready(p)
        p.send("/mod\t")
        p.settle(0.4)
        p.send("\r")
        p.expect("model: auto", 10, "`/mod` + Tab to run as /model")
        if "unknown command" in p.visible():
            raise Fail("Tab did not complete /mod to /model")
        quit_with_ctrl_d(p)
    finally:
        p.close()


@scenario
def an_unknown_command_suggests_the_closest(w):
    p = Pty(w, [w.repo], "unknown")
    try:
        prompt_ready(p)
        p.send("/provders\r")
        p.expect("unknown command /provders", 10, "the unknown-command message")
        p.expect("Did you mean /providers?", 5, "the suggestion")
        p.expect("/help", 5, "the pointer to /help")
        p.expect("› ", 5, "the prompt")
        if "→ claude" in p.visible():
            raise Fail("a mistyped command must not be sent to the model")
        quit_with_ctrl_d(p)
    finally:
        p.close()


@scenario
def history_persists_across_two_runs(w):
    first = Pty(w, [w.repo], "history-1")
    try:
        prompt_ready(first)
        ask(first, "remember this message quick")
        quit_with_ctrl_d(first)
    finally:
        first.close()
    history = os.path.join(w.jev_state, "history")
    if not os.path.exists(history):
        raise Fail("no history file was written")
    mode = os.stat(history).st_mode & 0o777
    if mode != 0o600:
        raise Fail(f"the history file should be private (0600), is {oct(mode)}")
    second = Pty(w, [w.repo], "history-2")
    try:
        prompt_ready(second)
        second.send("\x1b[A")
        second.expect("remember this message quick", 5, "the previous line recalled with the up arrow")
        second.send("\x15")  # clear the line
        quit_with_ctrl_d(second)
    finally:
        second.close()


@scenario
def alt_enter_and_paste_make_one_multi_line_message(w):
    p = Pty(w, [w.repo], "multiline")
    try:
        prompt_ready(p)
        p.send("first line\x1b\rexplain quick\r")
        p.expect(ANSWER, 15, "an answer to the two-line message")
        p.expect("› ", 10, "the prompt")
        if p.visible().count("→ claude") != 1:
            raise Fail("Alt-Enter should add a line, not send the message")
        before = p.visible().count(ANSWER)
        p.send("\x1b[200~pasted one\nexplain quick two\x1b[201~")
        p.settle(0.5)
        if p.visible().count(ANSWER) != before:
            raise Fail("a bracketed paste with a newline must not send anything by itself")
        p.send("\r")
        p.expect(ANSWER, 15, "an answer to the pasted message")
        p.expect("› ", 10, "the prompt")
        if p.visible().count("→ claude") != 2:
            raise Fail("the pasted two lines should be ONE message (one turn)")
        quit_with_ctrl_d(p)
    finally:
        p.close()


@scenario
def diff_and_undo_review_what_a_turn_changed(w):
    p = Pty(w, [w.repo], "undo")
    try:
        prompt_ready(p)
        p.send("edit a file quick\r")
        p.expect("new.txt", 15, "the changed file under the summary")
        p.expect("/diff to review, /undo to revert", 5, "the review hint")
        p.expect("› ", 5, "the prompt")
        p.send("/diff\r")
        p.expect("+hi", 10, "the diff")
        p.expect("› ", 5, "the prompt")
        p.send("/undo\r")
        p.expect("Revert 1 file? [y/N]", 10, "the confirmation through the editor")
        p.send("y\r")
        p.expect("reverted", 10, "the confirmation of the undo")
        if os.path.exists(os.path.join(w.repo, "new.txt")):
            raise Fail("/undo should have removed the file the turn created")
        quit_with_ctrl_d(p)
    finally:
        p.close()


@scenario
def a_failed_turn_shows_an_error_block_with_next_steps_and_the_session_goes_on(w):
    p = Pty(w, [w.repo], "failed")
    try:
        prompt_ready(p)
        p.send("make it fail\r")
        p.expect("The turn failed", 15, "the error title")
        p.expect("segmentation fault", 5, "the CLI's message")
        p.expect("jev-code doctor", 5, "a next step")
        p.expect("› ", 5, "the prompt")
        shown = p.visible()
        if "Other" in shown or "Err(" in shown or "{ " in shown:
            raise Fail(f"no Debug output may reach the user:\n{p.tail()}")
        ask(p, "explain quick")
        quit_with_ctrl_d(p)
    finally:
        p.close()


@scenario
def the_first_run_hint_shows_once_per_machine(w):
    marker = os.path.join(w.jev_state, "first-run-done")
    os.remove(marker)
    first = Pty(w, [w.repo], "first-run-1")
    try:
        prompt_ready(first)
        text = first.visible()
        if "Only one provider is ready" not in text or "/doctor" not in text:
            raise Fail(f"the first start should show the hint and point at /doctor:\n{first.tail()}")
        if not os.path.exists(marker):
            raise Fail("the marker file was not written after showing the hint")
        quit_with_ctrl_d(first)
    finally:
        first.close()
    second = Pty(w, [w.repo], "first-run-2")
    try:
        prompt_ready(second)
        if "Only one provider is ready" in second.visible():
            raise Fail("the hint must never be shown twice")
        second.send("/doctor\r")
        second.expect("claude is ready", 15, "/doctor still available")
        quit_with_ctrl_d(second)
    finally:
        second.close()


@scenario
def print_mode_prints_only_the_answer_on_stdout(w):
    r = run_cli(w, ["-p", "say hi quick", w.repo])
    if r.returncode != 0:
        raise Fail(f"-p should exit 0, got {r.returncode}; stderr:\n{r.stderr}")
    if r.stdout != ANSWER + "\n":
        raise Fail(f"stdout should be exactly the answer, got {r.stdout!r}")
    if "→ claude" not in r.stderr or "✓ claude" not in r.stderr:
        raise Fail(f"route and summary belong on stderr:\n{r.stderr}")
    r2 = run_cli(w, ["-p", "-"], stdin="read this prompt quick\n")
    if r2.returncode != 0 or r2.stdout != ANSWER + "\n":
        raise Fail(f"-p - should read the prompt from stdin: {r2.returncode} {r2.stdout!r} {r2.stderr!r}")


@scenario
def print_mode_exit_codes(w):
    r = run_cli(w, ["-p", "make it fail", w.repo])
    if r.returncode != 1 or r.stdout != "":
        raise Fail(f"a failed one-shot should exit 1 with an empty stdout, got {r.returncode} {r.stdout!r}")
    if "The turn failed" not in r.stderr or "jev-code doctor" not in r.stderr:
        raise Fail(f"the failure should be explained on stderr:\n{r.stderr}")
    empty = run_cli(w, ["-p", "  "])
    if empty.returncode != 2 or "empty" not in empty.stderr:
        raise Fail(f"an empty prompt is a usage error (2), got {empty.returncode}: {empty.stderr!r}")
    p = subprocess.Popen([BIN, "-p", "please stall forever", w.repo], cwd=w.repo, env=w.env(), stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    try:
        cli, child = w.pid_file("cli.pid"), w.pid_file("child.pid")
        time.sleep(0.5)
        p.send_signal(signal.SIGINT)
        try:
            code = p.wait(timeout=15)
        except subprocess.TimeoutExpired:
            raise Fail("-p did not stop after SIGINT")
        if code != 130:
            raise Fail(f"a one-shot stopped with Ctrl-C should exit 130, got {code}")
        wait_dead(cli, "the fake CLI")
        wait_dead(child, "the fake CLI's child process")
    finally:
        p.kill()
        p.wait()
        p.stdout.close()
        p.stderr.close()


@scenario
def no_color_output_has_no_escape_bytes(w):
    env = w.env(NO_COLOR="1")
    r = run_cli(w, ["-p", "say hi quick", w.repo], env=env)
    expect_no_escapes("-p stdout", r.stdout)
    expect_no_escapes("-p stderr", r.stderr)
    r = run_cli(w, ["--no-color", "-p", "say hi quick", w.repo])
    expect_no_escapes("--no-color stderr", r.stderr)
    r = run_cli(w, [w.repo], env=env, stdin="explain quick\n/help\n/status\n/quit\n")
    expect_no_escapes("a piped session's stdout", r.stdout)
    expect_no_escapes("a piped session's stderr", r.stderr)
    d = run_cli(w, ["doctor", w.repo], env=env)
    expect_no_escapes("doctor", d.stdout + d.stderr)
    # On a terminal, no colour and no spinner either (the line editor's own cursor movement stays).
    p = Pty(w, [w.repo], "no-color", env=env)
    try:
        prompt_ready(p)
        p.send("slow work please\r")
        p.expect(ANSWER, 20, "the answer")
        p.expect("› ", 10, "the prompt")
        raw = p.raw.decode("utf-8", "replace")
        if re.search(r"\x1b\[[0-9;]*m", raw):
            raise Fail("NO_COLOR=1 must not produce colour sequences in a session")
        if any(g in raw for g in SPINNER):
            raise Fail("NO_COLOR=1 must not draw the spinner status line")
        quit_with_ctrl_d(p)
    finally:
        p.close()


@scenario
def piped_stdin_still_works_line_by_line(w):
    r = run_cli(w, [w.repo], stdin="explain quick\n/status\nexplain again quick\n/quit\n")
    if r.returncode != 0:
        raise Fail(f"a piped session should exit 0, got {r.returncode}; stderr:\n{r.stderr}")
    if r.stdout.count(ANSWER) != 2:
        raise Fail(f"both piped prompts should be answered:\n{r.stdout}")
    if "jev-code" not in r.stdout or "Routing" not in r.stdout:
        raise Fail(f"the banner and /status belong on stdout:\n{r.stdout}")
    if "\r" in r.stderr.replace("\r\n", "\n"):
        raise Fail("no status-line redraws may be written when stderr is not a terminal")
    eof = run_cli(w, [w.repo], stdin="")
    if eof.returncode != 0:
        raise Fail(f"end of input should end the session with 0, got {eof.returncode}")


@scenario
def doctor_exit_codes(w):
    r = run_cli(w, ["doctor", w.repo])
    if r.returncode != 0:
        raise Fail(f"doctor on a healthy fake setup should exit 0, got {r.returncode}:\n{r.stdout}{r.stderr}")
    if "claude is ready" not in r.stdout or "Jev is reachable" not in r.stdout:
        raise Fail(f"doctor should report claude and Jev:\n{r.stdout}")
    broken = run_cli(w, ["doctor", w.repo], env=w.env(JEV_API_KEY=None))
    if broken.returncode != 1:
        raise Fail(f"doctor with a Problem (no Jev key) should exit 1, got {broken.returncode}:\n{broken.stdout}")
    if "Jev key is missing" not in broken.stdout or "JEV_API_KEY" not in broken.stdout:
        raise Fail(f"doctor should say how to fix the missing key:\n{broken.stdout}")


@scenario
def arguments_help_version_and_errors_never_panic(w):
    h = run_cli(w, ["--help"])
    if h.returncode != 0 or "-p, --print" not in h.stdout or "doctor" not in h.stdout:
        raise Fail(f"--help: {h.returncode} {h.stdout[:300]!r}")
    v = run_cli(w, ["--version"])
    if v.returncode != 0 or not v.stdout.startswith("jev-code "):
        raise Fail(f"--version: {v.returncode} {v.stdout!r}")
    bad = run_cli(w, ["--frobnicate"])
    if bad.returncode != 2 or "unknown option --frobnicate" not in bad.stderr or "panicked" in bad.stderr:
        raise Fail(f"an unknown flag is a usage error (2), got {bad.returncode}: {bad.stderr!r}")
    missing = run_cli(w, ["/definitely/not/a/dir"])
    if missing.returncode != 2 or "Can't open the project folder" not in missing.stderr:
        raise Fail(f"a missing directory should be explained (2), got {missing.returncode}: {missing.stderr!r}")
    nokey = run_cli(w, [w.repo], env=w.env(JEV_API_KEY=None), stdin="/quit\n")
    if nokey.returncode != 1 or "No Jev key found" not in nokey.stderr or "JEV_API_KEY" not in nokey.stderr:
        raise Fail(f"a missing key should be explained with next steps, got {nokey.returncode}: {nokey.stderr!r}")


@scenario
def continue_resumes_the_saved_session(w):
    p = Pty(w, [w.repo], "continue-1")
    try:
        prompt_ready(p)
        ask(p, "explain quick")
        quit_with_ctrl_d(p)
    finally:
        p.close()
    q = Pty(w, [w.repo], "continue-2")
    try:
        prompt_ready(q)
        if "previous session: 1 turn" not in q.visible() or "/resume" not in q.visible():
            raise Fail(f"a saved session should be offered at start:\n{q.tail()}")
        quit_with_ctrl_d(q)
    finally:
        q.close()
    c = Pty(w, ["-c", w.repo], "continue-3")
    try:
        prompt_ready(c)
        if "resumed 1 turn" not in c.visible():
            raise Fail(f"-c should resume the saved session:\n{c.tail()}")
        quit_with_ctrl_d(c)
    finally:
        c.close()


@scenario
def the_terminal_is_restored_when_the_program_ends(w):
    p = Pty(w, [w.repo], "restore")
    try:
        prompt_ready(p)
        p.send("slow work please\r")
        p.expect("› ", 25, "the prompt after a slow turn")
        quit_with_ctrl_d(p)
        raw = p.raw.decode("utf-8", "replace")
        if raw.rfind("\x1b[?25l") > raw.rfind("\x1b[?25h"):
            raise Fail("the cursor was left hidden at exit")
    finally:
        p.close()


# -------------------------------------------------------------------- runner


class Timeout(BaseException):
    pass


def on_alarm(signum, frame):
    raise Timeout()


def main():
    if not os.path.exists(BIN):
        print(f"missing {BIN}: run `cargo build --bin code` first", file=sys.stderr)
        return 2
    wanted = sys.argv[1:]
    jev, url = start_jev()
    signal.signal(signal.SIGALRM, on_alarm)
    failures = 0
    ran = 0
    for fn in SCENARIOS:
        name = fn.__name__.replace("_", " ")
        if wanted and not any(w in fn.__name__ for w in wanted):
            continue
        ran += 1
        world = World(url, fn.__name__[:12])
        started = time.time()
        signal.alarm(SCENARIO_TIMEOUT)
        try:
            fn(world)
            print(f"ok    {name} ({time.time() - started:.1f}s)")
        except Fail as e:
            failures += 1
            print(f"FAIL  {name}\n      {e}".replace("\n", "\n      "))
        except Timeout:
            failures += 1
            print(f"FAIL  {name}\n      timed out after {SCENARIO_TIMEOUT}s")
        except Exception:
            failures += 1
            print(f"FAIL  {name}\n" + traceback.format_exc())
        finally:
            signal.alarm(0)
            world.cleanup()
    jev.shutdown()
    print(f"\n{ran - failures}/{ran} scenarios passed")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
