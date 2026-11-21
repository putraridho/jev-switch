"""The grading rubric and the harness note appended to every answered prompt."""


# Wording reused from <data>/grade/BRIEF.md, the rubric of the original Opus grading.
RUBRIC = """\
Grade coding-assistant answers against the real code. The codebase (read-only snapshot) is your working directory; do not modify anything.

`expected` was written by another model reading the code: a strong hint, NOT gospel. Verify it; if it is wrong and an answer is right, the answer is correct. "[trap]" means the prompt has security-sounding words but is not security work.

The assistants ran read-only: for change requests they DESCRIBED the change. Grade the described change as if applied: right files, right approach, no bug introduced, pitfalls (tests, callers, types) handled.

For each answer open the relevant files and decide:
- "correct": a developer could rely on it; at most minor omissions.
- "partial": right direction but misses something material the developer would trip on.
- "wrong": factually wrong about the code, wrong cause, invented symbols, or a change that wouldn't work.
Be strict and concrete. Answers that refuse, stop without answering, or are an error message are "wrong"."""

ANSWER_NOTE = (
    "\n\n[Harness note: this is a read-only evaluation. Investigate with the read tools; if the request "
    "asks for changes, describe exactly what you would change (files, symbols, pitfalls) instead of "
    "editing. Keep the final answer under 400 words.]"
)
