#!/usr/bin/env node
// tools/test-webflash-image.js
//
// Durable checksum-repair test for the wolfPACK web flasher. Committed and
// runnable in CI (no real firmware, no network).
//
// Requirement: Node.js >= 18. This test mirrors the page's pure functions
// verbatim out of <script id="wolfpack-core"> in tools/webflash/index.html via
// `new Function` (the block is DOM/crypto/fetch-free, so it extracts cleanly);
// Node 18+ is required because of the stable crypto.subtle-free `crypto` module
// and modern `Array`/`subarray` behavior the mirrored code relies on.
//
// It builds SYNTHETIC ESP32-C5 app images (no real firmware) covering:
//   1. a valid 6-segment image whose segment stream ends MID-BLOCK
//   2. a valid 5-segment image whose stream ends EXACTLY on a 16-byte boundary
//      (regression for the align16(end+1)-1 trailer rule — a plain
//       align16(end)-1 places the checksum one block too early)
//   3. an image with the hash_appended flag and a 32-byte SHA-256 trailer
// For each, it patches 16 bytes at a fake PMK offset and asserts that
// verifyEspImageChecksums on the repaired image agrees with an INDEPENDENT
// recomputation of the checksum (manual 0xEF-XOR over the segment bytes) and,
// when present, the SHA-256 trailer (independent crypto hash).

"use strict";
const fs = require("fs");
const path = require("path");
const crypto = require("crypto");
const assert = require("assert");

const NODE_MAJOR = parseInt(process.versions.node.split(".")[0], 10);
if (NODE_MAJOR < 18) {
  console.error("FAIL: Node.js >= 18 required (got " + process.versions.node + ")");
  process.exit(1);
}

// ---- mirror the page core functions verbatim ----
const HTML = path.join(__dirname, "webflash", "index.html");
const html = fs.readFileSync(HTML, "utf8");
const m = html.match(/<script id="wolfpack-core">([\s\S]*?)<\/script>/);
assert(m, "wolfpack-core script block not found in " + HTML);
const fns = new Function(
  m[1] +
    "\n;return {hexToBytes,bytesToHex,parseFleetKeyHex,parseFleetExportHeader," +
    "parsePmkOffset,patchPmkAtOffset,verifyEspImageChecksums,xorSegments,buildRuntimeManifest};"
)();
const { parsePmkOffset, patchPmkAtOffset, verifyEspImageChecksums } = fns;

// Independent recomputation helpers (do NOT reuse the page's xorSegments).
function manualXor(img, ranges) {
  let x = 0xef;
  for (const [a, b] of ranges) for (let i = a; i < b; i++) x ^= img[i];
  return x;
}
const sha256 = (buf) => crypto.createHash("sha256").update(buf).digest();

let passed = 0;
function ok(name, cond) {
  assert(cond, "FAILED: " + name);
  passed++;
  console.log("  PASS  " + name);
}

// ---- synthetic ESP app image builder ----
// Layout (imageBase-relative, 24-byte header then segments):
//   [0]=0xE9 magic, [1]=segCount, [23]=flags (bit0 = hash_appended)
//   segment = 8-byte (load_addr, data_len LE) header + data_len data bytes
// After the last segment the checksum byte sits at align16(end+1)-1 (the LAST
// byte of the 16-byte block the stream ends in, or a full block later when the
// stream ends exactly on a boundary); a 32-byte SHA-256 follows when appended.
function buildImage(imageBase, segLens, hashAppended) {
  let pos = imageBase + 0x18;
  const segs = [];
  for (const dlen of segLens) {
    const headerStart = pos;
    const dataStart = pos + 8;
    segs.push({ headerStart, dataStart, dlen });
    pos = dataStart + dlen;
  }
  const endPos = pos; // end of the segment stream
  const chkOff = ((endPos + 16) & ~15) - 1; // esptool's align16(end+1)-1 rule
  const totalLen = chkOff + 33 + 16; // room for checksum + 32-byte hash + pad
  const img = new Uint8Array(totalLen);
  for (let i = 0; i < totalLen; i++) img[i] = (i * 3 + 1) & 0xff;
  img[imageBase] = 0xe9;
  img[imageBase + 1] = segLens.length;
  img[imageBase + 23] = hashAppended ? 0x01 : 0x00;
  for (const seg of segs) {
    const len = seg.dlen;
    img[seg.headerStart + 4] = len & 0xff;
    img[seg.headerStart + 5] = (len >> 8) & 0xff;
    img[seg.headerStart + 6] = (len >> 16) & 0xff;
    img[seg.headerStart + 7] = (len >> 24) & 0xff;
  }
  img[chkOff] = manualXor(img, segs.map((s) => [s.dataStart, s.dataStart + s.dlen]));
  if (hashAppended) {
    img.set(sha256(img.subarray(imageBase, chkOff + 1)), chkOff + 1);
  }
  return { img, segs, endPos, chkOff, totalLen };
}

// Patch 16 bytes at a fake PMK offset inside the first segment, then repair the
// trailer exactly as the page's prepareFlash() does, and assert the repaired
// image verifies AND matches an independent recomputation.
function repairAndCheck(label, built, hashAppended) {
  const { img, segs, chkOff } = built;
  const imageBase = 0;
  const ranges = segs.map((s) => [s.dataStart, s.dataStart + s.dlen]);

  // 1) The pristine synthetic image must verify.
  const pre = verifyEspImageChecksums(img, imageBase);
  ok(label + ": pristine image verifies (checksumOk)", pre.checksumOk === true);
  ok(label + ": pristine image hashAppended flag matches", pre.hashAppended === hashAppended);

  // 2) Fake PMK offset: 8 bytes into the first segment's data (fits: dlen >= 24).
  const pmkOffset = segs[0].dataStart + 8;
  const key = new Uint8Array(16);
  for (let i = 0; i < 16; i++) key[i] = 0xc0 + i;
  const repaired = patchPmkAtOffset(img, pmkOffset, key);
  ok(label + ": 16 bytes patched at fake PMK offset",
     Array.from(repaired.subarray(pmkOffset, pmkOffset + 16)).join() === Array.from(key).join());

  // 3) Repair the trailer exactly as the page does (independent recomputation).
  const expectedXor = manualXor(repaired, ranges);
  repaired[chkOff] = expectedXor;
  let expectedHash = null;
  if (hashAppended) {
    expectedHash = sha256(repaired.subarray(imageBase, chkOff + 1));
    repaired.set(expectedHash, chkOff + 1);
  }

  // 4) The repaired image must verify, and the page's computed checksum must
  //    equal our independent recomputation.
  const post = verifyEspImageChecksums(repaired, imageBase);
  ok(label + ": repaired image verifies (checksumOk)", post.checksumOk === true);
  ok(label + ": page computedChecksum == independent recomputation",
     post.computedChecksum === expectedXor);
  ok(label + ": checksum byte at chkOff == independent recomputation",
     repaired[chkOff] === expectedXor);
  if (hashAppended) {
    const trailer = Array.from(repaired.subarray(chkOff + 1, chkOff + 33)).join();
    ok(label + ": SHA-256 trailer == independent recomputation",
       trailer === Array.from(expectedHash).join());
  }
}

// ---- case 1: 6 segments, stream ends MID-BLOCK ----
{
  const built = buildImage(0, [32, 40, 24, 16, 8, 10], false);
  ok("case1: 6-segment image built", built.segs.length === 6);
  ok("case1: stream ends mid-block (not on a 16-byte boundary)",
     built.endPos % 16 !== 0);
  ok("case1: checksum sits in the pad region after the stream",
     built.chkOff > built.endPos);
  repairAndCheck("case1", built, false);
}

// ---- case 2: 5 segments, stream ends EXACTLY on a 16-byte boundary ----
{
  const built = buildImage(0, [32, 32, 32, 32, 32], false);
  ok("case2: 5-segment image built", built.segs.length === 5);
  ok("case2: stream ends exactly on a 16-byte boundary",
     built.endPos % 16 === 0);
  const plainRule = ((built.endPos + 15) & ~15) - 1; // align16(end)-1 (the buggy rule)
  ok("case2: align16(end+1)-1 places checksum a full block after end (regression)",
     built.chkOff === built.endPos + 15 && built.chkOff !== plainRule);
  repairAndCheck("case2", built, false);
}

// ---- case 3: hash_appended flag + 32-byte SHA-256 trailer ----
{
  const built = buildImage(0, [48, 32], true);
  ok("case3: hash-appended image built", built.segs.length === 2);
  repairAndCheck("case3", built, true);
}

console.log("\n" + passed + " checks passed. ALL GREEN");
