"""Scenarios: typing during a turn, esc, bang and backslash input."""

from .harness import *  # noqa: F401,F403


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
