#!/usr/bin/env bash
# Tier 3 SDIO: ban direct std::fs on /sdcard outside the SD worker VFS and boot mount.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

violations=0
while IFS= read -r -d '' file; do
  case " $file " in
    *"wardriver-control/src/storage/sd_vfs.rs"*) continue ;;
    *"wardriver-control/src/storage/sd_sessions.rs"*) continue ;;
    *"wardriver-control/src/control/boot_setup.rs"*) continue ;;
  esac
  if rg -q 'use std::fs\b|std::fs::' "$file"; then
    echo "ERROR: std::fs in $file (use SdIoHandle RPC / async enqueue)"
    violations=$((violations + 1))
  fi
done < <(find wardriver-control/src -name '*.rs' -print0)

if [ "$violations" -ne 0 ]; then
  echo "check-no-sd-fs: $violations violation(s)"
  exit 1
fi

echo "check-no-sd-fs: ok"
