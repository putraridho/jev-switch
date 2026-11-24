#!/bin/sh
# Starts `tauri dev` for Flashboard (docs/dashboard/TASKS.md rule 11, DESIGN 8.8).
#
#   dev.sh stub   the default (`pnpm dev`): stub `codex`/`claude` first on PATH and a
#                 temporary HOME, so no real CLI, login or state directory is touched.
#   dev.sh real   (`pnpm dev:real`): the real environment. For the lead only, after T04's
#                 manual check has confirmed the DESIGN 8.6 assumptions on this machine.
#
# Both use the identifier dev.jev.flashboard.dev, so a development run never collides with
# the single-instance name of an installed Flashboard.
set -eu

mode="${1:-stub}"
here="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
dashboard="$(dirname -- "$here")"
config='{"identifier":"dev.jev.flashboard.dev"}'

cd "$dashboard"

if [ "$mode" = "real" ]; then
  exec pnpm exec tauri dev --config "$config"
fi

if [ "$mode" != "stub" ]; then
  echo "usage: dev.sh [stub|real]" >&2
  exit 2
fi

# Never fall through to a real CLI: both stubs must be there (they are wrappers on this
# machine that can install or update, DESIGN 3.1).
for tool in codex claude; do
  if [ ! -x "$dashboard/e2e/smoke/stubs/$tool" ]; then
    echo "dev.sh: missing stub e2e/smoke/stubs/$tool; refusing to start against the real PATH" >&2
    exit 1
  fi
done

# cargo and rustup keep their real homes; everything else gets the temporary one.
CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}"
RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}"
PNPM_HOME="${PNPM_HOME:-$HOME/.local/share/pnpm}"
export CARGO_HOME RUSTUP_HOME PNPM_HOME

temp_home="$(mktemp -d "${TMPDIR:-/tmp}/flashboard-dev.XXXXXX")"
chmod 700 "$temp_home"
trap 'rm -rf "$temp_home"' EXIT INT TERM

HOME="$temp_home"
XDG_CONFIG_HOME="$temp_home/.config"
XDG_DATA_HOME="$temp_home/.local/share"
XDG_STATE_HOME="$temp_home/.local/state"
XDG_CACHE_HOME="$temp_home/.cache"
PATH="$dashboard/e2e/smoke/stubs:$PATH"
export HOME XDG_CONFIG_HOME XDG_DATA_HOME XDG_STATE_HOME XDG_CACHE_HOME PATH
unset CODEX_HOME CLAUDE_CONFIG_DIR JEV_USAGE_LOG JEV_ROUTER_ENV DEEPSEEK_API_KEY MISE_DATA_DIR MISE_CONFIG_DIR

pnpm exec tauri dev --config "$config"
