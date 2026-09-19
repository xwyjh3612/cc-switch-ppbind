$ErrorActionPreference = 'Stop'

$projectRoot = Split-Path -Parent $PSScriptRoot
$releaseDir = Join-Path $projectRoot 'release'
$targetDir = Join-Path $projectRoot 'target'

# Override machine-level CARGO_TARGET_DIR so Cargo work stays inside the project.
$env:CARGO_TARGET_DIR = $targetDir

$tauriArgs = @('tauri', 'build', '--bundles', 'nsis', 'msi')
if (-not $env:TAURI_SIGNING_PRIVATE_KEY) {
  $tauriArgs += @('--config', '{"bundle":{"createUpdaterArtifacts":false}}')
}

& pnpm @tauriArgs
if ($LASTEXITCODE -ne 0) {
  throw "Tauri build failed with exit code $LASTEXITCODE"
}

New-Item -ItemType Directory -Path $releaseDir -Force | Out-Null
$bundleRoot = Join-Path $targetDir 'release\bundle'

$nsisDir = Join-Path $bundleRoot 'nsis'
if (Test-Path -LiteralPath $nsisDir) {
  Get-ChildItem -LiteralPath $nsisDir -File -Filter '*-setup.exe' | Copy-Item -Destination $releaseDir -Force
}

$msiDir = Join-Path $bundleRoot 'msi'
if (Test-Path -LiteralPath $msiDir) {
  Get-ChildItem -LiteralPath $msiDir -File -Filter '*.msi' | Copy-Item -Destination $releaseDir -Force
}

$portableExe = Join-Path $targetDir 'release\cc-switch.exe'
if (Test-Path -LiteralPath $portableExe) {
  Copy-Item -LiteralPath $portableExe -Destination (Join-Path $releaseDir 'cc-switch.exe') -Force
}

Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'install-local.ps1') -Destination (Join-Path $releaseDir 'install-local.ps1') -Force
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'install-local.cmd') -Destination (Join-Path $releaseDir 'update-and-start.cmd') -Force

$hashLines = Get-ChildItem -LiteralPath $releaseDir -File |
  Where-Object { $_.Name -ne 'SHA256SUMS.txt' } |
  Sort-Object Name |
  ForEach-Object {
    $hash = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash
    "$hash  $($_.Name)"
  }
$hashLines | Set-Content -LiteralPath (Join-Path $releaseDir 'SHA256SUMS.txt') -Encoding ascii

Write-Host "Release artifacts: $releaseDir"

