"""Scenarios: permissions, doctor, sessions, mcp, workflows, skills, carried-over prefs."""

from .harness import *  # noqa: F401,F403


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
def mcp_picker_shows_health_opens_a_card_and_logs_in(w):
    # Three servers under claude: the fake `claude mcp list` says docs is connected, tracker
    # needs a login and files failed.
    servers = {
        "docs": {"type": "http", "url": "https://docs.example/mcp"},
        "tracker": {"type": "http", "url": "https://tracker.example/mcp"},
        "files": {"command": "npx", "args": ["files-mcp"]},
    }
    with open(os.path.join(w.home, ".claude.json"), "w") as f:
        json.dump({"mcpServers": servers}, f)
    p = Pty(w, [w.repo], "mcp-picker")
    try:
        prompt_ready(p)
        p.send("/mcp\r")
        p.expect("MCP servers", 10, "the picker opens")
        p.expect("checking servers", 5, "it says the check is running")
        p.expect("needs login", 10, "the health check fills in the rows")
        p.settle(0.5)
        screen = p.visible()
        for needle in ["● docs", "◐ tracker", "✗ files", "http · claude", "stdio · claude"]:
            if needle not in screen:
                raise Fail(f"the list should show {needle!r}:\n{p.screen_tail()}")
        # Esc leaves without doing anything.
        p.send("\x1b")
        wait_prompt(p, 5, "the prompt after Esc")
        # Type to find it, open its card, log in.
        p.send("/mcp\r")
        p.expect("MCP servers", 10, "the picker opens again")
        p.settle(0.3)
        p.send("track")
        p.settle(0.5)
        p.send("\r")
        p.expect("Log in with claude", 10, "the card offers the login")
        p.send("\r")
        p.expect("Signed in to `tracker`", 10, "the login ran")
        wait_prompt(p, 5)
        with open(os.path.join(w.pids, "mcp-login.txt")) as f:
            if f.read().strip() != "tracker":
                raise Fail("`claude mcp login` should have been run for tracker")
        quit_with_ctrl_d(p)
    finally:
        p.close()


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
