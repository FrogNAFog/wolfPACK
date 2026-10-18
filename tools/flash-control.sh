#!/usr/bin/env bash
# Flash the control-node ELF with espflash (build + board picker).
#
# Defaults: build release image when using target/ ELF; use prebuilt firmware/
# from an extracted release zip without rebuilding.
#
#   ./tools/flash-control.sh
#   ./tools/flash-control.sh --board xiao --port /dev/ttyACM0
#   ./tools/flash-control.sh --mgmt-ap-5ghz
#   ./tools/flash-control.sh --no-build --no-monitor
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

board_feature=""
do_build=""   # empty = auto from resolved image path
do_monitor=true
chip="${MCU:-esp32c5}"
port="${ESPFLASH_PORT:-${PORT:-}}"
image=""
image_explicit=false
mgmt_ap_5ghz=false
build_jobs=""

# Env alternative to --mgmt-ap-5ghz (non-zero / non-empty enables).
if [[ -n "${WARDRIVER_MGMT_AP_5GHZ:-}" && "${WARDRIVER_MGMT_AP_5GHZ}" != "0" ]]; then
  mgmt_ap_5ghz=true
fi

while [[ $# -gt 0 ]]; do
  case "$1" in
    --board)
      [[ $# -ge 2 ]] || { echo "error: --board requires xiao|marauder" >&2; exit 1; }
      board_feature="$2"
      shift 2
      ;;
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
    --mgmt-ap-5ghz) mgmt_ap_5ghz=true; shift ;;
    -j|--jobs)
      [[ $# -ge 2 ]] || { echo "error: -j requires a number" >&2; exit 1; }
      build_jobs="$2"
      shift 2
      ;;
    -h|--help)
      cat <<'EOF'
usage: ./tools/flash-control.sh [options]

  --board xiao|marauder   Skip interactive board picker (also accepts 1|2)
  --build                 Build release control ELF before flash
  --no-build              Flash without rebuild
  --image PATH            ELF path (overrides auto-resolve)
  --chip CHIP             espflash chip (default: $MCU or esp32c5)
  --port DEV              Serial port (default: $ESPFLASH_PORT / $PORT / auto)
  --mgmt-ap-5ghz          5 GHz management SoftAP (ch 161); default is 2.4 GHz ch 11
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

normalize_board() {
  case "$1" in
    1|xiao|board-xiao) echo "board-xiao" ;;
    2|marauder|board-marauder) echo "board-marauder" ;;
    *)
      echo "error: invalid board: $1 (use xiao, marauder, 1, or 2)" >&2
      exit 1
      ;;
  esac
}

board_short() {
  case "$1" in
    board-xiao) echo "xiao" ;;
    board-marauder) echo "marauder" ;;
  esac
}

if [[ -n "$board_feature" ]]; then
  board_feature="$(normalize_board "$board_feature")"
else
  echo "Select control board profile:"
  echo "  1) XIAO ESP32-C5 (board-xiao)"
  echo "  2) Marauder C5 adapter (board-marauder)"
  read -rp "Choice [1]: " choice
  choice="${choice:-1}"
  board_feature="$(normalize_board "$choice")"
fi

role="control-$(board_short "$board_feature")"
[[ "$mgmt_ap_5ghz" == true ]] && role="${role}-5ghz"

if [[ "$mgmt_ap_5ghz" == true ]]; then
  echo "profile: 5 GHz management AP (mgmt-ap-5ghz, ch 161)"
fi

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
  build_features="control ${board_feature}"
  [[ "$mgmt_ap_5ghz" == true ]] && build_features="${build_features} mgmt-ap-5ghz"
  suffix=""
  [[ "$mgmt_ap_5ghz" == true ]] && suffix=", mgmt-ap-5ghz"
  echo "building control (${board_feature}${suffix})..."
  cargo_build_args=(build --release --bin esp32-wardriver-control)
  [[ -n "$build_jobs" ]] && cargo_build_args+=(-j "$build_jobs")
  cargo_build_args+=(--features "${build_features}")
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
