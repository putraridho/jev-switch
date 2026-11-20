#!/usr/bin/env python3
"""Compare the working tree's binaries with a revision in isolated environments."""

import argparse
import difflib
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile

sys.path.insert(0, str(Path(__file__).resolve().parent))
from test_pty_ux import FAKE_CLAUDE, LONG_COMMAND, MULTILINE_COMMAND, start_jev


ROOT = Path(__file__).resolve().parent.parent
ISO_TIME = re.compile(r"\b\d{4}-\d\d-\d\d[T ]\d\d:\d\d:\d\d(?:\.\d+)?(?:Z|[+-]\d\d:?\d\d)?")
UNIX_TIME = re.compile(r"(?<!\d)[1-9]\d{9}(?:\d{3})?(?!\d)")
PID = re.compile(r'(?i)(\b(?:pid|process[_ -]?id)\b["\s:=]+)\d+')


def mask(text, paths=()):
    for path, replacement in sorted(paths, key=lambda pair: len(str(pair[0])), reverse=True):
        text = text.replace(str(path), replacement)
    text = ISO_TIME.sub("<TIMESTAMP>", text)
    text = UNIX_TIME.sub("<TIMESTAMP>", text)
    text = re.sub(r"(?<=<TIMESTAMP>)-\d+(?=\.|\b)", "-<PID>", text)
    # Per-run values that are not times of day: how long something took, and the probe cache's
    # fingerprint of the binary (two builds always differ there).
    text = re.sub(r'"(wall_ms|elapsed_ms|duration_ms|latency_ms)":\d+', r'"\1":<MS>', text)
    text = re.sub(r'"fingerprint":\d+', '"fingerprint":<FINGERPRINT>', text)
    text = re.sub(r"(?<=· )\d+(?:\.\d+)?(ms|s)\b", r"<DURATION>", text)
    text = re.sub(r"(?<=Worked for )\d+(?:\.\d+)?(ms|s)\b", r"<DURATION>", text)
    return PID.sub(r"\1<PID>", text)


def compare(label, old, new):
    differences = []
    for field in sorted(set(old) | set(new)):
        before = old.get(field)
        after = new.get(field)
        if before == after:
            continue
        before = "<missing>\n" if before is None else str(before)
        after = "<missing>\n" if after is None else str(after)
        before_lines = before.splitlines(True)
        after_lines = after.splitlines(True)
        if before_lines and not before_lines[-1].endswith("\n"):
            before_lines[-1] += "\n\\ No newline at end of file\n"
        if after_lines and not after_lines[-1].endswith("\n"):
            after_lines[-1] += "\n\\ No newline at end of file\n"
        differences.append("".join(difflib.unified_diff(before_lines, after_lines,
                                                        fromfile=f"{label}/old/{field}",
                                                        tofile=f"{label}/new/{field}")))
    return differences


def pinned_toolchain():
    """The channel `rust-toolchain.toml` of the working tree pins, so both builds use one compiler."""
    path = ROOT / "rust-toolchain.toml"
    if not path.exists():
        return None
    for line in path.read_text().splitlines():
        if line.strip().startswith("channel"):
            return line.split("=", 1)[1].strip().strip('"')
    return None


def build(source, build_home):
    """Build with the developer's own toolchain (a build runs nothing of ours), offline, into the
    source's own `target/`. Only the runs of the built programs are hermetic."""
    del build_home  # the build needs the real rustup and cargo homes
    env = os.environ.copy()
    env.update(CARGO_NET_OFFLINE="true", CARGO_TARGET_DIR=str(source / "target"))
    toolchain = pinned_toolchain()
    if toolchain:
        env["RUSTUP_TOOLCHAIN"] = toolchain
    subprocess.run(["cargo", "build", "--bins"], cwd=source, env=env, check=True)


def export(rev, directory, home):
    env = dict(os.environ, HOME=str(home), XDG_CONFIG_HOME=str(home),
               GIT_CONFIG_NOSYSTEM="1", GIT_CONFIG_GLOBAL=os.devnull)
    with tempfile.TemporaryFile() as archive:
        subprocess.run(["git", "archive", rev], cwd=ROOT, env=env, stdout=archive, check=True)
        archive.seek(0)
        subprocess.run(["tar", "-x", "-C", str(directory)], stdin=archive, env=env, check=True)


def make_bin(directory):
    directory.mkdir()
    for name in ("sh", "git", "cat", "sed", "printf", "sleep", "env", "tail"):
        executable = shutil.which(name)
        if not executable:
            raise RuntimeError(f"required executable not found: {name}")
        (directory / name).symlink_to(executable)
    claude = directory / "claude"
    claude.write_text(FAKE_CLAUDE.replace("@LONGCMD@", LONG_COMMAND).replace("@MULTILINE@", MULTILINE_COMMAND))
    claude.chmod(0o755)


def run_case(binary, args, input_text, directory, fake_bin, url, source, scratch):
    directory.mkdir()
    for name in ("home", "state", "data", "work", "pids", "tmp"):
        (directory / name).mkdir()
    env = {
        "PATH": str(fake_bin), "HOME": str(directory / "home"),
        "XDG_STATE_HOME": str(directory / "state"), "XDG_DATA_HOME": str(directory / "data"),
        "TMPDIR": str(directory / "tmp"),
        "JEV_API_KEY": "test", "JEV_TYPESAFE_URL": url,
        "CLAUDE_BIN": str(fake_bin / "claude"), "FAKE_PIDS": str(directory / "pids"),
        "JEV_TIMEOUT_MS": "3000", "JEV_ROUTER_ENV": str(directory / "home" / "empty.env"),
        "GIT_CONFIG_NOSYSTEM": "1", "GIT_CONFIG_GLOBAL": os.devnull,
        "LANG": "C.UTF-8", "TERM": "dumb", "NO_COLOR": "1",
    }
    result = subprocess.run([str(binary), *args], cwd=directory / "work", env=env,
                            input=input_text or "", capture_output=True, text=True, timeout=60)
    paths = [(directory / "home", "<HOME>"), (directory / "state", "<STATE>"),
             (directory / "data", "<DATA>"), (directory / "work", "<WORK>"),
             (directory / "pids", "<PIDS>"), (directory / "tmp", "<TMPDIR>"), (fake_bin, "<BIN>"),
             (source, "<REPO>"), (scratch, "<TMP>")]
    snapshot = {"stdout": mask(result.stdout, paths), "stderr": mask(result.stderr, paths),
                "exit code": str(result.returncode)}
    state = directory / "state"
    for path in sorted(state.rglob("*")):
        if path.is_file():
            name = mask(str(path.relative_to(state)), paths)
            key = f"state/{name}"
            if key in snapshot:
                raise RuntimeError(f"duplicate masked state path: {key}")
            snapshot[key] = mask(path.read_bytes().decode("utf-8", "surrogateescape"), paths)
    return result.returncode, snapshot


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("rev", nargs="?", default="ddd-start")
    rev = parser.parse_args().rev
    with tempfile.TemporaryDirectory(prefix="jev-parity-") as temp:
        scratch = Path(temp)
        old = scratch / "old"
        old.mkdir()
        build_home = scratch / "build-home"
        build_home.mkdir()
        export(rev, old, build_home)
        print(f"parity: building {rev}", flush=True)
        build(old, build_home)
        print("parity: building working tree", flush=True)
        build(ROOT, build_home)
        fake_bin = scratch / "bin"
        make_bin(fake_bin)
        server, url = start_jev()
        try:
            rpc = ('{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"parity","version":"1"}}}\n'
                   '{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}\n')
            cases = [("help", "code", ["--help"], None, False),
                     ("doctor", "code", ["doctor"], None, False),
                     ("print", "code", ["-p", "hello"], None, False),
                     ("delegate_mcp", "delegate_mcp", [], rpc, False),
                     ("ask", "ask", ["hello"], None, False)]
            for path in sorted((old / "src" / "bin").glob("*.rs")):
                cases.append((f"version-{path.stem}", path.stem, ["--version"], None, True))
            differences = []
            for index, (label, name, args, input_text, optional) in enumerate(cases):
                old_result, before = run_case(old / "target" / "debug" / name, args, input_text,
                                              scratch / f"{index}-old", fake_bin, url, old, scratch)
                if optional and old_result != 0:
                    continue
                _, after = run_case(ROOT / "target" / "debug" / name, args, input_text,
                                    scratch / f"{index}-new", fake_bin, url, ROOT, scratch)
                differences.extend(compare(label, before, after))
            if differences:
                for difference in differences:
                    print(difference, end="")
                return 1
            print("parity: no differences")
            return 0
        finally:
            server.shutdown()
            server.server_close()


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, subprocess.SubprocessError, RuntimeError) as error:
        print(f"parity: {error}", file=sys.stderr)
        sys.exit(1)
