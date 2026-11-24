# jev-router

A terminal coding agent that routes each turn to the best available model across Claude Code,
Codex, Gemini, Grok and DeepSeek, using the subscriptions you already have.

- **`jev-code`**: the coding agent. It picks a provider per task and fails over when one hits its limit.
- **`jev-claude`**: Claude Code with a model that switches between Haiku, Sonnet and Opus per turn.

## Requirements

- macOS or Linux (Windows through WSL)
- Rust 1.83+ and git
- A Jev API key
- At least one provider CLI, installed and logged in: `claude`, `codex`, `gemini` or `grok` (Gemini and Grok also accept an API key, `GEMINI_API_KEY` or `XAI_API_KEY`, in your shell or `~/.jev-router.env`; a key set there is used even if you are also logged in)

## Installation

```bash
git clone git@github.com:putraridho/jev-switch.git
cd jev-switch
./scripts/install.sh
```

Then add your key:

```bash
cp .jev-router.env.example ~/.jev-router.env
chmod 600 ~/.jev-router.env
$EDITOR ~/.jev-router.env    # set JEV_API_KEY
```

Check the setup with `jev-code doctor`.

To skip the build, download the newest release instead of running `scripts/install.sh`:

```bash
curl -fsSL https://raw.githubusercontent.com/putraridho/jev-switch/main/scripts/install-release.sh | sh
```

On Windows (experimental, see [Platforms](docs/platforms.md)), in PowerShell:

```powershell
irm https://raw.githubusercontent.com/putraridho/jev-switch/main/scripts/install-release.ps1 | iex
```

## Usage

```bash
cd your-project
jev-code
```

Type a request and press Enter. Type `/` to see all commands, and press `Shift-Tab` to switch
between permission modes (`ask`, `edit`, `plan`, `auto`).

To run a single prompt from a script:

```bash
jev-code -p "explain this codebase"
```

To update to the latest version, run `/update` inside jev-code. A downloaded release downloads the newest release; a build from a checkout updates the checkout.

## Documentation

- [Guide](docs/guide.md): commands, configuration, providers and how routing works
- [Keyboard shortcuts](docs/shortcuts.md)
- [Changelog](CHANGELOG.md)
