# jev-claude for Windows: the normal Claude Code interface with a "Jev Code" model that routes
# each turn (Sonnet, or Opus for hard and security work) through jev-gateway. Starts the gateway
# if it isn't running. Plain `claude` is unaffected. The same as scripts/jev-claude (bash).
$ErrorActionPreference = 'Stop'

$addr = if ($env:JEV_GATEWAY_ADDR) { $env:JEV_GATEWAY_ADDR } else { '127.0.0.1:18780' }
$home_dir = if ($env:HOME) { $env:HOME } else { $env:USERPROFILE }
$stateRoot = if ($env:XDG_STATE_HOME) { $env:XDG_STATE_HOME } else { Join-Path $home_dir '.local\state' }
$state = Join-Path $stateRoot 'jev-router'
New-Item -ItemType Directory -Force -Path $state | Out-Null

function Test-GatewayUp {
    try {
        Invoke-WebRequest -Uri "http://$addr/healthz" -UseBasicParsing -TimeoutSec 1 | Out-Null
        return $true
    } catch {
        return $false
    }
}

if (-not (Test-GatewayUp)) {
    $env:JEV_GATEWAY_ADDR = $addr
    Start-Process -FilePath 'jev-gateway' -WindowStyle Hidden `
        -RedirectStandardOutput (Join-Path $state 'gateway.out') `
        -RedirectStandardError (Join-Path $state 'gateway.err')
    for ($i = 0; $i -lt 30; $i++) {
        if (Test-GatewayUp) { break }
        Start-Sleep -Milliseconds 100
    }
    if (-not (Test-GatewayUp)) {
        [Console]::Error.WriteLine("jev-claude: jev-gateway didn't start; see $state\gateway.out and gateway.err")
        exit 1
    }
}

$env:ANTHROPIC_BASE_URL = "http://$addr"
$env:CLAUDE_CODE_GATEWAY_HINT_HEADERS = '1'
$env:ANTHROPIC_CUSTOM_MODEL_OPTION = 'jev-code[1m]'
$env:ANTHROPIC_CUSTOM_MODEL_OPTION_NAME = 'Jev Code'
$env:ANTHROPIC_CUSTOM_MODEL_OPTION_DESCRIPTION = 'Routes each turn: Sonnet, or Opus for hard and security work'

# The delegate tool (MCP server `jev`), per project folder; JEV_DELEGATE=off skips it. Written with
# `=` so a prompt typed after `--mcp-config <file>` is not read as another config file.
$mcp = @()
$delegate = Get-Command jev-delegate-mcp -ErrorAction SilentlyContinue
if (($env:JEV_DELEGATE -ne 'off') -and $delegate) {
    $sha = [System.Security.Cryptography.SHA1]::Create()
    $bytes = $sha.ComputeHash([System.Text.Encoding]::UTF8.GetBytes($PWD.Path))
    $key = (($bytes | ForEach-Object { $_.ToString('x2') }) -join '').Substring(0, 12)
    $cfg = Join-Path $state "mcp-delegate-claude-$key.json"
    $config = @{ mcpServers = @{ jev = @{ command = $delegate.Source; args = @('--dir', $PWD.Path) } } }
    $config | ConvertTo-Json -Depth 5 | Set-Content -Path $cfg -Encoding UTF8
    $mcp = @("--mcp-config=$cfg")
}

foreach ($a in $args) {
    if ($a -eq '--model' -or $a -like '--model=*') {
        & claude @mcp @args
        exit $LASTEXITCODE
    }
}

# Orchestrate mode (off by default; /orchestrate on turns it on for a session).
$dataRoot = if ($env:XDG_DATA_HOME) { $env:XDG_DATA_HOME } else { Join-Path $home_dir '.local\share' }
$mod = Join-Path $dataRoot 'jev-router\mods\jev-orchestrate'
$plugin = @()
if (Test-Path $mod -PathType Container) { $plugin = @('--plugin-dir', $mod) }
& claude @mcp --model 'jev-code[1m]' @plugin @args
exit $LASTEXITCODE
