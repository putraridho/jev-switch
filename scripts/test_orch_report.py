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


# What the gateway logs for a 0.95 multi-part prompt that is really a sequence (raw answers and the veto).
VETOED = check(0.95, sequential_p=0.9, vetoed=True)


class VetoTests(unittest.TestCase):
    def test_a_vetoed_check_is_counted_with_its_raw_score(self):
        rows = [check(0.2), VETOED, check(0.9, trigger=True, sequential_p=0.1, vetoed=False)]
        held = report.vetoes(rows)
        self.assertEqual(len(held), 1)
        self.assertEqual(held[0]["multi_part_p"], 0.95)

    def test_vetoes_come_highest_raw_score_first(self):
        low = check(0.8, vetoed=True)
        self.assertEqual([c["multi_part_p"] for c in report.vetoes([low, VETOED])], [0.95, 0.8])

    def test_a_sequence_that_never_reached_the_cutoff_was_not_held_back(self):
        # The veto applies to every sequence, whatever it scored; only those that would
        # otherwise have triggered were held back.
        self.assertEqual(report.vetoes([check(0.1, sequential_p=0.95, vetoed=True)]), [])
        reached = check(0.7, sequential_p=0.95, vetoed=True)
        self.assertEqual(report.vetoes([reached]), [reached])

    def test_a_vetoed_prompt_is_not_also_a_near_miss(self):
        self.assertEqual(report.near_misses([VETOED], 0.7), [])

    def test_near_misses_use_the_raw_multi_part(self):
        # Raw 0.65 is within 0.15 of the 0.7 cutoff.
        near = check(0.65, sequential_p=0.1, vetoed=False)
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

    def test_rows_without_a_number_are_skipped_even_at_a_low_cutoff(self):
        # At a cutoff of 0.1 the band reaches 0 and below: a missing score used to count as
        # 0 there and then crashed the `:.2f` formatting.
        missing = {"trigger": False, "prompt": "no score"}
        null = {"multi_part_p": None, "trigger": False}
        text = {"multi_part_p": "0.9", "trigger": False}
        near = check(0.05, cutoff=0.1)
        self.assertEqual(report.near_misses([missing, null, text, near], 0.1), [near])
        for c in report.near_misses([missing, null, text, near], 0.1):
            f"{c['multi_part_p']:.2f}"
        vetoed_without_score = {"vetoed": True, "cutoff": 0.0}
        vetoed_without_cutoff = check(0.9, vetoed=True, cutoff=None)
        self.assertEqual(report.vetoes([vetoed_without_score, vetoed_without_cutoff, VETOED]), [VETOED])


if __name__ == "__main__":
    unittest.main()
