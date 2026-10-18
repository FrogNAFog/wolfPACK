## Validate WiGLE CSV output

The control node writes WiGLE files to `/sdcard/wolfPACK/wigle/pending/*.csv` (and keeps other SD data under `/sdcard/wolfPACK/`).
The WiGLE `**MfgrId**` column is left **empty** on WiFi rows (it is for beacon manufacturer data, not which sniffer heard the AP); use the **fleet dashboard** `/` and `**/api/stats**` for per-sniffer attribution.
While capturing, the firmware **flushes** after each row and **periodically syncs file data to the SD**
(roughly every 25 rows or 1 second, whichever comes first) so sudden power loss loses at most a small tail of rows.
After copying files from the SD card to your machine, run the host-side
sanity validator:

```bash
python3 tools/validate_wiglecsv.py /path/to/sdcard/wolfPACK/wigle/pending --recursive
```

The validator checks the WiGLE v1.6 pre-header, fixed CSV header, row width,
field ranges, WiFi channel/frequency consistency, timestamps, RSSI, GPS
coordinates, altitude, accuracy, and suspicious sequential movement. Format
problems are reported as `ERROR`; suspicious but possible GPS observations,
such as altitude over 20,000 ft or impossible travel speed, are reported as
`WARN`.

### Primary reference: WiGLE Android export

wolfPACK CSV rows follow the **WiGLE Android app** upload shape, not every
example on [api.wigle.net/csvFormat.html](https://api.wigle.net/csvFormat.html).
When the website and the app disagree, the app wins.

| Column | WiGLE Android source |
|--------|----------------------|
| WiFi `RCOIs` | `WifiReceiver.getConcatenatedRcois` / `formatRcoi` |
| BLE `AuthMode` | `BluetoothReceiver.DEVICE_TYPE_LEGEND` label only (no `[LE]` suffix) |
| BLE `Channel` | blank (`ObservationUploader` omits null channel) |
| BLE `Frequency` | Bluetooth device-type code from Class of Device AD `0x0D`; omit when `0` |
| BLE `MfgrId` | decimal company ID; omit when `0` |

The public WiGLE BLE doc row uses `Misc [LE]` and Channel `0`. That is **not**
the Android export shape. Kismet `kismetdb_to_wiglecsv.cc` is a secondary
reference only when the Android app has no example for a field.

Useful options:

```bash
python3 tools/validate_wiglecsv.py file.csv --strict
python3 tools/validate_wiglecsv.py file.csv --json
python3 tools/validate_wiglecsv.py file.csv --max-speed-kmh 200 --max-altitude-ft 15000
```

## Find carry-along MAC addresses

Phones, hotspots, and other gear that moves with you show up on many GPS
coordinates along a wardrive. Fixed APs heard while passing a storefront do not.
To list likely carry-along MACs and optionally remove them before upload:

```bash
python3 tools/find_carry_macs.py /path/to/sdcard/wolfPACK/wigle/pending --recursive
python3 tools/find_carry_macs.py file.csv --json
python3 tools/find_carry_macs.py file.csv --scrub --dry-run
python3 tools/find_carry_macs.py file.csv --scrub --output-dir ./scrubbed --yes
python3 tools/find_carry_macs.py file.csv --scrub-macs aa:bb:cc:dd:ee:ff --in-place --backup --yes
```

Tune sensitivity with `--min-rows`, `--min-location-buckets`, `--min-geo-span-m`,
`--min-temporal-fraction`, and `--min-geo-fraction`. Scrubbing writes
`<name>.scrubbed.csv` beside the source unless `--output-dir` or `--in-place`
is set.
