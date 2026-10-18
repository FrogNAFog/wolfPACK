# Flashing — detailed reference — wolfPACK (ESP32-C5, Rust + ESP-IDF)

Detailed reference for building, flashing, erasing, and provisioning wolfPACK
firmware on ESP32-C5. **Start with [docs/ENV.md](ENV.md)** — it covers the
quick-start flashing path (prebuilt release kit, newcomer onboarding, Android
app, gateway) and the one-time host setup (toolchain install, `espup`,
`espflash`, `export-esp.sh`, `dialout`). This document assumes that setup is
done and goes deeper: PMK/provisioning internals, management SoftAP
credentials, erase/recovery, and operator troubleshooting.

This repo’s Rust target is pinned in `[.cargo/config.toml](../.cargo/config.toml)`:

- **Target triple**: `riscv32imac-esp-espidf`
- **MCU**: `esp32c5` (`MCU` env var)
- **ESP-IDF**: `v5.5.2` (`ESP_IDF_VERSION` env var)

## Roles (must pick one feature)

This crate uses mutually-exclusive Cargo features (`main.rs` will
`compile_error!` otherwise):

- **Control node**: `--features control`
- **Sniffer node**: `--features sniffer`
- **Gateway node**: `--features gateway`

## ESP-NOW PMK (generate before first build)

Fleet **control** and **sniffer** images encrypt ESP-NOW unicast traffic with a shared **16-byte PMK** baked in at **compile time** (see `[build.rs](../build.rs)`). If `.fleet/espnow_pmk.bin` is missing, `**cargo build` / `cargo check` will fail** with an explicit panic telling you to generate the PMK.

**Generate once per machine / clone** (from the **repository root**):

```bash
./tools/espnow-pmk gen
```

This runs `[wardriver-keygen](../wardriver-keygen/)` with an explicit **host** `rustc` triple (the repo’s `[.cargo/config.toml](.cargo/config.toml)` sets `build.target` to the ESP32 IDF triple, so a plain `cargo run --manifest-path wardriver-keygen/…` would try to link with `**ldproxy`** and fail on a PC).

**Remove** the PMK files:

```bash
./tools/espnow-pmk rm
```

**Manual equivalent** (any OS):

```bash
cargo run --manifest-path wardriver-keygen/Cargo.toml --bin espnow-pmk-gen \
  --target "$(rustc -vV | sed -n 's/^host: //p')" --
```

On **Windows**, use Git Bash / WSL for the script, or run the `cargo run … --target …` line in PowerShell with `--target` set to the `host:` line from `rustc -vV`.

It writes:


| Output                              | Purpose                                                                                                        |
| ----------------------------------- | -------------------------------------------------------------------------------------------------------------- |
| `.fleet/espnow_pmk.bin`             | 16 raw bytes consumed by `build.rs` (path is **gitignored**; do not commit)                                    |
| `.fleet/espnow_pmk.fingerprint.txt` | SHA-256 of the PMK (optional record; safe to share for “same binary secret?” checks without revealing the key) |


**Fleet rule:** every **control** and **sniffer** you want on the same air must be built from the **same** `espnow_pmk.bin`. Mixed PMKs → sniffers may hear broadcasts but **unicast** (e.g. `SnifferAck`) will not work end-to-end.

**Remove** the generated PMK (e.g. to force a new fleet secret):

```bash
./tools/espnow-pmk rm
```

After removal, run `./tools/espnow-pmk gen` again before building, or builds will fail until a PMK is provided.

**CI / headless builds:** materialize the key as a secret file instead of an env var — write `16 raw bytes` from your CI secret to `.fleet/espnow_pmk.bin` before building (e.g. `echo "$FLEET_PMK_HEX" | xxd -r -p > .fleet/espnow_pmk.bin`). Do not commit real fleet keys.

## Management SoftAP credentials

Control and sniffer images embed **management SoftAP** SSID and password at **compile time** (`wardriver-shared/build.rs`). Sniffers use these credentials for the HTTP bulk-dump path when control has not pushed newer values over ESP-NOW.

**Optional fleet file:** `.fleet/mgmt_ap.toml` (under the gitignored `.fleet/` directory). Generate the default template from the repository root:

```bash
./tools/mgmt-ap gen
```

This writes:

```toml
ssid = "wolfPACK"
password = "deflock123"
```

Edit the file before building if you want non-default credentials. Re-run `gen` to reset the template (overwrites the file).

**CI / headless builds:** set both `WARDRIVER_MGMT_AP_SSID` and `WARDRIVER_MGMT_AP_PASS` in the environment; `build.rs` prefers that pair when both are set. If the file is missing and neither env var is set, builds use `wolfPACK` / `deflock123`.

**Fleet rule:** every **control** and **sniffer** on the same fleet should be built from the **same** `.fleet/mgmt_ap.toml` (or the same env pair) so management SoftAP credentials match across images.

**5 GHz management AP:** add Cargo feature `mgmt-ap-5ghz` on **control and sniffer** builds to use ESP-NOW channel 161 instead of the default 2.4 GHz channel 11. Sniffers do not run a SoftAP, but they boot on `CONTROL_AP_CHANNEL` for hello/discovery before adopt; that must match the control node's management channel. After welcome, sniffers follow `control_channel` from ESP-NOW frames.

**Manual equivalent** (any OS):

```bash
cargo run --manifest-path wardriver-keygen/Cargo.toml --bin mgmt-ap-gen \
  --target "$(rustc -vV | sed -n 's/^host: //p')" --
```

## Build (release recommended)

Per-role build commands and ELF output paths are in [docs/ENV.md](ENV.md)
(Build / flash / monitor). The details that matter here:

- Build one role per invocation using `--bin esp32-wardriver-<role>` with that role's feature.
- Control and sniffer use **separate binary names**, so a release build of one role does not overwrite the other.
- Debug builds work but are large/slow; release is the normal workflow.
- The gateway builds the same way (`esp32-wardriver-gateway`, `--features gateway` — see ENV.md, Gateway node).

## Flash + monitor

Primary entry points (from the repo root, after `. ~/export-esp.sh`):

```bash
# Control: board picker (XIAO vs Marauder), build, flash, monitor
./tools/wardriver flash-control

# Sniffer fleet: parallel flash all connected ports
./tools/wardriver flash-sniffers --batch

# Sniffer fleet: one USB cable, plug/unplug between devices
./tools/wardriver flash-sniffers --sequential
```

`flash-control` options (see `tools/flash-control.sh`):

- `--board xiao|marauder` — skip the interactive picker
- `--mgmt-ap-5ghz` — 5 GHz management SoftAP (ch 161); default is 2.4 GHz ch 11
- `--no-build` — flash an existing release ELF (warns)
- `--port DEV` — serial port (`$ESPFLASH_PORT` / `$PORT` also work)
- `--no-monitor` — flash only

`flash-sniffers` requires `--batch` or `--sequential`. Batch mode accepts `--ports` (comma list) or auto-detects via `espflash list-ports`; `--max-parallel` defaults to 4. Build is off by default unless the ELF is missing or you pass `--build`. Use `--mgmt-ap-5ghz` when the control node was flashed with 5 GHz management AP (same boot ESP-NOW channel).

Prefer a browser over a terminal? The [Web flasher](Web%20Flasher.md) flashes from the same release artifacts with no toolchain (Chromium or Edge required).

## Release zip (no CI)

Maintainers build a standalone operator kit locally. The zip lands in `release/` (gitignored). **Do not commit** `release/` or firmware ELFs to the main repo — upload the zip via [GitHub Releases](https://docs.github.com/en/repositories/releasing-projects-on-github/managing-releases-in-a-repository) or another file host.

### Maintainer: build the zip

```bash
. ~/export-esp.sh
./tools/espnow-pmk gen    # once per fleet PMK, if needed
./tools/make-release-zip.sh
# or: ./tools/wardriver make-release
```

Output: `release/wolfPACK-v<version>-esp32c5.zip`. Flags: `--skip-build`, `--output PATH`, `--no-zip` (keep temp staging dir), `--test-host`.

### Operator: flash from the zip

1. Unzip anywhere (e.g. `unzip release/wolfPACK-v0.1.0-esp32c5.zip`).
2. `cd wolfPACK-v0.1.0`
3. `./tools/install-flash-tools.sh` — installs `espflash` only (no Rust/ESP-IDF build toolchain).
4. `./tools/wardriver flash-control` — board picker (XIAO vs Marauder).
5. `./tools/wardriver flash-sniffers --batch` or `--sequential`.
6. `./tools/wardriver flash-gateway` — the PackLink SoftAP node for the phone app.

Flash scripts find ELFs under `firmware/` automatically (`tools/flash-image-resolve.sh`). All boards in a fleet must use firmware from the **same** zip (shared ESP-NOW PMK and `proto_version`). The zip's fleet key is generated fresh per release build (`./tools/make-release-zip.sh`, default rotated PMK; `--pmk-from-fleet` builds with your private key instead).

Windows: use WSL2 with USB passthrough; native Windows is untested.

Cargo-based flash (uses the `runner = espflash flash --monitor` in `[.cargo/config.toml](.cargo/config.toml)`):

```bash
./tools/wardriver flash control              # control + board-xiao (default)
./tools/wardriver flash sniffer              # sniffer release image
BOARD=board-marauder ./tools/wardriver flash control
```

Cargo aliases (same as `flash control` / `flash sniffer`):

```bash
cargo flash-xiao                             # control + board-xiao
cargo flash-sniffer                            # sniffer
cargo flash-control                            # control (no board feature; pick board-xiao in features if needed)
```

**How Cargo picks a binary:** `[features]` only change which code compiles. With two `[[bin]]` entries, `cargo run` needs either `default-run` (one fixed name) or `--bin`. There is no “if `sniffer` feature, run sniffer bin” rule. This crate sets `default-run = "esp32-wardriver-control"`, so plain `cargo run --features "control board-xiao"` works for control. Sniffer uses `cargo flash-sniffer`, `./tools/wardriver flash sniffer`, or an explicit `--bin`.

**Fleet unbind without NVS erase:** hold the XIAO BOOT button for 5 seconds on control, gateway, or sniffer. See [docs/FLEET_UNBIND.md](FLEET_UNBIND.md).

Control (manual cargo):

```bash
. ~/export-esp.sh
cargo run --release --features "control board-xiao"
# or: cargo flash-xiao
# or: ./tools/wardriver flash control
```

Sniffer:

```bash
. ~/export-esp.sh
cargo flash-sniffer
# or: ./tools/wardriver flash sniffer
# or: cargo run --release --bin esp32-wardriver-sniffer --features sniffer
```

### Flash an already-built ELF (manual)

Pick the right `--port` for your machine (Linux often `/dev/ttyUSB0` or `/dev/ttyACM0`).

Control:

```bash
espflash flash --monitor \
  --port /dev/ttyUSB0 \
  --chip esp32c5 \
  target/riscv32imac-esp-espidf/release/esp32-wardriver-control
```

Sniffer:

```bash
espflash flash --monitor \
  --port /dev/ttyUSB0 \
  --chip esp32c5 \
  target/riscv32imac-esp-espidf/release/esp32-wardriver-sniffer
```

### List serial ports

```bash
espflash list-ports
```

## Monitor only (no flash)

```bash
espflash monitor --port /dev/ttyUSB0
```

## Erase flash / NVS

### Cargo shortcuts (recommended)

This repo defines Cargo aliases so you can erase without retyping chip/partition args:

```bash
# Full chip erase (wipes firmware + all partitions)
cargo erase-flash

# NVS-only erase (keeps flashed app; clears saved config/state)
cargo erase-nvs
```

Port selection:

- If you don’t set a port, `espflash` will try to auto-detect one.
- To override, pass `--port` after `--` (Cargo forwards args after `--` to the underlying command):

```bash
cargo erase-flash -- --port /dev/ttyACM0
cargo erase-nvs -- --port /dev/ttyACM0
```

### Full chip erase (simplest; wipes firmware + all data partitions)

`espflash`:

```bash
espflash erase-flash --port /dev/ttyUSB0 --chip esp32c5
```

`esptool.py` (if you use Espressif’s Python tooling):

```bash
esptool.py --chip esp32c5 -p /dev/ttyUSB0 erase_flash
```

After a full erase, you must **reflash** firmware.

### Erase only the NVS partition (keeps the flashed app)

This repo’s partition table is `[partitions.csv](../partitions.csv)`. The NVS partition **label** is `nvs`.

```bash
espflash erase-parts --port /dev/ttyUSB0 --chip esp32c5 nvs \
  --partition-table partitions.csv
```

Notes:

- Erasing `**nvs**` clears WiFi credentials, sniffer learned `control_mac`, and other ESP-IDF NVS keys.
- If you also suspect RF calibration issues, you can additionally erase `phy_init`:

```bash
espflash erase-parts --port /dev/ttyUSB0 --chip esp32c5 nvs phy_init \
  --partition-table partitions.csv
```

## Common “nothing works after swapping roles” tip

If a board previously ran the **control** image, ESP-IDF may have **WiFi config persisted in NVS**. The sniffer firmware forces a STA-only WiFi configuration on boot, but if you’re chasing weird state, `**erase-parts nvs`** is the quickest clean reset.