#!/usr/bin/env python3
"""Check that a commit only moves code, apart from migration scaffolding."""

import argparse
from collections import Counter
import os
from pathlib import PurePosixPath
import re
import subprocess
import sys


ALLOWED_LINE = re.compile(r"^\s*(?:mod\s|pub\s+mod\s|use\s|pub\s+use\s|pub\(crate\)\s+use\s|#\[cfg\(|#\[path|#\[macro_use\])")
VISIBILITY = re.compile(r"^(\s*)pub(?:\(crate\)|\(super\))?\s+")
SCAFFOLD = {"mod.rs", "lib.rs", "legacy.rs", "Cargo.toml", "README.md"}


def git(*args):
    env = dict(os.environ, GIT_CONFIG_NOSYSTEM="1", GIT_CONFIG_GLOBAL=os.devnull)
    return subprocess.check_output(["git", *args], env=env)


def changes(rev):
    fields = git("diff", "--no-ext-diff", "-M", "-C", "--name-status", "-z", f"{rev}~1", rev, "--").split(b"\0")
    index = 0
    while index < len(fields) - 1:
        status = fields[index].decode("ascii")
        count = 2 if status[0] in "RC" else 1
        paths = [field.decode("utf-8", "surrogateescape") for field in fields[index + 1:index + 1 + count]]
        yield status, paths
        index += count + 1


def unmatched(rev, old, new, copied=False):
    if PurePosixPath(old).name == "Cargo.toml" and PurePosixPath(new).name == "Cargo.toml":
        return [], []
    if copied:
        before = git("show", f"{rev}~1:{old}")
        after = git("show", f"{rev}:{new}")
        if b"\0" in before or b"\0" in after:
            return ([] if before == after else ["<binary content differs>"], [])
        removed = Counter(VISIBILITY.sub(r"\1", line) for line in before.decode("utf-8", "surrogateescape").splitlines() if not ALLOWED_LINE.match(line))
        added = Counter(VISIBILITY.sub(r"\1", line) for line in after.decode("utf-8", "surrogateescape").splitlines() if not ALLOWED_LINE.match(line))
        return list((removed - added).elements()), list((added - removed).elements())
    diff = git("diff", "--no-ext-diff", "--no-color", "--no-textconv", "-M", "--unified=0", f"{rev}~1", rev, "--", old, new)
    removed, added = Counter(), Counter()
    in_hunk = False
    for line in diff.decode("utf-8", "surrogateescape").splitlines():
        if line.startswith("Binary files "):
            return ["<binary content differs>"], []
        if line.startswith("@@ "):
            in_hunk = True
            continue
        if not in_hunk or not line.startswith(("-", "+")):
            continue
        content = line[1:]
        if ALLOWED_LINE.match(content):
            continue
        canonical = VISIBILITY.sub(r"\1", content)
        (removed if line[0] == "-" else added)[canonical] += 1
    return list((removed - added).elements()), list((added - removed).elements())


def scaffold(path):
    return PurePosixPath(path).name in SCAFFOLD or path.startswith("tests/architecture_baseline/")


MODULE_PATH = re.compile(r"\b(crate|jev_router|super)(::[a-z_][a-z0-9_]*)+::(?=[A-Za-z_{])")


def item_canonical(line):
    """A line as an item move sees it: visibility dropped, and a module path in front of a name
    (`crate::backend::ProviderId`, `crate::shared::ProviderId`) reduced to `crate::…::`, since
    a caller rewritten to the item's new home is part of the move."""
    return MODULE_PATH.sub("crate::…::", VISIBILITY.sub(r"\1", line)).strip()


def item_move(rev):
    """Items moved between files: every line removed anywhere in the commit is added somewhere,
    and the reverse, apart from `use`/`mod` lines, visibility and module paths. Scaffolding
    (`mod.rs`, `lib.rs`, baselines, the architecture test's tables) and docs are left out."""
    diff = git("diff", "--no-ext-diff", "--no-color", "--no-textconv", "-M", "--unified=0", f"{rev}~1", rev, "--", "*.rs", "*.py")
    removed, added = Counter(), Counter()
    path = ""
    for line in diff.decode("utf-8", "surrogateescape").splitlines():
        if line.startswith("+++ b/"):
            path = line[6:]
            continue
        if line.startswith(("--- ", "+++ ", "diff ", "index ", "@@ ")):
            continue
        if not line.startswith(("-", "+")) or path == "tests/architecture.rs":
            continue
        content = line[1:]
        if ALLOWED_LINE.match(content) or not content.strip() or content.lstrip().startswith("//!"):
            continue
        (removed if line[0] == "-" else added)[item_canonical(content)] += 1
    return list((removed - added).elements()), list((added - removed).elements())


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("rev", nargs="?", default="HEAD")
    parser.add_argument("--items", action="store_true", help="the commit moves items between files (Phase 1a): compare all removed and added lines")
    args = parser.parse_args()
    rev = args.rev
    if args.items:
        removed, added = item_move(rev)
        for line in removed:
            print(f"- {line}")
        for line in added:
            print(f"+ {line}")
        if removed or added:
            print(f"move check: {len(removed)} removed and {len(added)} added lines are not part of a move")
            return 1
        print("move check: pure item move")
        return 0
    failures = 0
    for status, paths in changes(rev):
        if status[0] in "RC":
            old, new = paths
            removed, added = unmatched(rev, old, new, status[0] == "C")
            if removed or added:
                failures += 1
                print(f"{old} -> {new}: unmatched lines")
                for line in removed:
                    print(f"- {line}")
                for line in added:
                    print(f"+ {line}")
        else:
            path = paths[0]
            if status[0] in "AD":
                print(f"{path}: not a move: added/deleted")
                failed = not scaffold(path)
            else:
                print(f"{path}: not a move: {status}")
                failed = True
            if failed:
                failures += 1
                removed, added = unmatched(rev, path, path)
                for line in removed:
                    print(f"- {line}")
                for line in added:
                    print(f"+ {line}")
    if failures:
        print(f"move check: {failures} failing file(s)")
        return 1
    print("move check: pure move")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except subprocess.CalledProcessError as error:
        print(f"move check: git failed ({error.returncode})", file=sys.stderr)
        sys.exit(1)
