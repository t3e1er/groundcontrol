<#
.SYNOPSIS
    Download the groundcontrol embedding model into a sidecar layout that mirrors the
    upstream Hugging Face repo 1:1 (Windows).

.DESCRIPTION
    Downloads the INT8-quantized ONNX weights + tokenizer for
    jinaai/jina-embeddings-v2-base-code into the exact upstream paths, so the
    groundcontrol embedder (resolve_model_files in
    crates/groundcontrol-core/src/embedding.rs) finds them as-is:

        <ModelsDir>\jina-embeddings-v2-base-code\onnx\model_quantized.onnx
        <ModelsDir>\jina-embeddings-v2-base-code\tokenizer.json

    The embedder resolves this via CTX_MODELS_DIR, or a `models\<model>\` dir next
    to the executable (production sidecar), or `..\models\<model>\` for cargo
    test/deps builds.

    Idempotent: skips download when files already exist with the expected size.

.PARAMETER ModelsDir
    Target models directory. Defaults to $env:CTX_MODELS_DIR, then .\models.

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File scripts\fetch-model.ps1
    powershell -ExecutionPolicy Bypass -File scripts\fetch-model.ps1 -ModelsDir target\debug\models
#>
[CmdletBinding()]
param(
    [string]$ModelsDir = $(if ($env:CTX_MODELS_DIR) { $env:CTX_MODELS_DIR } else { "./models" })
)

$ErrorActionPreference = "Stop"

# Upstream model (Apache-2.0). Revision pinned for reproducibility.
$HfRepo       = "jinaai/jina-embeddings-v2-base-code"
$HfRevision   = "516f4baf13dec4ddddda8631e019b5737c8bc250"
$ModelDirName = "jina-embeddings-v2-base-code"
$BaseUrl      = "https://huggingface.co/$HfRepo/resolve/$HfRevision"
$DestDir      = Join-Path $ModelsDir $ModelDirName

# Files mirrored verbatim from the HF repo (relative path + expected size + sha256).
# model_quantized.onnx = INT8 dynamic quantization (preferred, smallest).
#
# The SHA256 for model_quantized.onnx equals Hugging Face's own LFS content hash at
# the pinned revision (verified against the HF tree API). A mismatch means the bytes
# changed vs the model we build releases against, so the fetch fails rather than
# silently using a different model. Trust is critical for an MCP context server.
$Files = @(
    @{ Rel = "onnx/model_quantized.onnx"; Bytes = 161895621; Sha256 = "ed45870251c9f0cf656e78aab0d37a23489066df8a222bb1c8caf8a45f2cb16d" },
    @{ Rel = "tokenizer.json";            Bytes = 2561316;   Sha256 = "b01c78a902aa4facb2f47f95449f48e2f7bbfea5d2472ee2f6ce92323c6f86e5" }
)

function Get-FileSize([string]$Path) {
    if (Test-Path $Path) { return (Get-Item $Path).Length }
    return 0
}

function Test-Integrity([string]$Path, [long]$Bytes, [string]$Sha256, [string]$Rel) {
    if ((Get-FileSize $Path) -ne $Bytes) { return $false }
    $got = (Get-FileHash -Algorithm SHA256 -Path $Path).Hash.ToLower()
    return ($got -eq $Sha256.ToLower())
}

New-Item -ItemType Directory -Force -Path $DestDir | Out-Null

foreach ($f in $Files) {
    $dest = Join-Path $DestDir ($f.Rel -replace '/', '\')
    New-Item -ItemType Directory -Force -Path (Split-Path -Parent $dest) | Out-Null

    # Skip only if the existing file passes full integrity verification.
    if ((Test-Path $dest) -and (Test-Integrity $dest $f.Bytes $f.Sha256 $f.Rel)) {
        Write-Host "[=] $($f.Rel) already present and verified (sha256 ok), skipping."
        continue
    }

    $url  = "$BaseUrl/$($f.Rel)"
    $part = "$dest.part"
    Write-Host "[*] Downloading $($f.Rel) ($($f.Bytes) bytes)..."
    Invoke-WebRequest -Uri $url -OutFile $part -MaximumRedirection 5
    if (-not (Test-Integrity $part $f.Bytes $f.Sha256 $f.Rel)) {
        $got = (Get-FileHash -Algorithm SHA256 -Path $part).Hash.ToLower()
        Remove-Item -Force $part
        Write-Error "integrity check failed for $($f.Rel): sha256 got $got, expected $($f.Sha256). Refusing to use an unverified model."
        exit 1
    }
    Move-Item -Force $part $dest
    Write-Host "[+] $($f.Rel) ok (sha256 verified)."
}

# Attribution NOTICE for the redistributed weights (Apache-2.0).
$Notice = @'
# Bundled Embedding Model — Attribution & License

This directory contains a third-party pre-trained model redistributed unmodified.

- Model: jina-embeddings-v2-base-code
- Upstream: https://huggingface.co/jinaai/jina-embeddings-v2-base-code
- Pinned revision: 516f4baf13dec4ddddda8631e019b5737c8bc250
- License: Apache License 2.0 (https://www.apache.org/licenses/LICENSE-2.0)
- Copyright (c) Jina AI GmbH.

Bundled files (INT8 dynamic quantization, mirrored verbatim from upstream):
  jina-embeddings-v2-base-code/onnx/model_quantized.onnx
  jina-embeddings-v2-base-code/tokenizer.json

Integrity: see SHA256SUMS.txt in this directory. The ONNX hash matches Hugging
Face's own LFS content hash at the pinned revision.
'@
Set-Content -Path (Join-Path $ModelsDir "NOTICE.md") -Value $Notice -NoNewline

# Re-verifiable checksums for the sidecar, in `sha256sum -c` compatible format
# (lowercase hash, two spaces, forward-slash relative path).
$sumsPath = Join-Path $ModelsDir "SHA256SUMS.txt"
$modelsFull = (Resolve-Path $ModelsDir).Path.TrimEnd('\', '/')
$lines = Get-ChildItem -Path $DestDir -Recurse -File |
    Where-Object { $_.Name -ne "SHA256SUMS.txt" } |
    ForEach-Object {
        $rel = $_.FullName.Substring($modelsFull.Length).TrimStart('\', '/') -replace '\\', '/'
        $hash = (Get-FileHash -Algorithm SHA256 -Path $_.FullName).Hash.ToLower()
        "$hash  $rel"
    } | Sort-Object
Set-Content -Path $sumsPath -Value ($lines -join "`n")

$resolved = (Resolve-Path $ModelsDir).Path
Write-Host ""
Write-Host "[+] Model ready at: $DestDir (mirrors the Hugging Face repo layout)"
Write-Host "[+] Wrote NOTICE.md and SHA256SUMS.txt in $ModelsDir"
Write-Host "    Point groundcontrol at it with:  `$env:CTX_MODELS_DIR = `"$resolved`""
