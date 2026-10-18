#!/usr/bin/env bash
# Flash the gateway ELF with espflash (build + flash).
#
# Defaults: build the release gateway ELF when flashing from a repo checkout;
# use prebuilt firmware/ ELF from an extracted release zip without rebuilding.
#
#   ./tools/flash-gateway.sh
#   ./tools/flash-gateway.sh --port /dev/ttyACM1
#   ./tools/flash-gateway.sh --no-build --no-monitor
#
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=flash-image-resolve.sh
source "${SCRIPT_DIR}/flash-image-resolve.sh"
cd "${SCRIPT_DIR}/.."

# An external CARGO_TARGET_DIR breaks esp-idf-sys project-root resolution; set
# CARGO_WORKSPACE_DIR to the repo root so embuild short-circuits the pop-count.
export CARGO_WORKSPACE_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"

if ! command -v espflash >/dev/null 2>&1; then
  echo "error: espflash not found. Run: ./tools/install-flash-tools.sh" >&2
  exit 1
fi

do_build=""   # empty = auto from resolved image path
do_monitor=true
chip="${MCU:-esp32c5}"
port="${ESPFLASH_PORT:-${PORT:-}}"
image=""
image_explicit=false
build_jobs=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --build) do_build=true; shift ;;
    --no-build) do_build=false; shift ;;
    --image)
      [[ $# -ge 2 ]] || { echo "error: --image requires a path" >&2; exit 1; }
      image="$2"
      image_explicit=true
      shift 2
      ;;
    --chip)
      [[ $# -ge 2 ]] || { echo "error: --chip requires a value" >&2; exit 1; }
      chip="$2"
      shift 2
      ;;
    --port)
      [[ $# -ge 2 ]] || { echo "error: --port requires a device path" >&2; exit 1; }
      port="$2"
      shift 2
      ;;
    --monitor) do_monitor=true; shift ;;
    --no-monitor) do_monitor=false; shift ;;
    -j|--jobs)
      [[ $# -ge 2 ]] || { echo "error: -j requires a number" >&2; exit 1; }
      build_jobs="$2"
      shift 2
      ;;
    -h|--help)
      cat <<'EOF'
usage: ./tools/flash-gateway.sh [options]

  --image PATH            ELF path (overrides auto-resolve)
  --chip CHIP             espflash chip (default: $MCU or esp32c5)
  --port DEV              Serial port (default: $ESPFLASH_PORT / $PORT / auto)
  -j, --jobs N            Parallel cargo build jobs (default: .cargo/config.toml)
  --monitor               Open serial monitor after flash (default)
  --no-monitor            Flash only
EOF
      exit 0
      ;;
    *)
      echo "error: unknown option: $1" >&2
      exit 1
      ;;
  esac
done

role="gateway"

if [[ "$image_explicit" == false ]]; then
  if [[ "$do_build" == false ]]; then
    image="$(resolve_firmware_image "$role")"
  else
    # --build or auto: missing image is OK until after cargo build
    image="$(resolve_firmware_image "$role" 2>/dev/null || true)"
  fi
fi

# Auto: skip cargo build when flashing a release zip ELF under firmware/.
if [[ -z "$do_build" ]]; then
  if [[ -z "$image" || ! -f "$image" ]]; then
    do_build=true
  elif firmware_image_is_release "$image"; then
    do_build=false
  else
    do_build=true
  fi
fi

if [[ "$do_build" == true ]]; then
  echo "building gateway..."
  cargo_build_args=(build --release --bin esp32-wardriver-gateway)
  [[ -n "$build_jobs" ]] && cargo_build_args+=(-j "$build_jobs")
  cargo_build_args+=(--features gateway)
  cargo "${cargo_build_args[@]}"
  image="$(resolve_firmware_image "$role")"
else
  if [[ ! -f "$image" ]]; then
    echo "error: $image not found" >&2
    exit 1
  fi
  echo "flashing: $image"
fi

flash_args=(flash --chip "$chip" --non-interactive)
[[ -n "$port" ]] && flash_args+=(--port "$port")
[[ "$do_monitor" == true ]] && flash_args+=(--monitor)

exec espflash "${flash_args[@]}" "$image"