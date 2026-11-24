"""Scenarios: full screen, streaming, thinking, status line, resize, provider probes."""

from .harness import *  # noqa: F401,F403


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
    under it, the box at the bottom; PageUp is read, SGR mouse reporting is enabled for wheel
    scrolling, and both the mouse modes and the alternate screen are given back at exit. The tiny
    emulator does not know absolute cursor moves, so this reads the bytes written."""
    p = Pty(w, [w.repo], "fullscreen", env=w.env(JEV_INLINE=None), cols=130, rows=34)
    try:
        p.expect("Ask anything", 15, "the box")
        p.send("explain the code quick\r")
        p.expect(ANSWER, 20, "the answer in the chat pane")
        p.settle(0.5)
        raw = p.raw.decode("utf-8", "replace")
        for needle, what in (
            ("\x1b[?1049h", "the alternate screen"),
            ("\x1b[?1000h", "mouse reporting"),
            ("\x1b[?1006h", "SGR mouse encoding"),
            ("\x1b[1;1H", "rows drawn in place"),
            ("orchestrate on", "the header row"),
        ):
            if needle not in raw:
                raise Fail(f"full screen should use {what} ({needle!r}):\n{p.tail()}")
        # PageUp is a key the app reads, not text in the box.
        p.send("\x1b[5~")
        p.settle(0.4)
        if "[5~" in p.printed().split(ANSWER)[-1]:
            raise Fail("PageUp was typed into the box")
        p.send("\x04")
        code = p.wait_exit(10)
        if code != 0:
            raise Fail(f"Ctrl-D should exit with 0, got {code}")
        raw = p.raw.decode("utf-8", "replace")
        if raw.rfind("\x1b[?1000l") < raw.rfind("\x1b[?1000h") or raw.rfind("\x1b[?1006l") < raw.rfind("\x1b[?1006h"):
            raise Fail("mouse reporting was not turned off at exit")
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


@scenario
def mouse_wheel_scrolls_the_chat_and_an_arrow_still_recalls_history(w):
    """In full-screen mode with SGR mouse reporting enabled, mouse wheel events scroll the chat
    without typing into the box or recalling history, while an Up arrow still recalls the last
    message."""
    p = Pty(w, [w.repo], "wheel", env=w.env(JEV_INLINE=None), cols=130, rows=34)
    try:
        p.expect("Ask anything", 15, "the box")
        p.send("explain the code quick\r")
        p.expect(ANSWER, 20, "the answer in the chat pane")
        p.settle(1.0)
        working = "Reading a.txt"
        turns = p.printed().count(working)
        # SGR mouse wheel up (\x1b[<64;10;10M) scrolls the chat: Enter on the still-empty box sends nothing.
        p.send("\x1b[<64;10;10M")
        p.settle(0.5)
        p.send("\r")
        p.settle(1.0)
        if p.printed().count(working) != turns:
            raise Fail(f"mouse wheel must not recall history; the message was sent again:\n{p.tail()}")
        # One arrow is a key press: history comes back and Enter sends it.
        p.send("\x1b[A")
        p.settle(0.5)
        p.send("\r")
        p.expect(working, 20, "the working row of the recalled message")
        p.expect(ANSWER, 20, "the answer to the recalled message")
        p.settle(1.0)
        p.send("\x04")
        code = p.wait_exit(10)
        if code != 0:
            raise Fail(f"Ctrl-D should exit with 0, got {code}")
    finally:
        p.close()
