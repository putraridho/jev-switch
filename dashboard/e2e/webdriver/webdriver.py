#!/usr/bin/env python3
"""WebDriver end-to-end test on the real WebKitGTK view (docs/dashboard/DESIGN.md 11.4 stage 13).

    dashboard/e2e/webdriver/webdriver.py [path/to/jev-dashboard]

The same isolated launch as the smoke test (private activation-free bus, HOME and every XDG
dir inside a 0700 temp tree, a PATH with only the stub codex and claude), driven through
`tauri-driver` + `WebKitWebDriver`. Asserts: the overview shows the fixture numbers; detail
and settings open by keyboard; a threshold change lands in settings.json; axe finds nothing on
each screen inside the real view.

Exit codes: 0 pass, 1 fail, 3 NOT RUN (a tool or the display is missing; never a pass),
4 ran and passed except that axe could not be injected under the production CSP (the D16
assumption failed; axe then stays in stage 8 and the lead is told).
"""
import json
import os
import shutil
import signal
import stat
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request

HERE = os.path.dirname(os.path.abspath(__file__))
DASH = os.path.abspath(os.path.join(HERE, "..", ".."))
SMOKE = os.path.join(HERE, "..", "smoke")
PORT = 4444
AXE = os.path.join(DASH, "node_modules", "axe-core", "axe.min.js")


def not_run(reason):
    print("webdriver: NOT RUN: " + reason, file=sys.stderr)
    sys.exit(3)


def fail(reason):
    print("webdriver: FAIL: " + reason, file=sys.stderr)
    sys.exit(1)


class Driver:
    def __init__(self):
        self.base = "http://127.0.0.1:%d" % PORT
        self.session = None

    def call(self, method, path, body=None):
        data = None if body is None else json.dumps(body).encode()
        request = urllib.request.Request(
            self.base + path, data=data, method=method,
            headers={"Content-Type": "application/json"})
        try:
            with urllib.request.urlopen(request, timeout=60) as response:
                return json.loads(response.read().decode())["value"]
        except urllib.error.HTTPError as error:
            fail("%s %s: %s %s" % (method, path, error.code, error.read().decode()[:300]))

    def start(self, binary):
        value = self.call("POST", "/session", {"capabilities": {"alwaysMatch": {
            "tauri:options": {"application": binary}}}})
        self.session = value["sessionId"]

    def s(self, path):
        return "/session/%s%s" % (self.session, path)

    def script(self, source, *args):
        return self.call("POST", self.s("/execute/sync"), {"script": source, "args": list(args)})

    def script_async(self, source, *args):
        return self.call("POST", self.s("/execute/async"), {"script": source, "args": list(args)})

    def text(self):
        return self.script("return document.body.innerText;")

    def keys(self, *keys):
        actions = []
        for key in keys:
            actions.append({"type": "keyDown", "value": key})
            actions.append({"type": "keyUp", "value": key})
        self.call("POST", self.s("/actions"), {"actions": [{"type": "key", "id": "kbd", "actions": actions}]})

    def wait_text(self, needle, seconds=15):
        end = time.time() + seconds
        while time.time() < end:
            if needle in self.text():
                return
            time.sleep(0.2)
        html = ""
        try:
            html = str(self.script("return document.body.innerHTML"))[:1500]
        except SystemExit:
            pass
        fail("text %r never appeared; the view says: %r; html: %s" % (needle, self.text()[:400], html))

    def quit(self):
        if self.session:
            try:
                self.call("DELETE", "/session/" + self.session)
            except SystemExit:
                pass


def main():
    binary = os.path.abspath(sys.argv[1] if len(sys.argv) > 1 else os.path.join(DASH, "..", "target", "dist", "jev-dashboard"))
    if not os.access(binary, os.X_OK):
        fail("binary not found: " + binary + " (build stage 9 first)")
    tauri_driver = shutil.which("tauri-driver") or os.path.expanduser("~/.cargo/bin/tauri-driver")
    if not os.access(tauri_driver, os.X_OK):
        not_run("tauri-driver is missing: scripts/ci.sh --bootstrap installs it")
    native = shutil.which("WebKitWebDriver")
    if not native:
        not_run("WebKitWebDriver is missing: the user installs it with `sudo pacman -S webkitgtk-6.0`")
    dbus_daemon = shutil.which("dbus-daemon")
    dbus_run = shutil.which("dbus-run-session")
    if not (dbus_daemon and dbus_run):
        not_run("dbus-run-session / dbus-daemon are missing")
    runtime = os.environ.get("XDG_RUNTIME_DIR") or "/run/user/%d" % os.getuid()
    wayland = os.environ.get("WAYLAND_DISPLAY")
    if not wayland:
        not_run("WAYLAND_DISPLAY is not set: stage 13 needs a graphical session")
    socket_path = wayland if wayland.startswith("/") else os.path.join(runtime, wayland)
    if not stat.S_ISSOCK(os.stat(socket_path).st_mode if os.path.exists(socket_path) else 0):
        not_run("no Wayland socket at " + socket_path)

    tree = tempfile.mkdtemp()
    os.chmod(tree, 0o700)
    for name in ("home", "config", "data", "cache", "state/jev-router", "runtime", "bin"):
        os.makedirs(os.path.join(tree, name), mode=0o700, exist_ok=True)
    for stub in ("codex", "claude"):
        shutil.copy(os.path.join(SMOKE, "stubs", stub), os.path.join(tree, "bin", stub))
        os.chmod(os.path.join(tree, "bin", stub), 0o755)
    usage_path = os.path.join(tree, "state", "jev-router", "usage.jsonl")
    shutil.copy(os.path.join(DASH, "fixtures", "claude-gateway", "canary.jsonl"), usage_path)
    # The fixture's timestamps are old, so its windows have elapsed. Append one fresh row (the
    # first canary row with the clock and the windows moved to now) so the cards show live
    # numbers: 5-hour 50 %, 7-day 37 %.
    now = int(time.time())
    with open(os.path.join(DASH, "fixtures", "claude-gateway", "canary.jsonl")) as source:
        row = json.loads(source.readline())
    limits = row["ratelimit"]
    row["ts_ms"] = now * 1000
    for key in list(limits):
        if key.endswith("5h-utilization"):
            limits[key] = "0.50"
        elif key.endswith("7d-utilization"):
            limits[key] = "0.37"
        elif key.endswith("5h-reset") or key.endswith("unified-reset"):
            limits[key] = str(now + 3 * 3600)
        elif key.endswith("7d-reset"):
            limits[key] = str(now + 3 * 86400)
    with open(usage_path, "a") as out:
        out.write(json.dumps(row, separators=(",", ":")) + "\n")
    env = {
        "PATH": os.path.join(tree, "bin") + ":" + os.path.dirname(tauri_driver) + ":" + os.path.dirname(native),
        "HOME": os.path.join(tree, "home"),
        "XDG_CONFIG_HOME": os.path.join(tree, "config"), "XDG_DATA_HOME": os.path.join(tree, "data"),
        "XDG_CACHE_HOME": os.path.join(tree, "cache"), "XDG_STATE_HOME": os.path.join(tree, "state"),
        "XDG_RUNTIME_DIR": os.path.join(tree, "runtime"),
        "WAYLAND_DISPLAY": socket_path, "GDK_BACKEND": "wayland",
    }
    command = [dbus_run, "--dbus-daemon=" + dbus_daemon,
               "--config-file=" + os.path.join(SMOKE, "session-bus.conf"), "--",
               tauri_driver, "--port", str(PORT), "--native-driver", native]
    log = open(os.path.join(tree, "driver.log"), "wb")
    process = subprocess.Popen(command, env=env, stdout=log, stderr=log, start_new_session=True)
    driver = Driver()
    axe_injected = True
    try:
        end = time.time() + 20
        while True:
            try:
                urllib.request.urlopen(driver.base + "/status", timeout=1).read()
                break
            except (urllib.error.URLError, ConnectionError, OSError):
                if time.time() > end or process.poll() is not None:
                    fail("tauri-driver did not start; log: " + open(os.path.join(tree, "driver.log"), errors="replace").read()[-600:])
                time.sleep(0.2)

        driver.start(binary)

        # 1. The overview shows the fixture numbers (canary.jsonl: 5h 50 %, 7d 37 %; stub codex: 23 %, 61.5 %).
        driver.wait_text("Flashboard")
        driver.wait_text("Continue")
        driver.script("[...document.querySelectorAll('button')].find(b => b.textContent.trim() === 'Continue').click()")
        for number in ("50%", "37%", "23%"):
            driver.wait_text(number)
        print("webdriver: overview shows the fixture numbers")

        # 2. Detail and settings open by keyboard.
        driver.keys(",")
        driver.wait_text("Thresholds")
        driver.keys("")  # Escape
        driver.wait_text("Claude")
        driver.keys("1")  # first provider in the fixed order: Claude
        driver.wait_text("Claude")
        print("webdriver: detail and settings open by keyboard")

        # 3. Change a threshold; settings.json carries it.
        driver.keys("")
        driver.keys(",")
        driver.wait_text("Thresholds")
        element = driver.script(
            "const l=[...document.querySelectorAll('label')].find(x=>x.textContent.includes('Warning'));"
            "return l ? l.querySelector('input') : null;")
        if not element:
            fail("no Warning threshold input in Settings")
        key = [k for k in element if k.startswith("element-")][0]
        eid = element[key]
        driver.call("POST", driver.s("/element/%s/click" % eid), {})
        driver.call("POST", driver.s("/element/%s/value" % eid), {"text": "\ue010\ue003\ue003\ue003" + "70" + "\ue004"})
        settings_path = os.path.join(tree, "state", "jev-router", "dashboard", "settings.json")
        end = time.time() + 10
        saved = None
        while time.time() < end:
            if os.path.exists(settings_path):
                saved = json.load(open(settings_path))
                if saved.get("warn_bp") == 7000:
                    break
            time.sleep(0.2)
        if not saved or saved.get("warn_bp") != 7000:
            fail("settings.json does not carry warn_bp 7000: %r" % (saved,))
        print("webdriver: threshold change verified in settings.json")

        # 4. axe inside the real view on each screen (the D16 assumption: injectable under the CSP).
        axe_source = open(AXE).read()
        for label, keys in (("settings", []), ("overview", [""]), ("detail", ["1"])):
            if keys:
                driver.keys(*keys)
                time.sleep(0.5)
            try:
                injected = driver.script("return typeof window.axe;")
                if injected != "object":
                    driver.script(axe_source)
                found = driver.script_async(
                    "const done=arguments[arguments.length-1];"
                    "axe.run(document).then(r=>done(r.violations.map(v=>v.id+': '+v.nodes.length)));")
            except SystemExit:
                axe_injected = False
                print("webdriver: D16 FAILED: axe could not be injected under the production CSP (%s)" % label)
                break
            if found:
                fail("axe violations on %s: %s" % (label, found))
            print("webdriver: axe clean on " + label)
    finally:
        driver.quit()
        try:
            os.killpg(process.pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
        log.close()
        shutil.rmtree(tree, ignore_errors=True)

    if not axe_injected:
        print("webdriver: PASS except axe-in-view (axe stays in stage 8; tell the lead)")
        sys.exit(4)
    print("webdriver: PASS")


main()
