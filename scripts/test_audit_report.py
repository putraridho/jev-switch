"""Unit tests for provider_eval/audit_report.py. Fake audit logs in temp dirs; no network, no quota.

Run: python3 -I scripts/test_audit_report.py
"""
import contextlib
import io
import json
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import provider_eval.audit_report as ar  # noqa: E402
import provider_eval.cli as cli  # noqa: E402


def turn(acceptance="unverified", tokens=0, outcome="answered", rework="none", retries=0, verdict=None):
    row = {"ts_ms": 1, "project": "/p", "event": "turn", "model": "claude · m", "outcome": outcome,
           "elapsed_ms": 5, "prompt": "x"}
    quality = {"acceptance": acceptance, "spent_tokens": tokens, "rework": rework, "retries": retries}
    if acceptance != "unverified" or tokens or rework != "none" or retries:
        row["quality"] = quality
    if verdict:
        row["verdict"] = verdict
    return row


def write(path, rows, extra=()):
    Path(path).write_text("".join(json.dumps(r) + "\n" for r in rows) + "".join(e + "\n" for e in extra))


class AuditReport(unittest.TestCase):
    def test_an_answer_is_never_an_accepted_task(self):
        s = ar.summarise([turn(), turn(tokens=500), turn(outcome="failed")])
        self.assertEqual((s["accepted"], s["verified"], s["unverified"], s["rate"]), (0, 0, 3, None))
        self.assertEqual(s["failure_rate"], 1 / 3)

    def test_counts_cost_per_accepted_task_over_every_turn(self):
        s = ar.summarise([turn("accepted", 1000), turn("rejected_check_failed", 3000, rework="none"),
                          turn("accepted", 2000, rework="after_failed_check", retries=1)])
        self.assertEqual((s["accepted"], s["rejected"], s["rework"], s["retries"]), (2, 1, 1, 1))
        self.assertEqual(s["tokens_per_accepted"], 3000)
        self.assertEqual(s["rate"], 2 / 3)

    def test_old_lines_with_only_a_verdict_still_count(self):
        s = ar.summarise([turn(verdict="passed"), turn(verdict="failed"), turn()])
        self.assertEqual((s["accepted"], s["rejected"], s["unverified"]), (1, 1, 1))

    def test_a_reverted_turn_is_not_accepted(self):
        self.assertEqual(ar.acceptance_of(turn("rejected_reverted")), "rejected")

    def test_small_samples_are_marked_and_not_compared(self):
        a, b = ar.summarise([turn("accepted")] * 2), ar.summarise([turn("rejected_check_failed")] * 2)
        self.assertIn("(too few)", ar.format_arms({"router": a, "pinned": b}))
        self.assertIn("too few checked turns", ar.compare({"pinned": b, "router": a}))

    def test_enough_samples_can_separate_arms(self):
        good = ar.summarise([turn("accepted")] * 30)
        bad = ar.summarise([turn("rejected_check_failed")] * 30)
        self.assertIn("router accepts more", ar.compare({"pinned": bad, "router": good}))
        same = ar.summarise([turn("accepted")] * 6 + [turn("rejected_check_failed")] * 4)
        self.assertIn("not distinguishable", ar.compare({"pinned": same, "router": same}))

    def test_reads_logs_and_skips_junk_and_other_events(self):
        with tempfile.TemporaryDirectory() as d:
            log = Path(d) / "a.jsonl"
            write(log, [turn("accepted", 10)], extra=["not json", json.dumps({"event": "export", "path": "x"})])
            self.assertEqual(len(ar.read_turns(log)), 1)

    def test_the_cli_prints_one_column_per_arm_and_needs_real_files(self):
        with tempfile.TemporaryDirectory() as d:
            one, two = Path(d) / "one.jsonl", Path(d) / "two.jsonl"
            write(one, [turn("accepted", 100)] * 3)
            write(two, [turn(outcome="failed")])
            out = io.StringIO()
            with contextlib.redirect_stdout(out):
                cli.main(["audit", "--arm", f"pinned={one}", "--arm", f"router={two}"])
            text = out.getvalue()
            self.assertIn("| pinned | router |", text)
            self.assertIn("accepted tasks | 3 | 0", text)
            with self.assertRaises(SystemExit):
                cli.main(["audit", "--arm", f"x={Path(d) / 'missing.jsonl'}"])
            with self.assertRaises(SystemExit):
                cli.main(["audit", "--arm", "nolabel"])


if __name__ == "__main__":
    unittest.main()
