#!/usr/bin/env bash
# Real-binary smoke test, no driver (docs/dashboard/DESIGN.md 11.4 stage 10, TASKS T12).
#
#   dashboard/e2e/smoke/smoke.sh [path/to/jev-dashboard]
#
# Starts the `dist` binary on the current Wayland display, isolated from the user's session:
# a private activation-free bus (session-bus.conf), HOME, every XDG_*_HOME and
# XDG_RUNTIME_DIR inside a 0700 temp tree holding a synthetic, canary-laced usage.jsonl, and
# a PATH that holds only the stub `codex` and `claude`. Exit: 0 pass, 1 fail, 3 NOT RUN
# (no graphical session or no dbus-run-session; never a pass).
set -u

here="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
dashboard="$(cd -- "$here/../.." && pwd)"
bin="${1:-$dashboard/../target/dist/jev-dashboard}"
canary='JEVCANARY_9f3c1b7e_DO_NOT_SURFACE'
window_budget_ms=550

fail() { printf 'smoke: FAIL: %s\n' "$*" >&2; exit 1; }
not_run() { printf 'smoke: NOT RUN: %s\n' "$*" >&2; exit 3; }

bin="$(cd -- "$(dirname -- "$bin")" 2>/dev/null && pwd)/$(basename -- "$bin")"
[ -x "$bin" ] || fail "binary not found: $bin (build stage 9 first)"
command -v dbus-run-session >/dev/null 2>&1 || not_run "dbus-run-session is missing (pacman -S dbus)"
command -v sqlite3 >/dev/null 2>&1 || not_run "sqlite3 is missing (pacman -S sqlite)"
dbus_daemon="$(command -v dbus-daemon)" || not_run "dbus-daemon is missing (pacman -S dbus)"

# The real runtime directory, before any override: the Wayland socket lives there.
real_runtime="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"
wayland_name="${WAYLAND_DISPLAY:-}"
[ -n "$wayland_name" ] || not_run "WAYLAND_DISPLAY is not set: stage 10 needs a graphical session"
case "$wayland_name" in /*) socket="$wayland_name" ;; *) socket="$real_runtime/$wayland_name" ;; esac
[ -S "$socket" ] || not_run "no Wayland socket at $socket"

# What the real directories hold now: names only, sizes and times are not compared.
real_home="${HOME:?}"
real_config="${XDG_CONFIG_HOME:-$real_home/.config}"
real_data="${XDG_DATA_HOME:-$real_home/.local/share}"
real_cache="${XDG_CACHE_HOME:-$real_home/.cache}"
real_state="${XDG_STATE_HOME:-$real_home/.local/state}"
outside_names() {
  ls -1 "$real_config/autostart" 2>/dev/null | sed 's|^|autostart/|'
  ls -1d "$real_state"/jev-router/dashboard* 2>/dev/null
  ls -1d "$real_data"/dev.jev.flashboard* 2>/dev/null
  ls -1d "$real_cache"/dev.jev.flashboard* 2>/dev/null
}
outside_before="$(outside_names | sort)"

tree="$(mktemp -d)"
chmod 700 "$tree"
children=""
cleanup() {
  # shellcheck disable=SC2046
  kill $(app_pids) 2>/dev/null
  [ -n "$children" ] && kill "$children" 2>/dev/null
  rm -rf -- "$tree"
}
trap cleanup EXIT

mkdir -p "$tree"/{home,config,data,cache,state/jev-router,runtime,bin}
chmod 700 "$tree"/{home,config,data,cache,state,state/jev-router,runtime}
cp "$here"/stubs/codex "$here"/stubs/claude "$tree/bin/"
chmod 755 "$tree"/bin/codex "$tree"/bin/claude
usage="$tree/state/jev-router/usage.jsonl"
cp "$dashboard/fixtures/claude-gateway/canary.jsonl" "$usage"
chmod 600 "$usage"

# launch <name> [seconds-to-wait-for-Started]: starts the binary, stderr to $tree/<name>.err.
# Sets $pid to the app's process id (the child of dbus-run-session).
launch() {
  local name="$1"
  env -i \
    PATH="$tree/bin" \
    HOME="$tree/home" \
    XDG_CONFIG_HOME="$tree/config" XDG_DATA_HOME="$tree/data" \
    XDG_CACHE_HOME="$tree/cache" XDG_STATE_HOME="$tree/state" \
    XDG_RUNTIME_DIR="$tree/runtime" \
    WAYLAND_DISPLAY="$socket" \
    GDK_BACKEND=wayland \
    /usr/bin/dbus-run-session --dbus-daemon="$dbus_daemon" --config-file="$here/session-bus.conf" -- "$bin" \
    2>"$tree/$name.err" >/dev/null &
  session_pid=$!
  children="$session_pid"
}

waitfor() { # waitfor <file> <pattern> <seconds>
  local i=0 limit=$(($3 * 10))
  while [ "$i" -lt "$limit" ]; do
    grep -q -- "$2" "$1" 2>/dev/null && return 0
    kill -0 "$session_pid" 2>/dev/null || return 1
    sleep 0.1
    i=$((i + 1))
  done
  return 1
}

# Anchored: the dbus-run-session command line also contains the binary path.
app_pids() { pgrep -f -- "^$bin( |\$)" 2>/dev/null; }

stop() { # stop <seconds>: SIGTERM the app, wait for the session to end
  local app
  app="$(app_pids | head -n1)"
  [ -n "$app" ] && kill -TERM "$app" 2>/dev/null
  local i=0 limit=$(($1 * 10))
  while kill -0 "$session_pid" 2>/dev/null; do
    [ "$i" -ge "$limit" ] && return 1
    sleep 0.1
    i=$((i + 1))
  done
  wait "$session_pid" 2>/dev/null
  children=""
  return 0
}

samples() { sqlite3 -readonly "$tree/state/jev-router/dashboard/history.db" 'select count(*) from sample' 2>/dev/null || echo 0; }

# ---- launch 1: cold start (builds the font and shader caches), fetches, new sample ----
launch first
waitfor "$tree/first.err" 'flashboard: Started' 20 || fail "no Started line (see first launch stderr below)$(printf '\n'; cat "$tree/first.err")"
waitfor "$tree/first.err" 'SourceOk source=ClaudeGatewayLog' 20 || fail "no successful Claude-log fetch"
waitfor "$tree/first.err" 'SourceOk source=CodexAppServer' 30 || fail "no successful Codex fetch"
waitfor "$tree/first.err" 'WindowReady' 20 || fail "no WindowReady on the first launch"

before=$(samples)
now_ms=$(($(date +%s) * 1000))
sed '1!d; s/"ts_ms":[0-9]*/"ts_ms":'"$now_ms"'/' "$dashboard/fixtures/claude-gateway/sample.jsonl" >>"$usage"
i=0
while [ "$(samples)" -le "$before" ]; do
  [ "$i" -ge 30 ] && fail "no new sample within 3 s of an appended row (samples stayed at $before)"
  sleep 0.1
  i=$((i + 1))
done
stop 15 || fail "first launch did not exit on SIGTERM within 15 s"
[ "$(samples)" -ge 1 ] || fail "history.db has no rows"

# ---- launch 2: the second launch in the same tree is what WindowReady is judged on ----
launch second
waitfor "$tree/second.err" 'WindowReady' 20 || fail "no WindowReady on the second launch"
took=$(sed -n 's/.*WindowReady took_ms=\([0-9]*\).*/\1/p' "$tree/second.err" | head -n1)
[ -n "$took" ] || fail "WindowReady has no took_ms"
printf 'smoke: WindowReady took %s ms (budget %s ms)\n' "$took" "$window_budget_ms"
[ "$took" -le "$window_budget_ms" ] || fail "WindowReady took ${took} ms, over the ${window_budget_ms} ms budget"
stop 15 || fail "second launch did not exit on SIGTERM within 15 s"

# ---- launch 3: a hanging `codex`, then SIGTERM: clean exit and no child left ----
: >"$tree/bin/hang"
launch third
waitfor "$tree/third.err" 'flashboard: Started' 20 || fail "no Started line on the hanging launch"
sleep 2
stop 15 || fail "the hanging-codex launch did not exit on SIGTERM within 15 s"
sleep 1
if pgrep -f -- '^/usr/bin/sleep 86413$' >/dev/null 2>&1; then fail "a child of the stub codex is still running after exit"; fi
if [ -n "$(app_pids)" ]; then fail "the app is still running after exit"; fi
rm -f "$tree/bin/hang"

# ---- assertions on the tree ----
# The canary lives in the input log and in the claude stub's e-mail/org: it must be nowhere
# else, and never on stderr.
leaks="$(grep -rIl -e "$canary" -e 'canary-email' -e 'jevcanary_9f3c1b7e' "$tree" 2>/dev/null | grep -v -e "^$usage\$" -e "^$tree/bin/" || true)"
[ -z "$leaks" ] || fail "canary found in: $leaks"
# Binary files too (SQLite, WAL): grep -a.
leaks="$(grep -ral -e "$canary" -e 'canary-email' -e 'jevcanary_9f3c1b7e' "$tree" 2>/dev/null | grep -v -e "^$usage\$" -e "^$tree/bin/" || true)"
[ -z "$leaks" ] || fail "canary found in (binary): $leaks"

dash="$tree/state/jev-router/dashboard"
[ -d "$dash" ] || fail "no state directory $dash"
mode() { stat -c '%a' "$1"; }
[ "$(mode "$dash")" = 700 ] || fail "$dash is mode $(mode "$dash"), want 700"
for f in history.db history.db-wal history.db-shm settings.json; do
  [ -e "$dash/$f" ] || continue
  [ "$(mode "$dash/$f")" = 600 ] || fail "$f is mode $(mode "$dash/$f"), want 600"
done

# D8.10 says the view stores nothing. Measured on WebKitGTK 2.x: even an incognito view makes
# `<data>/dev.jev.flashboard/{WebKitCache,storage,mediakeys}` holding only an 8-byte `salt` and
# an empty `hsts-storage.sqlite`; no page data. So the check is on content: no storage kind
# that holds page data, and nothing in those directories except those two files.
store_dir="$tree/data/dev.jev.flashboard"
if find "$tree" \( -iname 'localstorage' -o -iname 'indexeddb' -o -iname 'cookies*' -o -iname 'serviceworkers' -o -iname 'cacheStorage' \) \
  -not -path "$tree/bin/*" 2>/dev/null | grep -q .; then
  fail "a WebKit page-data store exists in the temp tree: $(find "$tree" \( -iname 'localstorage' -o -iname 'indexeddb' -o -iname 'cookies*' \) | head -n3 | tr '\n' ' ')"
fi
if [ -d "$store_dir" ]; then
  extra="$(find "$store_dir" -type f -not -name salt -not -name hsts-storage.sqlite 2>/dev/null | head -n3 | tr '\n' ' ')"
  [ -z "$extra" ] || fail "unexpected files in the WebKit data directory: $extra"
  hsts="$store_dir/hsts-storage.sqlite"
  if [ -e "$hsts" ] && [ "$(sqlite3 "$hsts" 'select count(*) from HSTSEntries' 2>/dev/null || echo 0)" != 0 ]; then
    fail "the WebKit HSTS database is not empty"
  fi
fi
[ -e "$tree/config/autostart/dev.jev.flashboard.desktop" ] || fail "the autostart entry did not land in the temp config home"

outside_after="$(outside_names | sort)"
[ "$outside_before" = "$outside_after" ] || fail "names changed outside the tree: $(diff <(echo "$outside_before") <(echo "$outside_after") | tr '\n' ' ')"

printf 'smoke: PASS\n'
