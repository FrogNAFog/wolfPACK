#!/usr/bin/env bash
# Flash the sniffer ELF to one or many ESP32-C5 boards via espflash.
#
# Requires exactly one mode flag:
#   --batch       Flash all visible serial ports (USB hub / fleet bench)
#   --sequential  One cable: flash, unplug, plug next, repeat
#
# Match control 5 GHz fleets: pass --mgmt-ap-5ghz so boot ESP-NOW uses ch 161
# (same as control welcome `control_channel` before first adopt).
#
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=flash-image-resolve.sh
source "${SCRIPT_DIR}/flash-image-resolve.sh"
cd "${SCRIPT_DIR}/.."

# An external CARGO_TARGET_DIR breaks esp-idf-sys project-root resolution, so BLE
# sdkconfig is dropped and the sniffer loses its NimBLE FFI symbols. Set
# CARGO_WORKSPACE_DIR to the repo root so embuild short-circuits the pop-count.
export CARGO_WORKSPACE_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"

if ! command -v espflash >/dev/null 2>&1; then
  echo "error: espflash not found. Run: ./tools/install-flash-tools.sh" >&2
  exit 1
fi

mode=""
chip="${MCU:-esp32c5}"
image=""
image_explicit=false
ports_csv=""
max_parallel=4
do_build=false
mgmt_ap_5ghz=false
build_jobs=""

if [[ -n "${WARDRIVER_MGMT_AP_5GHZ:-}" && "${WARDRIVER_MGMT_AP_5GHZ}" != "0" ]]; then
  mgmt_ap_5ghz=true
fi

usage() {
  cat <<'EOF' >&2
usage: ./tools/flash-sniffers.sh --batch|--sequential [options]

  --batch              Flash every listed port (parallel, no monitor)
  --sequential         One USB cable: wait for device, flash, prompt for next

  --image PATH         Sniffer ELF (default: auto-resolve release zip or target/)
  --chip CHIP          espflash chip (default: $MCU or esp32c5)
  --ports a,b,c        Comma-separated ports; default: all from espflash list-ports
  --max-parallel N     Concurrent flashes in --batch mode (default: 4)
  --build              cargo build sniffer before flashing
  -j, --jobs N         Parallel cargo build jobs (default: .cargo/config.toml)
  --mgmt-ap-5ghz       Boot ESP-NOW on ch 161 (pair with 5 GHz control; default ch 11)
EOF
  exit 1
}

list_ports() {
  espflash list-ports 2>/dev/null | awk '/^\// { print $1 }'
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --batch|--sequential)
      [[ -z "$mode" ]] || { echo "error: pick only one of --batch or --sequential" >&2; exit 1; }
      mode="${1#--}"
      shift
      ;;
    --image) image="$2"; image_explicit=true; shift 2 ;;
    --chip) chip="$2"; shift 2 ;;
    --ports) ports_csv="$2"; shift 2 ;;
    --max-parallel) max_parallel="$2"; shift 2 ;;
    --build) do_build=true; shift ;;
    -j|--jobs)
      [[ $# -ge 2 ]] || { echo "error: -j requires a number" >&2; exit 1; }
      build_jobs="$2"
      shift 2
      ;;
    --mgmt-ap-5ghz) mgmt_ap_5ghz=true; shift ;;
    -h|--help) usage ;;
    *) echo "error: unknown option: $1" >&2; usage ;;
  esac
done

[[ -n "$mode" ]] || usage

sniffer_role="sniffer"
[[ "$mgmt_ap_5ghz" == true ]] && sniffer_role="sniffer-5ghz"

if [[ "$mgmt_ap_5ghz" == true ]]; then
  echo "profile: 5 GHz ESP-NOW boot channel (mgmt-ap-5ghz, ch 161)"
fi

if [[ "$image_explicit" == false ]]; then
  if image="$(resolve_firmware_image "$sniffer_role" 2>/dev/null)"; then
    :
  else
    image=""
  fi
fi

if [[ "$do_build" == true ]] || [[ -z "$image" ]] || [[ ! -f "$image" ]]; then
  build_features="sniffer"
  [[ "$mgmt_ap_5ghz" == true ]] && build_features="${build_features} mgmt-ap-5ghz"
  suffix=""
  [[ "$mgmt_ap_5ghz" == true ]] && suffix=" (mgmt-ap-5ghz)"
  echo "building sniffer release${suffix}..."
  cargo_build_args=(build --release --bin esp32-wardriver-sniffer)
  [[ -n "$build_jobs" ]] && cargo_build_args+=(-j "$build_jobs")
  cargo_build_args+=(--features "${build_features}")
  cargo "${cargo_build_args[@]}"
  image="$(resolve_firmware_image "$sniffer_role")"
fi

if [[ ! -f "$image" ]]; then
  echo "error: sniffer ELF not found (pass --build or extract a release zip)" >&2
  exit 1
fi

echo "using image: $image"

resolve_ports() {
  local -n _out=$1
  _out=()
  if [[ -n "$ports_csv" ]]; then
    IFS=',' read -r -a _out <<< "$ports_csv"
    return
  fi
  while IFS= read -r p; do
    [[ -n "$p" ]] && _out+=("$p")
  done < <(list_ports)
}

flash_port() {
  espflash flash --chip "$chip" --non-interactive --port "$1" "$image"
}

run_batch() {
  local -a ports=()
  resolve_ports ports
  if ((${#ports[@]} == 0)); then
    echo "error: no serial ports found (plug devices or pass --ports)" >&2
    exit 1
  fi

  echo "batch flash: ${#ports[@]} port(s), max-parallel=${max_parallel}"
  local -a ok=() fail=()
  local i=0
  while (( i < ${#ports[@]} )); do
    local -a pids=() batch_ports=()
    local j end=$(( i + max_parallel ))
    (( end > ${#ports[@]} )) && end=${#ports[@]}
    for (( j = i; j < end; j++ )); do
      local port="${ports[j]}"
      echo "flashing $port ..."
      flash_port "$port" &
      pids+=($!)
      batch_ports+=("$port")
    done
    local k=0
    for pid in "${pids[@]}"; do
      if wait "$pid"; then
        ok+=("${batch_ports[k]}")
      else
        fail+=("${batch_ports[k]}")
      fi
      ((k++)) || true
    done
    i=$end
  done

  echo ""
  echo "summary:"
  for p in "${ok[@]}"; do echo "  $p  OK"; done
  for p in "${fail[@]}"; do echo "  $p  FAIL"; done
  ((${#fail[@]} == 0))
}

run_sequential() {
  local count_ok=0

  while true; do
    local -a ports=()
    resolve_ports ports

    if ((${#ports[@]} == 0)); then
      echo "waiting for sniffer USB (plug in, Ctrl-C to quit)..."
      sleep 1
      continue
    fi

    if ((${#ports[@]} > 1)); then
      echo "error: ${#ports[@]} ports visible; unplug extras for --sequential" >&2
      printf '  %s\n' "${ports[@]}" >&2
      exit 1
    fi

    local port="${ports[0]}"
    echo "flashing $port ..."
    if flash_port "$port"; then
      ((count_ok++)) || true
    else
      echo "flash failed for $port" >&2
    fi

    read -rp "Connect next sniffer, then Enter (q to quit): " ans
    [[ "$ans" == "q" || "$ans" == "Q" ]] && break
  done

  echo "flashed ${count_ok} sniffer(s)"
  (( count_ok > 0 ))
}

case "$mode" in
  batch) run_batch ;;
  sequential) run_sequential ;;
  *) usage ;;
esac
