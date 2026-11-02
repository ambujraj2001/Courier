#!/usr/bin/env bash
# Build a .deb package for Ubuntu/Debian:
#   ./scripts/build-deb.sh
#   sudo apt install ./target/release/bundle/deb/courier_*_amd64.deb
set -euo pipefail

cd "$(dirname "$0")/.."

if ! cargo bundle --version >/dev/null 2>&1; then
  echo "==> Installing cargo-bundle"
  cargo install cargo-bundle --locked
fi

cargo bundle --release --format deb
ls -1 target/release/bundle/deb/*.deb
