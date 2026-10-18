#!/usr/bin/env python3
"""Validate WiGLE CSV files produced by wolfPACK (PACK-aligned WiGLE v1.6)."""

from __future__ import annotations

import argparse
import csv
import json
import math
import re
import sys
from dataclasses import asdict, dataclass
from datetime import datetime
from pathlib import Path
from typing import Iterable


WIGLE_VERSION = "WigleWifi-1.6"
CSV_HEADER = [
    "MAC",
    "SSID",
    "AuthMode",
    "FirstSeen",
    "Channel",
    "Frequency",
    "RSSI",
    "CurrentLatitude",
    "CurrentLongitude",
    "AltitudeMeters",
    "AccuracyMeters",
    "RCOIs",
    "MfgrId",
    "Type",
]
EXPECTED_COLUMN_COUNT = len(CSV_HEADER)
REQUIRED_PREHEADER_KEYS = {
    "appRelease",
    "model",
    "release",
    "device",
    "display",
    "board",
    "brand",
    "star",
    "body",
    "subBody",
}
MAC_RE = re.compile(r"^[0-9a-fA-F]{2}(:[0-9a-fA-F]{2}){5}$")
CELL_TYPES = frozenset({"GSM", "WCDMA", "LTE", "NR", "CDMA"})
CELL_KEY_RE = re.compile(r"^\d+_\d+_\d+$")
CELL_CAPS_RE = re.compile(r"^(GSM|WCDMA|LTE|NR|CDMA);\d+$")
UTC_FORMAT = "%Y-%m-%d %H:%M:%S"
EARTH_RADIUS_M = 6_371_000.0


@dataclass(frozen=True)
class Finding:
    severity: str
    code: str
    message: str
    file: str
    line: int | None = None


@dataclass(frozen=True)
class ValidationOptions:
    allow_any_extension: bool = False
    max_speed_kmh: float = 300.0
    max_altitude_ft: float = 20_000.0
    min_altitude_m: float = -500.0
    max_accuracy_m: float = 100.0
    same_time_jump_m: float = 100.0
    duplicate_window_sec: float = 60.0

    @property
    def max_altitude_m(self) -> float:
        return self.max_altitude_ft * 0.3048


@dataclass
class ParsedRow:
    line: int
    mac: str
    timestamp: datetime | None
    lat: float | None
    lon: float | None


WIGLE_FILE_SUFFIXES = (".csv", ".wiglecsv")


def _is_wigle_upload_suffix(path: Path) -> bool:
    return path.suffix.lower() in WIGLE_FILE_SUFFIXES


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Validate wolfPACK / PACK-aligned WiGLE-format .csv files (legacy .wiglecsv supported)."
    )
    parser.add_argument("paths", nargs="+", help="File or directory paths to validate.")
    parser.add_argument(
        "--recursive",
        action="store_true",
        help="Recursively scan directories for .csv (and legacy .wiglecsv) files.",
    )
    parser.add_argument(
        "--allow-any-extension",
        action="store_true",
        help="Do not require input files to end in .csv or .wiglecsv.",
    )
    parser.add_argument(
        "--strict",
        action="store_true",
        help="Exit non-zero when warnings are present.",
    )
    parser.add_argument("--json", action="store_true", help="Emit machine-readable JSON.")
    parser.add_argument(
        "--max-speed-kmh",
        type=float,
        default=300.0,
        help="Warn when sequential coordinates exceed this speed.",
    )
    parser.add_argument(
        "--max-altitude-ft",
        type=float,
        default=20_000.0,
        help="Warn when altitude exceeds this many feet.",
    )
    parser.add_argument(
        "--same-time-jump-m",
        type=float,
        default=100.0,
        help="Warn when same-timestamp coordinates jump farther than this.",
    )
    return parser.parse_args(argv)


def collect_files(
    paths: Iterable[str], recursive: bool, allow_any_extension: bool
) -> tuple[list[Path], list[Finding]]:
    files: list[Path] = []
    findings: list[Finding] = []
    for raw in paths:
        path = Path(raw)
        if path.is_file():
            files.append(path)
        elif path.is_dir():
            if recursive:
                if allow_any_extension:
                    iterator = path.rglob("*")
                else:
                    seen: set[Path] = set()
                    for pat in ("*.csv", "*.wiglecsv"):
                        for c in path.rglob(pat):
                            seen.add(c)
                    iterator = sorted(seen)
            else:
                iterator = path.iterdir()
            for candidate in iterator:
                if not candidate.is_file():
                    continue
                if allow_any_extension or _is_wigle_upload_suffix(candidate):
                    files.append(candidate)
        else:
            findings.append(
                Finding(
                    "ERROR",
                    "input-not-found",
                    f"input path does not exist: {path}",
                    str(path),
                )
            )
    return sorted(set(files)), findings


def validate_file(path: Path, options: ValidationOptions) -> list[Finding]:
    findings: list[Finding] = []
    if not options.allow_any_extension and not _is_wigle_upload_suffix(path):
        findings.append(
            Finding(
                "ERROR",
                "bad-extension",
                "file extension must be .csv (or legacy .wiglecsv)",
                str(path),
            )
        )

    try:
        with path.open("r", encoding="utf-8", newline="") as handle:
            rows = list(csv.reader(handle))
    except OSError as exc:
        return [
            Finding(
                "ERROR",
                "read-failed",
                f"could not read file: {exc}",
                str(path),
            )
        ]
    except UnicodeDecodeError as exc:
        return [
            Finding(
                "ERROR",
                "utf8-decode-failed",
                f"file is not valid UTF-8: {exc}",
                str(path),
            )
        ]
    except csv.Error as exc:
        return [
            Finding(
                "ERROR",
                "csv-parse-failed",
                f"CSV parse failed: {exc}",
                str(path),
            )
        ]

    if len(rows) < 2:
        findings.append(
            Finding(
                "ERROR",
                "missing-header",
                "file must contain WiGLE pre-header and CSV header rows",
                str(path),
            )
        )
        return findings

    findings.extend(validate_preheader(path, rows[0]))
    findings.extend(validate_header(path, rows[1]))

    previous_timestamp: datetime | None = None
    previous_geo: ParsedRow | None = None
    recent_by_mac: dict[str, ParsedRow] = {}

    for index, row in enumerate(rows[2:], start=3):
        parsed = validate_data_row(path, index, row, findings, options)
        if parsed is None:
            continue

        if parsed.timestamp is not None:
            if previous_timestamp is not None and parsed.timestamp < previous_timestamp:
                findings.append(
                    Finding(
                        "WARN",
                        "non-monotonic-time",
                        "FirstSeen moved backwards within the file",
                        str(path),
                        index,
                    )
                )
            previous_timestamp = parsed.timestamp

        previous_geo = check_coordinate_sequence(
            path, parsed, previous_geo, findings, options
        )
        check_duplicate_mac(path, parsed, recent_by_mac, findings, options)

    return findings


def validate_preheader(path: Path, row: list[str]) -> list[Finding]:
    findings: list[Finding] = []
    if not row:
        return [
            Finding(
                "ERROR",
                "missing-preheader",
                "first row must be a WiGLE pre-header",
                str(path),
                1,
            )
        ]

    if row[0] != WIGLE_VERSION:
        findings.append(
            Finding(
                "ERROR",
                "bad-preheader-version",
                f"pre-header must start with {WIGLE_VERSION}",
                str(path),
                1,
            )
        )

    fields: dict[str, str] = {}
    for cell in row[1:]:
        if "=" not in cell:
            findings.append(
                Finding(
                    "ERROR",
                    "bad-preheader-field",
                    f"pre-header field is not key=value: {cell}",
                    str(path),
                    1,
                )
            )
            continue
        key, value = cell.split("=", 1)
        fields[key] = value

    for key in sorted(REQUIRED_PREHEADER_KEYS - fields.keys()):
        findings.append(
            Finding(
                "ERROR",
                "missing-preheader-key",
                f"pre-header missing required key: {key}",
                str(path),
                1,
            )
        )

    expected_values = {"star": "Sol", "body": "3", "subBody": "0"}
    for key, expected in expected_values.items():
        actual = fields.get(key)
        if actual is not None and actual != expected:
            findings.append(
                Finding(
                    "ERROR",
                    "bad-preheader-location",
                    f"pre-header {key} must be {expected}, got {actual}",
                    str(path),
                    1,
                )
            )

    return findings


def validate_header(path: Path, row: list[str]) -> list[Finding]:
    if row == CSV_HEADER:
        return []
    return [
        Finding(
            "ERROR",
            "bad-header",
            "CSV header does not exactly match WiGLE v1.6 project header",
            str(path),
            2,
        )
    ]


def validate_data_row(
    path: Path,
    line: int,
    row: list[str],
    findings: list[Finding],
    options: ValidationOptions,
) -> ParsedRow | None:
    if len(row) != EXPECTED_COLUMN_COUNT:
        findings.append(
            Finding(
                "ERROR",
                "wrong-column-count",
                f"expected {EXPECTED_COLUMN_COUNT} columns, got {len(row)}",
                str(path),
                line,
            )
        )
        return None

    data = dict(zip(CSV_HEADER, row))
    mac = data["MAC"]
    row_type = data["Type"].strip()
    if row_type in CELL_TYPES:
        if not CELL_KEY_RE.match(mac):
            findings.append(
                Finding(
                    "ERROR",
                    "bad-cell-key",
                    f"invalid cell key (expected MCCMNC_LAC_CID style): {mac}",
                    str(path),
                    line,
                )
            )
    elif not MAC_RE.match(mac):
        findings.append(
            Finding("ERROR", "bad-mac", f"invalid MAC/BSSID: {mac}", str(path), line)
        )

    timestamp = parse_timestamp(data["FirstSeen"])
    if timestamp is None:
        findings.append(
            Finding(
                "ERROR",
                "bad-first-seen",
                f"FirstSeen must match {UTC_FORMAT}: {data['FirstSeen']}",
                str(path),
                line,
            )
        )

    lat, lon = validate_coordinates(path, line, data, findings)
    if row_type in CELL_TYPES:
        validate_cell_channel_frequency(path, line, data, findings)
    else:
        validate_channel_frequency(path, line, data, findings)
    validate_rssi(path, line, data, findings)
    validate_altitude(path, line, data, findings, options)
    validate_accuracy(path, line, data, findings, options)

    validate_type_and_side_fields(path, line, data, findings)

    return ParsedRow(line=line, mac=mac, timestamp=timestamp, lat=lat, lon=lon)


def validate_type_and_side_fields(
    path: Path, line: int, data: dict[str, str], findings: list[Finding]
) -> None:
    """Validate `Type` and related fields per WiGLE v1.6 spec.

    Spec: https://api.wigle.net/csvFormat-1_6.html
    """

    row_type = data["Type"].strip()
    if row_type == "":
        findings.append(Finding("ERROR", "bad-type", "Type must be present", str(path), line))
        return

    if row_type == "WIFI":
        return

    if row_type in ("BLE", "BT"):
        # WiGLE spec: Channel=0 for Bluetooth rows; Frequency is a Bluetooth device-type code (or blank).
        # MfgrId in spec examples is decimal; accept other formats but warn.
        raw_mfgr = data["MfgrId"].strip()
        if raw_mfgr == "":
            return
        # Warn on hex-ish manufacturer IDs like "004c" (Apple).
        if all(c in "0123456789abcdefABCDEF" for c in raw_mfgr) and any(
            c in "abcdefABCDEF" for c in raw_mfgr
        ):
            findings.append(
                Finding(
                    "WARN",
                    "bt-mfgrid-hex",
                    f"MfgrId looks hex ({raw_mfgr}); WiGLE examples use decimal manufacturer ID",
                    str(path),
                    line,
                )
            )
            return
        if parse_int(raw_mfgr) is None:
            findings.append(
                Finding(
                    "WARN",
                    "bt-mfgrid-nonint",
                    f"MfgrId should be blank or an integer, got {raw_mfgr}",
                    str(path),
                    line,
                )
            )
        return

    if row_type in CELL_TYPES:
        auth = data["AuthMode"].strip()
        if row_type == "CDMA":
            if not auth.startswith("CDMA;"):
                findings.append(
                    Finding(
                        "ERROR",
                        "bad-cell-caps",
                        f"CDMA capabilities must be CDMA;SystemId, got {auth}",
                        str(path),
                        line,
                    )
                )
        elif not CELL_CAPS_RE.match(auth):
            findings.append(
                Finding(
                    "ERROR",
                    "bad-cell-caps",
                    f"cell capabilities must be {row_type};MCCMNC, got {auth}",
                    str(path),
                    line,
                )
            )
        if data["RCOIs"].strip():
            findings.append(
                Finding(
                    "ERROR",
                    "cell-rcois",
                    "RCOIs must be blank for cell rows",
                    str(path),
                    line,
                )
            )
        if data["MfgrId"].strip():
            findings.append(
                Finding(
                    "ERROR",
                    "cell-mfgrid",
                    "MfgrId must be blank for cell rows",
                    str(path),
                    line,
                )
            )
        return

    # Unknown types are not produced by wolfPACK mobile/firmware export paths.
    findings.append(
        Finding(
            "ERROR",
            "bad-type",
            f"Unsupported Type for this project output: {row_type}",
            str(path),
            line,
        )
    )


def validate_coordinates(
    path: Path, line: int, data: dict[str, str], findings: list[Finding]
) -> tuple[float | None, float | None]:
    raw_lat = data["CurrentLatitude"]
    raw_lon = data["CurrentLongitude"]
    if raw_lat == "" and raw_lon == "":
        return None, None
    if raw_lat == "" or raw_lon == "":
        findings.append(
            Finding(
                "ERROR",
                "incomplete-coordinate",
                "latitude and longitude must both be blank or both be present",
                str(path),
                line,
            )
        )
        return None, None

    lat = parse_float(raw_lat)
    lon = parse_float(raw_lon)
    if lat is None or not -90.0 <= lat <= 90.0:
        findings.append(
            Finding(
                "ERROR",
                "bad-latitude",
                f"latitude must be numeric and within [-90, 90], got {raw_lat}",
                str(path),
                line,
            )
        )
    if lon is None or not -180.0 <= lon <= 180.0:
        findings.append(
            Finding(
                "ERROR",
                "bad-longitude",
                f"longitude must be numeric and within [-180, 180], got {raw_lon}",
                str(path),
                line,
            )
        )
    return lat, lon


def validate_channel_frequency(
    path: Path, line: int, data: dict[str, str], findings: list[Finding]
) -> None:
    row_type = data["Type"].strip()
    raw_channel = data["Channel"].strip()
    channel = parse_int(raw_channel) if raw_channel != "" else None
    raw_freq = data["Frequency"].strip()
    frequency = parse_int(raw_freq) if raw_freq != "" else None

    # Bluetooth rows: Android exports blank Channel; Kismet/spec may use 0.
    if row_type in ("BLE", "BT"):
        if channel is not None and channel != 0:
            findings.append(
                Finding(
                    "ERROR",
                    "bad-channel",
                    f"Bluetooth rows require blank or 0 Channel, got {channel}",
                    str(path),
                    line,
                )
            )
        if frequency is not None and frequency <= 0:
            findings.append(
                Finding(
                    "ERROR",
                    "bad-frequency",
                    f"Frequency must be blank or positive, got {frequency}",
                    str(path),
                    line,
                )
            )
            return
        # Heuristic: flag values that look like MHz.
        if frequency is not None and 2300 <= frequency <= 2600:
            findings.append(
                Finding(
                    "WARN",
                    "bt-frequency-mhz",
                    f"Bluetooth Frequency looks like MHz ({frequency}); WiGLE spec uses a Bluetooth device-type code (or blank)",
                    str(path),
                    line,
                )
            )
        return

    if channel is None:
        findings.append(
            Finding(
                "ERROR",
                "bad-channel",
                f"Channel must be an integer, got {data['Channel']}",
                str(path),
                line,
            )
        )
        return

    # WiFi rows: require positive channel and center frequency in MHz
    if channel <= 0:
        findings.append(
            Finding("ERROR", "bad-channel", f"Channel must be positive, got {channel}", str(path), line)
        )
        return
    if frequency is None:
        findings.append(
            Finding(
                "ERROR",
                "bad-frequency",
                f"Frequency must be an integer, got {data['Frequency']}",
                str(path),
                line,
            )
        )
        return
    if frequency <= 0:
        findings.append(
            Finding(
                "ERROR",
                "bad-frequency",
                f"Frequency must be positive, got {frequency}",
                str(path),
                line,
            )
        )
        return

    expected = expected_wifi_frequency_mhz(channel)
    if expected is None:
        findings.append(
            Finding(
                "WARN",
                "unknown-channel",
                f"Channel {channel} is outside the project frequency map",
                str(path),
                line,
            )
        )
    elif frequency != expected:
        findings.append(
            Finding(
                "ERROR",
                "channel-frequency-mismatch",
                f"Channel {channel} should map to {expected} MHz, got {frequency}",
                str(path),
                line,
            )
        )


def validate_cell_channel_frequency(
    path: Path, line: int, data: dict[str, str], findings: list[Finding]
) -> None:
    """Cell rows: blank Channel; Frequency is ARFCN/UARFCN/EARFCN/NRARFCN (CDMA: 0)."""
    row_type = data["Type"].strip()
    if data["Channel"].strip() != "":
        findings.append(
            Finding(
                "ERROR",
                "bad-channel",
                f"cell rows require blank Channel, got {data['Channel']!r}",
                str(path),
                line,
            )
        )
    raw_freq = data["Frequency"].strip()
    frequency = parse_int(raw_freq) if raw_freq != "" else None
    if frequency is None:
        findings.append(
            Finding(
                "ERROR",
                "bad-frequency",
                f"Frequency must be an integer, got {data['Frequency']}",
                str(path),
                line,
            )
        )
        return
    if row_type == "CDMA":
        if frequency != 0:
            findings.append(
                Finding(
                    "ERROR",
                    "bad-frequency",
                    f"CDMA rows require Frequency 0, got {frequency}",
                    str(path),
                    line,
                )
            )
    elif frequency < 0:
        findings.append(
            Finding(
                "ERROR",
                "bad-frequency",
                f"Frequency must be non-negative, got {frequency}",
                str(path),
                line,
            )
        )


def validate_rssi(
    path: Path, line: int, data: dict[str, str], findings: list[Finding]
) -> None:
    rssi = parse_int(data["RSSI"])
    if rssi is None or not -127 <= rssi <= 0:
        findings.append(
            Finding(
                "ERROR",
                "bad-rssi",
                f"RSSI must be an integer in [-127, 0], got {data['RSSI']}",
                str(path),
                line,
            )
        )


def validate_altitude(
    path: Path,
    line: int,
    data: dict[str, str],
    findings: list[Finding],
    options: ValidationOptions,
) -> None:
    raw = data["AltitudeMeters"]
    if raw == "":
        return
    altitude_m = parse_int(raw)
    if altitude_m is None:
        findings.append(
            Finding(
                "ERROR",
                "bad-altitude",
                f"AltitudeMeters must be an integer number of meters, got {raw}",
                str(path),
                line,
            )
        )
        return
    if altitude_m > options.max_altitude_m:
        findings.append(
            Finding(
                "WARN",
                "high-altitude",
                f"altitude {altitude_m} m exceeds {options.max_altitude_ft:g} ft",
                str(path),
                line,
            )
        )
    if altitude_m < options.min_altitude_m:
        findings.append(
            Finding(
                "WARN",
                "low-altitude",
                f"altitude {altitude_m} m is below {options.min_altitude_m:g} m",
                str(path),
                line,
            )
        )


def validate_accuracy(
    path: Path,
    line: int,
    data: dict[str, str],
    findings: list[Finding],
    options: ValidationOptions,
) -> None:
    raw = data["AccuracyMeters"]
    if raw == "":
        return
    accuracy_m = parse_float(raw)
    if accuracy_m is None or accuracy_m < 0:
        findings.append(
            Finding(
                "ERROR",
                "bad-accuracy",
                f"AccuracyMeters must be finite and non-negative, got {raw}",
                str(path),
                line,
            )
        )
        return
    if accuracy_m > options.max_accuracy_m:
        findings.append(
            Finding(
                "WARN",
                "high-accuracy",
                f"accuracy {accuracy_m:g} m exceeds {options.max_accuracy_m:g} m",
                str(path),
                line,
            )
        )


def check_coordinate_sequence(
    path: Path,
    parsed: ParsedRow,
    previous_geo: ParsedRow | None,
    findings: list[Finding],
    options: ValidationOptions,
) -> ParsedRow | None:
    if parsed.lat is None or parsed.lon is None or parsed.timestamp is None:
        return previous_geo
    if previous_geo is None:
        return parsed
    if (
        previous_geo.lat is None
        or previous_geo.lon is None
        or previous_geo.timestamp is None
    ):
        return parsed

    distance_m = haversine_m(previous_geo.lat, previous_geo.lon, parsed.lat, parsed.lon)
    elapsed_s = (parsed.timestamp - previous_geo.timestamp).total_seconds()
    if elapsed_s <= 0:
        if distance_m > options.same_time_jump_m:
            findings.append(
                Finding(
                    "WARN",
                    "same-time-coordinate-jump",
                    f"coordinate jump of {distance_m:.1f} m with no elapsed time",
                    str(path),
                    parsed.line,
                )
            )
    else:
        speed_kmh = distance_m / elapsed_s * 3.6
        if speed_kmh > options.max_speed_kmh:
            findings.append(
                Finding(
                    "WARN",
                    "improbable-speed",
                    f"sequential coordinates imply {speed_kmh:.1f} km/h over {elapsed_s:.0f} s",
                    str(path),
                    parsed.line,
                )
            )
    return parsed


def check_duplicate_mac(
    path: Path,
    parsed: ParsedRow,
    recent_by_mac: dict[str, ParsedRow],
    findings: list[Finding],
    options: ValidationOptions,
) -> None:
    if parsed.timestamp is None:
        return
    if not MAC_RE.match(parsed.mac) and not CELL_KEY_RE.match(parsed.mac):
        return
    key = parsed.mac.lower()
    previous = recent_by_mac.get(key)
    if previous is not None and previous.timestamp is not None:
        elapsed_s = (parsed.timestamp - previous.timestamp).total_seconds()
        if 0 <= elapsed_s < options.duplicate_window_sec:
            findings.append(
                Finding(
                    "WARN",
                    "duplicate-bssid-window",
                    f"duplicate BSSID repeated after {elapsed_s:.0f} s",
                    str(path),
                    parsed.line,
                )
            )
    recent_by_mac[key] = parsed


def expected_wifi_frequency_mhz(channel: int) -> int | None:
    if channel == 14:
        return 2484
    if 1 <= channel <= 13:
        return 2407 + channel * 5
    if 30 <= channel <= 196:
        return 5000 + channel * 5
    return None


def parse_timestamp(raw: str) -> datetime | None:
    try:
        return datetime.strptime(raw, UTC_FORMAT)
    except ValueError:
        return None


def parse_int(raw: str) -> int | None:
    try:
        return int(raw)
    except ValueError:
        return None


def parse_float(raw: str) -> float | None:
    try:
        value = float(raw)
    except ValueError:
        return None
    if not math.isfinite(value):
        return None
    return value


def haversine_m(lat1: float, lon1: float, lat2: float, lon2: float) -> float:
    phi1 = math.radians(lat1)
    phi2 = math.radians(lat2)
    d_phi = math.radians(lat2 - lat1)
    d_lambda = math.radians(lon2 - lon1)
    a = (
        math.sin(d_phi / 2.0) ** 2
        + math.cos(phi1) * math.cos(phi2) * math.sin(d_lambda / 2.0) ** 2
    )
    return 2.0 * EARTH_RADIUS_M * math.atan2(math.sqrt(a), math.sqrt(1.0 - a))


def summarize(files: list[Path], findings: list[Finding]) -> dict[str, int]:
    return {
        "files": len(files),
        "errors": sum(1 for finding in findings if finding.severity == "ERROR"),
        "warnings": sum(1 for finding in findings if finding.severity == "WARN"),
    }


def print_text_report(files: list[Path], findings: list[Finding]) -> None:
    summary = summarize(files, findings)
    for finding in findings:
        location = finding.file
        if finding.line is not None:
            location = f"{location}:{finding.line}"
        print(f"{finding.severity} {finding.code} {location}: {finding.message}")
    print(
        "Validated {files} file(s): {errors} error(s), {warnings} warning(s).".format(
            **summary
        )
    )


def main(argv: list[str] | None = None) -> int:
    args = parse_args(sys.argv[1:] if argv is None else argv)
    options = ValidationOptions(
        allow_any_extension=args.allow_any_extension,
        max_speed_kmh=args.max_speed_kmh,
        max_altitude_ft=args.max_altitude_ft,
        same_time_jump_m=args.same_time_jump_m,
    )

    files, findings = collect_files(args.paths, args.recursive, args.allow_any_extension)
    for path in files:
        findings.extend(validate_file(path, options))

    summary = summarize(files, findings)
    if args.json:
        print(
            json.dumps(
                {
                    "summary": summary,
                    "findings": [asdict(finding) for finding in findings],
                },
                indent=2,
                sort_keys=True,
            )
        )
    else:
        print_text_report(files, findings)

    if summary["errors"] > 0 or (args.strict and summary["warnings"] > 0):
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
