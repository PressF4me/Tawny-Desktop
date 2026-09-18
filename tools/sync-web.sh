#!/usr/bin/env bash
# Vendor the web client from the Tawny Android repo, which is its source of
# truth, into web/. Same arrangement as Tawny Docker: this repo builds from its
# own copy, so a Flathub build (which has no network) needs nothing else.
#
#   tools/sync-web.sh [path-to-Tawny-Android]
#
# Records the source commit in web/.source so a release can say what it ships.

set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SRC="${1:-$HERE/../Tawny Android}"
[ -f "$SRC/public/index.html" ] || { echo "no web client at $SRC/public" >&2; exit 1; }

# setup.* is Tawny Docker's /setup page, and sounds/_src holds the chime
# recordings; neither ships in the app.
rsync -a --delete \
  --exclude 'setup.html' --exclude 'setup.js' --exclude 'setup.css' \
  --exclude 'sounds/_src' --exclude 'config.json' \
  "$SRC/public/" "$HERE/web/"

rev="$(git -C "$SRC" rev-parse --short HEAD 2>/dev/null || echo unknown)"
dirty="$(git -C "$SRC" status --porcelain -- public 2>/dev/null | head -1)"
printf '%s%s\n' "$rev" "${dirty:+-dirty}" > "$HERE/web/.source"
echo "web/ synced from Tawny Android $rev${dirty:+ (uncommitted changes)}"
