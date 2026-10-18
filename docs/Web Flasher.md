# Web flasher - wolfPACK (ESP32-C5)

Flashing wolfPACK boards from a browser, with no terminal, no toolchain, and no
install. Open the page, pick a device role, create (or import) a fleet key, and
the page flashes the board over USB through [ESP Web
Tools](https://espressif.github.io/esp-web-flasher/). This is an alternative to
the USB + script path in [ENV.md](ENV.md); the terminal workflow is unchanged.

## What it is

Zero-terminal browser flashing for the **ESP32-C5**. The page flashes one merged
8 MB image per board at offset `0x0`, and exposes one image per flavor:

| Flavor | Board |
| --- | --- |
| `control-xiao` | control node on a Seeed XIAO C5 |
| `control-marauder` | control node on a Marauder C5 |
| `sniffer` | fleet sniffer |
| `gateway` | PackLink SoftAP node for the phone app |

The firmware is built once per release as a full merged image; the page only
swaps your fleet key into it and hands the result to the web installer.

## Browser and security requirements

- **Chromium desktop only** - Chrome or Edge. The installer drives WebSerial,
  which Firefox, Safari, and mobile browsers do not expose.
- **Secure context** - the page must be served over **HTTPS** (or `localhost` for
  local testing). A non-secure page cannot open the USB device.

A browser that fails either check is told so, and the page shows its built-in
fallback panel ("Web flashing unavailable here"). The panel points at the USB +
script quick start and at `docs/ENV.md` (Quick start), so unsupported visitors
still have a path forward.

## Fleet key flow

A **fleet key** is a 16-byte secret that binds every board you flash from this
browser into one private network. Each named fleet gets its own key.

**Create.** Type a fleet name and press **Create fleet**. The page generates 16
random bytes in-browser with `crypto.getRandomValues` and stores them in this
browser's `localStorage`. The raw key is never shown on the page - only its
**SHA-256 fingerprint** is, in the "Key fingerprint (SHA-256)" box.

**Export.** **Export .hex** downloads `wolfPACK-fleet-<name>.hex`. Its contents
are the 32-character hex key with a header line carrying the fleet name and
fingerprint:

```
# wolfPACK fleet <name>  fingerprint: <sha256>
<32 hex chars>
```

The key is the only thing that ever leaves your machine, and it does so only via
the file you choose to save or share.

**Import.** **Import .hex file** or **Import pasted key** loads a saved key into
the current fleet. Import checks:

- **Fingerprint match (hard reject):** if the file's header fingerprint does not
  match the imported bytes, it stops with "Import failed: key does not match its
  exported fingerprint - possible wrong file."
- **Name mismatch (confirm):** if the export's fleet name differs from the current
  one, it asks before storing the key under the current name.
- **Overwrite (confirm):** if the fleet already holds a different key, it asks
  before replacing it.

**Multiple fleets.** You can keep any number of named fleets; the chip list above
the fingerprint box switches between them.

**Keep your key safe.** The exported key is the only copy. As the page warns:
"Keep your key safe. Losing the exported key orphans this fleet." If you lose it,
boards already flashed with it can no longer be rejoined from this browser, and
there is no recovery path. To move to a new browser, export the key first, then
import it there. A full-chip erase wipes the firmware **and** the board-baked
fleet key (back to unprovisioned); browser-stored and exported fleet keys are
untouched - see the flashing flow below.

## Flashing flow

1. **Pick a role.** `sniffer` and `gateway` are one click each. For control
   boards, click **control**, then pick **Xiao Expansion Base**
   (`control-xiao`) or **C5 Marauder Adapter** (`control-marauder`). Selecting
   a flavor chip loads that flavor's manifest and its `pmks.json` sidecar entry.
2. **Verify, then patch.** The page hashes the 16 bytes at the recorded PMK offset
   inside the merged image and compares them to the sha256 recorded in the release
   `pmks.json` sidecar. This is the **version-skew guard**: it proves the image and
   the sidecar are from the same release before any key is patched in. On success
   it overwrites just those 16 bytes with your fleet key, and the status line
   reads "Ready - <flavor> patched with the <fleet> key."
3. **Install.** The patched image is handed to `esp-web-tools`. On first connect
   the browser asks for permission to use the USB device; click **Connect** and, if
   the board does not auto-reset into download mode, hold its BOOT button while the
   dialog opens.

**Erase on first install.** The page forces the installer's erase prompt
(`new_install_prompt_erase`), so a first install runs a full-chip erase. That erase
**wipes the key copy on the device** - to restore a board after an erase, reflash
it with the same fleet key. The erase does **not** touch the fleet keys stored in
this browser; those live in `localStorage` and are unaffected. There is no
standalone erase button: esp-web-tools 10.4.0 only offers full-chip erase
inside the installer's first-install dialog (NVS-only erase is not exposed at
all).

## Where the artifacts come from

Every release zip carries a `webflash/` directory containing the page, the merged
8 MB images, the per-flavor `manifest-<flavor>.json` files, and the `pmks.json`
sidecar. The page is fully self-contained, so operators can **self-host** it on
any HTTPS server (or `localhost`): serve the whole `webflash/` directory and open
`index.html`. A GitHub Pages publishing path is planned; its link will land here
once it is live.

## Troubleshooting

- **"installer library failed to load (CDN unreachable)"** - the esp-web-tools
  library loads from a CDN and the network is blocking it. Check your network or
  proxy and reload; if the CDN stays unreachable, use the USB + script path.
- **"version skew: pmks.json does not match the image - use the matching
  release"** - the merged image and the `pmks.json` sidecar are from two different
  releases. Use the artifacts from a single release zip; do not mix `webflash/`
  across releases.
- **WebSerial permission dialog** - on first connect the browser asks to use the
  USB device. Click **Connect**; if the board does not auto-reset, hold BOOT while
  the dialog is open so it enters download mode.
- **Port permissions** - on Linux, make sure your user can open serial ports
  (see the `dialout` note in [ENV.md](ENV.md) Quick start). Web flashing talks to
  the device over USB WebSerial, but the same user-permission requirement applies
  to the underlying access.
