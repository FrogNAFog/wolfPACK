#!/usr/bin/env bash
set -euo pipefail

if ! command -v espflash >/dev/null 2>&1; then
  echo "error: espflash not found. Install with: cargo install cargo-espflash espflash --locked" >&2
  exit 1
fi

chip="${MCU:-esp32c5}"

port="${ESPFLASH_PORT:-${PORT:-}}"
port_args=()
if [[ -n "${port}" ]]; then
  port_args=(--port "${port}")
fi

exec espflash erase-flash "${port_args[@]}" --chip "${chip}"
