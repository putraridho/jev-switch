"""Scenarios: learning commands, editing, history, diffs, print mode, colour, stdin."""

from .harness import *  # noqa: F401,F403


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
