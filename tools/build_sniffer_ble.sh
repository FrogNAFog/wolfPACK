#!/usr/bin/env bash
set -euo pipefail

# An external CARGO_TARGET_DIR (shared target dir) breaks esp-idf-sys project-root
# resolution — the OUT_DIR pop-count lands off-tree, so BLE sdkconfig is dropped and
# the sniffer loses its NimBLE FFI symbols. Point CARGO_WORKSPACE_DIR at the repo
# root so embuild short-circuits the pop-count.
export CARGO_WORKSPACE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# Build a unified sniffer image (WiFi + BLE-capable).
#
# Why this script exists:
# - Enabling BT/NimBLE in `sdkconfig.defaults.ble` must be coordinated with actually
#   initializing NimBLE at runtime (`src/ble_sniffer.rs`).
# - BLE is enabled via `sdkconfig.defaults.ble` by default in `Cargo.toml`.

if [[ -z "${IDF_PATH:-}" ]]; then
  echo "error: ESP-IDF env not loaded. Run: . ~/export-esp.sh" >&2
  exit 1
fi

mode="${1:-release}"
if [[ "${mode}" == "debug" ]]; then
  cargo build --bin esp32-wardriver-sniffer --features sniffer
else
  cargo build --release --bin esp32-wardriver-sniffer --features sniffer
fi

echo "ok: built sniffer at target/riscv32imac-esp-espidf/${mode}/esp32-wardriver-sniffer"

