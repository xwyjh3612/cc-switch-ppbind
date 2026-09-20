$ErrorActionPreference = 'Stop'

$projectRoot = Split-Path -Parent $PSScriptRoot
$releaseDir = Join-Path $projectRoot 'release'
$targetDir = Join-Path $projectRoot 'target-fast'

# Keep the fast profile cache separate from the size-optimized formal release cache.
$env:CARGO_TARGET_DIR = $targetDir
$env:CARGO_PROFILE_RELEASE_LTO = 'false'
$env:CARGO_PROFILE_RELEASE_CODEGEN_UNITS = '16'
$env:CARGO_PROFILE_RELEASE_OPT_LEVEL = '1'
$env:CARGO_PROFILE_RELEASE_INCREMENTAL = 'true'
$env:CARGO_PROFILE_RELEASE_STRIP = 'false'

New-Item -ItemType Directory -Path $targetDir -Force | Out-Null
$fastConfigPath = Join-Path $targetDir 'tauri.fast.conf.json'
'{"bundle":{"createUpdaterArtifacts":false}}' |
  Set-Content -LiteralPath $fastConfigPath -Encoding utf8

$tauriArgs = @(
  'tauri', 'build',
  '--bundles', 'nsis',
  '--config', $fastConfigPath
)

& pnpm @tauriArgs
if ($LASTEXITCODE -ne 0) {
  throw "Fast Tauri build failed with exit code $LASTEXITCODE"
}

New-Item -ItemType Directory -Path $releaseDir -Force | Out-Null

$nsisDir = Join-Path $targetDir 'release\bundle\nsis'
if (Test-Path -LiteralPath $nsisDir) {
  Get-ChildItem -LiteralPath $nsisDir -File -Filter '*-setup.exe' |
    Copy-Item -Destination $releaseDir -Force
}

$portableExe = Join-Path $targetDir 'release\cc-switch.exe'
if (Test-Path -LiteralPath $portableExe) {
  Copy-Item -LiteralPath $portableExe `
    -Destination (Join-Path $releaseDir 'cc-switch.exe') -Force
}

Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'install-local.ps1') `
  -Destination (Join-Path $releaseDir 'install-local.ps1') -Force
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'install-local.cmd') `
  -Destination (Join-Path $releaseDir 'update-and-start.cmd') -Force

function Get-Sha256Hex([string]$Path) {
  $sha256 = [System.Security.Cryptography.SHA256]::Create()
  try {
    $stream = [System.IO.File]::OpenRead($Path)
    try {
      return ([System.BitConverter]::ToString($sha256.ComputeHash($stream))).Replace('-', '')
    }
    finally {
      $stream.Dispose()
    }
  }
  finally {
    $sha256.Dispose()
  }
}

$hashLines = Get-ChildItem -LiteralPath $releaseDir -File |
  Where-Object { $_.Name -notin @('SHA256SUMS.txt', 'install-result.log') } |
  Sort-Object Name |
  ForEach-Object {
    $hash = Get-Sha256Hex $_.FullName
    "$hash  $($_.Name)"
  }
$hashLines | Set-Content `
  -LiteralPath (Join-Path $releaseDir 'SHA256SUMS.txt') -Encoding ascii

Write-Host "Fast release artifacts: $releaseDir"
