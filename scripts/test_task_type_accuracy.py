"""Unit tests for task_type_accuracy.py. Synthetic rows; no network.

Run: python3 -I scripts/test_task_type_accuracy.py
"""
import json
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import provider_eval.config as pe_config  # noqa: E402
import task_type_accuracy as tta  # noqa: E402


def row(i, category, task, conf=0.9, model="claude-sonnet-5-5", prompt=None, **kw):
    return {"id": f"P{i}", "category": category, "prompt": prompt or f"prompt {i}", "task": task,
            "task_confidence": conf, "security_p": 0.1, "very_hard_p": 0.1, "continues_p": 0.1,
            "multi_part_p": 0.1, "model": model, **kw}


ROWS = [
    row(1, "security", "security", 0.95, "claude-fable-5-2"),
    row(2, "security", "review", 0.60, "claude-opus-5-5", prompt="audit the login flow"),
    row(3, "debug", "hard_debug", 0.80, "gpt-6.1-sol"),
    row(4, "refactor", "feature", 0.45),                      # mapped to feature: agrees
    row(5, "locate", "feature", 0.65, prompt="where is X"),   # explain -> feature: miss
    row(6, "trace", "feature", 0.75, prompt="trace Y"),       # explain -> feature: miss
    row(7, "explain", "explain", 0.90, "gpt-6.1-terra"),
    row(8, "mystery", "explain", 0.90),                       # unmapped category: not measured
    {"id": "P9", "category": "debug", "failed": True},
]


class Analyze(unittest.TestCase):
    def setUp(self):
        self.res = tta.analyze(ROWS)

    def test_counts_failures_and_unmapped_separately(self):
        r = self.res
        self.assertEqual((r["rows"], r["failed"], r["answered"], r["unmapped"], r["mapped"]), (9, 1, 8, 1, 7))

    def test_agreement(self):
        self.assertEqual(self.res["agree"], 4)  # P1, P3, P4, P7
        self.assertAlmostEqual(self.res["agreement"], 4 / 7)

    def test_confusion_matrix(self):
        c = self.res["confusion"]
        self.assertEqual(c[("explain", "feature")], 2)
        self.assertEqual(c[("security", "review")], 1)
        self.assertEqual(c[("feature", "feature")], 1)
        self.assertEqual(sum(c.values()), 7)

    def test_precision_and_recall(self):
        t = self.res["per_type"]
        # Jev said feature 3 times (P4, P5, P6), one of which the mapping agrees with.
        self.assertEqual((t["feature"]["predicted"], t["feature"]["tp"]), (3, 1))
        self.assertAlmostEqual(t["feature"]["precision"], 1 / 3)
        self.assertAlmostEqual(t["feature"]["recall"], 1.0)
        # Three explain-mapped prompts (P5, P6, P7); Jev found one.
        self.assertAlmostEqual(t["explain"]["recall"], 1 / 3)
        self.assertAlmostEqual(t["explain"]["precision"], 1.0)
        # Never predicted by Jev among mapped rows: precision undefined, not zero.
        self.assertEqual(t["review"]["recall"], None)
        self.assertEqual(t["review"]["precision"], 0.0)

    def test_confidence_shares_cover_every_answered_prompt(self):
        c = self.res["confidence"]
        self.assertEqual(c["n"], 8)
        self.assertAlmostEqual(c["below_0.5"], 1 / 8)  # only P4 (0.45)
        self.assertAlmostEqual(c["below_0.7"], 3 / 8)   # P2 0.60, P4 0.45, P5 0.65

    def test_decision_differs_from_staying_on_sonnet(self):
        # P1 fable, P2 opus, P3 sol, P7 terra differ; P4, P5, P6, P8 stay on Sonnet.
        self.assertEqual(self.res["differ"], 4)
        self.assertAlmostEqual(self.res["differ_share"], 0.5)

    def test_none_model_counts_as_a_difference(self):
        res = tta.analyze([row(1, "debug", "hard_debug", model="none")])
        self.assertEqual(res["differ"], 1)

    def test_at_most_n_examples_per_pair(self):
        rows = [row(i, "locate", "feature", prompt=f"q{i}") for i in range(5)]
        res = tta.analyze(rows, examples=3)
        self.assertEqual(len(res["miss_examples"][("explain", "feature")]), 3)
        self.assertEqual(res["confusion"][("explain", "feature")], 5)

    def test_uses_the_single_task_map_of_provider_eval(self):
        self.assertIs(tta.TASK_MAP, pe_config.TASK_MAP)

    def test_custom_map_and_empty_input(self):
        res = tta.analyze([row(1, "x", "docs")], task_map={"x": "docs"})
        self.assertEqual(res["agreement"], 1.0)
        empty = tta.analyze([])
        self.assertIsNone(empty["agreement"])
        self.assertIsNone(empty["differ_share"])
        tta.render(empty)  # must not crash on no data


class Render(unittest.TestCase):
    def test_report_has_every_section_and_the_caveat(self):
        text = tta.render(tta.analyze(ROWS))
        for needle in ("approximate", "not proof", "Agreement", "Confusion matrix", "Per type",
                       "below 0.5: 12.5%", "below 0.7: 37.5%", "differs from staying on", "50.0%",
                       "1 failed", "mapped explain -> Jev feature  (2 prompts)", "[P5]", "where is X"):
            self.assertIn(needle, text)

    def test_pairs_limit(self):
        rows = ROWS + [row(20, "debug", "review")]
        res = tta.analyze(rows)
        self.assertIn("1 of 3 confusion pairs", tta.render(res, pairs=1))
        self.assertIn("3 of 3 confusion pairs", tta.render(res, pairs=0))

    def test_long_prompts_are_clipped(self):
        text = tta.render(tta.analyze([row(1, "locate", "feature", prompt="word " * 200)]))
        self.assertIn("...", text)
        self.assertLess(max(len(line) for line in text.splitlines()), 260)


class Files(unittest.TestCase):
    def test_load_rows_skips_blank_and_broken_lines(self):
        with tempfile.TemporaryDirectory() as d:
            p = Path(d) / "out.jsonl"
            p.write_text(json.dumps(ROWS[0]) + "\n\nnot json\n" + json.dumps(ROWS[1]) + "\n")
            self.assertEqual([r["id"] for r in tta.load_rows(p)], ["P1", "P2"])


if __name__ == "__main__":
    unittest.main()
