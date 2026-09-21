---
title: "Installation & Standalone Binaries"
description: "Installing native precompiled release binaries and the bundled ONNX embedding model sidecar."
category: "building"
status: "active"
tags: ["install", "binaries", "powershell", "curl", "sidecar", "embeddings"]
related:
  - "[[docs/architecture/building/index]]"
  - "[[docs/architecture/building/build-from-source]]"
  - "[[docs/architecture/building/client-setup]]"
---

# Installation & Standalone Binaries

`groundcontrol` release archives bundle both the native `groundcontrol` executable and the local 768-dimensional ONNX embedding model sidecar (`jina-embeddings-v2-base-code`). No external Python environment, C-compiler, or Docker container is required.

---

## One-Command Shell Installers

### Windows (PowerShell)
Run in PowerShell (as your standard user account):
```powershell
irm https://raw.githubusercontent.com/t3e1er/groundcontrol/master/install.ps1 | iex
```
* **Install Location**: `%LOCALAPPDATA%\Programs\groundcontrol\`
* **Sidecar Location**: `%LOCALAPPDATA%\Programs\groundcontrol\models\jina-embeddings-v2-base-code\`
* **Path Registration**: Automatically adds `groundcontrol` to your user `PATH`.

### macOS & Linux (Bash / Zsh)
Run in terminal:
```bash
curl -fsSL https://raw.githubusercontent.com/t3e1er/groundcontrol/master/install.sh | sh
```
* **Install Location**: `~/.local/bin/groundcontrol` (or `/usr/local/bin` if root)
* **Sidecar Location**: `~/.local/share/groundcontrol/models/jina-embeddings-v2-base-code/`

---

## Manual Binary Download

Direct release archives are published for every tagged version on GitHub Releases:
* **Windows**: `groundcontrol-vX.Y.Z-x86_64-pc-windows-msvc.zip`
* **Linux x86_64**: `groundcontrol-vX.Y.Z-x86_64-unknown-linux-gnu.tar.gz`
* **Linux aarch64**: `groundcontrol-vX.Y.Z-aarch64-unknown-linux-gnu.tar.gz`
* **macOS Apple Silicon**: `groundcontrol-vX.Y.Z-aarch64-apple-darwin.tar.gz`

### Sidecar Layout
The executable expects the embedding model sidecar in one of three locations:
1. An environment variable: `CTX_MODELS_DIR=/path/to/models`
2. A `models/` directory adjacent to the `groundcontrol` executable:
   ```
   groundcontrol-dir/
   ├── groundcontrol (or groundcontrol.exe)
   └── models/
       └── jina-embeddings-v2-base-code/
           ├── model.onnx
           └── tokenizer.json
   ```
3. A relative `../models/` directory (used during development and `cargo test`).

---

## Automated Configuration Bootstrapping

`groundcontrol` eliminates static bundled configuration files in favor of **self-bootstrapping lazy generation on first run**:

* **Central Machine Config**: Persisted at `${GROUNDCONTROL_CACHE_DIR}/config.toml` (managed via [`ensure_global_config`](file:///c:/dev/ctx/groundcontrol/crates/groundcontrol-common/src/config.rs)).
  - Generated automatically on first CLI command, agent touch, or installer execution.
  - Generates secure random API keys for connected agents (`antigravity`, `claude`, `cursor`, `windsurf`, `vscode`, `zed`, `roo`, `kiro`) and an internal `daemon_key` for GraphView sidecar telemetry relay.
  - Zero manual editing required; customizable at any time via `groundcontrol config set <key> <val>`.
* **Coding Agent Auto-Installer**: The installer scripts (`install.ps1` and `install.sh`) invoke `groundcontrol install -y` via [`run_install`](file:///c:/dev/ctx/groundcontrol/crates/groundcontrol-cli/src/installer/mod.rs).
  - Detects installed editors (Antigravity IDE, Cursor, Claude Desktop, Windsurf, Zed, VS Code, Kiro CLI).
  - Registers the zero-arg `groundcontrol` executable in each agent's MCP configuration.
  - Supports `--auth` to inject per-client `GROUNDCONTROL_API_KEY` credentials automatically.

---

## Repository Configuration Modes

`groundcontrol` strictly separates machine configuration from repository configuration:

### 1. Zero-Config Indexing (Agent First-Touch)
When an agent connects or `groundcontrol index` is run on a repository without a `groundcontrol.toml`:
* Automatically reads the repository's `.gitignore` and merges it into default exclusions in-memory.
* Indexes directly into central storage (`${GROUNDCONTROL_CACHE_DIR}/corpora/<name>/`) without polluting git working tree state or writing untracked files.
* Registers the repository in central `config.toml` `[corpora.<name>]` so future runs mount it automatically.
* Emits an advisory tip: `[i] No groundcontrol.toml found. Indexed using defaults + local .gitignore. Run 'groundcontrol init' to commit a local groundcontrol.toml.`

### 2. Explicit Repository Configuration (`groundcontrol init`)
To commit project-specific indexing rules, templates, and document patterns:
```bash
groundcontrol init
```
* Generates a clean `<repo_root>/groundcontrol.toml` populated with parsed `.gitignore` rules, standard safety exclusions (`.git`, `.index`, `node_modules`), document patterns (`docs/**`, `wiki/**`), and template schema paths.
* Automatically registers the repository in central machine `config.toml` `[corpora.<name>]`.

---

## Verification

Confirm installation and inspect system capabilities:
```bash
groundcontrol --version
groundcontrol status --scope indexing
```
If GPU acceleration is active, the status output displays the detected DirectX 12 Compute adapter (e.g. NVIDIA GeForce, AMD Radeon, or Intel Arc).
