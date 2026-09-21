#!/usr/bin/env bash
#
# fetch-model.sh — download the groundcontrol embedding model into a sidecar layout
# that mirrors the upstream Hugging Face repo 1:1 (no renaming).
#
# Downloads the INT8-quantized ONNX weights + tokenizer for
# `jinaai/jina-embeddings-v2-base-code` into the exact upstream paths, so the
# groundcontrol embedder (see `resolve_model_files` in
# crates/groundcontrol-core/src/embedding.rs) finds them as-is:
#
#   <MODELS_DIR>/jina-embeddings-v2-base-code/onnx/model_quantized.onnx
#   <MODELS_DIR>/jina-embeddings-v2-base-code/tokenizer.json
#
# The embedder resolves this via any of (in priority order):
#   1. CTX_MODELS_DIR                         (set to <MODELS_DIR>)
#   2. <exe_dir>/models/<model>/              (production sidecar next to binary)
#   3. <exe_dir>/../models/<model>/           (cargo test/deps: target/<profile>/models/)
#
# Usage:
#   scripts/fetch-model.sh [MODELS_DIR]
#
# MODELS_DIR defaults to $CTX_MODELS_DIR, then ./models (repo-local; gitignored).
# The simplest, build-profile-independent approach is to export
# CTX_MODELS_DIR=<MODELS_DIR> so `cargo test` finds the model.
#
# Idempotent: skips download when files already exist with the expected size.
set -euo pipefail

# Upstream model (Apache-2.0). Revision pinned for reproducibility.
HF_REPO="jinaai/jina-embeddings-v2-base-code"
HF_REVISION="516f4baf13dec4ddddda8631e019b5737c8bc250"
MODEL_DIR_NAME="jina-embeddings-v2-base-code"

# Files mirrored verbatim from the HF repo: "<relative path> <expected bytes> <sha256>".
# model_quantized.onnx = INT8 dynamic quantization (preferred, smallest).
#
# The SHA256 for model_quantized.onnx equals Hugging Face's own LFS content hash
# at the pinned revision (verified against the HF tree API), so a mismatch means
# the bytes changed vs the model we build releases against — CI must fail, not
# silently bake a different model into a signed release. Trust is critical for an
# MCP context server: the download is content-verified here, and the release
# archive is then SHA256-summed as a whole.
FILES=(
  "onnx/model_quantized.onnx 161895621 ed45870251c9f0cf656e78aab0d37a23489066df8a222bb1c8caf8a45f2cb16d"
  "tokenizer.json 2561316 b01c78a902aa4facb2f47f95449f48e2f7bbfea5d2472ee2f6ce92323c6f86e5"
)

MODELS_DIR="${1:-${CTX_MODELS_DIR:-./models}}"
DEST_DIR="${MODELS_DIR}/${MODEL_DIR_NAME}"
BASE_URL="https://huggingface.co/${HF_REPO}/resolve/${HF_REVISION}"

file_size() {
  # Portable byte size (Linux `stat -c`, macOS/BSD `stat -f`).
  stat -c%s "$1" 2>/dev/null || stat -f%z "$1" 2>/dev/null || echo 0
}

sha256_of() {
  # Portable SHA256 (Linux coreutils `sha256sum`, macOS/BSD `shasum -a 256`).
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | awk '{print $1}'
  else
    echo "[ERROR] no sha256sum/shasum available to verify model integrity" >&2
    exit 1
  fi
}

verify() {
  # Fail hard unless the file matches BOTH the expected size and SHA256.
  local path="$1" want_bytes="$2" want_sha="$3" rel="$4"
  local got_bytes got_sha
  got_bytes="$(file_size "$path")"
  if [ "$got_bytes" != "$want_bytes" ]; then
    echo "[ERROR] size mismatch for ${rel}: got ${got_bytes}, expected ${want_bytes}" >&2
    return 1
  fi
  got_sha="$(sha256_of "$path")"
  if [ "$got_sha" != "$want_sha" ]; then
    echo "[ERROR] SHA256 mismatch for ${rel}:" >&2
    echo "          got      ${got_sha}" >&2
    echo "          expected ${want_sha}" >&2
    echo "        Refusing to use an unverified model (supply-chain integrity)." >&2
    return 1
  fi
  return 0
}

download() {
  local rel="$1" want_bytes="$2" want_sha="$3"
  local dest="${DEST_DIR}/${rel}"
  mkdir -p "$(dirname "$dest")"

  # Skip only if the existing file passes full integrity verification.
  if [ -f "$dest" ] && verify "$dest" "$want_bytes" "$want_sha" "$rel" 2>/dev/null; then
    echo "[=] ${rel} already present and verified (sha256 ok), skipping."
    return 0
  fi

  echo "[*] Downloading ${rel} (${want_bytes} bytes)..."
  curl -fSL --retry 3 --retry-delay 2 "${BASE_URL}/${rel}" -o "${dest}.part"
  if ! verify "${dest}.part" "$want_bytes" "$want_sha" "$rel"; then
    rm -f "${dest}.part"
    exit 1
  fi
  mv "${dest}.part" "$dest"
  echo "[+] ${rel} ok (sha256 verified)."
}

mkdir -p "$DEST_DIR"
for entry in "${FILES[@]}"; do
  download $entry
done

# Attribution NOTICE for the redistributed weights (Apache-2.0). Shipped in the
# release archive next to the model so downstream users get the license grant.
cat > "${MODELS_DIR}/NOTICE.md" <<'NOTICE'
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
NOTICE

# Re-verifiable checksums for the sidecar. `sha256sum -c SHA256SUMS.txt` (run from
# this directory) lets any user independently confirm the shipped model.
(
  cd "$MODELS_DIR"
  if command -v sha256sum >/dev/null 2>&1; then
    find "$MODEL_DIR_NAME" -type f ! -name 'SHA256SUMS.txt' -print0 | sort -z | xargs -0 sha256sum > SHA256SUMS.txt
  else
    find "$MODEL_DIR_NAME" -type f ! -name 'SHA256SUMS.txt' -print0 | sort -z | xargs -0 shasum -a 256 > SHA256SUMS.txt
  fi
)

echo ""
echo "[+] Model ready at: ${DEST_DIR} (mirrors the Hugging Face repo layout)"
echo "[+] Wrote ${MODELS_DIR}/NOTICE.md and ${MODELS_DIR}/SHA256SUMS.txt"
echo "    Point groundcontrol at it with:  export CTX_MODELS_DIR=\"$(cd "$MODELS_DIR" && pwd)\""
