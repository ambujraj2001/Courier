#!/usr/bin/env bash
# Build Courier from source and install it for the current user on Linux
# (Ubuntu, Debian and most other distributions).
#
#   ./scripts/install-linux.sh                 # build and install to ~/.local
#   ./scripts/install-linux.sh --install-deps  # also apt-get the build tools
#
# Installs:
#   ~/.local/bin/courier
#   ~/.local/share/applications/courier.desktop
#   ~/.local/share/icons/hicolor/<size>/apps/courier.png
#
# COURIER_PREFIX overrides ~/.local (e.g. /usr/local, run with sudo).
set -euo pipefail

cd "$(dirname "$0")/.."

if [[ "$(uname -s)" != "Linux" ]]; then
  echo "This script is for Linux. On macOS, use ./scripts/install-macos.sh" >&2
  exit 1
fi

prefix="${COURIER_PREFIX:-$HOME/.local}"

if [[ "${1:-}" == "--install-deps" ]]; then
  if command -v apt-get >/dev/null 2>&1; then
    echo "==> Installing build dependencies with apt-get (needs sudo)"
    sudo apt-get update
    sudo apt-get install -y build-essential pkg-config curl \
      libxkbcommon-dev libwayland-dev libx11-dev libxcursor-dev libxrandr-dev libxi-dev \
      libgl1-mesa-dev libegl1-mesa-dev xdg-utils xdg-desktop-portal
  else
    echo "apt-get not found; install a C compiler, pkg-config and the X11/Wayland/GL development packages for your distribution." >&2
  fi
fi

if ! command -v cargo >/dev/null 2>&1; then
  echo "Rust is not installed. Install it with:" >&2
  echo "  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh" >&2
  echo "then open a new terminal (or run: source \"\$HOME/.cargo/env\") and re-run this script." >&2
  exit 1
fi

if ! command -v cc >/dev/null 2>&1; then
  echo "A C compiler is required (the JavaScript engine is written in C)." >&2
  echo "On Ubuntu: sudo apt-get install build-essential   (or re-run with --install-deps)" >&2
  exit 1
fi

echo "==> Building Courier (release). The first build takes a few minutes."
cargo build --release --locked

bin_dir="$prefix/bin"
apps_dir="$prefix/share/applications"
icons_dir="$prefix/share/icons/hicolor"

echo "==> Installing to $prefix"
install -Dm755 target/release/courier "$bin_dir/courier"
for size in 16 32 128 256 512; do
  install -Dm644 "assets/icon/icon_${size}x${size}.png" "$icons_dir/${size}x${size}/apps/courier.png"
done
desktop_tmp="$(mktemp)"
sed "s|@EXEC@|$bin_dir/courier|" packaging/linux/courier.desktop > "$desktop_tmp"
install -Dm644 "$desktop_tmp" "$apps_dir/courier.desktop"
rm -f "$desktop_tmp"

command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$apps_dir" >/dev/null 2>&1 || true
command -v gtk-update-icon-cache >/dev/null 2>&1 && gtk-update-icon-cache -q -t "$icons_dir" >/dev/null 2>&1 || true

echo "==> Done. Launch Courier from your applications menu, or run:"
echo "    $bin_dir/courier"
case ":$PATH:" in
  *":$bin_dir:"*) ;;
  *) echo "    (add $bin_dir to your PATH to run it as just 'courier')" ;;
esac
