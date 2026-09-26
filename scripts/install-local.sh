#!/usr/bin/env bash
# groundcontrol Local Developer Installer for macOS and Linux
# Mirrors install.sh behavior using local cargo build artifacts instead of GitHub releases.

set -e

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BUILD_TYPE="release"
INSTALL_DIR="${CTXV_INSTALL_DIR:-${CXTV_INSTALL_DIR:-$HOME/.local/bin}}"
SKIP_PATH=false
CONFIGURE_AGENTS=false
AGENTS=""

while [ $# -gt 0 ]; do
    case "$1" in
        --debug)
            BUILD_TYPE="debug"
            shift
            ;;
        --release)
            BUILD_TYPE="release"
            shift
            ;;
        --dir)
            INSTALL_DIR="$2"
            shift 2
            ;;
        --skip-path)
            SKIP_PATH=true
            shift
            ;;
        --configure-agents)
            CONFIGURE_AGENTS=true
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

SOURCE_EXE="$REPO_ROOT/target/$BUILD_TYPE/groundcontrol"

if [ ! -f "$SOURCE_EXE" ]; then
    echo "[ERROR] Local binary not found at $SOURCE_EXE." >&2
    echo "        Please run 'cargo build --workspace --all-features --$BUILD_TYPE' first." >&2
    exit 1
fi

echo "[*] Installing groundcontrol from local $BUILD_TYPE build..."
echo "    Source:      $SOURCE_EXE"
echo "    Destination: $INSTALL_DIR"

mkdir -p "$INSTALL_DIR"

# Install binary & alias
cp -f "$SOURCE_EXE" "$INSTALL_DIR/groundcontrol"
chmod +x "$INSTALL_DIR/groundcontrol"
ln -sf "$INSTALL_DIR/groundcontrol" "$INSTALL_DIR/ctxv"

# Check and copy models sidecar if present
SOURCE_MODELS="$REPO_ROOT/models"
[ ! -d "$SOURCE_MODELS" ] && SOURCE_MODELS="$(dirname "$SOURCE_EXE")/models"
DEST_MODELS="$INSTALL_DIR/models"
if [ -d "$SOURCE_MODELS" ]; then
    echo "[*] Syncing local models sidecar..."
    rm -rf "$DEST_MODELS"
    cp -r "$SOURCE_MODELS" "$DEST_MODELS"
fi

# Ensure $INSTALL_DIR is in PATH
if [ "$SKIP_PATH" = false ]; then
    case ":$PATH:" in
        *":$INSTALL_DIR:"*)
            echo "[+] $INSTALL_DIR is already in your PATH."
            ;;
        *)
            SHELL_RC=""
            if [ -n "$ZSH_VERSION" ] || [ -f "$HOME/.zshrc" ]; then
                SHELL_RC="$HOME/.zshrc"
            elif [ -f "$HOME/.bashrc" ]; then
                SHELL_RC="$HOME/.bashrc"
            elif [ -f "$HOME/.bash_profile" ]; then
                SHELL_RC="$HOME/.bash_profile"
            fi

            if [ -n "$SHELL_RC" ]; then
                if ! grep -q "$INSTALL_DIR" "$SHELL_RC" 2>/dev/null; then
                    echo "export PATH=\"\$PATH:$INSTALL_DIR\"" >> "$SHELL_RC"
                    echo "[+] Added $INSTALL_DIR to $SHELL_RC."
                    echo "    (Run 'source $SHELL_RC' or restart shell for changes to take effect)."
                fi
            fi
            ;;
    esac
fi

# Coding agent configuration (optional, disabled by default to preserve custom configs)
if [ "$CONFIGURE_AGENTS" = true ]; then
    echo "[*] Configuring installed coding agents..."
    INSTALL_ARGS=("install" "-y" "--dir=$INSTALL_DIR")
    if [ -n "$AGENTS" ]; then
        INSTALL_ARGS+=("--agents=$AGENTS")
    fi
    "$INSTALL_DIR/groundcontrol" "${INSTALL_ARGS[@]}"
fi

echo ""
echo "[+] Successfully installed local build to: $INSTALL_DIR/groundcontrol"
INSTALLED_VERSION="$("$INSTALL_DIR/groundcontrol" --version)"
echo "    Installed version: $INSTALLED_VERSION"
echo ""
