# Flashboard

A small desktop instrument that shows how much of your coding-assistant plans and credits
you have used: Claude, Codex, Antigravity and DeepSeek today. It is a Tauri app (Rust shell, Svelte UI)
in the `jev-router` workspace. Linux only.

## Build and install

```sh
pnpm --dir dashboard install --frozen-lockfile
pnpm --dir dashboard build                                   # the shell embeds dashboard/dist
pnpm --dir dashboard tauri build --no-bundle -- --profile dist --locked
dashboard/scripts/install.sh            # installs into ~/.local/bin
dashboard/scripts/install.sh /some/prefix/bin
dashboard/scripts/install.sh --uninstall [dest]
```

`install.sh` uses no `sudo` and no network. It writes only:

- `<dest>/jev-dashboard` (replaced by rename, so a running instance is never half-replaced);
- `$XDG_DATA_HOME/applications/dev.jev.flashboard.desktop` with an absolute `Exec`;
- hicolor icons `dev.jev.flashboard.png` (16, 32, 48, 64, 128, 256 px) under `$XDG_DATA_HOME/icons`.

`XDG_DATA_HOME` defaults to `~/.local/share`. `--uninstall` removes those files and the
autostart entry the app wrote.

## Autostart

On by default. The app writes `$XDG_CONFIG_HOME/autostart/dev.jev.flashboard.desktop`
(usually `~/.config/autostart/`) that starts it with `--hidden`, so it lives in the tray
until you open it. Turn it off in Settings, Behaviour; the app removes the file.

The autostarted app finds `codex` and `claude` through the `PATH` of your session. If the
usage cards say a tool was not found, check that your session `PATH` contains the
directory that holds it.

## Optional Hyprland window rule

```
windowrule = float on, match:class ^dev\.jev\.flashboard$
windowrule = size 480 720, match:class ^dev\.jev\.flashboard$
```

Adjust to your Hyprland version's rule syntax.

## Privacy

What the app does:

- Reads the gateway's `usage.jsonl` (and `usage.jsonl.1`) in `~/.local/state/jev-router/`,
  keeping only the timestamp and rate-limit fields. Other files there are never opened.
- Runs `codex login status` and `claude auth status` to learn whether you are logged in and
  your plan, and asks `codex` for its usage. It starts the program your `PATH` names, and
  strips secrets from the child's environment. If a mise wrapper is first on `PATH`, the
  app treats a tool as installed only when its real executable is on disk, and sets mise's
  offline variables so the wrapper does not download anything.
- Runs `agy -p /usage` to read Antigravity's remaining quota (every 15 minutes while the
  window is open, hourly otherwise), with `agy`'s auto-updater switched off for that run.
  `agy` talks to Google with your login and writes its own logs and session files.
- Calls `api.deepseek.com` only when a DeepSeek key is configured, over HTTPS, with no
  proxy or redirects.
- Stores history in `<state dir>/dashboard/history.db` and settings in `settings.json`,
  mode 0600 in a 0700 directory. The web view stores nothing.

What it never does: no telemetry, no updater, no fonts or assets fetched at runtime, no
link opening. The UI sees only numbers, closed enums and short labels, never a prompt,
key, e-mail, account id or path.

Honest limits: the directory watch delivers the names of entries created in the gateway's
state directory (they are compared and dropped); the `codex` and `claude` children talk to
their own vendors with your login; the guarantees cover this app, not those programs.

## Troubleshooting

- **Card says a tool was not found:** see the `PATH` note above.
- **Nothing updates:** open Settings, About and diagnostics; each source shows when it
  last succeeded and its last failure kind.
- **No tray icon:** the app notes it in diagnostics and then behaves as a normal window.
- **Window never appears at login:** that is `--hidden`; open it from the tray or the
  launcher entry.

## Tests

`scripts/ci.sh` runs every stage; `scripts/ci.sh --release` also fails on a skipped or
not-run stage. The screen-reader pass is manual: see `docs/screen-reader-script.md`.
