#!/usr/bin/env bash
# Build Courier and install Courier.app on macOS.
#
#   ./scripts/install-macos.sh            # installs to /Applications
#   ./scripts/install-macos.sh --user     # installs to ~/Applications
#
# COURIER_INSTALL_DIR overrides the destination folder.
set -euo pipefail

cd "$(dirname "$0")/.."

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "This script is for macOS. On Linux, use ./scripts/install-linux.sh" >&2
  exit 1
fi

dest="${COURIER_INSTALL_DIR:-/Applications}"
if [[ "${1:-}" == "--user" ]]; then
  dest="${COURIER_INSTALL_DIR:-$HOME/Applications}"
fi

if ! command -v cargo >/dev/null 2>&1; then
  echo "Rust is not installed. Install it with:" >&2
  echo "  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh" >&2
  exit 1
fi

if ! xcode-select -p >/dev/null 2>&1; then
  echo "Xcode Command Line Tools are required. Install them with:" >&2
  echo "  xcode-select --install" >&2
  exit 1
fi

if ! cargo bundle --version >/dev/null 2>&1; then
  echo "==> Installing cargo-bundle"
  cargo install cargo-bundle --locked
fi

echo "==> Building Courier (release)"
cargo bundle --release --format osx

app="target/release/bundle/osx/Courier.app"
echo "==> Signing (ad hoc, for this Mac)"
codesign --force --deep -s - "$app"

echo "==> Installing to $dest"
mkdir -p "$dest"
if [[ -w "$dest" ]]; then
  rm -rf "$dest/Courier.app"
  cp -R "$app" "$dest/"
else
  sudo rm -rf "$dest/Courier.app"
  sudo cp -R "$app" "$dest/"
fi

echo "==> Done. Open Courier from Launchpad or Spotlight, or run:"
echo "    open \"$dest/Courier.app\""
