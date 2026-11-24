"""Scenarios: tool approval cards."""

from .harness import *  # noqa: F401,F403


# ------------------------------------------------------------ tool approvals

QUESTION = "Allow Bash?"
CHOICES = ("1. Yes", "2. Yes, and don't ask again", "3. No, and tell the model what to do instead")


def answer_card(p, key):
    """Press a key at the card once it has been up long enough to be read."""
    p.settle(0.35)
    p.send(key)


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
def elicitation_returns_a_boolean_answer_even_in_auto_mode(w):
    p = Pty(w, ["--mode", "auto", w.repo], "elicitation-auto")
    try:
        prompt_ready(p)
        p.send("elicitation please quick\r")
        p.expect("lunagraph needs your input", 15, "the MCP question in auto mode")
        p.expect("Run pnpm install?", 5, "the server's question")
        p.expect("Approve", 5, "the boolean field")
        answer_card(p, "\x1b[B")  # false -> true
        p.send("\r")  # review
        p.expect("Send answer", 5, "the final confirmation")
        p.send("\x1b[A\r")  # decline -> send
        p.expect("FAKE ELICITATION DONE", 10, "the result after the answer")
        reply = host_reply(w)
        if reply != {"subtype": "success", "request_id": "elicit-1", "response": {"action": "accept", "content": {"approve": True}}}:
            raise Fail(f"wrong elicitation response: {reply}")
        if settings_rules(w) is not None:
            raise Fail("an elicitation must not save a permission rule")
        wait_prompt(p, 10, "the next prompt")
        quit_with_ctrl_d(p)
    finally:
        p.close()


@scenario
def elicitation_decline_returns_to_the_running_turn(w):
    p = Pty(w, [w.repo], "elicitation-decline")
    try:
        prompt_ready(p)
        p.send("elicitation please quick\r")
        p.expect("lunagraph needs your input", 15, "the MCP question")
        answer_card(p, "\r")  # retain false, review
        p.expect("Send answer", 5, "the final confirmation")
        p.send("\r")  # decline is selected initially
        p.expect("FAKE ELICITATION DONE", 10, "the result after declining")
        if host_reply(w)["response"] != {"action": "decline"}:
            raise Fail(f"expected decline: {host_reply(w)}")
        wait_prompt(p, 10, "the next prompt")
        quit_with_ctrl_d(p)
    finally:
        p.close()


@scenario
def elicitation_escape_cancels_the_question(w):
    p = Pty(w, [w.repo], "elicitation-cancel")
    try:
        prompt_ready(p)
        p.send("elicitation please quick\r")
        p.expect("lunagraph needs your input", 15, "the MCP question")
        answer_card(p, "\x1b")
        p.expect("FAKE ELICITATION DONE", 10, "the result after cancelling")
        if host_reply(w)["response"] != {"action": "cancel"}:
            raise Fail(f"expected cancel: {host_reply(w)}")
        wait_prompt(p, 10, "the next prompt")
        quit_with_ctrl_d(p)
    finally:
        p.close()


@scenario
def elicitation_withdrawal_removes_the_form_without_an_answer(w):
    p = Pty(w, [w.repo], "elicitation-withdraw")
    try:
        prompt_ready(p)
        p.send("elicitation withdraw quick\r")
        p.expect("lunagraph needs your input", 15, "the MCP question")
        p.expect("FAKE ELICITATION DONE", 10, "the result after withdrawal")
        wait_prompt(p, 10, "the next prompt")
        if os.path.exists(os.path.join(w.pids, "reply.json")):
            raise Fail("a withdrawn form must not send a response")
        if "lunagraph needs your input" in screen_text(p.vt):
            raise Fail("the withdrawn form is still on screen")
        quit_with_ctrl_d(p)
    finally:
        p.close()
