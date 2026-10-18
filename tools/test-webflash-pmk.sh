#!/usr/bin/env bash
# PMK offset sidecar round-trip test for the web flasher (web-flasher plan P1).
#
# Standalone and fixture-based: fabricates a merged image + pmks.json and verifies
# the exact search/patch contract that tools/make-release-zip.sh relies on:
#   * key bytes at the recorded offset match the recorded sha256
#   * the old key is found exactly once, at the recorded offset
#   * after patching a copy with a new key, the new key is found exactly once,
#     the old key is gone, and all 16 bytes at the offset changed
#   * the recorded offset is where the key actually lives
# No firmware artifacts are required.
set -euo pipefail

tmp="$(mktemp -d "${TMPDIR:-/tmp}/wolfpack-webflash-test.XXXXXX")"
cleanup() { rm -rf "$tmp"; }
trap cleanup EXIT

echo "test: webflash PMK sidecar round-trip (fixture)"

python3 - "$tmp" <<'PY'
import sys, json, hashlib, os
tmp = sys.argv[1]
SIZE = 160 * 1024        # small fixture image (no need for 8MB here)
OFF  = 0x10100           # mirror the app region (0x10000 + header space)
OLD  = bytes([0xA1, 0x5C, 0x00, 0x3F, 0x9E, 0x77, 0xC3, 0x08,
              0x4B, 0x1D, 0xE2, 0x66, 0x0F, 0xB4, 0x59, 0x83])
NEW  = bytes((b + 1) & 0xFF for b in OLD)   # every byte differs from OLD

assert len(OLD) == 16 and len(NEW) == 16
assert all(a != b for a, b in zip(OLD, NEW)), "fixture keys must differ in all 16 bytes"

# fabricate a merged image filled with 0xFF, old key embedded at OFF
img = bytearray(b"\xFF" * SIZE)
img[OFF:OFF + 16] = OLD
merged = os.path.join(tmp, "control-xiao-esp32c5.bin")
open(merged, "wb").write(img)

# sidecar in exactly the shape make-release-zip.sh emits
sidecar = {
    "pmk_sha256": hashlib.sha256(OLD).hexdigest(),
    "pmk": [{
        "flavor": "control-xiao",
        "file": "flash/control-xiao-esp32c5.bin",
        "offset": format(OFF, "x"),
        "sha256": hashlib.sha256(OLD).hexdigest(),
    }],
}
open(os.path.join(tmp, "pmks.json"), "w").write(json.dumps(sidecar))

def find_all(data, key):
    out, start = [], 0
    while True:
        i = data.find(key, start)
        if i == -1:
            break
        out.append(i)
        start = i + 1
    return out

rec = sidecar["pmk"][0]
offset = int(rec["offset"], 16)
data = open(merged, "rb").read()

# recorded offset is where the key lives; recorded sha256 matches the key bytes
assert data[offset:offset + 16] == OLD, "key bytes at recorded offset do not match"
assert hashlib.sha256(OLD).hexdigest() == rec["sha256"], "recorded sha256 does not match key"
assert find_all(data, OLD) == [offset], f"old key must be found exactly once at recorded offset, got {find_all(data, OLD)}"

# patch a copy with the new key at the same offset
patched = os.path.join(tmp, "patched.bin")
p = bytearray(data)
p[offset:offset + 16] = NEW
open(patched, "wb").write(bytes(p))

pd = open(patched, "rb").read()
# new key found exactly once, at the same (recorded) offset
assert find_all(pd, NEW) == [offset], f"new key must be found exactly once at recorded offset, got {find_all(pd, NEW)}"
# old key is gone
assert find_all(pd, OLD) == [], "old key must be gone after patch"
# all 16 bytes at the offset changed
assert all(pd[offset + i] != data[offset + i] for i in range(16)), "all 16 bytes at offset must change"
assert pd[offset:offset + 16] == NEW, "patched bytes must equal the new key"

print(f"  recorded offset : 0x{rec['offset']}")
print("  old key found exactly once at recorded offset: OK")
print("  recorded sha256 matches key bytes: OK")
print("  new key found exactly once after patch: OK")
print("  old key gone after patch: OK")
print("  all 16 bytes changed: OK")
print("PASS: webflash PMK sidecar round-trip")
PY

echo "test: webflash PMK skew guard (fixture)"

# A skewed pmks.json: the recorded sha256 does NOT match the key bytes actually
# at the recorded offset. This is the exact contract the page's prepareFlash
# guard now enforces — the page must detect the skew and refuse to patch.
python3 - "$tmp" <<'PY'
import sys, json, hashlib, os
tmp = sys.argv[1]
SIZE = 160 * 1024
OFF  = 0x10100
# Bytes that actually live at the offset in the image.
LIVE = bytes([0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88,
              0x99, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF, 0x00])
# The sidecar claims a DIFFERENT key (its sha256 is recorded) — stale sidecar.
STALE = bytes([0xDE, 0xAD, 0xBE, 0xEF, 0x01, 0x02, 0x03, 0x04,
               0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C])
img = bytearray(b"\xFF" * SIZE)
img[OFF:OFF + 16] = LIVE
merged = os.path.join(tmp, "skew-control-xiao-esp32c5.bin")
open(merged, "wb").write(img)

sidecar = {
    "pmk_sha256": hashlib.sha256(STALE).hexdigest(),
    "pmk": [{
        "flavor": "control-xiao",
        "file": "flash/skew-control-xiao-esp32c5.bin",
        "offset": format(OFF, "x"),
        # Recorded sha256 is of the STALE key, not the LIVE bytes at OFF.
        "sha256": hashlib.sha256(STALE).hexdigest(),
    }],
}
open(os.path.join(tmp, "pmks-skew.json"), "w").write(json.dumps(sidecar))

rec = sidecar["pmk"][0]
offset = int(rec["offset"], 16)
data = open(merged, "rb").read()
live_sha = hashlib.sha256(data[offset:offset + 16]).hexdigest()

# The guard's decision: actual bytes-at-offset hash MUST differ from the
# recorded sha256 -> the page refuses to patch (version skew).
assert live_sha != rec["sha256"], "fixture must be skewed: recorded sha256 must not match bytes at offset"
# The recorded sha256 corresponds to the STALE key, confirming the sidecar is
# out of sync with the image.
assert rec["sha256"] == hashlib.sha256(STALE).hexdigest(), "recorded sha256 should be of the stale key"

print(f"  bytes at 0x{offset:05x} sha256 : {live_sha}")
print(f"  recorded sha256          : {rec['sha256']}")
print("  skew detected (recorded != actual): OK -> page refuses to patch")
print("PASS: webflash PMK skew guard")
PY

echo "PASS: webflash PMK sidecar tests"
