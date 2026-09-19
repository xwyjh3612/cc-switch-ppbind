[CmdletBinding()]
param(
  [string]$InstallerPath,
  [switch]$NoLaunch
)

$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
$releaseDir = Join-Path $projectRoot 'release'

if ([string]::IsNullOrWhiteSpace($InstallerPath)) {
  $InstallerPath = Get-ChildItem -LiteralPath $releaseDir -File -Filter '*-setup.exe' |
    Sort-Object LastWriteTime -Descending |
    Select-Object -First 1 -ExpandProperty FullName
}

if ([string]::IsNullOrWhiteSpace($InstallerPath) -or -not (Test-Path -LiteralPath $InstallerPath -PathType Leaf)) {
  throw "Installer not found: $InstallerPath"
}

$InstallerPath = (Resolve-Path -LiteralPath $InstallerPath).Path
$installedExe = Join-Path $env:LOCALAPPDATA 'CC Switch\cc-switch.exe'

Write-Host "Stopping running CC Switch..."
Get-Process -Name 'cc-switch' -ErrorAction SilentlyContinue |
  Stop-Process -Force -ErrorAction SilentlyContinue
Start-Sleep -Milliseconds 800

Write-Host "Installing: $InstallerPath"
$install = Start-Process -FilePath $InstallerPath -ArgumentList '/S' -Wait -PassThru
if ($install.ExitCode -ne 0) {
  throw "Installer failed with exit code $($install.ExitCode)"
}

if (-not $NoLaunch) {
  if (-not (Test-Path -LiteralPath $installedExe -PathType Leaf)) {
    throw "Installed executable not found: $installedExe"
  }

  Write-Host "Starting CC Switch..."
  Start-Process -FilePath $installedExe | Out-Null
}

Write-Host 'CC Switch update completed.'
