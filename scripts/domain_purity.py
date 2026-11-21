"""Domain purity check: lines of domain production code (tests and comments stripped) that use
serde or process-level IO (files, the environment, processes, and Path methods that ask the disk). Usage: python3 -I purity.py <src-root>"""
import os, re, sys
BAD = re.compile(r"\bserde(_json)?\b|std::fs|std::env|std::process|std::net|\bCommand::new|\bfs::(read|write|create|remove|rename|metadata|canonicalize|symlink_metadata|copy|File)|\benv::var|\bFile::(open|create)|\.canonicalize\(|\.is_dir\(\)|\.is_file\(\)|\.exists\(\)|\bsymlink_metadata\b|\bread_dir\b|\bmetadata\(")
def strip_tests(text):
    out, lines, i = [], text.split("\n"), 0
    while i < len(lines):
        l = lines[i]
        if l.strip().startswith("#[cfg(test)]"):
            j = i + 1
            if j < len(lines) and re.match(r"\s*(pub(\(\w+\))?\s+)?mod\s+\w+\s*\{", lines[j]):
                depth = 0
                while j < len(lines):
                    depth += lines[j].count("{") - lines[j].count("}")
                    j += 1
                    if depth <= 0:
                        break
                i = j
                continue
            if j < len(lines) and re.match(r"\s*mod\s+\w+;", lines[j]):
                i = j + 1
                continue
        out.append((i + 1, l))
        i += 1
    return out
root = sys.argv[1]
total = 0
for d, _, fs in os.walk(root):
    if "/domain" not in d + "/" or "/tests" in d:
        continue
    for f in fs:
        if not f.endswith(".rs") or f == "tests.rs":
            continue
        p = os.path.join(d, f)
        hits = [(n, l.strip()) for n, l in strip_tests(open(p).read()) if not l.lstrip().startswith("//") and BAD.search(l)]
        if hits:
            total += len(hits)
            print(f"{len(hits):3} {p}")
            for n, l in hits[:4]:
                print(f"      {n}: {l[:110]}")
print("TOTAL", total)
