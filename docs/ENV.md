# Toolchain setup — wolfPACK (ESP32-C5)

This guide covers two paths. Pick the one that matches what you want to do:

- **I want to flash prebuilt firmware onto boards** → start at [Quick start: flash a board](#quick-start-flash-a-board). No Rust toolchain needed.
- **I want to build the firmware from source** → start at [Building from source](#building-from-source).

Target hardware: **ESP32-C5** (RISC-V). Everything below is Linux or macOS (WSL2 works but is untested).

## Quick start: flash a board

Flash a release kit without installing Rust, ESP-IDF, or `espup`. You need only Python 3 and USB access.

1. **Download a release kit.** Grab the latest `wolfPACK-v*-esp32c5.zip` from the project's GitHub Releases and unzip it.
2. **Install the flash tools** (installs `espflash`; one-time):

   ```bash
   ./tools/install-flash-tools.sh
   ```

3. **Plug in the board over USB.** On Linux, make sure your user can open serial ports:

   ```bash
   sudo usermod -aG dialout $USER && newgrp dialout
   ```

4. **Flash it:**

   ```bash
   ./tools/wardriver flash-control        # the control node (board picker)
   ./tools/wardriver flash-sniffers --sequential   # sniffers, one cable at a time
   ./tools/wardriver flash-sniffers --batch        # sniffers, parallel (cables listed via espflash)
   ```

The control node is the gateway/SoftAP role; the sniffers are the fleet. For board picks (`--board marauder`, …) see [Flashing — detailed](Flashing%20-%20Detailed.md).

**Quick start: flash a board** above covers control and sniffers from a release
kit. **Gateway node:** release kits (`./tools/wardriver make-release`) also
carry a prebuilt `esp32-wardriver-gateway` ELF — flash it with
`./tools/wardriver flash-gateway` (no Rust toolchain needed when flashing from
an extracted kit; `flash gateway` (cargo runner) still builds from source).

**Phone app:** the Android app is built from `wardriver-android/` with Gradle and installed with `adb` — see [Android app](#android-app).

**No terminal?** Use the [Web flasher](Web%20Flasher.md) to flash a board from a browser (Chromium or Edge required).

That is the entire flashing path. The sections below are only needed if you build firmware yourself.

## Building from source

### Prerequisites

- Linux or macOS (Windows via WSL2 works but was not tested here).
- Rust (stable + nightly) via [rustup](https://rustup.rs).
- Python 3.10+ (required by `espup` / ESP-IDF).

### Install the Espressif toolchain

```bash
cargo install espup --locked
espup install
cargo install espflash cargo-espflash --locked
```

`espup install` sets up the RISC-V GCC, `ldproxy`, and ESP-IDF (v5.5.2 or newer is required for C5 support). Source the exported environment in every shell you build from:

```bash
. ~/export-esp.sh
```

Persist it via your shell rc or [direnv](https://direnv.net/). If a build fails complaining about missing `ldproxy` or a C compiler, you forgot to source it in that shell.

ESP-IDF: v5.5.2+. Rust target: `riscv32imac-esp-espidf` (pinned in `rust-toolchain.toml`; `cargo` picks it up automatically after `espup install`). The RISC-V family (C3/C5/C6/H2) uses upstream Rust nightly — the Xtensa Rust fork is only for ESP32/S2/S3.

### Flash tool behavior

- The firmware must be built for **exactly one** role feature: `--features control`, `--features sniffer`, or `--features gateway`; both omitting them and enabling more than one are compile errors (see [README.md](../README.md)).
- First build fetches and compiles ESP-IDF; expect 5–10 minutes on a cold cache.

### Build / flash / monitor

From the repo root:

```bash
# Build (release is strongly recommended; debug binaries are large)
cargo build --release --bin esp32-wardriver-control --features control
# or: cargo build --release --bin esp32-wardriver-sniffer --features sniffer

# Flash helpers (board picker, fleet batch/sequential)
./tools/wardriver flash-control
./tools/wardriver flash-sniffers --batch
./tools/wardriver flash-sniffers --sequential

# Cargo runner path (build + flash + serial monitor via .cargo/config.toml)
./tools/wardriver flash control
./tools/wardriver flash sniffer
./tools/wardriver flash gateway
```

Or flash an already-built binary manually:

```bash
espflash flash --monitor \
  target/riscv32imac-esp-espidf/release/esp32-wardriver-control
# sniffer: …/esp32-wardriver-sniffer
```

For PMK provisioning, credential overrides, erase/recovery, and serial troubleshooting details, see [Flashing — detailed](Flashing%20-%20Detailed.md).

### Gateway node

The gateway is a separate image from control and sniffers (PackLink SoftAP coprocessor):

```bash
./tools/wardriver flash-gateway
```

From a repo checkout this auto-builds (`cargo build --release --bin esp32-wardriver-gateway --features gateway`) and flashes; inside an extracted release kit it flashes the prebuilt `firmware/esp32-wardriver-gateway` ELF without a toolchain. On a dev machine you can also use the cargo runner path `./tools/wardriver flash gateway`, which builds and flashes in one step.

### Android app

The phone app lives in `wardriver-android/` (Kotlin, Jetpack Compose, Room). There are two sides to getting it onto your phone: building the APK, and enabling the phone to accept it (sideloading).

**Phone prerequisites — enable sideloading (one-time per phone):**

1. **Enable Developer options.** Open Settings → About phone → tap "Build number" seven times. When it says "You are now a developer!", the new Settings → System → Developer options menu appears (location varies by vendor).
2. **Enable USB debugging** in Developer options.
3. **Trust your computer.** Connect the phone by USB and run `adb devices`. The phone shows a "Allow USB debugging?" dialog — check "Always allow" and accept. Verify: `adb devices` lists your phone as `device` (not `unauthorized`).
4. USB debugging stays enabled while you use `adb`. You can turn Developer options off later, but then `adb install` no longer works and updates require re-enabling it (or copying the APK to the phone and installing manually, see below).

**Build prerequisites (build machine):** Android Studio (or a standalone Android SDK) plus JDK 21, and `wardriver-android/local.properties` pointing at your SDK (`sdk.dir=...`; created automatically by Android Studio, manually otherwise).

**Build and install:**

```bash
cd wardriver-android
./gradlew assembleDebug          # debug APK: app/build/outputs/apk/debug/app-debug.apk
adb install -r app/build/outputs/apk/debug/app-debug.apk
```

The app installs as a normal sideloaded package (Android may warn about "apps from unknown sources" when the debugger is not used — `adb install` bypasses that; if you copy the APK to the phone and tap it, allow "Install unknown apps" for the file manager).

Debug builds are sufficient for local use. The phone talks to the gateway over the `wolfPACK` SoftAP (TCP 192.168.71.1:4710), so the phone must join that network — see the gateway docs under `docs/`.

### Release packaging (maintainers only)

`./tools/wardriver make-release` builds the firmware and produces `release/wolfPACK-v*-esp32c5.zip` for GitHub Releases. See [Flashing — detailed](Flashing%20-%20Detailed.md#release-zip-no-ci) (release zip: no CI).

### Notes

- ESP-IDF component manager writes `managed_components/` and `dependencies.lock`; both are gitignored.
- Debug binaries are large; prefer `--release` for anything you actually flash.