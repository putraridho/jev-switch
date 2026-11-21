"""The runner: selection, parallel execution, the pass cache, and main()."""

from .harness import *  # noqa: F401,F403

ENTRY = os.path.join(ROOT, "scripts", "test_pty_ux.py")


class Timeout(BaseException):
    pass


def on_alarm(signum, frame):
    raise Timeout()


def jobs_wanted():
    """How many scenarios run at once: `JEV_PTY_JOBS`, else 4 (or fewer on a small machine).
    Every scenario has its own home, repo and terminal, so they do not share anything."""
    try:
        return max(1, int(os.environ["JEV_PTY_JOBS"]))
    except (KeyError, ValueError):
        return max(1, min(4, os.cpu_count() or 1))


def pass_cache():
    """The scenarios that already passed on this exact binary: `target/pty-pass-<binary hash>/`
    holds one file per scenario, keyed by a hash of its source. Another build starts empty.
    `JEV_PTY_FRESH=1` ignores it. (A change to a helper above the scenarios is not seen: use it.)"""
    import hashlib
    import inspect

    with open(BIN, "rb") as f:
        binary = hashlib.sha256(f.read()).hexdigest()[:16]
    root = os.path.join(ROOT, "target", f"pty-pass-{binary}")
    os.makedirs(root, exist_ok=True)
    # Old builds' caches would pile up: keep only this binary's.
    for d in os.listdir(os.path.join(ROOT, "target")):
        if d.startswith("pty-pass-") and d != os.path.basename(root):
            shutil.rmtree(os.path.join(ROOT, "target", d), ignore_errors=True)

    def key(fn):
        return os.path.join(root, hashlib.sha256((fn.__name__ + inspect.getsource(fn)).encode()).hexdigest()[:16])

    return key


def run_parallel(selected, jobs):
    """Each scenario in its own process (`=name` selects exactly one), `jobs` at a time; the
    results print in the scenario order, as a serial run would."""
    from concurrent.futures import ThreadPoolExecutor

    def one(fn):
        started = time.time()
        r = subprocess.run([sys.executable, "-I", ENTRY, "=" + fn.__name__], stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, env={**os.environ, "JEV_PTY_JOBS": "1"})
        return fn, r.returncode, r.stdout, time.time() - started

    key = pass_cache()
    fresh = os.environ.get("JEV_PTY_FRESH", "") not in ("", "0")
    skipped = [] if fresh else [fn for fn in selected if os.path.exists(key(fn))]
    todo = [fn for fn in selected if fn not in skipped]
    failures = 0
    with ThreadPoolExecutor(max_workers=jobs) as pool:
        for fn, code, out, took in pool.map(one, todo):
            if code == 0:
                open(key(fn), "w").close()
            lines = [l for l in out.splitlines() if l.strip() and "scenarios passed" not in l]
            if code == 0:
                print(lines[0] if lines else f"ok    {fn.__name__.replace('_', ' ')} ({took:.1f}s)")
            else:
                failures += 1
                print("\n".join(lines) or f"FAIL  {fn.__name__.replace('_', ' ')}\n      exit {code}")
            sys.stdout.flush()
    note = f" ({len(skipped)} unchanged since they last passed on this build; JEV_PTY_FRESH=1 runs them)" if skipped else ""
    print(f"\n{len(selected) - failures}/{len(selected)} scenarios passed{note}")
    return 1 if failures else 0


def main():
    if not os.path.exists(BIN):
        print(f"missing {BIN}: run `cargo build --bin code` first", file=sys.stderr)
        return 2
    wanted = sys.argv[1:]
    exact = [w[1:] for w in wanted if w.startswith("=")]
    if exact:
        wanted = []
    selected = [
        fn
        for fn in SCENARIOS
        if (exact and fn.__name__ in exact)
        or (not exact and (not wanted or any(w in fn.__name__ or w in fn.__name__.replace("_", " ") for w in wanted)))
    ]
    jobs = jobs_wanted()
    if len(selected) > 1 and jobs > 1:
        return run_parallel(selected, jobs)
    jev, url = start_jev()
    signal.signal(signal.SIGALRM, on_alarm)
    failures = 0
    ran = 0
    for fn in selected:
        name = fn.__name__.replace("_", " ")
        ran += 1
        world = World(url, fn.__name__[:12])
        started = time.time()
        signal.alarm(SCENARIO_TIMEOUT)
        try:
            fn(world)
            print(f"ok    {name} ({time.time() - started:.1f}s)")
        except Fail as e:
            failures += 1
            print(f"FAIL  {name}\n      {e}".replace("\n", "\n      "))
        except Timeout:
            failures += 1
            print(f"FAIL  {name}\n      timed out after {SCENARIO_TIMEOUT}s")
        except Exception:
            failures += 1
            print(f"FAIL  {name}\n" + traceback.format_exc())
        finally:
            signal.alarm(0)
            world.cleanup()
    jev.shutdown()
    if ran == 0:
        # A filter that matched nothing tested nothing: not a pass.
        print(f"no scenario matches {' '.join(wanted)}", file=sys.stderr)
        return 1
    print(f"\n{ran - failures}/{ran} scenarios passed")
    return 1 if failures else 0
