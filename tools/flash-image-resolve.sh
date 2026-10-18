#!/usr/bin/env bash
# Shared firmware path resolution for flash-control.sh and flash-sniffers.sh.
# Source from tools/; do not execute directly.
#
# Roles: control-xiao | control-marauder | control-xiao-5ghz | control-marauder-5ghz | sniffer | sniffer-5ghz | gateway
# Priority: caller --image > WARDRIVER_IMAGE > extracted release zip > target/ ELF.

# Directory containing this file (tools/).
WARDRIVER_TOOLS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Repo root when developing; package root when MANIFEST.json sits beside tools/.
wardriver_package_root() {
  if [[ -f "${WARDRIVER_TOOLS_DIR}/../MANIFEST.json" ]]; then
    cd "${WARDRIVER_TOOLS_DIR}/.." && pwd
    return
  fi
  cd "${WARDRIVER_TOOLS_DIR}/.." && pwd
}

# Map role name to firmware filename under firmware/.
_firmware_basename() {
  case "$1" in
    control-xiao) echo "esp32-wardriver-control-xiao" ;;
    control-marauder) echo "esp32-wardriver-control-marauder" ;;
    control-xiao-5ghz) echo "esp32-wardriver-control-xiao-5ghz" ;;
    control-marauder-5ghz) echo "esp32-wardriver-control-marauder-5ghz" ;;
    sniffer) echo "esp32-wardriver-sniffer" ;;
    sniffer-5ghz) echo "esp32-wardriver-sniffer-5ghz" ;;
    gateway) echo "esp32-wardriver-gateway" ;;
    *)
      echo "flash-image-resolve: unknown role: $1" >&2
      return 1
      ;;
  esac
}

# True when path points at a prebuilt release ELF (under firmware/).
firmware_image_is_release() {
  [[ "$1" == */firmware/* ]]
}

# Print absolute path to the firmware ELF for role, or exit 1.
resolve_firmware_image() {
  local role="$1"
  local base repo_root pkg_root path

  base="$(_firmware_basename "$role")" || return 1

  # Optional global override (same ELF for any role — rarely used).
  if [[ -n "${WARDRIVER_IMAGE:-}" && -f "${WARDRIVER_IMAGE}" ]]; then
    echo "${WARDRIVER_IMAGE}"
    return 0
  fi

  pkg_root="$(wardriver_package_root)"

  # Extracted release zip: MANIFEST.json at package root, ELFs in firmware/.
  if [[ -f "${pkg_root}/MANIFEST.json" ]]; then
    path="${pkg_root}/firmware/${base}"
    if [[ -f "$path" ]]; then
      echo "$path"
      return 0
    fi
  fi

  # Developer fallback: cargo release output under repo target/.
  repo_root="${WARDRIVER_TOOLS_DIR}/.."
  case "$role" in
    sniffer|sniffer-5ghz)
      path="${repo_root}/target/riscv32imac-esp-espidf/release/esp32-wardriver-sniffer"
      ;;
    control-xiao|control-marauder|control-xiao-5ghz|control-marauder-5ghz)
      path="${repo_root}/target/riscv32imac-esp-espidf/release/esp32-wardriver-control"
      ;;
  esac
  if [[ -f "$path" ]]; then
    echo "$path"
    return 0
  fi

  echo "flash-image-resolve: no firmware image for role ${role}" >&2
  return 1
}
