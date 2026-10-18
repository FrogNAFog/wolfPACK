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

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ptable="${repo_root}/partitions.csv"
if [[ ! -f "${ptable}" ]]; then
  echo "error: partition table not found at ${ptable}" >&2
  exit 1
fi

exec espflash erase-parts "${port_args[@]}" --chip "${chip}" nvs --partition-table "${ptable}"
