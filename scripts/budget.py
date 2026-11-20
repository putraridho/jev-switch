#!/usr/bin/env python3
"""Size budgets of DDD.md 9.2: production lines per file, lines per function.

Run: python3 -I scripts/budget.py [--root src] [--max-file 400] [--max-fn 60]
                                  [--fail] [--max-offenders N]

Production code excludes the `#[cfg(test)]` module and items, found by brace matching on
code whose comments and strings were lexed away. Report mode (the default) exits 0.
`--fail` exits 1 on any offender; `--max-offenders N` exits 1 above N (the ratchet).
`--root` may be a glob such as `crates/*/src`.
"""
import argparse
import bisect
import re
import sys
from pathlib import Path

IDENT = re.compile(r"[A-Za-z0-9_]")
RAW_STR = re.compile(r'b?r(#*)"')
CHAR_LIT = re.compile(r"'(?:\\(?:x[0-9a-fA-F]{2}|u\{[0-9a-fA-F_]+\}|.)|[^\\'\n])'")


def _blank(text):
    return "".join(c if c == "\n" else " " for c in text)


def blank_out(src):
    """Return src with comments, string and char literals replaced by spaces (newlines kept)."""
    out = []
    i, n = 0, len(src)
    while i < n:
        c = src[i]
        two = src[i:i + 2]
        word_start = not (i and IDENT.match(src[i - 1]))
        raw = RAW_STR.match(src, i) if c in "rb" and word_start else None
        if two == "//":
            j = src.find("\n", i)
            j = n if j < 0 else j
            out.append(" " * (j - i))
            i = j
        elif two == "/*":
            depth, j = 1, i + 2
            while j < n and depth:
                if src[j:j + 2] == "/*":
                    depth, j = depth + 1, j + 2
                elif src[j:j + 2] == "*/":
                    depth, j = depth - 1, j + 2
                else:
                    j += 1
            out.append(_blank(src[i:j]))
            i = j
        elif raw:
            end = '"' + raw.group(1)
            j = src.find(end, raw.end())
            j = n if j < 0 else j + len(end)
            out.append(_blank(src[i:j]))
            i = j
        elif c == '"' or (c == "b" and word_start and src[i + 1:i + 2] == '"'):
            j = i + (2 if c == "b" else 1)
            while j < n and src[j] != '"':
                j += 2 if src[j] == "\\" else 1
            j = min(j + 1, n)
            out.append(_blank(src[i:j]))
            i = j
        elif c == "'" and CHAR_LIT.match(src, i):  # otherwise a lifetime or a label
            end = CHAR_LIT.match(src, i).end()
            out.append(" " * (end - i))
            i = end
        else:
            out.append(c)
            i += 1
    return "".join(out)


def match_brace(code, open_at):
    """Index of the `}` matching the `{` at open_at (the last index if unbalanced)."""
    depth = 0
    for j in range(open_at, len(code)):
        if code[j] == "{":
            depth += 1
        elif code[j] == "}":
            depth -= 1
            if depth == 0:
                return j
    return len(code) - 1


def body_start(code, start):
    """Index of the `{` that opens the item starting at start, or None if a `;` comes first."""
    paren = 0
    for j in range(start, len(code)):
        c = code[j]
        if c in "([":
            paren += 1
        elif c in ")]":
            paren -= 1
        elif c == ";" and paren <= 0:
            return None
        elif c == "{" and paren <= 0:
            return j
    return None


def item_end(code, start):
    """End index of the item starting at start: its `;` or its matching `}`."""
    body = body_start(code, start)
    if body is not None:
        return match_brace(code, body)
    semi = code.find(";", start)
    return len(code) - 1 if semi < 0 else semi


def skip_attributes(code, j):
    """Index after any `#[...]` attributes that start at or after j."""
    while True:
        m = re.compile(r"\s*#\s*\[").match(code, j)
        if not m:
            return j
        depth, p = 0, m.end() - 1
        while p < len(code):
            depth += code[p] == "["
            depth -= code[p] == "]"
            p += 1
            if depth == 0:
                break
        j = p


CFG_TEST = re.compile(r"#\s*\[\s*cfg\s*\(\s*test\s*\)\s*\]")
FN = re.compile(r"\bfn\s+([A-Za-z_][A-Za-z0-9_]*)")


def analyse(src):
    """Return (production_line_count, [(fn_name, first_line, length)]) for one file's text."""
    code = blank_out(src)
    starts = [0] + [m.end() for m in re.finditer("\n", code)]

    def line_of(index):
        return bisect.bisect_right(starts, index)

    total = src.count("\n") + (0 if src.endswith("\n") or not src else 1)
    spans, test_lines = [], set()
    for m in CFG_TEST.finditer(code):
        end = item_end(code, skip_attributes(code, m.end()))
        spans.append((m.start(), end))
        test_lines.update(range(line_of(m.start()), line_of(end) + 1))
    fns = []
    for m in FN.finditer(code):
        if any(a <= m.start() <= b for a, b in spans):
            continue
        body = body_start(code, m.end())
        if body is None:
            continue
        first, last = line_of(m.start()), line_of(match_brace(code, body))
        fns.append((m.group(1), first, last - first + 1))
    return total - len(test_lines), fns


def rust_files(root):
    for p in sorted(Path(root).rglob("*.rs")):
        if p.name.endswith("_tests.rs") or "tests" in p.relative_to(root).parts[:-1]:
            continue  # test-only files have their own budget
        yield p


def scan(root, max_file, max_fn):
    """Return (file offenders, function offenders, files scanned, functions seen), largest first."""
    files, fns, nfiles, nfns = [], [], 0, 0
    for p in rust_files(root):
        lines, found = analyse(p.read_text(encoding="utf-8", errors="replace"))
        nfiles += 1
        nfns += len(found)
        if lines > max_file:
            files.append((lines, str(p)))
        fns.extend((length, f"{p}:{first}", name) for name, first, length in found if length > max_fn)
    return files, fns, nfiles, nfns


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--root", default="src", help="folder to scan; a glob such as crates/*/src works")
    ap.add_argument("--max-file", type=int, default=400)
    ap.add_argument("--max-fn", type=int, default=60)
    ap.add_argument("--fail", action="store_true", help="exit 1 when there is any offender")
    ap.add_argument("--max-offenders", type=int, default=None, help="exit 1 when offenders exceed N")
    args = ap.parse_args(argv)
    globbed = any(c in args.root for c in "*?[")
    roots = sorted(Path(".").glob(args.root)) if globbed else [Path(args.root)]
    files, fns, nfiles, nfns = [], [], 0, 0
    for root in roots:
        f, g, a, b = scan(root, args.max_file, args.max_fn)
        files, fns, nfiles, nfns = files + f, fns + g, nfiles + a, nfns + b
    files.sort(key=lambda t: (-t[0], t[1]))
    fns.sort(key=lambda t: (-t[0], t[1]))
    print(f"files over {args.max_file} production lines:")
    for lines, path in files:
        print(f"  {lines:6}  {path}")
    print(f"functions over {args.max_fn} lines:")
    for length, where, name in fns:
        print(f"  {length:6}  {where}  {name}")
    offenders = len(files) + len(fns)
    print(f"totals: {nfiles} files scanned, {nfns} functions; "
          f"{len(files)} file offenders, {len(fns)} function offenders, {offenders} in all")
    if args.fail and offenders:
        return 1
    if args.max_offenders is not None and offenders > args.max_offenders:
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
