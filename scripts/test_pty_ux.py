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


import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))  # `python3 -I` drops the script dir

from pty_ux.harness import *  # noqa: F401,F403,E402  (parity.py imports the fakes from here)
from pty_ux import startup, input_output, approvals, commands, screen, turn_input  # noqa: F401,E402  (they register the scenarios, in order)
from pty_ux.runner import main  # noqa: E402

if __name__ == "__main__":
    sys.exit(main())
