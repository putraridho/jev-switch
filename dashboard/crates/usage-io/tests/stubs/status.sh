#!/bin/sh
if [ "$1" = login ]; then
    printf 'Logged in using ChatGPT\n' >&2
elif [ "$1" = auth ]; then
    printf '{"loggedIn":true,"planTier":"max_5x"}\n'
else
    printf '%s|%s|%s|%s\n' "${JEV_API_KEY-unset}" "$MISE_OFFLINE" "$TERM" "$PWD"
fi
