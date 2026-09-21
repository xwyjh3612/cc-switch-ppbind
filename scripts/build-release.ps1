$ErrorActionPreference = 'Stop'

$projectRoot = Split-Path -Parent $PSScriptRoot
$releaseDir = Join-Path $projectRoot 'release'
$targetDir = if ([string]::IsNullOrWhiteSpace($env:CARGO_TARGET_DIR)) {
  Join-Path $projectRoot 'target'
} else {
  [IO.Path]::GetFullPath($env:CARGO_TARGET_DIR)
}

# Respect an external CARGO_TARGET_DIR so build caches can live on a larger drive.
$env:CARGO_TARGET_DIR = $targetDir

$releaseConfigPath = Join-Path $targetDir 'tauri.release.conf.json'
'{"bundle":{"createUpdaterArtifacts":false}}' |
  Set-Content -LiteralPath $releaseConfigPath -Encoding utf8

$tauriArgs = @('tauri', 'build', '--bundles', 'nsis', 'msi')
if (-not $env:TAURI_SIGNING_PRIVATE_KEY) {
  $tauriArgs += @('--config', $releaseConfigPath)
}

& pnpm @tauriArgs
if ($LASTEXITCODE -ne 0) {
  throw "Tauri build failed with exit code $LASTEXITCODE"
}

New-Item -ItemType Directory -Path $releaseDir -Force | Out-Null
$bundleRoot = Join-Path $targetDir 'release\bundle'

$nsisDir = Join-Path $bundleRoot 'nsis'
if (Test-Path -LiteralPath $nsisDir) {
  Get-ChildItem -LiteralPath $nsisDir -File -Filter 'PPBind_*-setup.exe' | Copy-Item -Destination $releaseDir -Force
}

$msiDir = Join-Path $bundleRoot 'msi'
if (Test-Path -LiteralPath $msiDir) {
  Get-ChildItem -LiteralPath $msiDir -File -Filter 'PPBind_*.msi' | Copy-Item -Destination $releaseDir -Force
}

$portableExe = Join-Path $targetDir 'release\ppbind.exe'
if (Test-Path -LiteralPath $portableExe) {
  Copy-Item -LiteralPath $portableExe -Destination (Join-Path $releaseDir 'ppbind.exe') -Force
}

Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'install-local.ps1') -Destination (Join-Path $releaseDir 'install-local.ps1') -Force
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'install-local.cmd') -Destination (Join-Path $releaseDir 'update-and-start.cmd') -Force

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
$hashLines | Set-Content -LiteralPath (Join-Path $releaseDir 'SHA256SUMS.txt') -Encoding ascii

Write-Host "Release artifacts: $releaseDir"

