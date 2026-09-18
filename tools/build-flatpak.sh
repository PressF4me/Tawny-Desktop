#!/usr/bin/env bash
# Build the Flatpak locally and export a single-file bundle to dist/.
#
#   tools/build-flatpak.sh            build, install for this user, bundle
#   tools/build-flatpak.sh --no-install
#
# Needs: flatpak, the Flathub remote, and org.flatpak.Builder
#   flatpak install --user flathub org.flatpak.Builder

set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP=io.github.PressF4me.Tawny
MANIFEST="$HERE/packaging/flatpak/$APP.yml"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' "$HERE/src-tauri/Cargo.toml" | head -1)"

INSTALL=(--user --install)
[ "${1:-}" = "--no-install" ] && INSTALL=()

cd "$HERE"
flatpak run org.flatpak.Builder --force-clean --install-deps-from=flathub \
  --repo=repo "${INSTALL[@]}" build-dir "$MANIFEST"

mkdir -p dist
flatpak build-bundle repo "dist/Tawny-$VERSION-x86_64.flatpak" "$APP" \
  --runtime-repo=https://flathub.org/repo/flathub.flatpakrepo
echo "dist/Tawny-$VERSION-x86_64.flatpak"
