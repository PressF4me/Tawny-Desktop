#!/bin/sh
# Chromium's own sandbox cannot create namespaces inside Flatpak. zypak swaps
# it for Flatpak's sandbox (flatpak-spawn --sandbox), the same way every
# Chromium-based app on Flathub runs.
#
# zypak's default "spawn" zygote strategy dies under CEF with "Failed to send
# spawn request to supervisor: Bad file descriptor". The older "mimic"
# strategy works; each child still gets its own Flatpak sandbox.
export ZYPAK_ZYGOTE_STRATEGY_SPAWN=0
exec zypak-wrapper /app/lib/tawny/tawny "$@"
