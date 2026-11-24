"""The pseudo-terminal: screen emulation, the Pty driver, and prompt helpers."""

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

from .fakes import *  # noqa: F401,F403


# --------------------------------------------------------------------- the pty


CSI = re.compile(r"\x1b\[([0-9;?<>=]*)([ -/]*[@-~])")
OSC = re.compile(r"\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)")


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
            m = OSC.match(text, i)
            if m:
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
                m = OSC.match(text, i)
                if m:
                    i = m.end()
                    continue
                if i + 1 >= len(text) or text[i + 1] in ("[", "]"):
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


def strip_known_escapes(raw):
    return re.sub(r"\x1b\[[0-9;?<>=]*[ -/]*[@-~]|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)|\x1b[78]", "", raw)
