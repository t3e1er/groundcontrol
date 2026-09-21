#!/usr/bin/env bash
set -e

# groundcontrol Universal Installer for macOS and Linux
# Installs standalone native binary directly into ~/.local/bin/groundcontrol

REPO="${CTXV_GITHUB_REPO:-${CXTV_GITHUB_REPO:-t3e1er/groundcontrol}}"
INSTALL_DIR="${CTXV_INSTALL_DIR:-${CXTV_INSTALL_DIR:-$HOME/.local/bin}}"

FAST=false
SKIP_CHECKSUM=false
TAG=""
SKIP_MODELS=false
SKIP_RULES=false
AUTH=false
AGENTS=""

while [ $# -gt 0 ]; do
    case "$1" in
        --fast)
            FAST=true
            shift
            ;;
        --skip-checksum)
            SKIP_CHECKSUM=true
            shift
            ;;
        --tag)
            TAG="$2"
            shift 2
            ;;
        --skip-models)
            SKIP_MODELS=true
            shift
            ;;
        --skip-rules)
            SKIP_RULES=true
            shift
            ;;
        --auth)
            AUTH=true
            shift
            ;;
        --agents)
            AGENTS="$2"
            shift 2
            ;;
        *)
            shift
            ;;
    esac
done

# 1. Detect architecture & OS
ARCH=$(uname -m)
OS=$(uname -s | tr '[:upper:]' '[:lower:]')

case "$OS" in
    darwin)
        if [ "$ARCH" = "arm64" ]; then
            TARGET="aarch64-apple-darwin"
        elif [ "$ARCH" = "x86_64" ]; then
            TARGET="x86_64-apple-darwin"
        else
            echo "[ERROR] Unsupported macOS architecture: $ARCH" >&2
            exit 1
        fi
        ;;
    linux)
        if [ "$ARCH" = "x86_64" ]; then
            TARGET="x86_64-unknown-linux-gnu"
        elif [ "$ARCH" = "aarch64" ] || [ "$ARCH" = "arm64" ]; then
            TARGET="aarch64-unknown-linux-gnu"
        else
            echo "[ERROR] Unsupported Linux architecture: $ARCH" >&2
            exit 1
        fi
        ;;
    *)
        echo "[ERROR] Unsupported operating system: $OS (For Windows, run install.ps1)" >&2
        exit 1
        ;;
esac

# 2. Fetch release version
if [ -z "$TAG" ]; then
    echo "[*] Resolving latest release for $REPO..."
    TAG=$(curl -sSL -H "User-Agent: groundcontrol-installer" "https://api.github.com/repos/$REPO/releases/latest" | grep '"tag_name":' | sed -E 's/.*"([^"]+)".*/\1/')
    if [ -z "$TAG" ]; then
        echo "[ERROR] Failed to fetch latest release tag from https://api.github.com/repos/$REPO/releases/latest" >&2
        exit 1
    fi
else
    echo "[*] Installing specified release $TAG for $REPO..."
fi

ARCHIVE_NAME="groundcontrol-${TAG}-${TARGET}.tar.gz"
DOWNLOAD_URL="https://github.com/$REPO/releases/download/${TAG}/${ARCHIVE_NAME}"

echo "[*] Downloading $DOWNLOAD_URL..."
TMP_DIR=$(mktemp -d)
trap 'rm -rf "$TMP_DIR"' EXIT

curl -fsSL "$DOWNLOAD_URL" -o "$TMP_DIR/$ARCHIVE_NAME"

# Checksum validation (if checksums.txt exists)
if [ "$FAST" = false ] && [ "$SKIP_CHECKSUM" = false ]; then
    CHECKSUMS_URL="https://github.com/$REPO/releases/download/${TAG}/checksums.txt"
    if curl -fsSL -s -I "$CHECKSUMS_URL" >/dev/null 2>&1; then
        echo "[*] Verifying SHA-256 checksum..."
        curl -fsSL "$CHECKSUMS_URL" -o "$TMP_DIR/checksums.txt"
        EXPECTED_HASH=$(grep "$ARCHIVE_NAME" "$TMP_DIR/checksums.txt" | awk '{print $1}')
        if [ -n "$EXPECTED_HASH" ]; then
            if command -v sha256sum >/dev/null 2>&1; then
                ACTUAL_HASH=$(sha256sum "$TMP_DIR/$ARCHIVE_NAME" | awk '{print $1}')
            elif command -v shasum >/dev/null 2>&1; then
                ACTUAL_HASH=$(shasum -a 256 "$TMP_DIR/$ARCHIVE_NAME" | awk '{print $1}')
            fi
            if [ -n "$ACTUAL_HASH" ] && [ "$ACTUAL_HASH" != "$EXPECTED_HASH" ]; then
                echo "[ERROR] Checksum verification failed!" >&2
                echo "Expected: $EXPECTED_HASH" >&2
                echo "Actual:   $ACTUAL_HASH" >&2
                exit 1
            fi
            echo "[+] Checksum verified."
        fi
    fi
else
    echo "[*] Fast install mode: skipping remote checksum verification."
fi

echo "[*] Extracting binary..."
tar -xzf "$TMP_DIR/$ARCHIVE_NAME" -C "$TMP_DIR"

mkdir -p "$INSTALL_DIR"
EXTRACTED="$TMP_DIR/groundcontrol-${TAG}-${TARGET}"
cp "$EXTRACTED/groundcontrol" "$INSTALL_DIR/groundcontrol"
chmod +x "$INSTALL_DIR/groundcontrol"

# Copy updater script beside binary
if [ -f "$0" ]; then
    cp "$0" "$INSTALL_DIR/install.sh" 2>/dev/null || true
fi

# Install the bundled embedding model as a sidecar next to the binary so the
# embedder resolves it at <exe_dir>/models/<model>/ (no separate download).
if [ -d "$EXTRACTED/models" ]; then
    if [ "$SKIP_MODELS" = false ] && ([ "$FAST" = false ] || [ ! -d "$INSTALL_DIR/models" ]); then
        echo "[*] Installing bundled embedding model (sidecar)..."
        rm -rf "$INSTALL_DIR/models"
        cp -r "$EXTRACTED/models" "$INSTALL_DIR/models"
    else
        echo "[*] Preserving existing models directory."
    fi
fi

# Optional symlink for short shorthand alias `ctxv`
ln -sf "$INSTALL_DIR/groundcontrol" "$INSTALL_DIR/gc" 2>/dev/null || true

# GraphView convenience wrapper: allow direct invocation via `groundcontrol-graphview`
cat << 'EOF' > "$INSTALL_DIR/groundcontrol-graphview"
#!/bin/sh
exec "$(dirname "$0")/groundcontrol" graphview "$@"
EOF
chmod +x "$INSTALL_DIR/groundcontrol-graphview"

echo ""
echo "[+] Successfully installed 'groundcontrol' to $INSTALL_DIR/groundcontrol"
echo ""

# Auto-configure installed coding agents
echo "[*] Auto-configuring coding agents..."
INSTALL_ARGS="install -y --dir=$INSTALL_DIR"
if [ "$FAST" = true ]; then
    INSTALL_ARGS="$INSTALL_ARGS --fast"
fi
if [ "$SKIP_RULES" = true ]; then
    INSTALL_ARGS="$INSTALL_ARGS --rules=false"
fi
if [ "$AUTH" = true ]; then
    INSTALL_ARGS="$INSTALL_ARGS --auth"
fi
if [ -n "$AGENTS" ]; then
    INSTALL_ARGS="$INSTALL_ARGS --agents=$AGENTS"
fi
"$INSTALL_DIR/groundcontrol" $INSTALL_ARGS

# 3. Path hint
case ":$PATH:" in
    *":$INSTALL_DIR:"*) ;;
    *)
        echo "[NOTE] $INSTALL_DIR is not currently in your PATH."
        echo "   Add it to your shell config (~/.bashrc or ~/.zshrc):"
        echo "   export PATH=\"\$HOME/.local/bin:\$PATH\""
        echo ""
        ;;
esac

echo "[>] Quick check: run 'groundcontrol --version' to get started."
