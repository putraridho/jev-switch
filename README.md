# jev-router

A terminal coding agent that routes each turn to the best available model across Claude Code,
Codex, Gemini, Grok and DeepSeek, using the subscriptions you already have.

- **`jev-code`**: the coding agent. It picks a provider per task and fails over when one hits its limit.
- **`jev-claude`**: Claude Code with a model that switches between Haiku, Sonnet and Opus per turn.

## Requirements

- macOS, Linux, or Windows (native Windows is experimental; see [Platforms](docs/platforms.md))
- `git` (and Rust 1.83+ if building from source)
- A Jev API key
- At least one provider CLI, installed and logged in (`claude`, `codex`, `agy`, or `grok`), or an API key (`GEMINI_API_KEY`, `XAI_API_KEY`, or `DEEPSEEK_API_KEY`) in your shell or `~/.jev-router.env` (a key set there is used even if you are also logged in)

## Installation

Download the newest prebuilt release (no Rust required):

**macOS / Linux:**

```bash
curl -fsSL https://raw.githubusercontent.com/putraridho/jev-switch/main/scripts/install-release.sh | sh
```

**Windows (experimental, PowerShell):**

```powershell
irm https://raw.githubusercontent.com/putraridho/jev-switch/main/scripts/install-release.ps1 | iex
```

**Or build from source:**

```bash
git clone git@github.com:putraridho/jev-switch.git
cd jev-switch
./scripts/install.sh
```

Then add your Jev API key to `~/.jev-router.env` (on Windows, `%USERPROFILE%\.jev-router.env`):

```bash
printf 'JEV_API_KEY=your-key-here\n' > ~/.jev-router.env
chmod 600 ~/.jev-router.env
```

Check the setup with `jev-code doctor`.

## Usage

```bash
cd your-project
jev-code
```

Type a request and press Enter. Type `/` to see all commands, and press `Shift-Tab` to switch
between permission modes (`ask`, `edit`, `plan`, `auto`). Scroll the chat with the mouse wheel or
`PageUp`/`PageDown` (hold `Shift` to select text or click links, or run with `JEV_INLINE=1` for
your terminal's own scrollback).

To run a single prompt from a script:

```bash
jev-code -p "explain this codebase"
```

To update to the latest version, run `/update` inside jev-code (or `jev-code update` from the shell).
A downloaded release installs the newest GitHub release without rebuilding; a build from a checkout
updates the checkout.

## Documentation

- [Guide](docs/guide.md): commands, configuration, providers and how routing works
- [Keyboard shortcuts](docs/shortcuts.md)
- [Platforms](docs/platforms.md): macOS, Linux and experimental Windows support
- [Changelog](CHANGELOG.md)
