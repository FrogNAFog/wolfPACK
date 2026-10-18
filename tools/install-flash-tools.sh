#!/usr/bin/env bash
# One-time host setup for flashing prebuilt wolfPACK ELFs (espflash only).
#
# Does not install espup, ESP-IDF, or generate fleet PMK.
#
#   ./tools/install-flash-tools.sh
#   ./tools/install-flash-tools.sh --check-only
#
set -euo pipefail

check_only=false

usage() {
  cat <<'EOF'
usage: ./tools/install-flash-tools.sh [options]

  Installs espflash when missing. Checks Linux serial port permissions.

  --check-only   Exit 0 if espflash is on PATH, else exit 1
  -h, --help     Show this help
EOF
  exit 0
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --check-only) check_only=true; shift ;;
    -h|--help) usage ;;
    *) echo "error: unknown option: $1" >&2; exit 1 ;;
  esac
done

if command -v espflash >/dev/null 2>&1; then
  echo "espflash: $(command -v espflash)"
  espflash --version 2>/dev/null || true
  if [[ "$check_only" == true ]]; then
    exit 0
  fi
else
  if [[ "$check_only" == true ]]; then
    exit 1
  fi
  echo "espflash not found; attempting install..."
  if command -v cargo-binstall >/dev/null 2>&1; then
    cargo binstall espflash --no-confirm
  elif command -v cargo >/dev/null 2>&1; then
    cargo install espflash --locked
  else
    cat <<'EOF' >&2
error: could not install espflash automatically.

Install Rust from https://rustup.rs then re-run this script, or install a
prebuilt binary from https://github.com/esp-rs/espflash/releases
EOF
    exit 1
  fi
fi

# Linux: remind operator about USB serial permissions (do not run sudo here).
if [[ "$(uname -s)" == "Linux" ]]; then
  if ! id -nG "${USER}" 2>/dev/null | tr ' ' '\n' | grep -qxE 'dialout|uucp'; then
    echo "note: add your user to dialout for USB serial access:" >&2
    echo "  sudo usermod -aG dialout \$USER && newgrp dialout" >&2
  fi
fi

# Smoke: list-ports works even with no devices attached.
if command -v espflash >/dev/null 2>&1; then
  espflash list-ports >/dev/null 2>&1 || true
  echo "ready: run ./tools/wardriver flash-control or flash-sniffers"
fi
