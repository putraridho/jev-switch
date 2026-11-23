"""Measure the provider routing table from data instead of guessing.

Replays graded prompts from the lunagraph training run on each provider, grades the
answers blind, and compares quality per task type. Subcommands, in order:

  plan    choose the sample and print call counts (no model calls)
  run     ask every provider every prompt (spends quota; needs --yes)
  grade   blind-grade the answers (spends quota; needs --yes)
  report  tables + report.md (no model calls)
  audit   compare arms (router vs a pinned baseline) from audit logs (no model calls)

Run with `python3 -I scripts/provider_eval.py ...`. Pure logic is importable and tested
in scripts/test_provider_eval.py.
"""
import argparse
import datetime
import json
import sys
from pathlib import Path

from provider_eval.audit_report import build_report as build_audit_report
from provider_eval.answers import pending_answers, run_answers, snapshot_problem
from provider_eval.config import ASK_BIN, DATA_DIR, DEFAULT_GRADER, OUT_ROOT
from provider_eval.data import load_graded_items
from provider_eval.grading import grading_jobs, pick_cross, run_grading
from provider_eval.helpers import parse_spec, read_jsonl
from provider_eval.plan import build_plan, format_plan
from provider_eval.report import build_report


def load_plan(out_dir):
    path = Path(out_dir) / "plan.json"
    if not path.exists():
        sys.exit(f"no plan at {path}; run `plan` first")
    return json.loads(path.read_text())


def require_snapshot(path):
    problem = snapshot_problem(path)
    if problem:
        sys.exit(problem)


def cmd_plan(a):
    providers = [s.strip() for s in a.providers.split(",") if s.strip()]
    for s in providers + [a.grader] + ([a.cross_grader] if a.cross_grader else []):
        parse_spec(s)
    types = a.types.split(",") if a.types else None
    snapshot = Path(a.snapshot or Path(a.data) / "lunagraph-main")
    plan = build_plan(load_graded_items(a.data), providers, a.per_type, types, a.seed, snapshot,
                      a.grader, a.cross_grader, a.cross_n)
    out = Path(a.out or OUT_ROOT / datetime.date.today().isoformat())
    out.mkdir(parents=True, exist_ok=True)
    (out / "plan.json").write_text(json.dumps(plan, indent=1))
    print(format_plan(plan), f"\n\nwrote {out / 'plan.json'}", sep="")


def _confirm(a, plan, spent):
    if not a.yes:
        print(format_plan(plan))
        print(f"\nDRY RUN: {spent}. Re-run with --yes to spend the quota.")
        return False
    return True


def cmd_run(a):
    plan, out = load_plan(a.out), Path(a.out)
    snapshot = a.snapshot or plan["snapshot"]
    todo = pending_answers(plan, read_jsonl(out / "answers.jsonl"))
    if not _confirm(a, plan, f"{len(todo)} answer calls pending"):
        return
    require_snapshot(snapshot)
    run_answers(plan, out, a.ask_bin, snapshot, a.workers)


def cmd_grade(a):
    plan, out = load_plan(a.out), Path(a.out)
    snapshot = a.snapshot or plan["snapshot"]
    grader, cross = a.grader or plan["grader"], a.cross_grader or plan["cross_grader"]
    baselines = tuple(a.baselines.split(","))
    answers, grades = read_jsonl(out / "answers.jsonl"), read_jsonl(out / "grades.jsonl")
    cross_ids = pick_cross([i["id"] for i in plan["items"]], a.cross_n if a.cross_n is not None else plan["cross_n"], plan["seed"]) if cross else set()
    n_main = len(grading_jobs(plan, answers, grades, grader, None, baselines))
    n_cross = len(grading_jobs(plan, answers, grades, cross, cross_ids, baselines)) if cross else 0
    if not _confirm(a, plan, f"{n_main} grader calls ({grader}) + {n_cross} cross-grader calls"):
        return
    require_snapshot(snapshot)
    run_grading(plan, out, a.ask_bin, snapshot, grader, None, baselines, a.workers)
    if cross:
        run_grading(plan, out, a.ask_bin, snapshot, cross, cross_ids, baselines, a.workers)


def cmd_report(a):
    plan, out = load_plan(a.out), Path(a.out)
    text = build_report(plan, read_jsonl(out / "answers.jsonl"), read_jsonl(out / "grades.jsonl"), a.grader)
    (out / "report.md").write_text(text)
    print(text, f"wrote {out / 'report.md'}", sep="")


def parse_arm(text):
    label, sep, path = text.partition("=")
    if not sep or not label or not path:
        raise argparse.ArgumentTypeError("an arm is LABEL=PATH, for example router=router-audit.jsonl")
    return label, path


def cmd_audit(a):
    for _, path in a.arm:
        if not Path(path).is_file():
            sys.exit(f"no audit log at {path}")
    print(build_audit_report(a.arm), end="")


def make_parser():
    p = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    sub = p.add_subparsers(dest="cmd", required=True)

    def add(name, fn, help_):
        s = sub.add_parser(name, help=help_)
        s.set_defaults(fn=fn)
        return s

    def common(s, spend=False, out_required=True):
        s.add_argument("--out", required=out_required)
        if spend:
            s.add_argument("--yes", action="store_true", help="spend quota (default: dry run)")
            s.add_argument("--snapshot", help="repo snapshot dir (default: the plan's)")
            s.add_argument("--ask-bin", default=str(ASK_BIN))
            s.add_argument("--workers", type=int, default=2)

    s = add("plan", cmd_plan, "choose the sample, print call counts (no model calls)")
    common(s, out_required=False)
    s.add_argument("--providers", required=True, help="comma list of provider:model")
    s.add_argument("--per-type", type=int, default=5)
    s.add_argument("--types", help="comma list of router task types (default: all mapped)")
    s.add_argument("--snapshot")
    s.add_argument("--data", default=str(DATA_DIR))
    s.add_argument("--seed", type=int, default=1)
    s.add_argument("--grader", default=DEFAULT_GRADER)
    s.add_argument("--cross-grader")
    s.add_argument("--cross-n", type=int, default=10)
    common(add("run", cmd_run, "ask each provider each prompt (spends quota)"), spend=True)
    s = add("grade", cmd_grade, "blind-grade the answers (spends quota)")
    common(s, spend=True)
    s.add_argument("--grader")
    s.add_argument("--cross-grader")
    s.add_argument("--cross-n", type=int)
    s.add_argument("--baselines", default="sonnet", help="comma list of baseline answers to include: sonnet,opus")
    s = add("report", cmd_report, "tables and report.md (no model calls)")
    common(s)
    s.add_argument("--grader", help="grader whose grades fill the tables (default: the most used)")
    s = add("audit", cmd_audit, "accepted tasks, tokens per accepted task and failures per arm, from audit logs (no model calls)")
    s.add_argument("--arm", action="append", type=parse_arm, required=True, metavar="LABEL=PATH", help="one audit.jsonl per arm; the first is the baseline")
    return p


def main(argv=None):
    args = make_parser().parse_args(argv)
    args.fn(args)


if __name__ == "__main__":
    main()
