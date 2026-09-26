# groundcontrol Local Developer Installer for Windows
# Mirrors install.ps1 behavior using local cargo build artifacts instead of GitHub releases.

param(
    [string]$BuildType = "release",
    [string]$InstallDir = $(if ($env:CTXV_INSTALL_DIR) { $env:CTXV_INSTALL_DIR } elseif ($env:CXTV_INSTALL_DIR) { $env:CXTV_INSTALL_DIR } else { "$env:LOCALAPPDATA\Programs\groundcontrol\bin" }),
    [switch]$SkipPath,
    [switch]$SkipAgents,
    [switch]$ConfigureAgents,
    [string]$Agents
)

$ErrorActionPreference = 'Stop'

$RepoRoot = Split-Path -Parent $PSScriptRoot
$SourceExe = Join-Path $RepoRoot "target\$BuildType\groundcontrol.exe"

if (-not (Test-Path $SourceExe)) {
    Write-Error "Local binary not found at $SourceExe. Please run 'cargo build --workspace --all-features --$BuildType' first."
    exit 1
}

Write-Host "[*] Installing groundcontrol from local $BuildType build..." -ForegroundColor Cyan
Write-Host "    Source:      $SourceExe" -ForegroundColor DarkGray
Write-Host "    Destination: $InstallDir" -ForegroundColor DarkGray

# Ensure destination directory exists
if (-not (Test-Path $InstallDir)) {
    New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
}

$DestExe = Join-Path $InstallDir "groundcontrol.exe"

# In-place Windows executable retirement (retires locked running binary to allow hot upgrade)
if (Test-Path $DestExe) {
    $Timestamp = [DateTimeOffset]::UtcNow.ToUnixTimeSeconds()
    $RetiredExe = "$DestExe.retired-$Timestamp"
    try {
        Move-Item -Path $DestExe -Destination $RetiredExe -Force -ErrorAction Stop
        Write-Host "[*] Retired existing binary to $RetiredExe" -ForegroundColor Cyan
    } catch {
        # Continue with copy if rename fails
    }
}

# Clean up stale retired binaries older than 24h
Get-ChildItem -Path $InstallDir -Filter "groundcontrol.exe.retired-*" -ErrorAction SilentlyContinue |
    Where-Object { $_.LastWriteTime -lt (Get-Date).AddDays(-1) } |
    Remove-Item -Force -ErrorAction SilentlyContinue

# Copy binary & alias
Copy-Item -Path $SourceExe -Destination $DestExe -Force
Copy-Item -Path $SourceExe -Destination (Join-Path $InstallDir "ctxv.exe") -Force -ErrorAction SilentlyContinue

# GraphView convenience wrapper: allow direct invocation via groundcontrol-graphview
$GraphviewCmd = Join-Path $InstallDir "groundcontrol-graphview.cmd"
Set-Content -Path $GraphviewCmd -Value "@echo off`r`n`"%~dp0groundcontrol.exe`" graphview %*" -Force -Encoding ASCII

# Copy local install script for reproducibility
Copy-Item -Path $PSCommandPath -Destination (Join-Path $InstallDir "install-local.ps1") -Force -ErrorAction SilentlyContinue

# Check and copy models sidecar if present
$SourceModels = Join-Path $RepoRoot "models"
if (-not (Test-Path $SourceModels)) {
    $SourceModels = Join-Path (Split-Path -Parent $SourceExe) "models"
}
$DestModels = Join-Path $InstallDir "models"
if (Test-Path $SourceModels) {
    Write-Host "[*] Syncing local models sidecar..." -ForegroundColor Cyan
    if (Test-Path $DestModels) { Remove-Item -Recurse -Force $DestModels }
    Copy-Item -Recurse -Path $SourceModels -Destination $DestModels -Force
}

# Ensure $InstallDir is in User PATH (prepended to take priority)
if (-not $SkipPath) {
    $UserPath = [Environment]::GetEnvironmentVariable("Path", "User")
    $PathParts = if ($UserPath) { $UserPath -split ";" } else { @() }
    if ($PathParts -notcontains $InstallDir) {
        $NewPath = if ($UserPath) { "$InstallDir;$UserPath" } else { $InstallDir }
        [Environment]::SetEnvironmentVariable("Path", $NewPath, "User")
        $env:PATH = "$InstallDir;$env:PATH"
        Write-Host "[+] Added $InstallDir to User PATH." -ForegroundColor Yellow
        Write-Host "    (Restart terminal/IDE for PATH changes to take full effect)." -ForegroundColor Yellow
    } else {
        # Prepend to current session PATH
        $env:PATH = "$InstallDir;" + ($env:PATH -replace [regex]::Escape("$InstallDir;"), "")
        Write-Host "[+] $InstallDir is in User PATH (ensured priority in current session)." -ForegroundColor Green
    }

    # Also update ~/.cargo/bin/groundcontrol.exe if present to avoid stale binary shadowing
    $CargoBinExe = Join-Path $env:USERPROFILE ".cargo\bin\groundcontrol.exe"
    if (Test-Path $CargoBinExe) {
        try {
            $Timestamp = [DateTimeOffset]::UtcNow.ToUnixTimeSeconds()
            Move-Item -Path $CargoBinExe -Destination "$CargoBinExe.retired-$Timestamp" -Force -ErrorAction SilentlyContinue
            Copy-Item -Path $SourceExe -Destination $CargoBinExe -Force
            Write-Host "[+] Also updated cargo binary at $CargoBinExe to prevent shadowing." -ForegroundColor Green
        } catch {
            Write-Host "[!] Could not update $CargoBinExe directly: $_" -ForegroundColor DarkGray
        }
    }
}

# Coding agent configuration (optional, disabled by default to preserve custom configs)
if ($ConfigureAgents -and -not $SkipAgents) {
    Write-Host "[*] Configuring installed coding agents..." -ForegroundColor Cyan
    $InstallArgs = @("install", "-y", "--dir=$InstallDir")
    if ($Agents) { $InstallArgs += "--agents=$Agents" }
    & $DestExe @InstallArgs
}

Write-Host ""
Write-Host "[+] Successfully installed local build to: $DestExe" -ForegroundColor Green
$InstalledVersion = & $DestExe --version
Write-Host "    Installed version: $InstalledVersion" -ForegroundColor Green
Write-Host ""
