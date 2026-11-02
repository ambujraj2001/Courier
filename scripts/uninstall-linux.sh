#!/usr/bin/env bash
# Remove a Courier installed with ./scripts/install-linux.sh.
# Your data in ~/.local/share/Courier is kept; delete it yourself if you want.
set -euo pipefail

prefix="${COURIER_PREFIX:-$HOME/.local}"
rm -f "$prefix/bin/courier" "$prefix/share/applications/courier.desktop"
for size in 16 32 128 256 512; do
  rm -f "$prefix/share/icons/hicolor/${size}x${size}/apps/courier.png"
done
command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$prefix/share/applications" >/dev/null 2>&1 || true
echo "Courier removed. Your data is still in ${XDG_DATA_HOME:-$HOME/.local/share}/Courier"
