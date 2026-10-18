#!/usr/bin/env bash
# Build release ELFs and package a standalone operator zip (no CI required).
#
# Output: release/wolfPACK-<version>-esp32c5.zip (gitignored; upload via GitHub Releases).
# Staging uses a temp dir; nothing is committed to the main repo.
#
#   . ~/export-esp.sh
#   ./tools/espnow-pmk gen    # if needed
#   ./tools/make-release-zip.sh
#
set -euo pipefail
cd "$(dirname "$0")/.."
REPO_ROOT="$PWD"

# An external CARGO_TARGET_DIR breaks esp-idf-sys project-root resolution, so BLE
# sdkconfig is dropped and the sniffer loses its NimBLE FFI symbols. Set
# CARGO_WORKSPACE_DIR to the repo root so embuild short-circuits the pop-count.
export CARGO_WORKSPACE_DIR="${REPO_ROOT}"

# TARGET_ELF_DIR must honor CARGO_TARGET_DIR: when builds are redirected (the
# normal verification setup uses the external SSD), artifacts land there, not
# in REPO_ROOT/target — where a stale, older-release ELF would otherwise be
# copied into the kit (symptom: webflash PMK offset search finds 0 hits).
TARGET_ROOT="${CARGO_TARGET_DIR:-${REPO_ROOT}/target}"
version_override=""
skip_build=false
no_zip=false
do_test_host=false
do_mgmt_ap_5ghz=false
zip_output=""

usage() {
  cat <<'EOF'
usage: ./tools/make-release-zip.sh [options]

  Builds control (xiao + marauder), sniffer, and gateway release ELFs,
  stages a zip kit, and writes release/wolfPACK-<version>-esp32c5.zip.

  --version V      Override version from Cargo.toml (e.g. 0.1.0 or v0.1.0)
  --skip-build     Package from existing target/ ELFs only (ignored under the
                   default rotated-PMK policy; a key change forces a rebuild)
  --output PATH    Zip file path (default: release/wolfPACK-<v>-esp32c5.zip)
  --no-zip         Keep staging dir and print its path (debug)
  --test-host      Run cargo test-host before building
  --mgmt-ap-5ghz   Also package control ELFs with 5 GHz management AP (ch 161)
  --pmk-from-fleet Build with the private .fleet/espnow_pmk.bin instead of a
                   freshly rotated release key (keep such zips off public
                   Releases — they contain your fleet key)
  -h, --help       Show this help

Prerequisites: . ~/export-esp.sh, zip on PATH, esptool (webflash image
merging; pip install esptool). PMK: rotated per release by default (see
--pmk-from-fleet).
EOF
  exit 0
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --version) version_override="$2"; shift 2 ;;
    --skip-build) skip_build=true; shift ;;
    --output) zip_output="$2"; shift 2 ;;
    --no-zip) no_zip=true; shift ;;
    --test-host) do_test_host=true; shift ;;
    --mgmt-ap-5ghz) do_mgmt_ap_5ghz=true; shift ;;
    --pmk-from-fleet) pmk_policy="fleet"; shift ;;
    -h|--help) usage ;;
    *) echo "error: unknown option: $1" >&2; exit 1 ;;
  esac
done

# --- prerequisites ---
if ! command -v zip >/dev/null 2>&1; then
  echo "error: zip not found (install zip package)" >&2
  exit 1
fi

if ! command -v ldproxy >/dev/null 2>&1 && [[ -z "${IDF_PATH:-}" ]]; then
  echo "error: ESP toolchain not loaded — run: . ~/export-esp.sh" >&2
  exit 1
fi

# webflash generation merges images with esptool (elf2image + merge_bin). Only
# rotated-PMK releases carry webflash/, so esptool is required only then.
if [[ "${pmk_policy:-rotated}" == "rotated" ]]; then
  if ! command -v esptool >/dev/null 2>&1 && ! command -v esptool.py >/dev/null 2>&1; then
    echo "error: esptool not found (pip install esptool) — needed to merge webflash images" >&2
    exit 1
  fi
  ESPTOOL="$(command -v esptool 2>/dev/null || command -v esptool.py 2>/dev/null)"
fi

# --- fleet PMK for the release build ---
# Default: rotate — generate a fresh 16-byte PMK just for this zip so published
# release kits never contain the operator's private .fleet/espnow_pmk.bin.
release_pmk_file=""
cleanup_pmk=false
pmk_policy="${pmk_policy:-rotated}"
pmk_fp="null"
if [[ "$pmk_policy" == "rotated" ]]; then
  release_pmk_file="$(mktemp "${TMPDIR:-/tmp}/wolfpack-release-pmk.XXXXXX.bin")"
  cleanup_pmk=true
  head -c 16 /dev/urandom > "$release_pmk_file"
  export WARDRIVER_FLEET_PMK_FILE="$release_pmk_file"
  # Build inputs change when the key changes; force a real build so cached ELFs
  # from a private-key build can never be packaged by accident.
  skip_build=false
  pmk_fp="\"$(sha256sum "$release_pmk_file" | awk '{print $1}')\""
  echo "fleet PMK: freshly rotated for this release (fingerprint ${pmk_fp//\"/})"
else
  if [[ ! -f ".fleet/espnow_pmk.bin" ]]; then
    echo "error: fleet PMK missing — run: ./tools/espnow-pmk gen" >&2
    exit 1
  fi
  echo "fleet PMK: using private .fleet/espnow_pmk.bin (--pmk-from-fleet) — keep this zip off public Releases"
fi

# --- version ---
read_version_from_cargo() {
  awk -F'"' '/^version = / { print $2; exit }' Cargo.toml
}

version="${version_override:-$(read_version_from_cargo)}"
version="${version#v}"
version_tag="v${version}"

pkg_name="wolfPACK-${version_tag}"

if [[ -z "$zip_output" ]]; then
  zip_output="${REPO_ROOT}/release/${pkg_name}-esp32c5.zip"
fi

# --- metadata for MANIFEST.json ---
proto_version="$(awk '/pub const PROTO_VERSION: u8 = / { print $6; exit }' wardriver-proto/src/lib.rs | tr -d ';')"
esp_idf_version="$(awk -F'"' '/ESP_IDF_VERSION = / { print $2; exit }' .cargo/config.toml)"

git_commit="null"
if git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
  git_commit="\"$(git rev-parse --short HEAD)\""
fi

built_at="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

# --- temp staging ---
stage="$(mktemp -d "${TMPDIR:-/tmp}/wolfpack-release.XXXXXX")"
cleanup() { rm -rf "$stage"; [[ "$cleanup_pmk" == true && -n "$release_pmk_file" ]] && rm -f "$release_pmk_file"; }
if [[ "$no_zip" == false ]]; then
  trap cleanup EXIT
fi

stage_root="${stage}/${pkg_name}"
mkdir -p "${stage_root}/firmware" "${stage_root}/tools"

build_and_copy() {
  local features="$1"
  local dest_name="$2"
  local src="${TARGET_ROOT}/riscv32imac-esp-espidf/release/esp32-wardriver-control"
  if [[ "$features" == *sniffer* ]]; then
    src="${TARGET_ROOT}/riscv32imac-esp-espidf/release/esp32-wardriver-sniffer"
  elif [[ "$features" == gateway ]]; then
    src="${TARGET_ROOT}/riscv32imac-esp-espidf/release/esp32-wardriver-gateway"
  fi

  if [[ "$skip_build" == false ]]; then
    echo "building: ${features} ..."
  fi

  case "$features" in
    "control board-xiao")
      [[ "$skip_build" == true ]] || cargo build --release --bin esp32-wardriver-control --features "control board-xiao"
      ;;
    "control board-xiao mgmt-ap-5ghz")
      [[ "$skip_build" == true ]] || cargo build --release --bin esp32-wardriver-control --features "control board-xiao mgmt-ap-5ghz"
      ;;
    "control board-marauder")
      [[ "$skip_build" == true ]] || cargo build --release --bin esp32-wardriver-control --features "control board-marauder"
      ;;
    "control board-marauder mgmt-ap-5ghz")
      [[ "$skip_build" == true ]] || cargo build --release --bin esp32-wardriver-control --features "control board-marauder mgmt-ap-5ghz"
      ;;
    sniffer)
      [[ "$skip_build" == true ]] || cargo build --release --bin esp32-wardriver-sniffer --features sniffer
      ;;
    "sniffer mgmt-ap-5ghz")
      [[ "$skip_build" == true ]] || cargo build --release --bin esp32-wardriver-sniffer --features "sniffer mgmt-ap-5ghz"
      ;;
    gateway)
      [[ "$skip_build" == true ]] || cargo build --release --bin esp32-wardriver-gateway --features gateway
      ;;
    *)
      echo "error: unknown build features: ${features}" >&2
      exit 1
      ;;
  esac

  if [[ ! -f "$src" ]]; then
    echo "error: missing ELF: $src (build failed or use without --skip-build)" >&2
    exit 1
  fi
  cp "$src" "${stage_root}/firmware/${dest_name}"
}

if [[ "$do_test_host" == true ]]; then
  echo "running cargo test-host..."
  cargo test-host
  echo "running cargo test-host-base (v5 wire compat)..."
  cargo test-host-base
fi

build_and_copy "control board-xiao" "esp32-wardriver-control-xiao"
build_and_copy "control board-marauder" "esp32-wardriver-control-marauder"
if [[ "$do_mgmt_ap_5ghz" == true ]]; then
  build_and_copy "control board-xiao mgmt-ap-5ghz" "esp32-wardriver-control-xiao-5ghz"
  build_and_copy "control board-marauder mgmt-ap-5ghz" "esp32-wardriver-control-marauder-5ghz"
  build_and_copy "sniffer mgmt-ap-5ghz" "esp32-wardriver-sniffer-5ghz"
fi
build_and_copy "sniffer" "esp32-wardriver-sniffer"
build_and_copy "gateway" "esp32-wardriver-gateway"

# --- MANIFEST.json ---
if [[ "$pmk_fp" == "null" ]]; then
  pmk_json="null"
else
  pmk_json="\"${pmk_fp}\""
fi

cat > "${stage_root}/MANIFEST.json" <<EOF
{
  "package": "wolfPACK",
  "version": "${version}",
  "proto_version": ${proto_version},
  "esp_idf_version": "${esp_idf_version}",
  "pmk_fingerprint_sha256": ${pmk_json},
  "git_commit": ${git_commit},
  "built_at": "${built_at}"
}
EOF

# --- operator docs ---
cat > "${stage_root}/README.md" <<EOF
# wolfPACK ${version_tag} (ESP32-C5)

Standalone flash kit. Do not mix ELFs from different release zips on one fleet.

## Requirements

- Linux or macOS (Windows: use WSL2; USB passthrough required)
- USB cable to each ESP32-C5 board

## Steps

1. Install flash tools (once per machine):

   ./tools/install-flash-tools.sh

2. Flash the control node (pick XIAO or Marauder when prompted):

   ./tools/wardriver flash-control

   For 5 GHz management SoftAP (channel 161) on the control node:

   ./tools/wardriver flash-control --mgmt-ap-5ghz

3. Flash sniffers (USB hub or one cable at a time):

   ./tools/wardriver flash-sniffers --batch
   # or: ./tools/wardriver flash-sniffers --sequential

   When control uses 5 GHz management AP, flash sniffers with the same profile:

   ./tools/wardriver flash-sniffers --batch --mgmt-ap-5ghz

4. Flash the gateway node (the PackLink SoftAP for the phone app):

   ./tools/wardriver flash-gateway

All control, sniffer, and gateway boards must run firmware from this zip so
ESP-NOW PMK and proto version (${proto_version}) match.

See FLASH.txt for a command cheat sheet.
EOF

cat > "${stage_root}/FLASH.txt" <<EOF
wolfPACK ${version_tag} — flash commands (run from this directory after unzip)

  ./tools/install-flash-tools.sh
  ./tools/wardriver flash-control
  ./tools/wardriver flash-control --mgmt-ap-5ghz
  ./tools/wardriver flash-sniffers --batch
  ./tools/wardriver flash-sniffers --batch --mgmt-ap-5ghz
  ./tools/wardriver flash-sniffers --sequential
  ./tools/wardriver flash-gateway

Linux dialout (if serial permission denied):
  sudo usermod -aG dialout \$USER && newgrp dialout
EOF

# --- copy flash scripts into the zip ---
for f in install-flash-tools.sh flash-image-resolve.sh flash-control.sh flash-gateway.sh flash-sniffers.sh wardriver; do
  cp "${REPO_ROOT}/tools/${f}" "${stage_root}/tools/${f}"
  chmod +x "${stage_root}/tools/${f}"
done

# --- webflash (esp-web-tools) generation ---
# Merged single-image firmware, per-flavor manifests, and a PMK offset sidecar
# for the GitHub-Pages web flasher (web-flasher plan, phase P1).
#
# Policy gate: only rotated-PMK releases carry webflash/. A --pmk-from-fleet
# zip MUST never contain webflash/ — it would embed the operator's private
# fleet key. The gate below makes that impossible.
resolve_build_artifact() {
  # $1 = filename, $2 = REPO_ROOT-relative top-level path; fall back deep
  # into the embuild tree (only valid when unique — esp-idf-sys builds once
  # per feature-hash, so multiple matches mean "cannot guess" and we fail
  # loudly: stale artifact selection is worse than none for a release).
  local target_root="${CARGO_TARGET_DIR:-${REPO_ROOT}/target}"
  # $2 carries a REPO_ROOT-relative "target/" prefix; strip it for target_root.
  local rel="${2#target/}"
  if [[ -f "${target_root}/${rel}" ]]; then
    echo "${target_root}/${rel}"
    return 0
  fi
  if [[ -f "${REPO_ROOT}/$2" ]]; then
    echo "${REPO_ROOT}/$2"
    return 0
  fi
  local matches
  matches="$(find "${target_root}" -name "$1" -path '*/out/build/*' 2>/dev/null | sort)"
  if [[ -z "${matches}" ]]; then
    echo "resolve_build_artifact: no build artifact found for '$1' (expected at '${REPO_ROOT}/$2')" >&2
    return 1
  fi
  local count
  count="$(printf '%s\n' "${matches}" | wc -l)"
  if [[ "${count}" -gt 1 ]]; then
    {
      echo "resolve_build_artifact: multiple build artifacts match '$1'; refusing to guess. Candidates:"
      printf '%s\n' "${matches}"
    } >&2
    return 1
  fi
  echo "${matches}"
}

# The rotated release key is freshly generated per release, so it is unique by
# construction and appears exactly once in each merged image. Print the hex
# offset; fail loudly (non-zero exit) if it is not found exactly once.
pmk_find_offset() {
  local bin="$1" key="$2"
  python3 - "$bin" "$key" <<'PY'
import sys
bpath, kpath = sys.argv[1], sys.argv[2]
data = open(bpath, "rb").read()
key = open(kpath, "rb").read()
if len(key) != 16:
    print(f"error: webflash: PMK key must be 16 bytes, got {len(key)}", file=sys.stderr)
    sys.exit(1)
found, start = [], 0
while True:
    i = data.find(key, start)
    if i == -1:
        break
    found.append(i)
    start = i + 1
if len(found) != 1:
    print(f"error: webflash: PMK pattern found {len(found)} times in {bpath} (expected exactly 1)", file=sys.stderr)
    sys.exit(1)
print(format(found[0], "x"))
PY
}

generate_webflash() {
  local flavor elf_name app_bin bin offset staged_elf entry
  local flash_dir="${stage_root}/webflash/flash"
  mkdir -p "$flash_dir"

  # esptool 5.x renamed --fill-flash-size to --pad-to-size; pick per version.
  # 4.x spelling "--fill-flash-size" is the merge_bin padding flag — verified
  # against esptool 4.7.0 source: merge_bin (esptool/cmds.py) consumes
  # args.fill_flash_size, and __init__.py defines --fill-flash-size for
  # merge_bin (while --pad-to-size is the elf2image flag). Do not "fix" this to
  # --pad-to-size on 4.x; merge_bin rejects it there.
  local esptool_ver esptool_major fill_flag
  esptool_ver="$(${ESPTOOL} version 2>/dev/null | tail -1 || true)"
  esptool_major="$(printf '%s' "$esptool_ver" | grep -oE '[0-9]+' | head -1)"
  [[ "$esptool_major" =~ ^[0-9]+$ ]] || esptool_major=4
  if [[ "$esptool_major" -ge 5 ]]; then fill_flag="--pad-to-size"; else fill_flag="--fill-flash-size"; fi

  local BOOTLOADER_BIN PARTITION_TABLE_BIN
  BOOTLOADER_BIN="$(resolve_build_artifact "bootloader.bin" "target/riscv32imac-esp-espidf/release/bootloader.bin")"
  PARTITION_TABLE_BIN="$(resolve_build_artifact "partition-table.bin" "target/riscv32imac-esp-espidf/release/partition-table.bin")"
  if [[ -z "$BOOTLOADER_BIN" || -z "$PARTITION_TABLE_BIN" ]]; then
    echo "error: webflash: missing bootloader.bin / partition-table.bin build artifacts" >&2
    echo "  run a build once so embuild emits them under ${TARGET_ROOT}/ (riscv32imac-esp-espidf/release/)" >&2
    exit 1
  fi

  local pmk_entries=""
  for flavor in control-xiao control-marauder sniffer gateway; do
    case "$flavor" in
      control-xiao)     elf_name="esp32-wardriver-control-xiao" ;;
      control-marauder) elf_name="esp32-wardriver-control-marauder" ;;
      sniffer)          elf_name="esp32-wardriver-sniffer" ;;
      gateway)          elf_name="esp32-wardriver-gateway" ;;
    esac
    bin="${flash_dir}/${flavor}-esp32c5.bin"
    staged_elf="${stage_root}/firmware/${elf_name}"
    if [[ ! -f "$staged_elf" ]]; then
      echo "error: webflash: missing staged ELF for ${flavor}: ${staged_elf}" >&2
      exit 1
    fi

    echo "webflash: ${flavor} — elf2image + merge_bin (${fill_flag} 8MB)"
    app_bin="${stage}/webflash-app-${flavor}.bin"
    ${ESPTOOL} --chip esp32c5 elf2image --flash-freq 80m --flash-mode dio --flash-size 8MB -o "$app_bin" "$staged_elf"
# ESP32-C5 (ESP-IDF): bootloader at 0x2000 (the IDF build's own flash_args
# layout), partition table 0x8000, app 0x10000. The merged image is flashed
# at offset 0x0 as a whole; 0x0..0x2000 stays 0xFF.
    ${ESPTOOL} --chip esp32c5 merge_bin --flash-freq 80m --flash-mode dio --flash-size 8MB ${fill_flag} 8MB -o "$bin" \
      0x2000 "$BOOTLOADER_BIN" 0x8000 "$PARTITION_TABLE_BIN" 0x10000 "$app_bin"
    rm -f "$app_bin"

    # PMK offset sidecar: rotated key is unique by construction -> exactly once.
    offset="$(pmk_find_offset "$bin" "$release_pmk_file")"

    entry="    {\"flavor\": \"${flavor}\", \"file\": \"flash/${flavor}-esp32c5.bin\", \"offset\": \"0x${offset}\", \"image_base\": \"0x10000\", \"sha256\": \"${pmk_fp//\"/}\"}"
    if [[ -n "$pmk_entries" ]]; then pmk_entries="${pmk_entries},"$'\n'"${entry}"; else pmk_entries="${entry}"; fi

    cat > "${stage_root}/webflash/manifest-${flavor}.json" <<JSON
{
  "name": "wolfPACK ${flavor}",
  "version": "${version_tag}",
  "chip": "esp32c5",
  "flash_size": "8MB",
  "new_install_prompt_erase": true,
  "new_install_improv_wait_time": 0,
  "builds": [
    {
      "chipFamily": "ESP32-C5",
      "serialType": "cdc",
      "parts": [{ "path": "flash/${flavor}-esp32c5.bin", "offset": 0 }]
    },
    {
      "chipFamily": "ESP32-C5",
      "serialType": "uart",
      "parts": [{ "path": "flash/${flavor}-esp32c5.bin", "offset": 0 }]
    }
  ]
}
JSON
  done

  cat > "${stage_root}/webflash/pmks.json" <<JSON
{
  "pmk_sha256": "${pmk_fp//\"/}",
  "pmk": [
${pmk_entries}
  ]
}
JSON

  # Flasher page ships from the repo (tools/webflash/index.html) so the release
  # kit carries the real UI (web-flasher plan, phase P2).
  cp "${REPO_ROOT}/tools/webflash/index.html" "${stage_root}/webflash/index.html"
}

if [[ "$pmk_policy" == "rotated" ]]; then
  generate_webflash
  echo "webflash: generated ${stage_root}/webflash/ (4 merged images + manifests + pmks.json)"
else
  echo "webflash: skipped (${pmk_policy}) — fleet-key zips must never carry webflash/"
fi

if [[ "$no_zip" == true ]]; then
  trap - EXIT
  echo "staged (not zipped): ${stage_root}"
  exit 0
fi

mkdir -p "$(dirname "$zip_output")"
rm -f "$zip_output"
(
  cd "$stage"
  zip -r "$zip_output" "${pkg_name}"
)

echo "release zip: ${zip_output}"
echo "upload to GitHub Releases manually — do not commit release/ to git"
