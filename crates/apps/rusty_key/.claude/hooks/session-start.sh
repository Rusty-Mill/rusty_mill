#!/bin/bash
# SessionStart hook: provision the Tauri 2 desktop toolchain (Phase 15) so the
# `src-tauri` shell builds and the IPC smoke test runs headless. Rust + node are
# already in the base image; this adds the WebKitGTK system libs, xvfb, and the
# Tauri CLI. Web-only, idempotent, and cache-friendly (a warm container skips it).
set -euo pipefail

# Only provision in the remote (Claude Code on the web) environment.
if [ "${CLAUDE_CODE_REMOTE:-}" != "true" ]; then
  exit 0
fi

# Fast path: already provisioned (warm/cached container) → nothing to do.
if pkg-config --exists webkit2gtk-4.1 2>/dev/null && command -v cargo-tauri >/dev/null 2>&1; then
  echo "tauri toolchain already present — skipping provision"
  exit 0
fi

# System libraries for the Tauri WebKitGTK webview (Ubuntu 22.04+/24.04, Debian 12).
if ! pkg-config --exists webkit2gtk-4.1 2>/dev/null; then
  export DEBIAN_FRONTEND=noninteractive
  # Tolerate failures from pre-existing third-party PPAs (e.g. deadsnakes) — the
  # official archives still refresh, which is all the WebKitGTK libs need.
  sudo apt-get update || echo "apt-get update reported errors (third-party PPAs); continuing"
  sudo apt-get install -y --no-install-recommends \
    libwebkit2gtk-4.1-dev \
    libgtk-3-dev \
    libsoup-3.0-dev \
    libjavascriptcoregtk-4.1-dev \
    librsvg2-dev \
    libayatana-appindicator3-dev \
    libxdo-dev \
    libssl-dev \
    pkg-config \
    build-essential \
    xvfb
fi

# Tauri CLI (pinned to 2.x for reproducible builds).
if ! command -v cargo-tauri >/dev/null 2>&1; then
  cargo install tauri-cli --version "^2" --locked
fi

echo "tauri toolchain provisioned"
