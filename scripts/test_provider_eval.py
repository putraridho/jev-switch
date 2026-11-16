"""Unit tests for provider_eval.py. Fake data in temp dirs; no network, no quota.

Run: python3 -I -m unittest scripts/test_provider_eval.py
"""
import contextlib
import io
import json
import re
import stat
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import provider_eval as pe  # noqa: E402


def write_jsonl(path, rows):
    Path(path).parent.mkdir(parents=True, exist_ok=True)
    Path(path).write_text("".join(json.dumps(r) + "\n" for r in rows))


def make_data(root, n_sec=6):
    """A miniature train-lunagraph dir: n_sec security prompts (the first is a Sonnet miss),
    two debug prompts, one unmapped-category prompt, one prompt without a grade."""
    rows = [{"id": f"S-{i}", "category": "security", "prompt": f"p{i}", "context": "", "files": ["a.ts"],
             "expected": "e"} for i in range(n_sec)]
    rows += [{"id": f"D-{i}", "category": "debug", "prompt": f"d{i}", "context": "ctx", "files": [], "expected": "e"}
             for i in range(2)]
    rows += [{"id": "X-0", "category": "weird", "prompt": "x", "context": "", "files": [], "expected": ""},
             {"id": "U-0", "category": "debug", "prompt": "u", "context": "", "files": [], "expected": ""}]
    write_jsonl(root / "runs/sample1.jsonl", rows)
    ids = [r["id"] for r in rows if r["id"] != "U-0"]
    write_jsonl(root / "runs/sample1.sonnet.jsonl", [{"id": i, "model": "sonnet", "answer": f"sonnet {i}"} for i in ids])
    write_jsonl(root / "grade/s-00.out.jsonl",
                [{"id": i, "verdict": "partial" if i == "S-0" else "correct", "why": ""} for i in ids])
    write_jsonl(root / "runs/sample9.opus.jsonl", [{"id": "S-0", "model": "opus", "answer": "opus S-0"}])
    write_jsonl(root / "grade/o-00.out.jsonl", [{"id": "S-0", "verdict": "correct", "why": ""}])
    return root


class DataTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.data = make_data(Path(self.tmp.name))

    def tearDown(self):
        self.tmp.cleanup()

    def test_load_joins_answers_and_verdicts_and_drops_unusable_rows(self):
        items = {i["id"]: i for i in pe.load_graded_items(self.data)}
        self.assertNotIn("X-0", items)  # category not mapped
        self.assertNotIn("U-0", items)  # never answered or graded
        self.assertEqual(items["S-0"]["task_type"], "security")
        self.assertTrue(items["S-0"]["miss"])
        self.assertEqual(items["S-0"]["opus_verdict"], "correct")
        self.assertEqual(set(items["S-0"]["baselines"]), {"sonnet", "opus"})
        self.assertFalse(items["S-1"]["miss"])
        self.assertEqual(items["D-0"]["task_type"], "hard_debug")

    def test_sample_always_has_misses_and_n_per_type(self):
        items = pe.load_graded_items(self.data)
        chosen = pe.select_sample(items, per_type=2, seed=1)
        sec = [i for i in chosen if i["task_type"] == "security"]
        self.assertEqual(sorted(i["stratum"] for i in sec), ["miss", "sample", "sample"])
        self.assertIn("S-0", {i["id"] for i in sec})
        self.assertEqual(len([i for i in chosen if i["task_type"] == "hard_debug"]), 2)

    def test_sample_is_deterministic_and_respects_types(self):
        items = pe.load_graded_items(self.data)
        a = pe.select_sample(items, 2, ["security"], seed=7)
        b = pe.select_sample(items, 2, ["security"], seed=7)
        self.assertEqual([i["id"] for i in a], [i["id"] for i in b])
        self.assertEqual({i["task_type"] for i in a}, {"security"})

    def test_task_map_targets_are_router_task_types(self):
        valid = {"security", "hard_debug", "review", "feature", "frontend", "docs", "explain",
                 "current_info", "simple_edit", "unclear"}
        self.assertTrue(set(pe.TASK_MAP.values()) <= valid)

    def test_count_calls(self):
        c = pe.count_calls(10, 3, cross_n=4)
        self.assertEqual(c, {"answers": 30, "grading": 10, "cross_grading": 4, "total": 44})
        self.assertEqual(pe.count_calls(2, 1, cross_n=9)["cross_grading"], 2)

    def test_plan_dry_run_writes_plan_and_makes_no_calls(self):
        out = Path(self.tmp.name) / "out"
        with contextlib.redirect_stdout(io.StringIO()):
            pe.main(["plan", "--providers", "codex:m1,gemini:m2", "--per-type", "1", "--data", str(self.data),
                     "--out", str(out), "--cross-grader", "gemini:m2", "--cross-n", "3"])
        plan = json.loads((out / "plan.json").read_text())
        self.assertEqual(plan["calls"]["answers"], len(plan["items"]) * 2)
        self.assertEqual(plan["cross_n"], 3)


class SpecTests(unittest.TestCase):
    def test_parse_spec(self):
        self.assertEqual(pe.parse_spec("codex:gpt-6.1-sol"), ("codex", "gpt-6.1-sol"))
        for bad in ("codex", "nope:m", "codex:"):
            with self.assertRaises(ValueError):
                pe.parse_spec(bad)

    def test_provider_of_baseline_is_claude(self):
        self.assertEqual(pe.provider_of("baseline:sonnet"), "claude")
        self.assertEqual(pe.provider_of("gemini:x"), "gemini")


class BlindingTests(unittest.TestCase):
    def test_order_is_reproducible_and_labelled(self):
        cands = ["codex:a", "gemini:b", "baseline:sonnet"]
        o1 = pe.blind_order(cands, 1, "claude:opus", "P-1")
        self.assertEqual(o1, pe.blind_order(cands[::-1], 1, "claude:opus", "P-1"))
        self.assertEqual([l for l, _ in o1], ["A", "B", "C"])
        self.assertEqual(sorted(c for _, c in o1), sorted(cands))

    def test_order_varies_with_seed_and_grader(self):
        cands = [f"codex:m{i}" for i in range(8)]
        orders = {tuple(pe.blind_order(cands, s, g, "P")) for s in (1, 2) for g in ("claude:opus", "codex:x")}
        self.assertGreater(len(orders), 1)

    def test_prompt_hides_identities(self):
        item = {"category": "debug", "prompt": "why", "files": ["f"], "expected": "e", "context": ""}
        answers = {"codex:secret-model": "ans one", "baseline:sonnet": "ans two"}
        order = pe.blind_order(answers, 1, "claude:opus", "P")
        text = pe.grading_prompt(item, order, answers)
        self.assertNotIn("secret-model", text)
        self.assertNotIn("sonnet", text.lower().replace("not gospel", ""))
        self.assertIn("=== Answer A ===", text)
        self.assertIn("ans one", text)

    def test_deblind_maps_labels_back(self):
        order = [("A", "codex:m"), ("B", "baseline:sonnet")]
        rows = pe.deblind({"A": ("wrong", "x"), "B": ("correct", "y")}, order, {"prompt_id": "P"})
        got = {r["candidate"]: r["verdict"] for r in rows}
        self.assertEqual(got, {"codex:m": "wrong", "baseline:sonnet": "correct"})

    def test_deblind_marks_missing_label_unparsed(self):
        rows = pe.deblind({"A": ("correct", "")}, [("A", "x:y"), ("B", "z:w")], {})
        self.assertEqual({r["label"]: r["verdict"] for r in rows}, {"A": "correct", "B": "unparsed"})


class ParseTests(unittest.TestCase):
    def test_json_lines(self):
        text = 'ok\n{"label":"A","verdict":"Correct","why":"file.ts:3"}\n{"label":"B","verdict":"wrong","why":"x"}'
        got = pe.parse_grades(text, {"A", "B"})
        self.assertEqual(got["A"], ("correct", "file.ts:3"))
        self.assertEqual(got["B"][0], "wrong")

    def test_prose_fallback_and_label_filter(self):
        got = pe.parse_grades("A: partial\nB - wrong\nZ: correct", {"A", "B"})
        self.assertEqual({k: v[0] for k, v in got.items()}, {"A": "partial", "B": "wrong"})

    def test_junk_and_bad_verdicts_ignored(self):
        self.assertEqual(pe.parse_grades('{"label":"A","verdict":"great"}', {"A"}), {})
        self.assertEqual(pe.parse_grades("", {"A"}), {})
        self.assertEqual(pe.parse_grades(None, {"A"}), {})


def grade(pid, cand, verdict, grader="claude:opus", ttype="security"):
    return {"prompt_id": pid, "task_type": ttype, "candidate": cand, "grader": grader, "verdict": verdict}


class AggregationTests(unittest.TestCase):
    def test_tally_and_score(self):
        rows = [grade("1", "codex:m", "correct"), grade("2", "codex:m", "partial"),
                grade("3", "codex:m", "wrong"), grade("4", "codex:m", "unparsed")]
        c = pe.tally(rows)[("security", "codex:m")]
        self.assertEqual(c, {"n": 3, "correct": 1, "partial": 1, "wrong": 1})
        self.assertAlmostEqual(pe.score_of(c), 0.5)

    def test_wilson_bounds(self):
        lo, hi = pe.wilson(5, 10)
        self.assertTrue(0 < lo < 0.5 < hi < 1)
        self.assertEqual(pe.wilson(0, 0), (0.0, 0.0))
        self.assertGreater(pe.wilson(10, 10)[0], 0.6)

    def test_beats_baseline_is_paired(self):
        rows = []
        for pid, cand, base in [("1", "correct", "wrong"), ("2", "correct", "correct"), ("3", "partial", "wrong")]:
            rows += [grade(pid, "codex:m", cand), grade(pid, "baseline:sonnet", base)]
        rows.append(grade("9", "codex:m", "correct"))  # no baseline grade: not paired
        wins = dict(pe.beats_baseline(rows))
        self.assertEqual(wins[("security", "codex:m")]["n"], 3)
        self.assertAlmostEqual(wins[("security", "codex:m")]["delta"], 0.5)

    def test_no_win_when_not_better(self):
        rows = [grade("1", "codex:m", "wrong"), grade("1", "baseline:sonnet", "correct")]
        self.assertEqual(pe.beats_baseline(rows), [])

    def test_self_preference_flagged_when_own_provider_favored(self):
        rows = []
        for i in range(6):  # claude grader says correct, codex grader says wrong, on claude answers
            rows += [grade(str(i), "baseline:sonnet", "correct", "claude:opus"),
                     grade(str(i), "baseline:sonnet", "wrong", "codex:x")]
        flags = {f["grader"]: f for f in pe.self_preference(rows)}
        self.assertTrue(flags["claude:opus"]["flagged"])
        self.assertNotIn("codex:x", flags)  # codex answers were never graded

    def test_self_preference_needs_enough_data(self):
        rows = [grade("1", "baseline:sonnet", "correct", "claude:opus"),
                grade("1", "baseline:sonnet", "wrong", "codex:x")]
        self.assertEqual(pe.self_preference(rows), [])

    def test_call_stats_uses_latest_row_per_key(self):
        rows = [{"prompt_id": "1", "candidate": "codex:m", "ok": False, "error_kind": "quota", "wall_ms": 5},
                {"prompt_id": "1", "candidate": "codex:m", "ok": True, "wall_ms": 2000},
                {"prompt_id": "2", "candidate": "codex:m", "ok": False, "error_kind": "quota", "wall_ms": 5},
                {"prompt_id": "3", "candidate": "codex:m", "ok": False, "error_kind": "other", "wall_ms": 5}]
        s = pe.call_stats(rows)["codex:m"]
        self.assertEqual((s["calls"], s["ok"], s["quota"], s["errors"]), (3, 1, 1, 1))
        self.assertEqual(s["wall_mean"], 2.0)

    def test_suggest_chains_orders_by_score_and_needs_n(self):
        counts = {("security", "a:x"): {"n": 10, "correct": 9, "partial": 0, "wrong": 1},
                  ("security", "b:y"): {"n": 10, "correct": 5, "partial": 0, "wrong": 5},
                  ("security", "c:z"): {"n": 3, "correct": 3, "partial": 0, "wrong": 0},
                  ("security", "baseline:sonnet"): {"n": 10, "correct": 10, "partial": 0, "wrong": 0}}
        self.assertEqual(pe.suggest_chains(counts), {"security": ["a:x", "b:y"]})

    def test_small_n_is_caveated_in_report(self):
        table = pe.quality_table({("security", "a:x"): {"n": 3, "correct": 3, "partial": 0, "wrong": 0}})
        self.assertIn("low confidence", table)


class RunTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.out = Path(self.tmp.name)
        items = [{"id": f"P{i}", "task_type": "security", "category": "security", "prompt": "q", "context": "",
                  "files": [], "expected": "", "miss": False, "stratum": "sample",
                  "baselines": {"sonnet": "sonnet answer"}} for i in range(3)]
        self.plan = {"seed": 1, "providers": ["codex:m", "gemini:g"], "items": items}

    def tearDown(self):
        self.tmp.cleanup()

    def test_run_is_resumable_and_stops_provider_on_quota(self):
        calls = []

        def fake(ask_bin, spec, snap, prompt):
            calls.append(spec)
            if spec == "gemini:g":
                return {"ok": False, "text": "", "error_kind": "quota", "error": "limit", "wall_ms": 1}
            return {"ok": True, "text": "ans", "error_kind": None, "wall_ms": 10}

        quiet = lambda *_: None
        pe.run_answers(self.plan, self.out, "ask", "/snap", workers=1, runner=fake, log=quiet)
        self.assertEqual(calls.count("gemini:g"), 1)  # stopped after the first quota error
        self.assertEqual(calls.count("codex:m"), 3)
        calls.clear()
        pe.run_answers(self.plan, self.out, "ask", "/snap", workers=1, runner=fake, log=quiet)
        self.assertNotIn("codex:m", calls)  # finished calls are not repeated
        self.assertEqual(calls.count("gemini:g"), 1)  # quota rows are retried, then stop again

    def test_grading_end_to_end_with_fake_grader(self):
        write_jsonl(self.out / "answers.jsonl",
                    [{"prompt_id": "P0", "candidate": "codex:m", "ok": True, "text": "codex says"}])

        def grader(ask_bin, spec, snap, prompt):
            sections = re.findall(r"=== Answer (\w) ===\n(.*)\n", prompt)
            rows = [{"label": l, "verdict": "correct" if "codex says" in t else "wrong", "why": "w"} for l, t in sections]
            return {"ok": True, "text": "\n".join(map(json.dumps, rows))}

        quiet = lambda *_: None
        n = pe.run_grading(self.plan, self.out, "ask", "/snap", "claude:opus", runner=grader, log=quiet)
        self.assertEqual(n, 1)  # only P0 has a provider answer
        rows = pe.read_jsonl(self.out / "grades.jsonl")
        got = {r["candidate"]: r["verdict"] for r in rows}
        self.assertEqual(got, {"codex:m": "correct", "baseline:sonnet": "wrong"})
        self.assertEqual(pe.run_grading(self.plan, self.out, "ask", "/snap", "claude:opus", runner=grader, log=quiet), 0)

    def test_call_ask_wraps_a_fake_binary(self):
        fake = self.out / "ask"
        fake.write_text('#!/bin/sh\ncat >/dev/null\necho \'{"ok":true,"text":"hi","wall_ms":3}\'\n')
        fake.chmod(fake.stat().st_mode | stat.S_IEXEC)
        self.assertEqual(pe.call_ask(fake, "codex:m", self.out, "q")["text"], "hi")
        broken = pe.call_ask(self.out / "missing", "codex:m", self.out, "q")
        self.assertFalse(broken["ok"])
        self.assertEqual(broken["error_kind"], "other")

    def test_snapshot_problem(self):
        self.assertIn("does not exist", pe.snapshot_problem(self.out / "nope"))
        self.assertIsNone(pe.snapshot_problem(self.out))

    def test_report_builds_without_grades(self):
        text = pe.build_report(self.plan | {"items": [dict(i, miss=False) for i in self.plan["items"]]}, [], [])
        self.assertIn("No grades yet", text)


if __name__ == "__main__":
    unittest.main()
