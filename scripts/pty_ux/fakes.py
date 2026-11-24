"""The fakes: Jev, the model CLI, pager, slow codex, and the per-scenario World."""

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


ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
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
# `/mcp` asks `claude mcp list` for health; `mcp login <server>` records what it was asked.
if [ "$1" = "mcp" ] && [ "$2" = "list" ]; then
  sleep 0.5
  echo 'Checking MCP server health...'
  echo ''
  echo 'docs: https://docs.example/mcp (HTTP) - ✔ Connected'
  echo 'tracker: https://tracker.example/mcp (HTTP) - ⚠ Needs authentication'
  echo 'files: npx files-mcp - ✗ Failed to connect'
  exit 0
fi
if [ "$1" = "mcp" ] && [ "$2" = "login" ]; then
  echo "$3" > "$FAKE_PIDS/mcp-login.txt"
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
  *elicitation*)
    emit '{"type":"control_request","request_id":"elicit-1","request":{"subtype":"elicitation","mcp_server_name":"lunagraph","message":"Run pnpm install?","requested_schema":{"type":"object","properties":{"approve":{"type":"boolean","title":"Approve","description":"Let the agent do this.","default":false}},"required":["approve"]}}}'
    case "$last" in
      *withdraw*)
        sleep 0.7
        emit '{"type":"control_cancel_request","request_id":"elicit-1"}'
        ;;
      *)
        IFS= read -r reply
        printf '%s\n' "$reply" > "$FAKE_PIDS/reply.json"
        ;;
    esac
    answer="FAKE ELICITATION DONE"
    ;;
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
