"""Unit tests for the orchestrate check helpers in orch_report.py. Synthetic rows; no network.

Run: python3 -I scripts/test_orch_report.py
"""
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import orch_report as report  # noqa: E402


def check(p, trigger=False, **kw):
    return {"multi_part_p": p, "trigger": trigger, "cutoff": 0.7, "prompt": f"prompt {p}", **kw}


# What the gateway logs for a 0.95 multi-part prompt that is really a sequence.
VETOED = check(0.95, orch_score=0.3, sequential_p=0.9, vetoed=True)


class VetoTests(unittest.TestCase):
    def test_a_vetoed_check_is_counted_with_its_raw_score(self):
        rows = [check(0.2), VETOED, check(0.9, trigger=True, orch_score=0.9, sequential_p=0.1, vetoed=False)]
        held = report.vetoes(rows)
        self.assertEqual(len(held), 1)
        self.assertEqual(held[0]["multi_part_p"], 0.95)

    def test_vetoes_come_highest_raw_score_first(self):
        low = check(0.8, orch_score=0.3, vetoed=True)
        self.assertEqual([c["multi_part_p"] for c in report.vetoes([low, VETOED])], [0.95, 0.8])

    def test_a_vetoed_prompt_is_not_also_a_near_miss(self):
        self.assertEqual(report.near_misses([VETOED], 0.7), [])

    def test_near_misses_use_the_raw_multi_part_not_the_vetoed_score(self):
        # Raw 0.65 is within 0.15 of the 0.7 cutoff; a log that kept only the score (0.3) would hide it.
        near = check(0.65, orch_score=0.65, sequential_p=0.1, vetoed=False)
        self.assertEqual(report.near_misses([near, check(0.4)], 0.7), [near])

    def test_old_rows_without_the_new_fields_still_work(self):
        old_near, old_far = check(0.6), check(0.1)
        self.assertEqual(report.near_misses([old_near, old_far, check(0.9, trigger=True)], 0.7), [old_near])
        self.assertEqual(report.vetoes([old_near, old_far]), [])
        self.assertFalse(report.is_vetoed({"multi_part_p": None}))

    def test_a_failed_check_with_null_scores_is_neither(self):
        failed = {"multi_part_p": None, "vetoed": None, "trigger": False}
        self.assertEqual(report.near_misses([failed], 0.7), [])
        self.assertEqual(report.vetoes([failed]), [])


if __name__ == "__main__":
    unittest.main()
