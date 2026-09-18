#!/usr/bin/env bash
# Build the AppImage to dist/. Needs Rust and the Tauri CLI that matches the
# pinned crates:
#   cargo install tauri-cli --version '=3.0.0-alpha.1' --locked

set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# Tauri's AppImage bundler runs quick-sharun, which fetches anylinux.c from a
# path upstream removed on 2026-09-10 (pkgforge-dev/Anylinux-AppImages#866).
# Pin it to the last commit that has it until the bundler catches up.
export ANYLINUX_LIB_SOURCE="${ANYLINUX_LIB_SOURCE:-https://raw.githubusercontent.com/pkgforge-dev/Anylinux-AppImages/df9cd3246ccbcf61eb22fc321a52277351047b4b/useful-tools/lib/anylinux.c}"

command -v patchelf >/dev/null || { echo "patchelf is needed (pacman -S patchelf / apt install patchelf)" >&2; exit 1; }

cd "$HERE/src-tauri"
cargo tauri build --bundles appimage

mkdir -p "$HERE/dist"
cp target/release/bundle/appimage/*.AppImage "$HERE/dist/"
ls -1 "$HERE"/dist/*.AppImage
