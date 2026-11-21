"""Scenarios: banner, suggestions, update, turns, ctrl keys, help views."""

from .harness import *  # noqa: F401,F403


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
