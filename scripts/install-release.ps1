# Install the newest jev-code release on Windows (no Rust needed): download, check the SHA-256,
# install.
#
#   irm https://raw.githubusercontent.com/putraridho/jev-switch/main/scripts/install-release.ps1 | iex
#   .\install-release.ps1 [-Dest <folder>]       Dest defaults to %USERPROFILE%\.local\bin
#
# Afterwards `jev-code update` (or /update inside jev-code) keeps it current.
param([string]$Dest = (Join-Path $env:USERPROFILE '.local\bin'))
$ErrorActionPreference = 'Stop'
$repo = 'putraridho/jev-switch'

if ($env:PROCESSOR_ARCHITECTURE -ne 'AMD64') {
    throw "no prebuilt jev-code for Windows $($env:PROCESSOR_ARCHITECTURE); only x64 is built"
}
$latest = Invoke-RestMethod -Uri "https://api.github.com/repos/$repo/releases/latest" -Headers @{ 'User-Agent' = 'jev-code-installer' }
$tag = $latest.tag_name
if ($tag -notmatch '^v[0-9][0-9A-Za-z.+-]*$') { throw "could not find the newest release of $repo" }
$name = "jev-code-$tag-windows-x86_64"
$base = "https://github.com/$repo/releases/download/$tag"
$work = Join-Path ([System.IO.Path]::GetTempPath()) ("jev-install-" + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $work | Out-Null
try {
    Write-Host "downloading jev-code $tag for windows-x86_64"
    Invoke-WebRequest -Uri "$base/$name.zip" -OutFile (Join-Path $work "$name.zip") -UseBasicParsing
    Invoke-WebRequest -Uri "$base/$name.zip.sha256" -OutFile (Join-Path $work "$name.zip.sha256") -UseBasicParsing
    $want = ((Get-Content (Join-Path $work "$name.zip.sha256") -Raw).Trim() -split '\s+')[0].ToLowerInvariant()
    $got = (Get-FileHash -Algorithm SHA256 (Join-Path $work "$name.zip")).Hash.ToLowerInvariant()
    if ($want -ne $got) { throw "checksum mismatch (expected $want, got $got); nothing was installed" }
    Expand-Archive -Path (Join-Path $work "$name.zip") -DestinationPath $work
    New-Item -ItemType Directory -Force -Path $Dest | Out-Null
    $files = 'jev-ask.exe', 'jev-delegate-mcp.exe', 'jev-chat.exe', 'jev-gateway.exe', 'jev-claude.ps1', 'jev-claude.cmd', 'jev-code.exe'
    foreach ($f in $files) {
        $target = Join-Path $Dest $f
        $staged = Join-Path $Dest ".$f.new"
        $aside = Join-Path $Dest ".$f.old"
        Copy-Item (Join-Path $work "$name\$f") $staged -Force
        # A running program's file can be renamed but not overwritten.
        if (Test-Path $aside) { Remove-Item $aside -Force -ErrorAction SilentlyContinue }
        if (Test-Path $target) { Move-Item $target $aside -Force }
        Move-Item $staged $target -Force
    }
    $dataRoot = if ($env:XDG_DATA_HOME) { $env:XDG_DATA_HOME } else { Join-Path $env:USERPROFILE '.local\share' }
    $mods = Join-Path $dataRoot 'jev-router\mods'
    New-Item -ItemType Directory -Force -Path $mods | Out-Null
    $mod = Join-Path $mods 'jev-orchestrate'
    if (Test-Path $mod) { Remove-Item $mod -Recurse -Force }
    Copy-Item (Join-Path $work "$name\mods\jev-orchestrate") $mod -Recurse
    Stop-Process -Name 'jev-gateway' -ErrorAction SilentlyContinue
    Write-Host "installed jev-code $tag in $Dest, and the jev-orchestrate mod in $mods"
    if (($env:PATH -split ';') -notcontains $Dest) {
        Write-Host "note: $Dest is not on your PATH; add it to run jev-code from anywhere"
    }
    Write-Host "next: add your JEV_API_KEY to $env:USERPROFILE\.jev-router.env, then run: jev-code doctor"
} finally {
    Remove-Item $work -Recurse -Force -ErrorAction SilentlyContinue
}
