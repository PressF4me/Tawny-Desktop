#!/usr/bin/env bash
# Regenerate packaging/flatpak/cargo-sources.json from src-tauri/Cargo.lock.
# Flathub builds offline, so every crate has to be a pinned source in the
# manifest. Run this whenever Cargo.lock changes, and commit the result.

set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WORK="${XDG_CACHE_HOME:-$HOME/.cache}/tawny-flatpak-tools"

if [ ! -x "$WORK/venv/bin/python" ]; then
  mkdir -p "$WORK"
  git clone -q --depth 1 https://github.com/flatpak/flatpak-builder-tools.git "$WORK/fbt"
  python3 -m venv "$WORK/venv"
  "$WORK/venv/bin/pip" -q install aiohttp tomlkit
fi

"$WORK/venv/bin/python" "$WORK/fbt/cargo/flatpak-cargo-generator.py" \
  "$HERE/src-tauri/Cargo.lock" -o "$HERE/packaging/flatpak/cargo-sources.json"
echo "wrote packaging/flatpak/cargo-sources.json"
