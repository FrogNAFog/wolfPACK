# wolfPACK

ESP32-C5 version of the P.A.C.K. software for Linux. A lot of features are WIP or experimental. This is a hobby project and is full of bad code, bad UI, and bad decisions.  This was born out of a desire to detect and map Flock cameras and expanded from there.   While there are a few features that violate the "passive" part of the PACK name, what passive really means is that this software is for making observations.  There are no attacks here.

## How does it work?
One control node orchestrates wardriving with an expandable number of sniffer nodes.  Sniffer nodes can be assigned to BLE or Wi-Fi sniffing.  Wi-Fi sniffers receive channel buckets from the control node. In auto mode, channels across 2.4 GHz, 5 GHz, and DFS are mixed and balanced evenly across sniffers; in manual mode each sniffer is assigned a band and optional pin.  Once assigned, Sniffers leave the control channel for up to 5 seconds and collect data. Wi-Fi sniffers collect data via promiscuous mode.  Wigle rows are throttled on the sniffer side every 0.5 seconds.  Wigle rows are further throttled on the control side by distance, 50m, before relogging. Wigle rows must also improve the RSSI average to be recorded. 

If an alert is triggered on a sniffer, it pauses its work, sends the alert to control, and then resumes wardriving.  GPS data is kept for 30 seconds on control.  Control uses this history to match sniffer observations from the 10 second sweep to the correct location in history. Unique MAC Counter note: On long drives that return to the same area as the start, some MACs may be counted as "unique" due to memory and performance constraints of the ESP platform. The count is a best effort representation, not a precise metric.

Flock detection is essentially obsolete.  It will likely be removed in a future release.

Detections are logged to detectioncsv / mdetectioncsv files.  Future ingestion of these files is being considered.

## Hardware target
This project was designed from the beginning with the intention of having minimal hardware requirements.  As such, this wardriver cluster is headless and requires only two *required* peripherals on the control node: The SD card and the GPS.  The sniffer nodes require no additional hardware.
This repo supports **two control-node wiring profiles**, selected **at build time** via Cargo features:

- **`board-xiao`** (primary): **XIAO ESP32-C5** on the **Seeed Studio Expansion Board Base for XIAO**, with **stacked Seeed XIAO L76K GNSS** (on-board SPI microSD, shield passive buzzer, onboard SSD1306 OLED on D4/D5).
- **`board-marauder`**: original **Marauder C5 adapter**. The C5 Adapter from JCMK was used to prototype this repo and is still functional.

## Minimum Parts list
Control: 1x Xiao ESP32-c5, 1x Xiao Expansion Board Base, 1x Xiao L76k GPS. ~$35
Sniffer: 1x Xiao ESP32-C5, 1x rod antenna of your choice. Do not use included antenna for wardriving. ~$9 minimum. More based on antenna choice.

## Additional Part Suggestions:
Short USB **2.0** cables for power. USB 3.0 cables can emit noise harmful to wardriving.
USB 2.0 Hubs. **Note:** Be careful that the hub does not draw too much power from a single port on your pattery pack.  The test battery pack could not run all sniffers on one charging port, but it is an old pack with lower power output.
USB Battery pack
Hard Shell Equipment Case (Apache style)
External GPS Antenna

## Basic Assembly:
### Control:
Solder the GPS, C5, and headers together so that the short end of the headers is soldered to the board stack.  Insert the board stack into the expansion base. Insert SD card into the expansion base.

### Sniffer / Gateway: 
Attach antenna. Insert into case of your choice/design.

### Quick start: flash from release zip

1. Download and unzip anywhere (e.g. `unzip release/wolfPACK-v0.1.0-esp32c5.zip`).
2. `cd wolfPACK-v0.1.0`
3. `./tools/install-flash-tools.sh` — installs `espflash` only (no Rust/ESP-IDF build toolchain).
4. `./tools/wardriver flash-control` — board picker (XIAO vs Marauder).
5. `./tools/wardriver flash-sniffers --batch` or `--sequential`. Use batch when all sniffers are connected. Use sequential when flashing one at a time.


### Setup and Management

> [!WARNING]
> **wolfPACK broadcasts its own management SoftAP (default SSID `wolfPACK`) while it wardrives.** Any **other wardriving software you run alongside it** — Kismet, the WiGLE Wireless Network Detector phone app, another ESP32 wardriver — **will see and log this AP**. If you are running multiple wardrivers on the same outing, **add the wolfPACK management SoftAP BSSID to that other tool's exclusion / ignore list** so it does not pollute your captures. wolfPACK already excludes its own management SoftAP BSSID (and your saved home AP BSSID) from its own WiGLE CSV and from sniffer reports — see **WiGLE BSSID privacy** below.

The control node runs a SoftAP for setup and the live dashboard. Before powering sniffers and control together, configure Control's home zone and other settings you want to use.  Then proceed with connecting the sniffers.

- **Management SoftAP** SSID and WPA2 password are configurable on `**/setup`** (distinct from your home WiFi). Defaults: SSID **`wolfPACK`**, password **`deflock123`**.
- Base URL: `**http://192.168.71.1/**` — fleet UI at `**/**`, full stats at `**/diagnostics**`, sniffer roles at `**/sniffers**`. The live JSON field `**management_ap_ssid**` in `**/api/stats**` shows the configured SoftAP name (never the password).
- Here is a summary of the tabs:
**fleet** - Wardriving status dashboard.  Sniffer count, GPS status, CSV gate status, alert counts, etc.
**sniffers** - Configure sniffer roles and channel plan. Auto mode balances channels across sniffers (mixed 2.4+5+DFS buckets with equal counts); manual mode assigns per-sniffer band and pin.
**setup** - Set up your privacy "home zone" and Wi-Fi network for uploads.
**Uploads:** credentials for WiGLE and WDGWars are **optional**. If they are not set, boot does not run uploads.
**Sounds:** Configure which alerts make noises.  All active by default.
**Wigle filters:** Filter SSIDs and MACs from wigle logging
**Detections:** Activate and deactivate detections. Add false postive macs to the ignore list.
**Capture:** WIP/Untested. For performing targeted packet captures using the wolfPACK. Pauses wardriving.
**Diagnostics:** Counters for troubleshooting. Abandon hope all ye who enter.



## Build & flash

See [docs/ENV.md](docs/ENV.md) (quick start) and [docs/Flashing - Detailed.md](docs/Flashing%20-%20Detailed.md) (full flashing/erase/provisioning reference). 

**Fleet flash:** `PROTO_VERSION` changes require **control + sniffers** on the same firmware version; mixed versions will drop frames.

### Build commands (control)

Pick **one** board profile:

```bash
. ~/export-esp.sh

# Control node on XIAO ESP32-C5 + Seeed expansion base (primary)
cargo build --release --bin esp32-wardriver-control --features "control board-xiao"

# Control node on Marauder C5 adapter (original dev hardware)
cargo build --release --bin esp32-wardriver-control --features "control board-marauder"
```

## CSV formats 

New WiGLE sessions use **WiGLE v1.6** headers; Detection alert sessions use **`Detection-detectioncsv-2.0`** (WiGLE column prefix + Detection extension fields — see [`docs/DETECTIONCSV.md`](docs/DETECTIONCSV.md)). Pre-headers identify the device as **wolfPACK** (`brand=wolfPACK`). Older `det-*.detectioncsv` files written before v2 are not migrated automatically.


## Roles: control vs sniffer

The firmware builds into **three role images**:

- **Control node:** GPS + SD + optional WiGLE upload + SoftAP web UI (`/`, `/setup`, `/wigle-filters`, `/api/`*); ESP-NOW hub; merges sniffer scan data into WiGLE CSV on SD.
- **Sniffer node(s):** scan-only; sends scan reports; **no** SoftAP, **no** HTTP server, **no** management pages — pairing and fleet UI are only on the control node. No SD, no upload.
- **Sniffer node(s):** Emulates the ESP-Now functions of the control node and passes the rest of the functions to the Android app.
