[CmdletBinding()]
param(
  [switch]$NoLaunch
)

$ErrorActionPreference = 'Stop'

$projectRoot = Split-Path -Parent $PSScriptRoot
$targetDir = Join-Path $projectRoot 'target-debug-fast'

# Debug builds favour compile speed. Do not use this profile for releases.
$env:CARGO_TARGET_DIR = $targetDir
$env:CARGO_PROFILE_DEV_DEBUG = '0'
$env:CARGO_PROFILE_DEV_INCREMENTAL = 'true'
$env:CARGO_PROFILE_DEV_CODEGEN_UNITS = '256'
$env:CARGO_PROFILE_DEV_LTO = 'false'
$env:CARGO_PROFILE_DEV_OPT_LEVEL = '0'

New-Item -ItemType Directory -Path $targetDir -Force | Out-Null
$devConfigPath = Join-Path $targetDir 'tauri.dev-fast.conf.json'
'{"bundle":{"createUpdaterArtifacts":false}}' |
  Set-Content -LiteralPath $devConfigPath -Encoding utf8

& pnpm tauri build --debug --no-bundle --config $devConfigPath
if ($LASTEXITCODE -ne 0) {
  throw "Fast debug build failed with exit code $LASTEXITCODE"
}

$builtExe = Join-Path $targetDir 'debug\ppbind.exe'
if (-not (Test-Path -LiteralPath $builtExe -PathType Leaf)) {
  throw "Debug executable not found: $builtExe"
}

$installedDir = Join-Path $env:LOCALAPPDATA 'PPBind'
$installedExe = Join-Path $installedDir 'ppbind.exe'
New-Item -ItemType Directory -Path $installedDir -Force | Out-Null

Write-Host 'Stopping running PPBind...'
Get-Process -Name 'ppbind' -ErrorAction SilentlyContinue |
  Stop-Process -Force -ErrorAction SilentlyContinue
Start-Sleep -Milliseconds 600

Write-Host "Updating debug executable: $installedExe"
Copy-Item -LiteralPath $builtExe -Destination $installedExe -Force

if (-not $NoLaunch) {
  Write-Host 'Starting PPBind...'
  Start-Process -FilePath $installedExe | Out-Null
}

Write-Host 'Fast debug update completed.'
