# ctxvault Universal Installer for Windows
# Installs standalone native binary directly into %LOCALAPPDATA%\Programs\ctxvault\bin\ctxvault.exe

param(
    [switch]$Fast,
    [switch]$SkipChecksum,
    [string]$Tag,
    [switch]$SkipModels,
    [switch]$SkipRules,
    [switch]$Auth,
    [string]$Agents,
    [string]$Repo = $(if ($env:CTXV_GITHUB_REPO) { $env:CTXV_GITHUB_REPO } elseif ($env:CXTV_GITHUB_REPO) { $env:CXTV_GITHUB_REPO } else { "t3e1er/ctxvault" }),
    [string]$InstallDir = $(if ($env:CTXV_INSTALL_DIR) { $env:CTXV_INSTALL_DIR } elseif ($env:CXTV_INSTALL_DIR) { $env:CXTV_INSTALL_DIR } else { "$env:LOCALAPPDATA\Programs\ctxvault\bin" })
)

$ErrorActionPreference = 'Stop'

if (-not $Tag) {
    Write-Host "[*] Resolving latest release for $Repo..." -ForegroundColor Cyan
    try {
        $Release = Invoke-RestMethod -Uri "https://api.github.com/repos/$Repo/releases/latest" -Headers @{ "User-Agent" = "ctxvault-installer" }
        $Tag = $Release.tag_name
    } catch {
        Write-Error "Failed to query latest release from GitHub API: $_"
        exit 1
    }
} else {
    Write-Host "[*] Installing specified release $Tag for $Repo..." -ForegroundColor Cyan
}

if (-not $Tag) {
    Write-Error "Could not parse release tag name."
    exit 1
}

$Target = "x86_64-pc-windows-msvc"
$ArchiveName = "ctxvault-$Tag-$Target.zip"
$DownloadUrl = "https://github.com/$Repo/releases/download/$Tag/$ArchiveName"

Write-Host "[*] Downloading $DownloadUrl..." -ForegroundColor Cyan
$TempDir = Join-Path $env:TEMP ("ctxvault-install-" + [Guid]::NewGuid().ToString())
New-Item -ItemType Directory -Force -Path $TempDir | Out-Null

$ZipFile = Join-Path $TempDir $ArchiveName

try {
    Invoke-WebRequest -Uri $DownloadUrl -OutFile $ZipFile -UseBasicParsing

    # SHA-256 Digest Validation
    if (-not $Fast -and -not $SkipChecksum) {
        $ChecksumUrls = @(
            "https://github.com/$Repo/releases/download/$Tag/SHA256SUMS.txt",
            "https://github.com/$Repo/releases/download/$Tag/checksums.txt"
        )
        $ChecksumFile = Join-Path $TempDir "checksums.txt"
        $ChecksumFound = $false
        foreach ($Url in $ChecksumUrls) {
            try {
                Invoke-WebRequest -Uri $Url -OutFile $ChecksumFile -UseBasicParsing -ErrorAction SilentlyContinue
                if ((Test-Path $ChecksumFile) -and ((Get-Item $ChecksumFile).Length -gt 0)) {
                    $ChecksumFound = $true
                    break;
                }
            } catch { }
        }

        if ($ChecksumFound) {
            $ExpectedHash = Get-Content $ChecksumFile | Select-String $ArchiveName | ForEach-Object { ($_ -split '\s+')[0] }
            if ($ExpectedHash) {
                Write-Host "[*] Verifying SHA-256 checksum..." -ForegroundColor Cyan
                $ActualHash = (Get-FileHash -Path $ZipFile -Algorithm SHA256).Hash.ToLower()
                if ($ActualHash -ne $ExpectedHash.ToLower()) {
                    Write-Error "Checksum verification failed! Expected: $ExpectedHash, Actual: $ActualHash"
                    exit 1
                }
                Write-Host "[+] Checksum verified ($ActualHash)." -ForegroundColor Green
            }
        } else {
            Write-Host "[*] Checksum file not available, skipping verification." -ForegroundColor DarkGray
        }
    } else {
        Write-Host "[*] Fast install mode: skipping remote checksum verification." -ForegroundColor DarkGray
    }

    Write-Host "[*] Extracting binary..." -ForegroundColor Cyan
    Expand-Archive -Path $ZipFile -DestinationPath $TempDir -Force

    New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null

    $SourceExe = Get-ChildItem -Path $TempDir -Filter "ctxvault.exe" -Recurse | Select-Object -First 1
    if (-not $SourceExe) {
        Write-Error "ctxvault.exe not found in extracted archive."
        exit 1
    }

    # In-place Windows executable retirement (retires locked running binary to allow hot upgrade)
    $DestExe = Join-Path $InstallDir "ctxvault.exe"
    if (Test-Path $DestExe) {
        $Timestamp = [DateTimeOffset]::UtcNow.ToUnixTimeSeconds()
        $RetiredExe = "$DestExe.retired-$Timestamp"
        try {
            Move-Item -Path $DestExe -Destination $RetiredExe -Force -ErrorAction Stop
            Write-Host "[*] Retired existing binary to $RetiredExe" -ForegroundColor Cyan
        } catch {
            # Continue with copy if rename not needed or fails
        }
    }
    # Clean up stale retired binaries older than 24h
    Get-ChildItem -Path $InstallDir -Filter "ctxvault.exe.retired-*" -ErrorAction SilentlyContinue |
        Where-Object { $_.LastWriteTime -lt (Get-Date).AddDays(-1) } |
        Remove-Item -Force -ErrorAction SilentlyContinue

    Copy-Item -Path $SourceExe.FullName -Destination "$InstallDir\ctxvault.exe" -Force
    # Optional alias copy
    Copy-Item -Path $SourceExe.FullName -Destination "$InstallDir\ctxv.exe" -Force -ErrorAction SilentlyContinue

    # GraphView convenience wrapper: allow direct invocation via ctxvault-graphview
    $GraphviewCmd = Join-Path $InstallDir "ctxvault-graphview.cmd"
    Set-Content -Path $GraphviewCmd -Value "@echo off`r`n`"%~dp0ctxvault.exe`" graphview %*" -Force -Encoding ASCII

    # Place updater script beside binary so ctxvault update points to local immutable script
    if ($PSCommandPath -and (Test-Path $PSCommandPath)) {
        Copy-Item -Path $PSCommandPath -Destination (Join-Path $InstallDir "install.ps1") -Force -ErrorAction SilentlyContinue
    }

    # Install the bundled embedding model as a sidecar next to the binary so the
    # embedder resolves it at <exe_dir>\models\<model>\ (no separate download).
    $SourceModels = Join-Path $SourceExe.Directory.FullName "models"
    $DestModels = Join-Path $InstallDir "models"
    if (Test-Path $SourceModels) {
        if (-not $SkipModels -and (-not $Fast -or -not (Test-Path $DestModels))) {
            Write-Host "[*] Installing bundled embedding model (sidecar)..." -ForegroundColor Cyan
            if (Test-Path $DestModels) { Remove-Item -Recurse -Force $DestModels }
            Copy-Item -Recurse -Path $SourceModels -Destination $DestModels -Force
        } else {
            Write-Host "[*] Preserving existing models directory." -ForegroundColor DarkGray
        }
    }

    Write-Host ""
    Write-Host "[+] Successfully installed 'ctxvault.exe' to $InstallDir\ctxvault.exe" -ForegroundColor Green
    Write-Host ""

    # Ensure $InstallDir is in User PATH
    $UserPath = [Environment]::GetEnvironmentVariable("Path", "User")
    if ($UserPath -split ";" -notcontains $InstallDir) {
        $NewPath = if ($UserPath) { "$UserPath;$InstallDir" } else { $InstallDir }
        [Environment]::SetEnvironmentVariable("Path", $NewPath, "User")
        $env:PATH = "$env:PATH;$InstallDir"
        Write-Host "[+] Added $InstallDir to your User PATH environment variable." -ForegroundColor Yellow
        Write-Host "    (Restart your terminal/IDE for PATH changes to take full effect)." -ForegroundColor Yellow
    }

    # Auto-configure installed coding agents
    Write-Host "[*] Auto-configuring coding agents..." -ForegroundColor Cyan
    $InstallArgs = @("install", "-y", "--dir=$InstallDir")
    if ($Fast) { $InstallArgs += "--fast" }
    if ($SkipRules) { $InstallArgs += "--rules=false" }
    if ($Auth) { $InstallArgs += "--auth" }
    if ($Agents) { $InstallArgs += "--agents=$Agents" }
    & "$InstallDir\ctxvault.exe" @InstallArgs

    Write-Host "[>] Run 'ctxvault --version' to verify your installation." -ForegroundColor Cyan
} finally {
    Remove-Item -Path $TempDir -Recurse -Force -ErrorAction SilentlyContinue
}
