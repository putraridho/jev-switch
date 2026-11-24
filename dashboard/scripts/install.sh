#!/bin/sh
set -u

fail() {
    printf '%s\n' "Flashboard: $*" >&2
    exit 1
}

mode=install
if [ "${1-}" = "--uninstall" ]; then
    mode=uninstall
    shift
fi
[ "$#" -le 1 ] || fail 'usage: install.sh [--uninstall] [dest]'
if [ "$#" -eq 1 ]; then
    dest=$1
else
    [ -n "${HOME-}" ] || fail 'HOME is not set; specify a destination'
    dest=$HOME/.local/bin
fi
[ -n "$dest" ] || fail 'destination must not be empty'

if [ "$mode" = uninstall ]; then
    if [ -n "${XDG_CONFIG_HOME-}" ]; then
        config=$XDG_CONFIG_HOME
    else
        [ -n "${HOME-}" ] || fail 'HOME is not set and XDG_CONFIG_HOME is unset'
        config=$HOME/.config
    fi
    status=0
    rm -f -- "$dest/jev-dashboard" || { printf '%s\n' 'Flashboard: could not remove the installed binary' >&2; status=1; }
    rm -f -- "$config/autostart/dev.jev.flashboard.desktop" || { printf '%s\n' 'Flashboard: could not remove the autostart entry' >&2; status=1; }
    exit "$status"
fi

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd) || fail 'cannot locate the script directory'
source=$script_dir/../../target/dist/jev-dashboard
[ -f "$source" ] || fail "built binary not found: $source"

created_dest=0
if [ ! -d "$dest" ]; then
    mkdir -p -- "$dest" || fail "cannot create destination: $dest"
    created_dest=1
fi
stage=$(mktemp -d "$dest/.jev-dashboard-install.XXXXXXXX") || {
    [ "$created_dest" -eq 0 ] || rmdir -- "$dest" 2>/dev/null || :
    fail "cannot create temporary directory in $dest"
}
cleanup() {
    if [ -e "$stage/old" ] || [ -L "$stage/old" ]; then
        if [ ! -e "$dest/jev-dashboard" ] && [ ! -L "$dest/jev-dashboard" ]; then
            mv -- "$stage/old" "$dest/jev-dashboard" || printf '%s\n' "Flashboard: old binary remains at $stage/old" >&2
        fi
    fi
    rm -f -- "$stage/new"
    rmdir -- "$stage" 2>/dev/null || :
    [ "$created_dest" -eq 0 ] || rmdir -- "$dest" 2>/dev/null || :
}
trap cleanup 0
trap 'exit 1' HUP INT TERM
cp -- "$source" "$stage/new" || fail 'cannot copy the built binary'
[ ! -d "$dest/jev-dashboard" ] || fail 'destination binary is a directory'
if [ -e "$dest/jev-dashboard" ] || [ -L "$dest/jev-dashboard" ]; then
    mv -- "$dest/jev-dashboard" "$stage/old" || fail 'cannot move the old binary aside'
    if ! mv -- "$stage/new" "$dest/jev-dashboard"; then
        mv -- "$stage/old" "$dest/jev-dashboard" || fail "cannot restore old binary; it remains at $stage/old"
        fail 'cannot install the new binary; the old binary was restored'
    fi
    rm -f -- "$stage/old" || fail "installed new binary but cannot remove old binary at $stage/old"
else
    mv -- "$stage/new" "$dest/jev-dashboard" || fail 'cannot install the new binary'
fi
