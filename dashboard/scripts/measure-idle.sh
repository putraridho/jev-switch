#!/usr/bin/env bash
# Idle resource budgets (docs/dashboard/DESIGN.md 11.3): launches the release binary hidden
# under dbus-run-session against a temp tree with stub CLIs and measures the fetch-free
# window from 60 s to 660 s after launch, summed over the whole process tree and all threads.
#
#   dashboard/scripts/measure-idle.sh [--floor <empty-scaffold-binary>] [app-binary]
#
# Budgets: CPU time < 0.5 s, wakeups (voluntary context switches) < 2 per minute, RSS <= 60 MB,
# binary <= 15 MB. The floor binary is T00's empty scaffold built the same way; when given it
# is measured identically and printed beside the app's figures (no budget). A budget is never
# met by changing how it is measured: a miss prints FAIL and exits 1.
# MEASURE_FROM / MEASURE_TO (seconds) shorten the window for a trial run; any other values
# than 60 / 660 print "NOT THE D11.3 WINDOW" and never count as a pass.
set -u

here="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
dashboard="$(cd -- "$here/.." && pwd)"
floor=""
if [ "${1:-}" = "--floor" ]; then floor="${2:?--floor needs a binary}"; shift 2; fi
bin="${1:-$dashboard/../target/dist/jev-dashboard}"
from="${MEASURE_FROM:-60}"
to="${MEASURE_TO:-660}"

bin="$(cd -- "$(dirname -- "$bin")" 2>/dev/null && pwd)/$(basename -- "$bin")"
[ -x "$bin" ] || { echo "measure-idle: binary not found: $bin" >&2; exit 1; }
command -v dbus-run-session >/dev/null 2>&1 || { echo "measure-idle: NOT RUN: dbus-run-session is missing" >&2; exit 3; }
dbus_daemon="$(command -v dbus-daemon)" || { echo "measure-idle: NOT RUN: dbus-daemon is missing" >&2; exit 3; }
real_runtime="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"
wayland_name="${WAYLAND_DISPLAY:-}"
[ -n "$wayland_name" ] || { echo "measure-idle: NOT RUN: WAYLAND_DISPLAY is not set" >&2; exit 3; }
case "$wayland_name" in /*) socket="$wayland_name" ;; *) socket="$real_runtime/$wayland_name" ;; esac
[ -S "$socket" ] || { echo "measure-idle: NOT RUN: no Wayland socket at $socket" >&2; exit 3; }

ticks_per_s="$(getconf CLK_TCK)"

# descendants <pid>: the pid and every descendant, one per line.
descendants() {
  local pid="$1" child
  echo "$pid"
  for child in $(pgrep -P "$pid" 2>/dev/null); do descendants "$child"; done
}

# tree_figures <root-pid>: "ticks wakeups rss_kb procs" summed over the tree and all threads.
tree_figures() {
  local pid ticks=0 wake=0 rss=0 procs=0 t v r
  for pid in $(descendants "$1"); do
    [ -d "/proc/$pid" ] || continue
    procs=$((procs + 1))
    for task in /proc/"$pid"/task/*; do
      [ -r "$task/stat" ] || continue
      # utime and stime are fields 14 and 15 after the ") " that ends the command name.
      t="$(sed 's/^.*) //' "$task/stat" | awk '{print $12 + $13}')"
      ticks=$((ticks + ${t:-0}))
      v="$(awk '/^voluntary_ctxt_switches/ {print $2}' "$task/status" 2>/dev/null)"
      wake=$((wake + ${v:-0}))
    done
    r="$(awk '/^VmRSS/ {print $2}' "/proc/$pid/status" 2>/dev/null)"
    rss=$((rss + ${r:-0}))
  done
  echo "$ticks $wake $rss $procs"
}

# measure <binary> <label>: prints "<label> cpu_s wakeups_per_min rss_mb procs_at_end".
measure() {
  local binary="$1" label="$2" tree session app
  tree="$(mktemp -d)"
  chmod 700 "$tree"
  mkdir -p "$tree"/{home,config,data,cache,state/jev-router,runtime,bin}
  chmod 700 "$tree"/{home,config,data,cache,state,state/jev-router,runtime}
  cp "$dashboard/e2e/smoke/stubs/codex" "$dashboard/e2e/smoke/stubs/claude" "$tree/bin/"
  chmod 755 "$tree"/bin/*
  cp "$dashboard/fixtures/claude-gateway/sample.jsonl" "$tree/state/jev-router/usage.jsonl"
  chmod 600 "$tree/state/jev-router/usage.jsonl"
  env -i PATH="$tree/bin" HOME="$tree/home" \
    XDG_CONFIG_HOME="$tree/config" XDG_DATA_HOME="$tree/data" XDG_CACHE_HOME="$tree/cache" \
    XDG_STATE_HOME="$tree/state" XDG_RUNTIME_DIR="$tree/runtime" \
    WAYLAND_DISPLAY="$socket" GDK_BACKEND=wayland \
    /usr/bin/dbus-run-session --dbus-daemon="$dbus_daemon" --config-file="$dashboard/e2e/smoke/session-bus.conf" \
    -- "$binary" --hidden >/dev/null 2>"$tree/stderr" &
  session=$!
  sleep "$from"
  # Anchored: the dbus-run-session command line also contains the binary path.
  app="$(pgrep -f -- "^$binary( |\$)" | head -n1)"
  if [ -z "$app" ]; then
    echo "measure-idle: $label exited before the window opened; stderr:" >&2
    cat "$tree/stderr" >&2
    kill "$session" 2>/dev/null; rm -rf -- "$tree"
    return 1
  fi
  read -r t0 w0 _ _ <<<"$(tree_figures "$app")"
  sleep $((to - from))
  if ! kill -0 "$app" 2>/dev/null; then
    echo "measure-idle: $label died during the window" >&2
    rm -rf -- "$tree"
    return 1
  fi
  read -r t1 w1 rss procs <<<"$(tree_figures "$app")"
  kill -TERM "$app" 2>/dev/null
  wait "$session" 2>/dev/null
  rm -rf -- "$tree"
  awk -v l="$label" -v t0="$t0" -v t1="$t1" -v w0="$w0" -v w1="$w1" -v rss="$rss" \
    -v procs="$procs" -v hz="$ticks_per_s" -v secs=$((to - from)) \
    'BEGIN { printf "%s %.2f %.2f %.1f %d\n", l, (t1 - t0) / hz, (w1 - w0) * 60 / secs, rss / 1024, procs }'
}

app_line="$(measure "$bin" app)" || exit 1
floor_line=""
if [ -n "$floor" ]; then floor_line="$(measure "$floor" floor)" || exit 1; fi

size=$(wc -c <"$bin")
printf '\n%-8s %10s %14s %9s %6s\n' "" "cpu s" "wakeups/min" "rss MB" "procs"
printf '%-8s %10s %14s %9s %6s\n' $app_line
if [ -n "$floor_line" ]; then
  printf '%-8s %10s %14s %9s %6s\n' $floor_line
else
  echo "floor: not measured (pass --floor <binary of T00's empty scaffold>)"
fi
printf 'binary %d bytes (budget 15728640)\n' "$size"

read -r _ cpu wake rss _ <<<"$app_line"
status=0
awk -v c="$cpu" 'BEGIN { exit !(c < 0.5) }' || { echo "FAIL: CPU $cpu s >= 0.5 s"; status=1; }
awk -v w="$wake" 'BEGIN { exit !(w < 2) }' || { echo "FAIL: wakeups $wake/min >= 2/min"; status=1; }
awk -v r="$rss" 'BEGIN { exit !(r <= 60) }' || { echo "FAIL: RSS $rss MB > 60 MB"; status=1; }
[ "$size" -le 15728640 ] || { echo "FAIL: binary over 15 MB"; status=1; }
if [ "$from" != 60 ] || [ "$to" != 660 ]; then
  echo "NOT THE D11.3 WINDOW (${from}s to ${to}s): this run is not a pass"
  [ "$status" -eq 0 ] && status=3
fi
[ "$status" -eq 0 ] && echo "idle budgets: PASS"
exit "$status"
