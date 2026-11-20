"""Run: python3 -I scripts/test_parity.py"""

from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
import parity


class ParityTests(unittest.TestCase):
    def test_mask_dynamic_values(self):
        text = '/tmp/example/file 2026-11-20T12:34:56.123Z 1763642096 1763642096123 pid=1234 "process_id":42'
        self.assertEqual(parity.mask(text, [("/tmp/example", "<TMP>")]),
                         '<TMP>/file <TIMESTAMP> <TIMESTAMP> <TIMESTAMP> pid=<PID> "process_id":<PID>')
        self.assertEqual(parity.mask("1763642096123-1234.json"), "<TIMESTAMP>-<PID>.json")

    def test_comparison_reports_output_exit_and_files(self):
        old = {"stdout": "same\n", "exit code": "0", "state/log": "before\n"}
        new = {"stdout": "same\n", "exit code": "1", "state/log": "after\n", "state/new": "x\n"}
        diffs = parity.compare("case", old, new)
        self.assertEqual(len(diffs), 3)
        self.assertIn("-before", "".join(diffs))
        self.assertIn("+after", "".join(diffs))
        self.assertIn("<missing>", "".join(diffs))
        self.assertEqual(parity.compare("case", old, old), [])
        self.assertIn("No newline", parity.compare("case", {"stdout": "text"}, {"stdout": "text\n"})[0])


if __name__ == "__main__":
    unittest.main()
