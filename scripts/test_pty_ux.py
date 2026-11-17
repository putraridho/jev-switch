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

import codecs
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
import unicodedata
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
# With `--input-format stream-json` (what jev-code's session uses) it speaks the host
# control protocol: the prompt is ONE JSON line on stdin, a tool that needs approval is a
# `control_request` on stdout and the answer is a `control_response` line on stdin, as in
# tests/fixtures/claude/. Without it the prompt is plain text until EOF, as in `claude -p`.
if [ "$1" = "auth" ]; then
  echo '{"loggedIn": true, "planTier": "pro"}'
  exit 0
fi
host=0
for arg in "$@"; do
  [ "$arg" = "--input-format" ] && host=1
done
if [ $host = 1 ]; then
  # One JSON line: its "\n" escapes stand for the line breaks of the prompt.
  IFS= read -r prompt
  last=$(printf '%s' "$prompt" | sed 's/.*\\n//')
else
  prompt=$(cat)
  last=$(printf '%s' "$prompt" | tail -n 1)
fi
echo $$ > "$FAKE_PIDS/cli.pid"
printf '%s\n' "$@" > "$FAKE_PIDS/args.txt"
answer="FAKE ANSWER hello"
emit() { printf '%s\n' "$1"; }
emit '{"type":"system","subtype":"init","session_id":"sess-1"}'
case "$last" in
  *approve*)
    emit '{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t9","name":"Bash","input":{"command":"cargo test --quiet"}}]}}'
    # Claude Code reads the project's allow rules itself: a saved rule means no question.
    if [ $host = 1 ] && grep -q 'cargo test:\*' .claude/settings.local.json 2>/dev/null; then
      answer="FAKE ALLOWED by rule"
    elif [ $host = 1 ]; then
      emit '{"type":"control_request","request_id":"req-1","request":{"subtype":"can_use_tool","tool_name":"Bash","input":{"command":"cargo test --quiet"}}}'
      IFS= read -r reply
      printf '%s\n' "$reply" > "$FAKE_PIDS/reply.json"
      case "$reply" in
        *'"behavior":"allow"'*) answer="FAKE ALLOWED" ;;
        *) answer="FAKE DENIED" ;;
      esac
    else
      emit '{"type":"system","subtype":"permission_denied","tool_name":"Bash"}'
      answer="FAKE DENIED headless"
    fi
    ;;
  *subagent*|*broadread*)
    # A sub-agent call whose prompt is the part that matters, or a read of a system folder
    # that must never be remembered for good.
    case "$last" in
      *subagent*) req='{"subtype":"can_use_tool","tool_name":"Task","input":{"description":"tidy up","subagent_type":"general-purpose","prompt":"Read the code.\nThen delete the build folder and push to main."}}' ;;
      *) req='{"subtype":"can_use_tool","tool_name":"Read","input":{"file_path":"/etc/ssh/sshd_config"}}' ;;
    esac
    emit "{\"type\":\"control_request\",\"request_id\":\"req-1\",\"request\":$req}"
    IFS= read -r reply
    printf '%s\n' "$reply" > "$FAKE_PIDS/reply.json"
    case "$reply" in
      *'"behavior":"allow"'*) answer="FAKE ALLOWED" ;;
      *) answer="FAKE DENIED" ;;
    esac
    ;;
  *pagewithdraw*)
    # A command too long for the screen (it goes through the pager), taken back while the
    # pager is open; the turn then ends without waiting for any reply.
    emit '{"type":"control_request","request_id":"req-1","request":{"subtype":"can_use_tool","tool_name":"Bash","input":{"command":"@MULTILINE@"}}}'
    sleep 1
    emit '{"type":"control_cancel_request","request_id":"req-1"}'
    sleep 1
    answer="FAKE CARRIED ON"
    ;;
  *withdraw*)
    # Asks, then takes the question back a second later (a hook settled it, say) and goes on
    # without waiting for any reply: the person never got to answer.
    emit '{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t9","name":"Bash","input":{"command":"cargo test --quiet"}}]}}'
    emit '{"type":"control_request","request_id":"req-1","request":{"subtype":"can_use_tool","tool_name":"Bash","input":{"command":"cargo test --quiet"}}}'
    sleep 1
    emit '{"type":"control_cancel_request","request_id":"req-1"}'
    sleep 2
    answer="FAKE CARRIED ON"
    ;;
  *stall*)
    sleep 300 &
    echo $! > "$FAKE_PIDS/child.pid"
    emit '{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t0","name":"Bash","input":{"command":"sleep 300"}}]}}'
    sleep 120
    ;;
  *longcmd*)
    emit '{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"@LONGCMD@"}}]}}'
    sleep 5
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
  *edit*)
    emit '{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t3","name":"Write","input":{"file_path":"new.txt"}}]}}'
    echo hi > new.txt ;;
esac
emit "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"$answer\"}]}}"
emit "{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\"result\":\"$answer\",\"session_id\":\"sess-1\",\"usage\":{\"input_tokens\":10,\"cache_read_input_tokens\":5,\"cache_creation_input_tokens\":0,\"output_tokens\":3}}"
"""

# A 200+ column tool command, with the control characters a real tool call can carry (colour,
# bell, a lone ESC): the status line must show it on ONE row and without them. The JSON
# escapes are written out here and decoded by the program.
LONG_COMMAND = (
    "ls -R | head -100 && echo --- && ls | wc -l && cat log --oneline --decorate --graph --all | head -50"
    " && echo \\u001b[31mred\\u0007 \\u001b and then a good deal more text to push the line well past any narrow terminal"
)

# A 61-line command (JSON escapes, decoded by the program): taller than the 30-row test
# terminal, so its approval question goes through the pager.
MULTILINE_COMMAND = "\\n".join(f"echo line-{i}" for i in range(1, 61)) + "\\nrm -rf ~"

# A pager that keeps what it was given, says it is open, ignores Ctrl-C (as `less` does) and
# stays until the test creates `pager.close`.
FAKE_PAGER = """#!/bin/sh
trap '' INT
cat > "$FAKE_PIDS/paged.txt"
: > "$FAKE_PIDS/pager.open"
while [ ! -e "$FAKE_PIDS/pager.close" ]; do sleep 0.05; done
"""

# `codex login status` that takes longer than the 5 s startup probe cap but succeeds inside the retry cap.
FAKE_SLOW_CODEX = """#!/bin/sh
if [ "$1" = "login" ]; then
  sleep 7
  echo "Logged in using ChatGPT"
  exit 0
fi
exit 0
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
            f.write(FAKE_CLAUDE.replace("@LONGCMD@", LONG_COMMAND).replace("@MULTILINE@", MULTILINE_COMMAND))
        self.pager = os.path.join(self.bin, "fake-pager")
        with open(self.pager, "w") as f:
            f.write(FAKE_PAGER)
        os.chmod(self.pager, 0o755)
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

    def add_slow_codex(self):
        path = os.path.join(self.bin, "codex")
        with open(path, "w") as f:
            f.write(FAKE_SLOW_CODEX)
        os.chmod(path, 0o755)

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


class VT:
    """A tiny terminal emulator: enough to know what is ON SCREEN, which `clean` cannot, because
    it has no width. Printable characters auto-wrap at the terminal width (xterm style: writing
    in the last column only arms the wrap, the next printable character performs it), `\\r`,
    `\\n`, erase-line (`ESC[K`, `ESC[1K`, `ESC[2K`), erase-below (`ESC[J`), cursor moves, and cursor
    show/hide are applied; SGR and every other sequence are ignored. Rows never scroll away, so a
    leftover fragment stays visible for the assertions. A resize crops rows, like a terminal that
    does not reflow."""

    SEQ = re.compile(r"\x1b\[([0-9;?]*)([ -/]*[@-~])")

    def __init__(self, cols):
        self.cols = cols
        self.rows = [[]]
        self.row = 0
        self.col = 0
        self.pending_wrap = False
        self.cursor_visible = True
        self.decoder = codecs.getincrementaldecoder("utf-8")("replace")
        self.tail = ""

    # -- input
    def feed(self, data):
        text = self.tail + self.decoder.decode(data)
        self.tail = ""
        i = 0
        while i < len(text):
            c = text[i]
            if c == "\x1b":
                m = self.SEQ.match(text, i)
                if m:
                    self.csi(m.group(1), m.group(2))
                    i = m.end()
                    continue
                if i + 1 >= len(text) or text[i + 1] == "[":
                    self.tail = text[i:]  # an escape sequence split across two reads
                    return
                i += 2  # ESC 7, ESC 8, ESC =, ...
                continue
            i += 1
            if c == "\r":
                self.col, self.pending_wrap = 0, False
            elif c == "\n":
                self.line_feed()
            elif c in "\x07\x08\x0e\x0f" or ord(c) < 0x20:
                if c == "\x08":
                    self.col = max(0, self.col - 1)
                    self.pending_wrap = False
            else:
                self.put(c)

    def width_of(self, c):
        if unicodedata.combining(c):
            return 0
        return 2 if unicodedata.east_asian_width(c) in "WF" else 1

    def put(self, c):
        w = self.width_of(c)
        if w == 0:
            return
        if self.pending_wrap or self.col + w > self.cols:
            self.line_feed()
            self.col = 0
        line = self.rows[self.row]
        while len(line) < self.col:
            line.append(" ")
        del line[self.col : self.col + w]
        line[self.col : self.col] = [c] + [""] * (w - 1)
        self.col += w
        if self.col >= self.cols:
            self.col, self.pending_wrap = self.cols - 1, True

    def line_feed(self):
        self.pending_wrap = False
        self.row += 1
        while len(self.rows) <= self.row:
            self.rows.append([])

    def csi(self, params, final):
        nums = [int(p) if p.isdigit() else 0 for p in params.lstrip("?").split(";")] if params else []
        n = nums[0] if nums and nums[0] else 1
        line = self.rows[self.row]
        if params.startswith("?"):
            if final == "l" and 25 in nums:
                self.cursor_visible = False
            elif final == "h" and 25 in nums:
                self.cursor_visible = True
        elif final == "K":
            mode = nums[0] if nums else 0
            if mode == 0:
                del line[self.col :]
            elif mode == 1:
                line[: self.col + 1] = [" "] * min(self.col + 1, len(line))
            else:
                del line[:]
        elif final == "J":
            mode = nums[0] if nums else 0
            if mode == 0:
                del line[self.col :]
                del self.rows[self.row + 1 :]
            elif mode in (2, 3):
                self.rows, self.row, self.col = [[]], 0, 0
        elif final == "C":
            self.col = min(self.cols - 1, self.col + n)
            self.pending_wrap = False
        elif final == "D":
            self.col = max(0, self.col - n)
            self.pending_wrap = False
        elif final == "G":
            self.col = min(self.cols - 1, n - 1)
            self.pending_wrap = False
        elif final == "A":
            self.row = max(0, self.row - n)
        elif final == "B":
            self.row += n
            while len(self.rows) <= self.row:
                self.rows.append([])

    def resize(self, cols):
        self.cols = cols
        for line in self.rows:
            del line[cols:]
        self.col = min(self.col, cols - 1)
        self.pending_wrap = False

    # -- what is on screen
    def screen(self):
        return ["".join(line).rstrip() for line in self.rows]

    def status_rows(self):
        """Rows that carry a status-line frame (a spinner glyph) or what is left of one."""
        return [r for r in self.screen() if any(g in r for g in SPINNER) or " · Bash" in r]


class Pty:
    def __init__(self, world, args, name, env=None, cwd=None, cols=100, rows=30):
        self.name = name
        self.raw = b""
        self.counts = {}  # how many times each needle had been seen when expect() last matched it
        self.exit = None
        self.vt = VT(cols)
        self.on_chunk = None  # called with the emulated screen after every read
        env = env if env is not None else world.env()
        pid, fd = pty.fork()
        if pid == 0:
            try:
                fcntl.ioctl(0, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))
                os.chdir(cwd or world.repo)
                os.execve(BIN, [BIN, *args], env)
            finally:
                os._exit(127)
        self.pid, self.fd = pid, fd
        self.eof = False
        self.rows = rows

    def resize(self, cols):
        """Resize the window like a user dragging its edge: the kernel tells the program with SIGWINCH."""
        fcntl.ioctl(self.fd, termios.TIOCSWINSZ, struct.pack("HHHH", self.rows, cols, 0, 0))
        self.vt.resize(cols)

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
                self.vt.feed(chunk)
                if self.on_chunk:
                    self.on_chunk(self.vt)
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


MODE_HINT = re.compile(r"\s*\[(ask|edit|plan|auto) mode \u00b7 shift-tab to change\]\s*$")


def at_empty_prompt(p):
    # An empty prompt shows the permission mode as a dim hint after it.
    return MODE_HINT.sub("", p.visible().rstrip("\n").split("\n")[-1]).strip() == "›"


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
        # `/mod` is ambiguous since /mode exists; `/stat` has one completion.
        p.send("/stat\t")
        p.settle(0.4)
        p.send("\r")
        p.expect("Orchestrate", 10, "`/stat` + Tab to run as /status")
        if "unknown command" in p.visible():
            raise Fail("Tab did not complete /stat to /status")
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


# ------------------------------------------------------------ tool approvals

QUESTION = "Allow Bash: cargo test --quiet?"


def settings_rules(w):
    path = os.path.join(w.repo, ".claude", "settings.local.json")
    if not os.path.exists(path):
        return None
    with open(path) as f:
        return json.load(f).get("permissions", {}).get("allow", [])


def host_reply(w):
    with open(os.path.join(w.pids, "reply.json")) as f:
        return json.loads(f.read())["response"]


@scenario
def a_tool_that_needs_approval_asks_and_y_lets_it_run_once(w):
    p = Pty(w, [w.repo], "approve-yes")
    try:
        prompt_ready(p)
        p.send("approve it quick\r")
        shown = p.expect(QUESTION, 15, "the approval question")
        question = [l for l in shown.split("\n") if QUESTION in l][-1]
        for part in ("[y]es", "[n]o", "[a]lways in this project"):
            if part not in question:
                raise Fail(f"the question should offer {part!r}: {question!r}")
        if spinner_in(p.visible().split(QUESTION)[-1]):
            raise Fail("the status line must stay away while the question waits")
        p.send("y")
        p.expect("✓ allowed", 10, "the allowed line")
        p.expect("FAKE ALLOWED", 10, "the answer after approval")
        p.expect("› ", 10, "the next prompt")
        answer = host_reply(w)["response"]
        if answer.get("behavior") != "allow" or answer.get("updatedInput", {}).get("command") != "cargo test --quiet":
            raise Fail(f"the host answer should allow and echo the input: {answer}")
        if host_reply(w)["request_id"] != "req-1":
            raise Fail("the answer must carry the request id")
        if settings_rules(w) is not None:
            raise Fail("a plain yes must not save any rule")
        quit_with_ctrl_d(p)
    finally:
        p.close()


def spinner_in(text):
    return any(c in text for c in SPINNER)


@scenario
def n_denies_with_a_reason_and_the_turn_goes_on(w):
    p = Pty(w, [w.repo], "approve-no")
    try:
        prompt_ready(p)
        p.send("approve it quick\r")
        p.expect(QUESTION, 15, "the approval question")
        p.send("n")
        p.expect("✗ denied", 10, "the denied line")
        p.expect("FAKE DENIED", 10, "the answer after the denial")
        answer = host_reply(w)["response"]
        if answer.get("behavior") != "deny" or "denied" not in answer.get("message", ""):
            raise Fail(f"the host answer should deny with a reason: {answer}")
        p.expect("› ", 10, "the next prompt")
        quit_with_ctrl_d(p)
    finally:
        p.close()


@scenario
def always_saves_a_private_rule_and_the_next_turn_does_not_ask_again(w):
    p = Pty(w, [w.repo], "approve-always")
    try:
        prompt_ready(p)
        p.send("approve it quick\r")
        p.expect(QUESTION, 15, "the approval question")
        p.send("a")
        p.expect("always in this project: Bash(cargo test:*)", 10, "the saved-rule line")
        p.expect("FAKE ALLOWED", 10, "the answer after approval")
        p.expect("› ", 10, "the next prompt")
        if settings_rules(w) != ["Bash(cargo test:*)"]:
            raise Fail(f"the rule should be saved in .claude/settings.local.json: {settings_rules(w)}")
        mode = os.stat(os.path.join(w.repo, ".claude", "settings.local.json")).st_mode & 0o777
        if mode != 0o600:
            raise Fail(f"the settings file should be private (0600), is {oct(mode)}")
        p.send("approve it quick again\r")
        p.expect("FAKE ALLOWED by rule", 15, "the second turn, answered without a question")
        if p.visible().count(QUESTION) != 1:
            raise Fail(f"the second turn must not ask again:\n{p.tail()}")
        p.expect("› ", 10, "the next prompt")
        p.send("/permissions\r")
        p.expect("Bash(cargo test:*)", 10, "the rule in /permissions")
        quit_with_ctrl_d(p)
    finally:
        p.close()


@scenario
def a_sub_agent_question_shows_the_whole_prompt_before_the_key(w):
    p = Pty(w, [w.repo], "approve-subagent")
    try:
        prompt_ready(p)
        p.send("subagent please\r")
        shown = p.expect("[y]es", 15, "the sub-agent question")
        for part in ("Allow Task:", "description=tidy up", "subagent_type=general-purpose", "prompt=Read the code.",
                     "Then delete the build folder and push to main."):
            if part not in shown:
                raise Fail(f"the question should show {part!r}:\n{p.tail()}")
        p.send("n")
        p.expect("✗ denied", 10, "the denied line")
        p.expect("› ", 10, "the next prompt")
        quit_with_ctrl_d(p)
    finally:
        p.close()


@scenario
def a_read_of_a_system_folder_is_never_offered_always(w):
    p = Pty(w, [w.repo], "approve-broad")
    try:
        prompt_ready(p)
        p.send("broadread please\r")
        shown = p.expect("[y]es", 15, "the read question")
        question = shown.split("Allow Read:")[-1]
        if "[a]lways" in question:
            raise Fail(f"a read in /etc/ssh must not offer always:\n{p.tail()}")
        p.expect("too broad or too sensitive to allow for good", 10, "the reason always is missing")
        p.send("a")
        time.sleep(0.5)
        if "✓" in p.visible().split("Allow Read:")[-1]:
            raise Fail(f"`a` must not answer a question that does not offer it:\n{p.tail()}")
        p.send("y")
        p.expect("✓ allowed", 10, "the allowed line")
        p.expect("FAKE ALLOWED", 10, "the answer")
        p.expect("› ", 10, "the next prompt")
        if settings_rules(w) is not None:
            raise Fail(f"nothing may be saved: {settings_rules(w)}")
        quit_with_ctrl_d(p)
    finally:
        p.close()


@scenario
def ctrl_c_at_the_question_denies_stops_the_turn_and_leaves_no_process_behind(w):
    p = Pty(w, [w.repo], "approve-ctrl-c")
    try:
        prompt_ready(p)
        p.send("approve it quick\r")
        p.expect(QUESTION, 15, "the approval question")
        cli = w.pid_file("cli.pid")
        p.send("\x03")
        p.expect("stopped", 10, "the `stopped` summary after Ctrl-C")
        p.expect("› ", 10, "the prompt after the stopped turn")
        if not p.still_running():
            raise Fail(f"Ctrl-C at the question must not exit the program (exit {p.exit}):\n{p.tail()}")
        wait_dead(cli, "the fake CLI")
        if settings_rules(w) is not None:
            raise Fail("Ctrl-C must not save a rule")
        ask(p, "explain quick")
        quit_with_ctrl_d(p)
    finally:
        p.close()


@scenario
def an_arrow_key_or_enter_at_the_question_does_not_answer_it(w):
    p = Pty(w, [w.repo], "approve-keys")
    try:
        prompt_ready(p)
        p.send("approve it quick\r")
        p.expect(QUESTION, 15, "the approval question")
        # Up arrow is Esc [ A: the `A` must not be read as "always"; Enter and a stray letter say nothing.
        p.send("\x1b[A")
        p.send("\rq")
        p.settle(0.6)
        if "✓" in p.visible().split(QUESTION)[-1] or "denied" in p.visible().split(QUESTION)[-1]:
            raise Fail(f"nothing should have answered the question yet:\n{p.tail()}")
        if settings_rules(w) is not None:
            raise Fail("an arrow key must not save a rule")
        p.send("n")
        p.expect("✗ denied", 10, "the denial once `n` is pressed")
        p.expect("› ", 10, "the next prompt")
        quit_with_ctrl_d(p)
    finally:
        p.close()


def screen_text(vt):
    """The emulated screen, row by row."""
    return "\n".join("".join(row).rstrip() for row in vt.rows)


def local_flags(p):
    """The terminal's local mode flags as the program left them (read off the pty master)."""
    return termios.tcgetattr(p.fd)[3]


@scenario
def a_question_the_cli_withdraws_is_erased_and_the_terminal_and_keyboard_are_left_as_they_were(w):
    p = Pty(w, [w.repo], "withdrawn")
    try:
        prompt_ready(p)
        p.send("withdraw it please\r")
        p.expect(QUESTION, 15, "the approval question")
        if screen_text(p.vt).count(QUESTION) != 1:
            raise Fail(f"the question should be on screen once:\n{p.tail()}")
        asking = local_flags(p)
        if asking & (termios.ICANON | termios.ECHO | termios.ISIG):
            raise Fail("while the question waits the terminal should be raw (no ICANON, ECHO, ISIG)")
        # Nobody presses a key: a second later the CLI takes the question back.
        p.expect("(the tool call was withdrawn)", 10, "the withdrawal line")
        p.settle(0.3)
        screen = screen_text(p.vt)
        if QUESTION in screen or "[y]es" in screen:
            raise Fail(f"the withdrawn question should be gone from the screen:\n{screen}")
        if "✓" in screen.split("withdrawn")[-1] or "denied" in screen or "stopped" in screen:
            raise Fail(f"a withdrawn question is not an answer:\n{screen}")
        # Raw mode is off again: Ctrl-C is a signal. The turn is still running, so keys are read
        # one at a time for type-ahead (no ICANON), and echo stays off under the status line.
        now = local_flags(p)
        if not now & termios.ISIG:
            raise Fail(f"the terminal was left in raw mode after the withdrawal (lflag {now:#x})")
        if now & termios.ECHO:
            raise Fail("echo should be as it was during the turn (off), not switched on early")
        # A key typed now belongs to the next prompt: no stale reader may take it.
        p.send("z")
        p.expect("FAKE CARRIED ON", 15, "the turn going on without the answer")
        p.expect("› ", 10, "the next prompt")
        p.settle(0.4)
        if not p.visible().rstrip("\n").split("\n")[-1].strip().endswith("› z"):
            raise Fail(f"the key typed after the withdrawal should be waiting at the next prompt:\n{p.tail()}")
        if os.path.exists(os.path.join(w.pids, "reply.json")):
            raise Fail("a withdrawn question must never be answered")
        if settings_rules(w) is not None:
            raise Fail("a withdrawn question must not save a rule")
        # The prompt works normally: the keys echo, Ctrl-C is a signal again.
        p.send("yz")
        p.settle(0.4)
        if not p.visible().rstrip("\n").split("\n")[-1].strip().endswith("› zyz"):
            raise Fail(f"typing at the prompt should echo:\n{p.tail()}")
        p.send("\x03")
        p.expect("Ctrl-C again", 10, "the Ctrl-C hint at the prompt")
        ask(p, "explain quick")
        quit_with_ctrl_d(p)
    finally:
        p.close()


def cli_args(w):
    with open(os.path.join(w.pids, "args.txt")) as f:
        return f.read().split("\n")


@scenario
def without_a_terminal_a_tool_that_needs_approval_is_left_to_claude_and_the_hint_is_not_in_the_answer(w):
    # `-p` and piped input have nobody to ask: Claude Code runs as plain `claude -p` (allowed
    # tools and permission mode decide), without the host prompt protocol or the session note.
    r = run_cli(w, ["-p", "approve it quick", w.repo], timeout=30)
    if r.returncode != 0 or r.stdout.strip() != "FAKE DENIED headless":
        raise Fail(f"-p should print only the answer: {r.returncode} {r.stdout!r} {r.stderr!r}")
    if "denied Bash" not in r.stderr:
        raise Fail(f"a denied tool should be reported on stderr:\n{r.stderr}")
    args = cli_args(w)
    if "--permission-prompt-tool" in args or "--input-format" in args or any("jev-code asks the user" in a for a in args):
        raise Fail(f"-p must not use the host prompt protocol or the session note: {args}")
    if "--permission-mode" not in args:
        raise Fail(f"-p keeps the permission mode: {args}")
    piped = run_cli(w, [w.repo], stdin="approve it quick\n/quit\n", timeout=30)
    if piped.returncode != 0 or "FAKE DENIED headless" not in piped.stdout:
        raise Fail(f"a piped session should be decided by claude, not hang: {piped.returncode} {piped.stdout!r} {piped.stderr!r}")
    if "denied Bash" in piped.stdout:
        raise Fail(f"the denial hint must never be in the answer stream:\n{piped.stdout}")
    if "denied Bash" not in piped.stderr:
        raise Fail(f"the hint should be on stderr:\n{piped.stderr}")
    if "--permission-prompt-tool" in cli_args(w):
        raise Fail(f"a piped session must not use the host prompt protocol: {cli_args(w)}")
    if os.path.exists(os.path.join(w.pids, "reply.json")):
        raise Fail("nothing may be asked of the host where nobody can answer")


def wait_for_file(p, path, what, timeout=15):
    deadline = time.time() + timeout
    while not os.path.exists(path):
        if time.time() >= deadline:
            raise Fail(f"{what} did not happen within {timeout}s:\n{p.tail()}")
        p.pump(0.05)


@scenario
def a_question_withdrawn_while_its_pager_is_open_is_never_shown_and_ctrl_c_there_does_not_stop_the_turn(w):
    p = Pty(w, [w.repo], "pager-withdrawn", env=w.env(PAGER=w.pager))
    try:
        prompt_ready(p)
        p.send("pagewithdraw please\r")
        wait_for_file(p, os.path.join(w.pids, "pager.open"), "the pager opening")
        cli = w.pid_file("cli.pid")
        # No key reader holds the terminal while the pager has it: line mode and signals are on.
        paging = local_flags(p)
        if not (paging & termios.ICANON and paging & termios.ISIG):
            raise Fail(f"the terminal should be the pager's, not in raw mode (lflag {paging:#x})")
        # Ctrl-C inside the pager belongs to the pager.
        p.send("\x03")
        # The engine keeps reading the CLI while the pager is open: the withdrawal arrives and
        # the turn runs to its end without waiting for the pager.
        deadline = time.time() + 10
        while alive(cli):
            if time.time() >= deadline:
                raise Fail(f"the CLI should finish while the pager is open:\n{p.tail()}")
            p.pump(0.05)
        p.settle(0.5)
        shown = p.visible().split("pagewithdraw please")[-1]
        for held in ("withdrawn", "FAKE CARRIED ON", "stopped", "[y]es", "›"):
            if held in shown:
                raise Fail(f"nothing may be printed over the pager ({held!r}):\n{p.tail()}")
        open(os.path.join(w.pids, "pager.close"), "w").close()
        p.expect("(the tool call was withdrawn)", 10, "the withdrawal line once the pager closed")
        p.expect("FAKE CARRIED ON", 10, "the answer, held until the pager closed")
        p.expect("› ", 10, "the next prompt")
        shown = p.visible().split("pagewithdraw please")[-1]
        if "[y]es" in shown or "stopped" in shown:
            raise Fail(f"a question withdrawn while paged is never shown, and the turn was not stopped:\n{p.tail()}")
        with open(os.path.join(w.pids, "paged.txt")) as f:
            paged = f.read()
        if "line-60" not in paged or "rm -rf ~" not in paged:
            raise Fail(f"the pager should have had the whole command:\n{paged[-300:]}")
        if os.path.exists(os.path.join(w.pids, "reply.json")):
            raise Fail("a withdrawn question must never be answered")
        # The keyboard works at the prompt, and a Ctrl-C there is a signal again.
        p.send("\x03")
        p.expect("Ctrl-C again", 10, "the Ctrl-C hint at the prompt")
        ask(p, "explain quick")
        quit_with_ctrl_d(p)
    finally:
        p.close()


@scenario
def permissions_command_lists_adds_and_removes_rules(w):
    p = Pty(w, [w.repo], "permissions")
    try:
        prompt_ready(p)
        p.send("/permissions\r")
        p.expect("none yet", 10, "the empty list")
        p.expect("› ", 5, "the prompt")
        p.send("/permissions allow Bash(ls:*)\r")
        p.expect("✓ allowed Bash(ls:*)", 10, "the confirmation")
        p.expect("› ", 5, "the prompt")
        if settings_rules(w) != ["Bash(ls:*)"]:
            raise Fail(f"the rule should be in the project file: {settings_rules(w)}")
        p.send("/permissions remove Bash(ls:*)\r")
        p.expect("✓ removed Bash(ls:*)", 10, "the removal")
        p.expect("› ", 5, "the prompt")
        if settings_rules(w) != []:
            raise Fail(f"the rule should be gone: {settings_rules(w)}")
        p.send("/permissions allow nonsense(\r")
        p.expect("not a permission rule", 10, "the refusal of a bad rule")
        quit_with_ctrl_d(p)
    finally:
        p.close()


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
    for flag in ["--version", "-V", "-v"]:
        v = run_cli(w, [flag])
        if v.returncode != 0 or not v.stdout.startswith("jev-code "):
            raise Fail(f"{flag}: {v.returncode} {v.stdout!r}")
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
    # Session 1 has two turns. Session 2 starts, is offered session 1, ignores the offer and
    # types a prompt: that must NOT destroy session 1 (each session has its own file).
    p = Pty(w, [w.repo], "continue-1")
    try:
        prompt_ready(p)
        ask(p, "explain quick")
        ask(p, "explain again")
        quit_with_ctrl_d(p)
    finally:
        p.close()
    q = Pty(w, [w.repo], "continue-2")
    try:
        prompt_ready(q)
        if "previous session: 2 turns" not in q.visible() or "/resume" not in q.visible():
            raise Fail(f"a saved session should be offered at start:\n{q.tail()}")
        ask(q, "a different topic")
        quit_with_ctrl_d(q)
    finally:
        q.close()
    # -c loads the MOST RECENT session (session 2: one turn), and continues it in place.
    c = Pty(w, ["-c", w.repo], "continue-3")
    try:
        prompt_ready(c)
        if "resumed 1 turn" not in c.visible():
            raise Fail(f"-c should resume the most recent saved session:\n{c.tail()}")
        # /resume again steps back to the one before: the original two turns are still there.
        c.send("/resume\r")
        c.expect("resumed 2 turns", 10, "the original session survived the newer one")
        quit_with_ctrl_d(c)
    finally:
        c.close()


@scenario
def the_permission_mode_carries_over_to_the_next_session(w):
    def start(args, name):
        p = Pty(w, args + [w.repo], name)
        prompt_ready(p)
        return p

    def hint(p):
        m = MODE_HINT.search(p.visible().rstrip("\n").split("\n")[-1])
        return m.group(1) if m else None

    # /mode is kept: the next session starts in it, with no flag.
    p = start([], "mode-1")
    try:
        if hint(p) != "edit":
            raise Fail(f"a first session starts in edit mode:\n{p.tail()}")
        p.send("/mode auto\r")
        p.expect("mode: auto", 10, "/mode confirms")
        quit_with_ctrl_d(p)
    finally:
        p.close()
    # Shift-Tab is kept too, even when the line is thrown away: auto -> ask.
    q = start([], "mode-2")
    try:
        if hint(q) != "auto":
            raise Fail(f"the mode set with /mode should carry over:\n{q.tail()}")
        q.send("\x1b[Z")
        q.expect("ask mode", 10, "shift-tab moves to the next mode")
        quit_with_ctrl_d(q)
    finally:
        q.close()
    # --mode wins for its own session but is not itself kept.
    r = start(["--mode", "plan"], "mode-3")
    try:
        if hint(r) != "plan":
            raise Fail(f"--mode should win over the kept mode:\n{r.tail()}")
        quit_with_ctrl_d(r)
    finally:
        r.close()
    t = start([], "mode-4")
    try:
        if hint(t) != "ask":
            raise Fail(f"the mode kept by Shift-Tab should carry over, and --mode should not replace it:\n{t.tail()}")
        quit_with_ctrl_d(t)
    finally:
        t.close()
    saved = os.path.join(w.state, "jev-router", "mode")
    if not os.path.isfile(saved) or os.stat(saved).st_mode & 0o777 != 0o600:
        raise Fail(f"the kept mode should be a private file at {saved}")


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


def watch_status_rows(p):
    """From now on, fail the moment the emulated screen shows more than one status-line row:
    a frame that wrapped, or leftovers a redraw did not erase."""

    def check(vt):
        rows = vt.status_rows()
        if len(rows) > 1:
            shown = "\n".join(f"      | {r}" for r in rows[:6])
            raise Fail(f"the status line is on {len(rows)} rows at {vt.cols} columns (it must always be one):\n{shown}")

    p.on_chunk = check


def status_width(vt):
    rows = [line for line in vt.rows if any(g in "".join(line) for g in SPINNER)]
    if len(rows) != 1:
        raise Fail(f"expected exactly one status row, found {len(rows)}: {vt.status_rows()}")
    return sum(vt.width_of(c) for c in rows[0] if c)


def strip_known_escapes(raw):
    return re.sub(r"\x1b\[[0-9;?]*[ -/]*[@-~]|\x1b[78]", "", raw)


def expect_one_row_status_then_nothing_left(p, label):
    left = p.vt.status_rows()
    if left:
        raise Fail(f"{label}: fragments of the status line are left on screen after the turn:\n" + "\n".join(f"      | {r}" for r in left))
    tool_rows = [r for r in p.vt.screen() if "⏺ Bash" in r]
    if len(tool_rows) != 1:
        raise Fail(f"{label}: the long tool call should print as ONE row, found {len(tool_rows)}: {tool_rows}")
    if any("[31m" in r or "\x07" in r for r in p.vt.screen()):
        raise Fail(f"{label}: the control characters of the tool command leaked to the screen")


@scenario
def the_status_line_is_one_row_on_a_narrow_terminal_whatever_columns_says(w):
    # A 60-column window, COLUMNS unset (shells rarely export it) and a 200-char tool command.
    p = Pty(w, [w.repo], "narrow", env=w.env(COLUMNS=None), cols=60)
    try:
        prompt_ready(p)
        watch_status_rows(p)
        p.send("longcmd please\r")
        p.expect("→ claude", 15, "the route line")
        p.expect("⏺ Bash", 15, "the tool line")
        p.settle(2.0)
        width = status_width(p.vt)
        if not 20 < width <= 59:
            raise Fail(f"the status frame should fill a 60-column window up to the last-but-one column, it is {width} columns wide")
        p.expect("✓ claude", 20, "the end of the turn")
        p.expect("› ", 10, "the next prompt")
        expect_one_row_status_then_nothing_left(p, "narrow")
        # Nothing but known escape sequences was written for the turn: no lone ESC, bell or other control byte.
        raw = p.raw.decode("utf-8", "replace")
        turn = strip_known_escapes(raw[raw.find("→ claude") : raw.find("✓ claude")])
        stray = sorted({c for c in turn if ord(c) < 0x20 and c not in "\r\n"} | ({"\x7f"} if "\x7f" in turn else set()))
        if stray:
            raise Fail(f"stray control bytes were written during the turn: {[hex(ord(c)) for c in stray]}")
        quit_with_ctrl_d(p)
    finally:
        p.close()


@scenario
def the_status_line_follows_a_resize_in_the_middle_of_a_turn(w):
    # COLUMNS lies (200) and the window starts at 100: only the live window size can be right.
    p = Pty(w, [w.repo], "resize", env=w.env(COLUMNS="200"), cols=100)
    try:
        prompt_ready(p)
        watch_status_rows(p)
        p.send("longcmd please\r")
        p.expect("⏺ Bash", 15, "the tool line")
        p.settle(0.5)
        if not 60 < status_width(p.vt) <= 99:
            raise Fail("at 100 columns the long status line should use most of the row")
        for cols in (50, 30, 120, 41):
            p.resize(cols)
            p.settle(0.45)
            width = status_width(p.vt)
            if width > cols - 1:
                raise Fail(f"after resizing to {cols} columns the status frame is {width} columns wide (the last column must stay free)")
            if cols >= 41 and width < min(cols - 1, 40):
                raise Fail(f"after resizing to {cols} columns the status line is cut far short ({width} columns)")
        p.expect("✓ claude", 20, "the end of the turn")
        p.expect("› ", 10, "the next prompt")
        left = p.vt.status_rows()
        if left:
            raise Fail("fragments of the status line are left after the resized turn:\n" + "\n".join(f"      | {r}" for r in left))
        quit_with_ctrl_d(p)
    finally:
        p.close()


def providers_line(p, name):
    """The row `/providers` prints for one provider (`● codex    -   all models`)."""
    p.send("/providers\r")
    deadline = time.time() + 10
    while True:
        text = p.visible()
        shown = text[text.rfind("› /providers") :] if "› /providers" in text else ""
        rows = re.findall(rf"^\s*[●○✕?]\s+{name}\b.*$", shown, re.M)
        if rows:
            return rows[-1]
        if time.time() > deadline:
            return ""
        p.pump(0.1)


@scenario
def a_slow_provider_probe_is_retried_in_the_background_applied_after_the_turn_and_reported_then(w):
    w.add_slow_codex()  # `codex login status` takes 7 s: past the 5 s startup cap, inside the 20 s retry cap
    started = time.time()
    p = Pty(w, [w.repo], "slow-probe-turn")
    try:
        p.expect("› ", 20, "the prompt")
        if time.time() - started > 9:
            raise Fail("the banner waited for the slow probe: startup must only wait for the 5 s cap")
        banner = p.visible().split("› ")[0]
        if "? codex" not in banner:
            raise Fail(f"a probe that timed out shows as `? codex` in the banner:\n{banner}")
        watch_status_rows(p)
        p.send("please stall forever\r")
        p.expect("⏺ Bash", 15, "the tool line of the stalled turn")
        # The retry answers ~7 s after the banner, mid-turn. Routing never changes under a
        # running turn, and the notice only reports what was applied, so it must NOT appear
        # yet, and the live status line must stay one row.
        p.settle(10.0)
        if "ready now" in p.visible():
            raise Fail(f"the notice was printed before the answer was applied (mid-turn):\n{p.tail()}")
        if len(p.vt.status_rows()) != 1:
            raise Fail("the live status line must stay a single row")
        p.send("\x03")
        p.expect("stopped", 10, "the `stopped` summary")
        # As soon as the turn ends the answer is applied and reported, before the next prompt.
        p.expect("codex is ready now", 10, "the notice right after the turn ends")
        p.expect("› ", 10, "the prompt")
        if p.visible().count("codex is ready now") != 1:
            raise Fail(f"the notice should be printed once:\n{p.tail()}")
        line = providers_line(p, "codex")
        if "● codex" not in line:
            raise Fail(f"/providers should show codex ready (●) after the retry, got {line!r}:\n{p.tail()}")
        quit_with_ctrl_d(p)
    finally:
        p.close()


@scenario
def a_slow_provider_probe_that_finishes_at_the_prompt_is_reported_at_the_next_input(w):
    w.add_slow_codex()
    p = Pty(w, [w.repo], "slow-probe-idle")
    try:
        prompt_ready(p, 20)
        if "? codex" not in p.visible().split("› ")[0]:
            raise Fail(f"the banner should show `? codex`:\n{p.tail()}")
        # Idle at the prompt, the retry finishes ~7 s after the banner. Nothing may be printed
        # over the line being typed, so the notice waits for the next input.
        p.send("typed ")
        p.settle(9.0)
        if "ready now" in p.visible():
            raise Fail(f"the notice landed inside the prompt line:\n{p.tail()}")
        p.send("\x15\r")  # clear the line, send nothing
        p.expect("codex is ready now", 10, "the notice at the next input")
        line = providers_line(p, "codex")
        if "● codex" not in line:
            raise Fail(f"/providers should show codex ready (●), got {line!r}:\n{p.tail()}")
        if p.visible().count("codex is ready now") != 1:
            raise Fail("the notice should be printed once")
        quit_with_ctrl_d(p)
    finally:
        p.close()


# -------------------------------------------------------------------- runner


class Timeout(BaseException):
    pass


def on_alarm(signum, frame):
    raise Timeout()


@scenario
def typing_during_a_turn_queues_a_message_that_runs_after_it(w):
    p = Pty(w, [w.repo], "queue")
    try:
        prompt_ready(p)
        p.send("slow work please\r")
        p.expect("→ claude", 15, "the route of the slow turn")
        p.settle(0.3)
        p.send("and then hello")
        p.expect("and then hello▏", 5, "the typed message at the end of the status line")
        p.send("\r")
        p.expect("(queued: after this turn)", 5, "the queued line")
        p.expect("✓ claude", 20, "the end of the slow turn")
        p.expect("› and then hello", 10, "the queued message run as the next one")
        p.expect(ANSWER, 20, "the answer to the queued message")
        p.expect("› ", 10, "the next prompt")
        quit_with_ctrl_d(p)
    finally:
        p.close()


@scenario
def esc_stops_the_turn_and_sends_what_was_typed_right_after(w):
    p = Pty(w, [w.repo], "esc")
    try:
        prompt_ready(p)
        p.send("slow work please\r")
        p.expect("→ claude", 15, "the route of the slow turn")
        p.settle(0.3)
        p.send("do this now")
        p.expect("do this now▏", 5, "the typed message on the status line")
        p.send("\x1b")
        p.expect("(queued: next, after the stop)", 5, "the message sent with Esc")
        p.expect("stopped", 10, "the stopped turn")
        p.expect("› do this now", 10, "the message run at once")
        p.expect(ANSWER, 20, "its answer")
        quit_with_ctrl_d(p)
    finally:
        p.close()


@scenario
def backslash_enter_adds_a_line_and_bang_runs_a_shell_command(w):
    p = Pty(w, [w.repo], "backslash")
    try:
        prompt_ready(p)
        start = len(p.raw)
        p.send("first line\\\r")
        p.settle(0.6)
        if "→ claude" in p.raw[start:].decode("utf-8", "replace"):
            raise Fail(f"backslash then Enter must not send the message:\n{p.tail()}")
        ask(p, "second line")
        p.send("!echo shell-says-hi\r")
        p.expect("shell-says-hi", 10, "the shell output")
        p.expect("exit 0", 10, "the exit status")
        p.expect("› ", 10, "the next prompt")
        quit_with_ctrl_d(p)
    finally:
        p.close()


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
