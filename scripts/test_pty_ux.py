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
stream=0
for arg in "$@"; do
  [ "$arg" = "--input-format" ] && host=1
  [ "$arg" = "--include-partial-messages" ] && stream=1
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
  *realistic*)
    # A turn as a real one looks: narration, reads, a command with output, an edit, an answer.
    emit '{"type":"assistant","message":{"content":[{"type":"thinking","thinking":"The user reports empty input crashes the parser.\nI should read the parser first, then run its tests.\nIf the empty case returns None, callers may unwrap it.\nCheck the callers too."}]}}'
    emit '{"type":"assistant","message":{"content":[{"type":"text","text":"Let me look at how the parser handles empty input."},{"type":"tool_use","id":"r1","name":"Read","input":{"file_path":"a.txt"}}]}}'
    emit '{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"r1","content":"1\thello\n2\tworld\n3\t!"}]}}'
    emit '{"type":"assistant","message":{"content":[{"type":"tool_use","id":"r2","name":"Bash","input":{"command":"cargo test parser"}}]}}'
    emit '{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"r2","content":"running 4 tests\ntest empty ... FAILED\n\ntest result: FAILED. 3 passed; 1 failed"}]}}'
    emit '{"type":"assistant","message":{"content":[{"type":"text","text":"The empty case returns early without a token. Fixing that."},{"type":"tool_use","id":"r3","name":"Edit","input":{"file_path":"a.txt","old_string":"if s.is_empty() { return None; }","new_string":"if s.is_empty() {\n    return Some(Token::Eof);\n}"}}]}}'
    emit '{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"r3","content":"The file a.txt has been updated."}]}}'
    answer="Fixed: an empty input now yields **Token::Eof** instead of nothing.\\n\\n- the parser test passes\\n- no other caller relied on None"
    ;;
  *streamtest*)
    # With --include-partial-messages the answer is written piece by piece before the whole
    # message and the result arrive. Without it, only the whole answer is sent.
    if [ $stream = 1 ]; then
      emit '{"type":"stream_event","parent_tool_use_id":null,"event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"STREAMCHUNKONE "}}}'
      sleep 1.2
      emit '{"type":"stream_event","parent_tool_use_id":null,"event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"STREAMCHUNKTWO"}}}'
      sleep 0.4
    fi
    answer="STREAMCHUNKONE STREAMCHUNKTWO"
    ;;
  *offernext*)    # An answer that ends by offering the next step: the box suggests that step. The one
    # that pauses leaves time to queue a message during the turn.
    case "$last" in *pause*) sleep 1.5 ;; esac
    answer="Done, the build passes.\\n\\nWant me to commit this and tag v9.9.9?"
    ;;
  *)
    emit '{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t1","name":"Read","input":{"file_path":"a.txt"}}]}}'
    emit '{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"1\thello"}]}}'
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
            # These scenarios test the inline layout; the full-screen one has its own.
            "JEV_INLINE": "1",
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
    """Running and not a zombie. Linux has /proc; elsewhere (macOS) ask ps."""
    try:
        with open(f"/proc/{pid}/stat") as f:
            state = f.read().rsplit(")", 1)[1].split()[0]
        return state != "Z"
    except FileNotFoundError:
        if os.path.isdir("/proc/self"):
            return False
    r = subprocess.run(["ps", "-o", "stat=", "-p", str(pid)], stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
    state = r.stdout.strip()
    return bool(state) and not state.startswith("Z")


def wait_dead(pid, what, timeout=5):
    deadline = time.time() + timeout
    while time.time() < deadline:
        if not alive(pid):
            return
        time.sleep(0.05)
    raise Fail(f"{what} (pid {pid}) is still running after the turn was stopped")


# --------------------------------------------------------------------- the pty


CSI = re.compile(r"\x1b\[([0-9;?<>=]*)([ -/]*[@-~])")


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

    SEQ = re.compile(r"\x1b\[([0-9;?<>=]*)([ -/]*[@-~])")

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
        nums = [int(p) if p.isdigit() else 0 for p in params.lstrip("?<>=").split(";")] if params else []
        if params[:1] in ("<", ">", "="):
            return  # keyboard-protocol and other private requests: nothing on screen
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
        """What the emulated terminal shows, every row since the start (it never scrolls)."""
        return "\n".join(self.vt.screen())

    def printed(self):
        """Everything written, escape sequences removed: every redraw of the live area stays
        in it, so a needle drawn again counts again (`visible` folds redraws together)."""
        return strip_known_escapes(self.raw.decode("utf-8", "replace"))

    def expect(self, needle, timeout=10, what=None):
        deadline = time.time() + timeout
        while True:
            text = self.printed()
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

    def screen_tail(self, n=40):
        """The last rows as the emulated terminal shows them."""
        return "    | " + "\n    | ".join(self.vt.screen()[-n:])

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
        # Close our end first: a killed process whose terminal output nobody reads can be held
        # up draining it (macOS), and the wait would never end.
        running = self.poll() is None
        if running:
            try:
                os.kill(self.pid, signal.SIGKILL)
            except ProcessLookupError:
                running = False
        try:
            os.close(self.fd)
        except OSError:
            pass
        if running:
            try:
                os.waitpid(self.pid, 0)
            except ChildProcessError:
                pass


MODES = ("ask mode", "edit mode", "plan mode", "auto mode")


def footer(p):
    """The last row of the live area: the key hints under the box, a notice, or a card's keys."""
    rows = [r for r in p.vt.screen() if r.strip()]
    return rows[-1] if rows else ""


def idle_now(p):
    """The input box is up and no turn runs: the box's bottom edge names the mode, and neither
    the working row nor the keys of a running turn are there."""
    rows = [r for r in p.vt.screen() if r.strip()][-12:]
    boxed = any(r.rstrip().endswith(tuple(f"{m} ─╯" for m in MODES)) for r in rows)
    busy = any("esc to stop" in r for r in rows) or footer(p).lstrip().startswith("Enter:queue")
    return boxed and not busy


def wait_prompt(p, timeout=15, what="the prompt"):
    """Wait for the input box to be back after something happened: new output arrived since
    this was called (or since the last prompt), no turn is running, and the screen has been
    quiet for a moment (a turn starts within milliseconds of Enter, so a quiet idle screen
    is a finished one)."""
    deadline = time.time() + timeout
    seen = p.counts.get("<prompt>", 0)
    quiet_since, size = time.time(), len(p.raw)
    while True:
        p.pump(0.05)
        if len(p.raw) != size:
            quiet_since, size = time.time(), len(p.raw)
        if size > seen and idle_now(p) and time.time() - quiet_since >= 0.3:
            p.counts["<prompt>"] = size
            return p.visible()
        if time.time() >= deadline or (p.eof and p.poll() is not None):
            raise Fail(f"{what} (the input box, idle) within {timeout}s, got:\n{p.screen_tail()}")


def prompt_ready(p, timeout=15):
    wait_prompt(p, timeout, "the prompt")


def ask(p, text, answer=ANSWER, timeout=20):
    """Send a message, wait for its answer and for the next prompt."""
    p.send(text + "\r")
    p.expect(answer, timeout, "the answer")
    wait_prompt(p, timeout, "the next prompt")


def at_empty_prompt(p):
    """The input box is idle and empty: it shows its placeholder."""
    return idle_now(p) and any("Ask anything" in r for r in p.vt.screen())


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


def banner_line(p):
    """The first row on screen: the one-line session header."""
    rows = [r for r in p.vt.screen() if r.strip()]
    return rows[0] if rows else ""


@scenario
def the_banner_is_one_line_naming_the_ready_providers(w):
    p = Pty(w, [w.repo], "banner")
    try:
        prompt_ready(p)
        rows = [r for r in p.vt.screen() if r.strip()]
        line = banner_line(p)
        if not re.match(r"jev-code \d+\.\d+\.\d+\S* · ", line):
            raise Fail(f"the banner should start with the name and version:\n{p.screen_tail()}")
        if "claude ready" not in line:
            raise Fail(f"the banner should name the ready provider in words:\n{p.screen_tail()}")
        if "⇄ auto" not in rows[1] or "╭" not in rows[2]:
            raise Fail(f"the header row and the input box should follow the one banner line:\n{p.screen_tail()}")
        if "checking providers" in p.visible().replace("checking providers...", "", 1):
            raise Fail("the progress notice must be erased, not left as a banner line")
        quit_with_ctrl_d(p)
    finally:
        p.close()


@scenario
def an_answer_that_offers_a_next_step_suggests_it_and_right_takes_it(w):
    p = Pty(w, [w.repo], "suggest")
    try:
        prompt_ready(p)
        ask(p, "please offernext", answer="Want me to commit this and tag v9.9.9?")
        rows = p.vt.screen()
        box = [r for r in rows if "│ ❯ commit this and tag v9.9.9" in r]
        if not box or not any("→ to use" in r for r in rows):
            raise Fail(f"the empty box should show the offered step and how to take it:\n{p.screen_tail()}")
        p.send("\x1b[C")  # Right
        p.settle(0.3)
        p.send("\r")
        p.expect("❯ commit this and tag v9.9.9", 15, "the suggestion, sent as typed")
        p.expect(ANSWER, 15, "its answer")
        wait_prompt(p, 15, "the next prompt")
        if any("→ to use" in r for r in p.vt.screen()):
            raise Fail(f"an answer without an offer leaves no suggestion:\n{p.screen_tail()}")
        # What is typed is completed from earlier messages, dim; Right takes the rest.
        p.send("please off")
        p.settle(0.3)
        if not any("│ ❯ please offernext" in r for r in p.vt.screen()):
            raise Fail(f"the typed text should be followed by the rest of the earlier message:\n{p.screen_tail()}")
        p.send("\x1b[C")
        p.settle(0.2)
        p.send("\x15")  # Ctrl-U: clear the box again before quitting
        # A message queued during the turn answers the offer: once it has run, even a command
        # that brings no answer, the offer is gone.
        p.send("please offernextpause\r")
        p.expect("❯ please offernextpause", 15, "the sent message")
        p.settle(0.3)
        p.send("/keys\r")
        p.expect("Want me to commit this and tag v9.9.9?", 15, "the answer with its offer")
        wait_prompt(p, 15, "the prompt after the queued command")
        p.settle(0.5)
        if any("→ to use" in r for r in p.vt.screen()):
            raise Fail(f"a queued message that ran should clear the offer:\n{p.screen_tail()}")
        quit_with_ctrl_d(p)
    finally:
        p.close()


@scenario
def update_shows_the_new_commits_asks_then_pulls_and_reinstalls(w):
    # A throwaway checkout and its upstream: JEV_SOURCE_DIR keeps the real repository out of it.
    up, clone = os.path.join(w.tmp, "src-upstream"), os.path.join(w.tmp, "src-clone")
    os.makedirs(os.path.join(up, "scripts"))
    git = lambda cwd, *a: subprocess.run(["git", "-c", "user.email=t@t", "-c", "user.name=t", *a], cwd=cwd, check=True, capture_output=True)
    with open(os.path.join(up, "Cargo.toml"), "w") as f:
        f.write('[package]\nversion = "0.0.1"\n')
    with open(os.path.join(up, "scripts", "install.sh"), "w") as f:
        f.write("#!/bin/sh\necho FAKE INSTALL RAN\n: > installed\n")
    os.chmod(os.path.join(up, "scripts", "install.sh"), 0o755)
    git(up, "init", "-q", "-b", "main")
    git(up, "add", "-A")
    git(up, "commit", "-qm", "first")
    git(w.tmp, "clone", "-q", up, clone)
    with open(os.path.join(up, "Cargo.toml"), "w") as f:
        f.write('[package]\nversion = "9.9.9"\n')
    git(up, "commit", "-qam", "release 9.9.9")

    p = Pty(w, [w.repo], "update", env=w.env(JEV_SOURCE_DIR=clone))
    try:
        prompt_ready(p)
        p.send("/update\r")
        p.expect("→ 9.9.9: 1 new commit on origin/main", 15, "the version change and the commit count")
        p.expect("release 9.9.9", 5, "the incoming commit")
        p.expect("Pull it and reinstall jev-code now?", 10, "the confirmation card")
        p.settle(0.3)
        p.send("y")
        p.expect("FAKE INSTALL RAN", 15, "the install script's output")
        p.expect("jev-code 9.9.9 is installed", 15, "the result")
        if not os.path.exists(os.path.join(clone, "installed")):
            raise Fail("the install script should have run in the checkout")
        wait_prompt(p, 10, "the prompt")
        # Now nothing is newer upstream; the running program is older than the checkout, so
        # it offers to reinstall without pulling, and No leaves everything as it is.
        p.send("/update\r")
        p.expect("the checkout is newer than the installed program", 15, "the reinstall offer")
        p.expect("Reinstall jev-code from the checkout now?", 10, "its confirmation card")
        p.settle(0.3)
        p.send("n")
        p.expect("Left as it is.", 10, "the refusal")
        wait_prompt(p, 10, "the prompt")
        # A tracked change in the checkout stops it before anything happens.
        with open(os.path.join(clone, "Cargo.toml"), "a") as f:
            f.write("# local edit\n")
        p.send("/update\r")
        p.expect("uncommitted changes to tracked files", 15, "the refusal with local changes")
        quit_with_ctrl_d(p)
    finally:
        p.close()


@scenario
def a_typed_prompt_runs_a_turn_with_tools_answer_and_summary(w):
    p = Pty(w, [w.repo], "turn")
    try:
        prompt_ready(p)
        p.send("explain the code quick\r")
        p.expect("❯ explain the code quick", 15, "the sent message, kept in the scrollback")
        p.expect("◇ Explored 1 file", 15, "the run of tool calls as one row")
        p.expect(ANSWER, 15, "the answer")
        p.expect("✓ claude", 15, "the turn summary")
        summary = wait_prompt(p, 15, "the next prompt")
        line = [l for l in summary.split("\n") if "✓ claude" in l][-1]
        if "⇄ claude · claude-sonnet-5-5" not in summary:
            raise Fail(f"the route row should lead the turn with the provider and model:\n{p.screen_tail()}")
        for part in ("in 15", "out 3"):
            if part in line:
                raise Fail(f"token counts belong to /cost, not every turn: {line!r}")
        if not re.search(r"Worked for \d+(ms|s)\b", line):
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
        wait_prompt(p, 10, "the next prompt")
        turn = p.raw[start:].decode("utf-8", "replace")
        frames = sum(turn.count(g) for g in SPINNER)
        if frames < 5:
            raise Fail(f"the status line should be redrawn about every 100 ms (saw {frames} frames in a 3 s turn)")
        last = max(turn.rfind(g) for g in SPINNER)
        if "\x1b[J" not in turn[last:]:
            raise Fail("the status line was not erased (no \\x1b[J after the last spinner frame)")
        if turn.rfind("\x1b[?25h") < turn.find("\x1b[?25l"):
            raise Fail("the cursor was hidden for the status line and never shown again")
        # The line carries the model and what is running.
        if "Running cargo test" not in strip_known_escapes(turn):
            raise Fail("the tool call should show on the working row")
        quit_with_ctrl_d(p)
    finally:
        p.close()


@scenario
def ctrl_c_stops_the_turn_not_the_program_and_leaves_no_process_behind(w):
    p = Pty(w, [w.repo], "ctrl-c-turn")
    try:
        prompt_ready(p)
        p.send("please stall forever\r")
        p.expect("Running sleep 300", 15, "the working row of the stalled turn")
        cli, child = w.pid_file("cli.pid"), w.pid_file("child.pid")
        if not (alive(cli) and alive(child)):
            raise Fail("the fake CLI and its child should be running before Ctrl-C")
        p.send("\x03")
        p.expect("Stopped after", 10, "the `stopped` summary after Ctrl-C")
        wait_prompt(p, 10, "the prompt after the stopped turn")
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
        p.settle(0.3)
        p.send("\x03")
        p.settle(0.4)
        if not at_empty_prompt(p):
            raise Fail(f"Ctrl-C on typed text should clear the box:\n{p.screen_tail()}")
        p.send("\x03")
        p.expect("Press Ctrl-C again to exit", 5, "the exit hint")
        if not p.still_running():
            raise Fail("Ctrl-C at the prompt must not exit at once")
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
        p.expect("Press Ctrl-C again to exit", 5, "the exit hint")
        # Past the 2 s exit window and the 3 s the hint stays up, so a new hint is drawn.
        p.settle(3.4)
        p.send("\x03")
        p.expect("Press Ctrl-C again to exit", 5, "the hint again, because the window had passed")
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
        wait_prompt(p, 5, "the prompt")
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
def agents_and_orchestration_views_show_recorded_workers(w):
    workers = os.path.join(w.state, "jev-router", "workers")
    os.makedirs(workers, exist_ok=True)
    record = {"worker_id": "w1", "path": w.repo, "repo": w.repo, "provider": "codex", "model": "gpt-x", "created_ms": 1000, "running": True}
    with open(os.path.join(workers, "w1.json"), "w") as f:
        json.dump(record, f)
    p = Pty(w, [w.repo], "agents")
    try:
        prompt_ready(p)
        p.send("/agents\r")
        p.expect("codex gpt-x", 10, "the recorded worker in /agents")
        if "running" not in p.visible():
            raise Fail(f"/agents should name the worker's state in words:\n{p.tail(1500)}")
        wait_prompt(p, 5, "the prompt")
        p.send("/orchestration\r")
        p.expect("merge → review → report", 10, "the flow diagram in /orchestration")
        wait_prompt(p, 5, "the prompt")
        quit_with_ctrl_d(p)
    finally:
        p.close()


@scenario
def help_search_tips_cheat_sheet_and_whatsnew_teach_the_tool(w):
    p = Pty(w, [w.repo], "learn")
    try:
        prompt_ready(p)
        p.send("/help diff\r")
        p.expect("Help matching", 10, "the search heading")
        wait_prompt(p, 5, "the prompt")
        if "/diff [" not in p.visible() or "/status" in p.visible().split("Help matching")[-1]:
            raise Fail(f"/help diff should list /diff and only matches:\n{p.tail(1500)}")
        p.send("/tips\r")
        p.expect("Shift-Tab cycles the mode", 10, "the tips")
        wait_prompt(p, 5, "the prompt")
        p.send("/whatsnew\r")
        p.expect("New since", 10, "the changelog")
        wait_prompt(p, 5, "the prompt")
        p.send("?")
        p.expect("vim input mode", 10, "the cheat sheet after ? on an empty box")
        wait_prompt(p, 5, "the prompt")
        quit_with_ctrl_d(p)
    finally:
        p.close()


@scenario
def vim_mode_edits_in_normal_mode_and_sends_on_enter(w):
    p = Pty(w, [w.repo], "vim", env=w.env(JEV_VIM="1"))
    try:
        prompt_ready(p)
        p.send("xexplain quick")
        p.settle(0.3)
        p.send("\x1b")
        p.settle(0.5)
        if "NORMAL" not in p.visible():
            raise Fail(f"Esc should enter vim normal mode:\n{p.screen_tail()}")
        p.send("0x")
        p.settle(0.3)
        p.send("\r")
        p.expect(ANSWER, 15, "an answer to the message edited in normal mode")
        wait_prompt(p, 10, "the prompt")
        quit_with_ctrl_d(p)
    finally:
        p.close()


@scenario
def reduced_motion_keeps_the_working_row_still(w):
    p = Pty(w, [w.repo], "still", env=w.env(JEV_REDUCED_MOTION="1"))
    try:
        prompt_ready(p)
        start = len(p.raw)
        p.send("slow work please\r")
        p.expect("✓ claude", 20, "the end of the slow turn")
        wait_prompt(p, 10, "the next prompt")
        turn = p.raw[start:].decode("utf-8", "replace")
        if any(g in turn for g in SPINNER):
            raise Fail(f"no spinner frame should show with JEV_REDUCED_MOTION:\n{p.tail()}")
        if "•" not in turn:
            raise Fail("a still marker should stand in for the spinner")
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
        wait_prompt(p, 5, "the prompt")
        if "✓ claude" in p.visible():
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
        wait_prompt(p, 10, "the prompt")
        if p.visible().count("✓ claude") != 1:
            raise Fail("Alt-Enter should add a line, not send the message")
        before = p.visible().count(ANSWER)
        p.send("\x1b[200~pasted one\nexplain quick two\x1b[201~")
        p.settle(0.5)
        if p.visible().count(ANSWER) != before:
            raise Fail("a bracketed paste with a newline must not send anything by itself")
        p.send("\r")
        p.expect(ANSWER, 15, "an answer to the pasted message")
        wait_prompt(p, 10, "the prompt")
        if p.visible().count("✓ claude") != 2:
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
        wait_prompt(p, 5, "the prompt")
        p.send("/diff\r")
        p.expect("+ hi", 10, "the diff")
        wait_prompt(p, 5, "the prompt")
        p.send("/undo\r")
        p.expect("Revert 1 file?", 10, "the confirmation card")
        p.settle(0.3)
        p.send("y")
        p.expect("reverted", 10, "the confirmation of the undo")
        if os.path.exists(os.path.join(w.repo, "new.txt")):
            raise Fail("/undo should have removed the file the turn created")
        quit_with_ctrl_d(p)
    finally:
        p.close()


@scenario
def diff_all_and_the_checkpoint_list_name_the_changed_file(w):
    p = Pty(w, [w.repo], "diffall")
    try:
        prompt_ready(p)
        p.send("edit a file quick\r")
        p.expect("/diff to review, /undo to revert", 15, "the review hint")
        wait_prompt(p, 5, "the prompt")
        p.send("/diff --all\r")
        p.expect("+ hi", 10, "the net diff of the session")
        wait_prompt(p, 5, "the prompt")
        p.send("/rewind\r")
        p.expect("/rewind <number>", 10, "the numbered turns and how to rewind")
        quit_with_ctrl_d(p)
    finally:
        p.close()


@scenario
def diff_review_asks_per_hunk_and_a_revert_still_leaves_undo_working(w):
    p = Pty(w, [w.repo], "review")
    try:
        prompt_ready(p)
        p.send("edit a file quick\r")
        p.expect("/diff to review, /undo to revert", 15, "the review hint")
        wait_prompt(p, 5, "the prompt")
        p.send("/diff --review\r")
        p.expect("Revert this hunk?", 10, "the per-hunk question")
        p.settle(0.3)
        p.send("n")
        p.expect("1 kept, 0 reverted", 10, "the review summary")
        wait_prompt(p, 5, "the prompt")
        if not os.path.exists(os.path.join(w.repo, "new.txt")):
            raise Fail("declining the hunk must leave the file")
        p.send("/undo\r")
        p.expect("Revert 1 file?", 10, "the confirmation card")
        p.settle(0.3)
        p.send("y")
        p.expect("reverted", 10, "the confirmation of the undo")
        quit_with_ctrl_d(p)
    finally:
        p.close()


@scenario
def rewind_asks_before_putting_files_back(w):
    p = Pty(w, [w.repo], "rewind")
    try:
        prompt_ready(p)
        p.send("edit a file quick\r")
        p.expect("/diff to review, /undo to revert", 15, "the review hint")
        wait_prompt(p, 5, "the prompt")
        p.send("/rewind 1\r")
        p.expect("Would undo 1 turn", 10, "what a rewind would do")
        p.expect("Rewind to before turn 1?", 10, "the question")
        p.settle(0.3)
        p.send("y")
        p.expect("rewound", 10, "the confirmation of the rewind")
        if os.path.exists(os.path.join(w.repo, "new.txt")):
            raise Fail("/rewind 1 should have removed the file the turn created")
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
        wait_prompt(p, 5, "the prompt")
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
def the_answer_has_a_gutter_naming_who_wrote_it_when_asked_for_and_not_by_default(w):
    p = Pty(w, [w.repo], "gutter", env=w.env(JEV_CODE_GUTTER="1"))
    try:
        prompt_ready(p)
        p.send("explain the code quick\r")
        p.expect("▌ ◆ claude · claude-sonnet-5-5", 15, "the attribution header above the answer")
        p.expect("▌ " + ANSWER, 15, "the answer behind the gutter")
        wait_prompt(p, 15, "the next prompt")
        quit_with_ctrl_d(p)
    finally:
        p.close()
    off = Pty(w, [w.repo], "gutter-off")
    try:
        prompt_ready(off)
        off.send("explain the code quick\r")
        off.expect(ANSWER, 15, "the answer")
        wait_prompt(off, 15, "the next prompt")
        if "▌" in off.visible():
            raise Fail("the answer should have no gutter unless JEV_CODE_GUTTER=1")
        quit_with_ctrl_d(off)
    finally:
        off.close()


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
        wait_prompt(p, 10, "the prompt")
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

QUESTION = "Allow Bash?"
CHOICES = ("1. Yes", "2. Yes, and don't ask again", "3. No, and tell the model what to do instead")


def answer_card(p, key):
    """Press a key at the card once it has been up long enough to be read."""
    p.settle(0.35)
    p.send(key)


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
        p.expect(QUESTION, 15, "the approval card")
        p.expect(CHOICES[2], 5, "the last choice")
        screen = screen_text(p.vt)
        for part in ("cargo test --quiet",) + CHOICES:
            if part not in screen:
                raise Fail(f"the card should show {part!r}:\n{screen}")
        if spinner_in(screen.split(QUESTION)[-1]):
            raise Fail("the working row must stay away while the question waits")
        answer_card(p, "y")
        p.expect("✓ allowed", 10, "the allowed line")
        p.expect("FAKE ALLOWED", 10, "the answer after approval")
        wait_prompt(p, 10, "the next prompt")
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
        answer_card(p, "n")
        p.expect("✗ denied", 10, "the denied line")
        p.expect("FAKE DENIED", 10, "the answer after the denial")
        answer = host_reply(w)["response"]
        if answer.get("behavior") != "deny" or "denied" not in answer.get("message", ""):
            raise Fail(f"the host answer should deny with a reason: {answer}")
        wait_prompt(p, 10, "the next prompt")
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
        answer_card(p, "2")
        p.expect("always in this project: Bash(cargo test:*)", 10, "the saved-rule line")
        p.expect("FAKE ALLOWED", 10, "the answer after approval")
        wait_prompt(p, 10, "the next prompt")
        if settings_rules(w) != ["Bash(cargo test:*)"]:
            raise Fail(f"the rule should be saved in .claude/settings.local.json: {settings_rules(w)}")
        mode = os.stat(os.path.join(w.repo, ".claude", "settings.local.json")).st_mode & 0o777
        if mode != 0o600:
            raise Fail(f"the settings file should be private (0600), is {oct(mode)}")
        p.send("approve it quick again\r")
        p.expect("FAKE ALLOWED by rule", 15, "the second turn, answered without a question")
        if QUESTION in p.printed().split("approve it quick again")[-1]:
            raise Fail(f"the second turn must not ask again:\n{p.tail()}")
        wait_prompt(p, 10, "the next prompt")
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
        p.expect("1. Yes", 15, "the sub-agent question")
        shown = screen_text(p.vt)
        for part in ("Allow Task?", "description=tidy up", "subagent_type=general-purpose", "prompt=Read the code.",
                     "Then delete the build folder and push to main."):
            if part not in shown:
                raise Fail(f"the question should show {part!r}:\n{shown}")
        answer_card(p, "n")
        p.expect("✗ denied", 10, "the denied line")
        wait_prompt(p, 10, "the next prompt")
        quit_with_ctrl_d(p)
    finally:
        p.close()


@scenario
def a_read_of_a_system_folder_is_never_offered_always(w):
    p = Pty(w, [w.repo], "approve-broad")
    try:
        prompt_ready(p)
        p.send("broadread please\r")
        p.expect("1. Yes", 15, "the read question")
        p.expect("too broad or too sensitive to allow for good", 10, "the reason always is missing")
        if "don't ask again" in screen_text(p.vt).split("Allow Read?")[-1]:
            raise Fail(f"a read in /etc/ssh must not offer always:\n{screen_text(p.vt)}")
        answer_card(p, "a")
        time.sleep(0.5)
        if "✓" in p.visible().split("Allow Read?")[-1]:
            raise Fail(f"`a` must not answer a question that does not offer it:\n{p.tail()}")
        p.send("y")
        p.expect("✓ allowed", 10, "the allowed line")
        p.expect("FAKE ALLOWED", 10, "the answer")
        wait_prompt(p, 10, "the next prompt")
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
        p.settle(0.3)
        p.send("\x03")
        p.expect("Stopped after", 10, "the `stopped` summary after Ctrl-C")
        wait_prompt(p, 10, "the prompt after the stopped turn")
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
def keys_typed_ahead_never_answer_a_card_and_arrows_only_move_the_choice(w):
    p = Pty(w, [w.repo], "approve-keys")
    try:
        prompt_ready(p)
        p.send("approve it quick\r")
        p.expect(QUESTION, 15, "the approval question")
        # Enter and y the moment the card appears were typed ahead: they must not answer it.
        p.send("\ry")
        p.settle(0.5)
        if "✓" in p.visible().split(QUESTION)[-1] or "denied" in p.visible().split(QUESTION)[-1]:
            raise Fail(f"keys typed ahead must not answer the card:\n{p.tail()}")
        # Up arrow is Esc [ A: it moves the choice (the `A` is not "always"); a stray letter says nothing.
        p.send("\x1b[Aq")
        p.settle(0.4)
        if "✓" in p.visible().split(QUESTION)[-1] or "denied" in p.visible().split(QUESTION)[-1]:
            raise Fail(f"an arrow key or a stray letter must not answer:\n{p.tail()}")
        if "● 3. No" not in screen_text(p.vt):
            raise Fail(f"↑ from the first choice should select the last:\n{screen_text(p.vt)}")
        if settings_rules(w) is not None:
            raise Fail("an arrow key must not save a rule")
        p.send("\r")
        p.expect("✗ denied", 10, "the denial once `n` is pressed")
        wait_prompt(p, 10, "the next prompt")
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
def a_question_the_cli_withdraws_is_erased_and_the_keyboard_stays_with_the_box(w):
    p = Pty(w, [w.repo], "withdrawn")
    try:
        prompt_ready(p)
        p.send("withdraw it please\r")
        p.expect(QUESTION, 15, "the approval question")
        if screen_text(p.vt).count(QUESTION) != 1:
            raise Fail(f"the question should be on screen once:\n{screen_text(p.vt)}")
        # The screen owns the keyboard the whole session: raw keys, Ctrl-C read as a key.
        if local_flags(p) & (termios.ICANON | termios.ECHO | termios.ISIG):
            raise Fail("while the session runs the terminal should be raw (no ICANON, ECHO, ISIG)")
        # Nobody presses a key: a second later the CLI takes the question back.
        p.expect("(the tool call was withdrawn)", 10, "the withdrawal line")
        p.settle(0.3)
        screen = screen_text(p.vt)
        if QUESTION in screen or "1. Yes" in screen:
            raise Fail(f"the withdrawn question should be gone from the screen:\n{screen}")
        if "✓" in screen.split("withdrawn")[-1] or "denied" in screen or "Stopped after" in screen:
            raise Fail(f"a withdrawn question is not an answer:\n{screen}")
        # A key typed now goes into the box, not to a stale question.
        p.send("z")
        p.expect("FAKE CARRIED ON", 15, "the turn going on without the answer")
        p.settle(0.5)
        if os.path.exists(os.path.join(w.pids, "reply.json")):
            raise Fail("a withdrawn question must never be answered")
        if settings_rules(w) is not None:
            raise Fail("a withdrawn question must not save a rule")
        if "│ ❯ z" not in screen_text(p.vt):
            raise Fail(f"the key typed after the withdrawal should be in the box:\n{screen_text(p.vt)}")
        p.send("yz")
        p.settle(0.4)
        if "│ ❯ zyz" not in screen_text(p.vt):
            raise Fail(f"typing in the box should show:\n{screen_text(p.vt)}")
        p.send("\x15")  # clear the box
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
def a_long_command_scrolls_inside_its_card_and_a_withdrawal_takes_the_card_away(w):
    p = Pty(w, [w.repo], "long-card", env=w.env(PAGER=w.pager))
    try:
        prompt_ready(p)
        p.send("pagewithdraw please\r")
        p.expect("Allow Bash?", 15, "the card of the long command")
        p.expect("more lines · pgup/pgdn to scroll", 5, "the scroll hint of a long command")
        if os.path.exists(os.path.join(w.pids, "pager.open")):
            raise Fail("a long command scrolls in its card: no pager takes the screen")
        if "line-1" not in screen_text(p.vt):
            raise Fail(f"the card should start with the first line:\n{screen_text(p.vt)}")
        # Page down to the end: the whole command can be read before answering.
        p.settle(0.3)
        for _ in range(15):
            p.send("\x1b[6~")
        deadline = time.time() + 5
        while "rm -rf ~" not in screen_text(p.vt):
            if time.time() > deadline:
                raise Fail(f"paging down should reach the end of the command:\n{screen_text(p.vt)}")
            p.pump(0.05)
        # The CLI withdraws it: the card goes, the turn carries on, nothing was answered.
        p.expect("(the tool call was withdrawn)", 10, "the withdrawal line")
        p.expect("FAKE CARRIED ON", 10, "the turn going on")
        wait_prompt(p, 10, "the next prompt")
        if "Allow Bash?" in screen_text(p.vt) or "Stopped after" in p.visible().split("pagewithdraw please")[-1]:
            raise Fail(f"a withdrawn card is gone and the turn was not stopped:\n{screen_text(p.vt)}")
        if os.path.exists(os.path.join(w.pids, "reply.json")):
            raise Fail("a withdrawn question must never be answered")
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
        wait_prompt(p, 5, "the prompt")
        p.send("/permissions allow Bash(ls:*)\r")
        p.expect("✓ allowed Bash(ls:*)", 10, "the confirmation")
        wait_prompt(p, 5, "the prompt")
        if settings_rules(w) != ["Bash(ls:*)"]:
            raise Fail(f"the rule should be in the project file: {settings_rules(w)}")
        p.send("/permissions remove Bash(ls:*)\r")
        p.expect("✓ removed Bash(ls:*)", 10, "the removal")
        wait_prompt(p, 5, "the prompt")
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
def rename_names_the_session_in_the_header_and_in_resume(w):
    p = Pty(w, [w.repo], "rename-1")
    try:
        prompt_ready(p)
        p.send("/rename\r")
        p.expect("This session has no name yet", 10, "/rename alone says how")
        wait_prompt(p, 5)
        ask(p, "explain quick")
        p.send("/rename parser   fix\r")
        p.expect("this session is now called \u201cparser fix\u201d", 10, "the confirmation")
        wait_prompt(p, 5)
        if not any("\u201cparser fix\u201d" in r and "\u21c4" in r for r in p.vt.screen()):
            raise Fail(f"the header row should show the session's name:\n{p.screen_tail()}")
        quit_with_ctrl_d(p)
    finally:
        p.close()
    q = Pty(w, [w.repo], "rename-2")
    try:
        prompt_ready(q)
        ask(q, "another topic quick")
        q.send("/resume\r")
        q.expect("resumed 1 turn", 10, "the named session resumed")
        wait_prompt(q, 5)
        if not any("\u201cparser fix\u201d" in r and "\u21c4" in r for r in q.vt.screen()):
            raise Fail(f"a resumed session keeps its name in the header row:\n{q.screen_tail()}")
        quit_with_ctrl_d(q)
    finally:
        q.close()


@scenario
def workflows_shows_orchestration_and_is_in_the_menu(w):
    p = Pty(w, [w.repo], "workflows")
    try:
        prompt_ready(p)
        p.send("/work")
        p.expect("/workflows", 5, "/workflows in the command menu")
        p.send("\x15")
        p.settle(0.2)
        p.send("/workflows\r")
        p.expect("Recent delegations", 10, "the workflows screen")
        shown = p.printed()
        for part in ("Orchestration on", "Open workers", "none: nothing is running or waiting for review", "none in this project yet"):
            if part not in shown:
                raise Fail(f"/workflows should show {part!r}:\n{p.screen_tail()}")
        wait_prompt(p, 5)
        p.send("/workers\r")
        p.expect("Open workers", 10, "/workers is the same screen")
        quit_with_ctrl_d(p)
    finally:
        p.close()


@scenario
def skill_starts_lists_shows_and_completes_a_skill(w):
    p = Pty(w, [w.repo], "skill")
    try:
        prompt_ready(p)
        p.send("/skill\r")
        p.expect("no skills yet", 10, "the empty state")
        wait_prompt(p, 5)
        p.send("/skill new db query the database\r")
        p.expect("started skill /db", 10, "the new skill")
        wait_prompt(p, 5)
        if not os.path.exists(os.path.join(w.repo, ".jev", "skills", "db", "SKILL.md")):
            raise Fail("the skill file should be written in the project")
        p.send("/skill\r")
        p.expect("skills (1)", 10, "the list")
        shown = p.printed()
        if "query the database" not in shown or "/skill show <name>" not in shown:
            raise Fail(f"the list should show the description and how to use it:\n{p.screen_tail()}")
        wait_prompt(p, 5)
        p.send("/skill show db\r")
        p.expect("run it: /skill db", 10, "the skill text")
        wait_prompt(p, 5)
        p.send("/skill ")
        p.expect("query the database", 5, "the skill and its description in the menu")
        p.send("\x15")
        p.settle(0.2)
        p.send("/skill dbb\r")
        p.expect("Did you mean /skill db?", 10, "the near-miss suggestion")
        quit_with_ctrl_d(p)
    finally:
        p.close()


@scenario
def the_permission_mode_carries_over_to_the_next_session(w):
    def start(args, name):
        p = Pty(w, args + [w.repo], name)
        prompt_ready(p)
        return p

    def hint(p):
        """The mode the input box's bottom edge names."""
        for row in reversed(p.vt.screen()):
            m = re.search(r"(ask|edit|plan|auto) mode ─╯", row)
            if m:
                return m.group(1)
        return None

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
        q.settle(0.3)
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
def orchestrate_starts_on_and_it_and_provider_carry_over_to_the_next_session(w):
    def start(name):
        p = Pty(w, [w.repo], name)
        prompt_ready(p)
        return p

    def banner_routing(p):
        return "routing off" if "Claude only" in banner_line(p) else "routing on"

    # A first session: orchestration is on, routing is on. Turn both off.
    p = start("flags-1")
    try:
        p.send("/orchestrate\r")
        p.expect("orchestrate: on", 10, "orchestration is on by default")
        if banner_routing(p) != "routing on":
            raise Fail(f"routing starts on:\n{p.tail()}")
        p.send("/orchestrate off\r")
        p.expect("orchestrate: off", 10, "/orchestrate off confirms")
        p.send("/provider off\r")
        p.expect("routing: off", 10, "/provider off confirms")
        wait_prompt(p, 10, "the prompt")
        quit_with_ctrl_d(p)
    finally:
        p.close()
    # The next one starts the way they were left.
    q = start("flags-2")
    try:
        if banner_routing(q) != "routing off":
            raise Fail(f"/provider off should carry over:\n{q.tail()}")
        q.send("/orchestrate\r")
        q.expect("orchestrate: off", 10, "/orchestrate off should carry over")
        q.send("/orchestrate on\r")
        q.expect("orchestrate: on", 10, "/orchestrate on confirms")
        q.send("/provider auto\r")
        q.expect("routing: on", 10, "/provider auto confirms")
        wait_prompt(q, 10, "the prompt")
        quit_with_ctrl_d(q)
    finally:
        q.close()
    r = start("flags-3")
    try:
        if banner_routing(r) != "routing on":
            raise Fail(f"/provider auto should carry over:\n{r.tail()}")
        r.send("/orchestrate\r")
        r.expect("orchestrate: on", 10, "/orchestrate on should carry over")
        quit_with_ctrl_d(r)
    finally:
        r.close()
    for name, want in (("orchestrate", "on"), ("routing", "on")):
        path = os.path.join(w.state, "jev-router", name)
        if not os.path.isfile(path) or open(path).read() != want or os.stat(path).st_mode & 0o777 != 0o600:
            raise Fail(f"{path} should hold {want!r} and be private")


@scenario
def the_terminal_is_restored_when_the_program_ends(w):
    p = Pty(w, [w.repo], "restore")
    try:
        prompt_ready(p)
        p.send("slow work please\r")
        wait_prompt(p, 25, "the prompt after a slow turn")
        quit_with_ctrl_d(p)
        raw = p.raw.decode("utf-8", "replace")
        if raw.rfind("\x1b[?25l") > raw.rfind("\x1b[?25h"):
            raise Fail("the cursor was left hidden at exit")
    finally:
        p.close()


@scenario
def full_screen_draws_header_chat_and_box_and_leaves_the_terminal_clean(w):
    """Without JEV_INLINE the app takes the alternate screen: the header row on top, the chat
    under it, the box at the bottom; the wheel and PageUp are read; and the alternate screen
    and the mouse are given back at exit. The tiny emulator does not know absolute cursor
    moves, so this reads the bytes written."""
    p = Pty(w, [w.repo], "fullscreen", env=w.env(JEV_INLINE=None), cols=130, rows=34)
    try:
        p.expect("Ask anything", 15, "the box")
        p.send("explain the code quick\r")
        p.expect(ANSWER, 20, "the answer in the chat pane")
        p.settle(0.5)
        raw = p.raw.decode("utf-8", "replace")
        for needle, what in (("\x1b[?1049h", "the alternate screen"), ("\x1b[?1000h", "mouse reporting"), ("\x1b[1;1H", "rows drawn in place"), ("orchestrate on", "the header row")):
            if needle not in raw:
                raise Fail(f"full screen should use {what} ({needle!r}):\n{p.tail()}")
        # PageUp and the wheel are keys the app reads, not text in the box.
        p.send("\x1b[5~")
        p.send("\x1b[<64;10;10M")
        p.settle(0.4)
        if "[5~" in p.printed().split(ANSWER)[-1] or "64;10;10" in p.printed().split(ANSWER)[-1]:
            raise Fail("PageUp or the wheel was typed into the box")
        p.send("\x04")
        code = p.wait_exit(10)
        if code != 0:
            raise Fail(f"Ctrl-D should exit with 0, got {code}")
        raw = p.raw.decode("utf-8", "replace")
        if raw.rfind("\x1b[?1049l") < raw.rfind("\x1b[?1049h"):
            raise Fail("the alternate screen was not left at exit")
        if raw.rfind("\x1b[?25l") > raw.rfind("\x1b[?25h"):
            raise Fail("the cursor was left hidden at exit")
    finally:
        p.close()


@scenario
def a_streamed_answer_is_drawn_before_the_turn_ends(w):
    """With `--include-partial-messages`, the full-screen chat shows the answer as it is
    written: the first piece is on screen while the fake CLI still holds the second. The turn
    then commits the whole answer and the choice of asking for partial text is visible."""
    p = Pty(w, [w.repo], "stream", env=w.env(JEV_INLINE=None), cols=130, rows=34)
    try:
        p.expect("Ask anything", 15, "the box")
        p.send("streamtest please\r")
        p.expect("STREAMCHUNKONE", 10, "the first piece of the answer while it is written")
        if "✓ claude" in p.printed():
            raise Fail(f"the answer was committed before the turn ended:\n{p.tail()}")
        p.expect("✓ claude", 20, "the end of the turn")
        p.expect("STREAMCHUNKONE STREAMCHUNKTWO", 10, "the finished answer in the chat")
        if "--include-partial-messages" not in cli_args(w):
            raise Fail(f"the streaming turn should ask for partial messages: {cli_args(w)}")
        p.settle(0.3)
        p.send("\x04")
        if p.wait_exit(10) != 0:
            raise Fail("Ctrl-D should exit with 0")
    finally:
        p.close()


def thinking_pref(w, value):
    path = os.path.join(w.state, "jev-router", "thinking")
    with open(path) as f:
        got = f.read().strip()
    if got != value:
        raise Fail(f"thinking should be kept as {value!r}, the file says {got!r}")


@scenario
def thinking_shows_the_whole_reasoning_when_turned_on_and_the_choice_is_kept(w):
    """`/thinking on` prints the model's reasoning in full for the turn; the choice is saved and
    a new session starts with it. `/thinking off` goes back to the one-line trace."""
    p = Pty(w, [w.repo], "thinking")
    try:
        prompt_ready(p)
        p.send("/thinking on\r")
        p.expect("thinking: on", 10, "the confirmation")
        p.send("realistic please\r")
        p.expect("The user reports empty input", 20, "the whole reasoning")
        p.expect("… 1 more line of thinking", 10, "the rest of the reasoning counted")
        p.expect("Fixed: an empty input", 20, "the answer")
        p.expect("✓ claude", 20, "the end of the turn")
        wait_prompt(p, 10, "the next prompt")
        thinking_pref(w, "on")
        p.send("/thinking off\r")
        p.expect("thinking: off", 10, "the toggle off")
        thinking_pref(w, "off")
        quit_with_ctrl_d(p)
    finally:
        p.close()
    # The saved choice is read at startup: no need to set it again.
    p = Pty(w, [w.repo], "thinking2")
    try:
        prompt_ready(p)
        p.send("/thinking\r")
        p.expect("thinking: off", 10, "the saved choice")
        quit_with_ctrl_d(p)
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
    return re.sub(r"\x1b\[[0-9;?<>=]*[ -/]*[@-~]|\x1b[78]", "", raw)


def expect_one_row_status_then_nothing_left(p, label):
    left = p.vt.status_rows()
    if left:
        raise Fail(f"{label}: fragments of the status line are left on screen after the turn:\n" + "\n".join(f"      | {r}" for r in left))
    tool_rows = [r for r in p.vt.screen() if "◇ Ran 1 command" in r]
    if len(tool_rows) != 1:
        raise Fail(f"{label}: the run with the long tool call should print as ONE row, found {len(tool_rows)}: {tool_rows}")
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
        p.expect("Running ls", 15, "the working row of the long command")
        p.settle(2.0)
        width = status_width(p.vt)
        if not 20 < width <= 59:
            raise Fail(f"the status frame should fill a 60-column window up to the last-but-one column, it is {width} columns wide")
        p.expect("✓ claude", 20, "the end of the turn")
        wait_prompt(p, 10, "the next prompt")
        expect_one_row_status_then_nothing_left(p, "narrow")
        # Nothing but known escape sequences was written for the turn: no lone ESC, bell or other control byte.
        raw = p.raw.decode("utf-8", "replace")
        plain = strip_known_escapes(raw)
        turn = plain[plain.find("longcmd please") : plain.find("✓ claude")]
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
        p.expect("Running ls", 15, "the working row of the long command")
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
        wait_prompt(p, 10, "the next prompt")
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
        shown = text[text.rfind("❯ /providers") :] if "❯ /providers" in text else ""
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
        wait_prompt(p, 20, "the prompt")
        if time.time() - started > 9:
            raise Fail("the banner waited for the slow probe: startup must only wait for the 5 s cap")
        if "codex" in banner_line(p):
            raise Fail(f"a probe that timed out is not named ready in the banner:\n{banner_line(p)}")
        watch_status_rows(p)
        p.send("please stall forever\r")
        p.expect("Running sleep 300", 15, "the working row of the stalled turn")
        # The retry answers ~7 s after the banner, mid-turn. Routing never changes under a
        # running turn, and the notice only reports what was applied, so it must NOT appear
        # yet, and the live status line must stay one row.
        p.settle(10.0)
        if "ready now" in p.visible():
            raise Fail(f"the notice was printed before the answer was applied (mid-turn):\n{p.tail()}")
        if len(p.vt.status_rows()) != 1:
            raise Fail("the live status line must stay a single row")
        p.send("\x03")
        p.expect("Stopped after", 10, "the `stopped` summary")
        # As soon as the turn ends the answer is applied and reported, before the next prompt.
        p.expect("codex is ready now", 10, "the notice right after the turn ends")
        wait_prompt(p, 10, "the prompt")
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
        if "codex" in banner_line(p):
            raise Fail(f"the banner should not name codex ready yet:\n{banner_line(p)}")
        # Idle at the prompt, the retry finishes ~7 s after the banner. The notice goes above
        # the input box at the next key, and what is being typed stays as it is.
        p.send("typed ")
        p.settle(9.0)
        p.send("x")
        p.expect("codex is ready now", 10, "the notice at the next key")
        p.settle(0.3)
        if "│ ❯ typed x" not in screen_text(p.vt):
            raise Fail(f"the notice must not disturb what is being typed:\n{screen_text(p.vt)}")
        p.send("\x15")  # clear the box
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
        p.expect("esc to stop", 15, "the working row of the slow turn")
        p.settle(0.3)
        p.send("and then hello")
        p.expect("│ ❯ and then hello", 5, "the typed message in the box")
        p.send("\r")
        p.expect("#1 and then hello", 5, "the queued message above the box")
        p.expect("✓ claude", 20, "the end of the slow turn")
        p.expect("❯ and then hello", 10, "the queued message run as the next one")
        p.expect(ANSWER, 20, "the answer to the queued message")
        wait_prompt(p, 10, "the next prompt")
        quit_with_ctrl_d(p)
    finally:
        p.close()


@scenario
def the_input_box_stays_under_the_working_row_for_the_whole_turn(w):
    p = Pty(w, [w.repo], "field")
    try:
        prompt_ready(p)
        p.send("slow work please\r")
        # Before anything is typed the box is there, saying what it is for during a turn.
        p.expect("Type to queue a message for after this turn", 15, "the box with its turn hint")
        p.settle(0.2)
        rows = [r for r in p.vt.screen() if r.strip()]
        work = [i for i, r in enumerate(rows) if "esc to stop" in r]
        box = [i for i, r in enumerate(rows) if r.startswith("╭")]
        if not work or not box or [b for b in box if b > work[-1]][:1] != [work[-1] + 1]:
            raise Fail(f"the box should sit right under the working row:\n{p.screen_tail()}")
        # Under load a frame can be read half drawn: wait for the footer rather than read it once.
        deadline = time.time() + 5
        while not footer(p).lstrip().startswith("Enter:queue") and time.time() < deadline:
            p.pump(0.1)
        if not footer(p).lstrip().startswith("Enter:queue") or "Esc" in footer(p):
            raise Fail(f"the footer should say what Enter does now, and leave Esc to the working row:\n{p.screen_tail()}")
        p.send("and then hello")
        p.expect("│ ❯ and then hello", 5, "what is typed in the box")
        p.send("\r")
        p.expect("#1 and then hello", 5, "the waiting message above the box")
        p.expect("✓ claude", 20, "the end of the turn")
        p.expect(ANSWER, 20, "the answer to the queued message")
        wait_prompt(p, 10, "the next prompt")
        if "esc to stop" in screen_text(p.vt):
            raise Fail(f"the working row must be gone once the turn is over:\n{p.screen_tail()}")
        quit_with_ctrl_d(p)
    finally:
        p.close()


@scenario
def esc_stops_the_turn_and_sends_what_was_typed_right_after(w):
    p = Pty(w, [w.repo], "esc")
    try:
        prompt_ready(p)
        p.send("slow work please\r")
        p.expect("esc to stop", 15, "the working row of the slow turn")
        p.settle(0.3)
        p.send("do this now")
        p.expect("│ ❯ do this now", 5, "the typed message in the box")
        p.send("\x1b")
        p.expect("Stopped after", 10, "the stopped turn")
        p.expect("❯ do this now", 10, "the message run at once")
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
        if "✓ claude" in p.raw[start:].decode("utf-8", "replace") or "│   " not in screen_text(p.vt):
            raise Fail(f"backslash then Enter must add a line, not send the message:\n{p.screen_tail()}")
        ask(p, "second line")
        p.send("!echo shell-says-hi\r")
        p.expect("shell-says-hi", 10, "the shell output")
        p.expect("exit 0", 10, "the exit status")
        wait_prompt(p, 10, "the next prompt")
        quit_with_ctrl_d(p)
    finally:
        p.close()


def jobs_wanted():
    """How many scenarios run at once: `JEV_PTY_JOBS`, else 4 (or fewer on a small machine).
    Every scenario has its own home, repo and terminal, so they do not share anything."""
    try:
        return max(1, int(os.environ["JEV_PTY_JOBS"]))
    except (KeyError, ValueError):
        return max(1, min(4, os.cpu_count() or 1))


def pass_cache():
    """The scenarios that already passed on this exact binary: `target/pty-pass-<binary hash>/`
    holds one file per scenario, keyed by a hash of its source. Another build starts empty.
    `JEV_PTY_FRESH=1` ignores it. (A change to a helper above the scenarios is not seen: use it.)"""
    import hashlib
    import inspect

    with open(BIN, "rb") as f:
        binary = hashlib.sha256(f.read()).hexdigest()[:16]
    root = os.path.join(ROOT, "target", f"pty-pass-{binary}")
    os.makedirs(root, exist_ok=True)
    # Old builds' caches would pile up: keep only this binary's.
    for d in os.listdir(os.path.join(ROOT, "target")):
        if d.startswith("pty-pass-") and d != os.path.basename(root):
            shutil.rmtree(os.path.join(ROOT, "target", d), ignore_errors=True)

    def key(fn):
        return os.path.join(root, hashlib.sha256((fn.__name__ + inspect.getsource(fn)).encode()).hexdigest()[:16])

    return key


def run_parallel(selected, jobs):
    """Each scenario in its own process (`=name` selects exactly one), `jobs` at a time; the
    results print in the scenario order, as a serial run would."""
    from concurrent.futures import ThreadPoolExecutor

    def one(fn):
        started = time.time()
        r = subprocess.run([sys.executable, "-I", os.path.abspath(__file__), "=" + fn.__name__], stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, env={**os.environ, "JEV_PTY_JOBS": "1"})
        return fn, r.returncode, r.stdout, time.time() - started

    key = pass_cache()
    fresh = os.environ.get("JEV_PTY_FRESH", "") not in ("", "0")
    skipped = [] if fresh else [fn for fn in selected if os.path.exists(key(fn))]
    todo = [fn for fn in selected if fn not in skipped]
    failures = 0
    with ThreadPoolExecutor(max_workers=jobs) as pool:
        for fn, code, out, took in pool.map(one, todo):
            if code == 0:
                open(key(fn), "w").close()
            lines = [l for l in out.splitlines() if l.strip() and "scenarios passed" not in l]
            if code == 0:
                print(lines[0] if lines else f"ok    {fn.__name__.replace('_', ' ')} ({took:.1f}s)")
            else:
                failures += 1
                print("\n".join(lines) or f"FAIL  {fn.__name__.replace('_', ' ')}\n      exit {code}")
            sys.stdout.flush()
    note = f" ({len(skipped)} unchanged since they last passed on this build; JEV_PTY_FRESH=1 runs them)" if skipped else ""
    print(f"\n{len(selected) - failures}/{len(selected)} scenarios passed{note}")
    return 1 if failures else 0


def main():
    if not os.path.exists(BIN):
        print(f"missing {BIN}: run `cargo build --bin code` first", file=sys.stderr)
        return 2
    wanted = sys.argv[1:]
    exact = [w[1:] for w in wanted if w.startswith("=")]
    if exact:
        wanted = []
    selected = [
        fn
        for fn in SCENARIOS
        if (exact and fn.__name__ in exact)
        or (not exact and (not wanted or any(w in fn.__name__ or w in fn.__name__.replace("_", " ") for w in wanted)))
    ]
    jobs = jobs_wanted()
    if len(selected) > 1 and jobs > 1:
        return run_parallel(selected, jobs)
    jev, url = start_jev()
    signal.signal(signal.SIGALRM, on_alarm)
    failures = 0
    ran = 0
    for fn in selected:
        name = fn.__name__.replace("_", " ")
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
    if ran == 0:
        # A filter that matched nothing tested nothing: not a pass.
        print(f"no scenario matches {' '.join(wanted)}", file=sys.stderr)
        return 1
    print(f"\n{ran - failures}/{ran} scenarios passed")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
