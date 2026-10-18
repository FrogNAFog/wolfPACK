#!/usr/bin/env python3
"""Find MAC addresses that likely belong to devices carried during a wardrive.

WiGLE CSV rows for APs at fixed locations cluster at a few GPS coordinates.
Rows for phones, laptops, or hotspots moving with the operator appear at many
locations along the route and across most of the capture window.

Use --scrub to write copies (or in-place backups) with selected MACs removed.
"""

from __future__ import annotations

import argparse
import csv
import json
import shutil
import sys
from dataclasses import asdict, dataclass, field
from datetime import datetime
from pathlib import Path
from typing import Iterable

_TOOLS_DIR = Path(__file__).resolve().parent
if str(_TOOLS_DIR) not in sys.path:
    sys.path.insert(0, str(_TOOLS_DIR))

from validate_wiglecsv import (
    CSV_HEADER,
    EXPECTED_COLUMN_COUNT,
    MAC_RE,
    UTC_FORMAT,
    _is_wigle_upload_suffix,
    collect_files,
    haversine_m,
    parse_timestamp,
)


@dataclass(frozen=True)
class DetectionOptions:
    min_rows: int = 5
    min_rows_with_coords: int = 3
    min_location_buckets: int = 4
    min_geo_span_m: float = 400.0
    min_temporal_fraction: float = 0.25
    min_geo_fraction: float = 0.35
    location_bucket_decimals: int = 4
    allow_any_extension: bool = False


@dataclass
class MacObservation:
    line: int
    mac: str
    ssid: str
    row_type: str
    timestamp: datetime | None
    lat: float | None
    lon: float | None


@dataclass
class MacProfile:
    mac: str
    row_count: int = 0
    rows_with_coords: int = 0
    ssids: dict[str, int] = field(default_factory=dict)
    types: dict[str, int] = field(default_factory=dict)
    first_seen: datetime | None = None
    last_seen: datetime | None = None
    location_buckets: set[tuple[float, float]] = field(default_factory=set)
    coord_points: list[tuple[float, float]] = field(default_factory=list)

    def dominant_ssid(self) -> str:
        if not self.ssids:
            return ""
        return max(self.ssids.items(), key=lambda item: item[1])[0]

    def dominant_type(self) -> str:
        if not self.types:
            return ""
        return max(self.types.items(), key=lambda item: item[1])[0]

    def geo_span_m(self) -> float:
        points = self.coord_points
        if len(points) < 2:
            return 0.0
        max_distance = 0.0
        for index, (lat1, lon1) in enumerate(points):
            for lat2, lon2 in points[index + 1 :]:
                max_distance = max(max_distance, haversine_m(lat1, lon1, lat2, lon2))
        return max_distance

    def temporal_span_s(self) -> float:
        if self.first_seen is None or self.last_seen is None:
            return 0.0
        return max(0.0, (self.last_seen - self.first_seen).total_seconds())


@dataclass(frozen=True)
class FileContext:
    path: Path
    time_span_s: float
    geo_span_m: float
    data_row_count: int
    rows_with_coords: int


@dataclass(frozen=True)
class CarryCandidate:
    mac: str
    file: str
    row_count: int
    rows_with_coords: int
    location_buckets: int
    geo_span_m: float
    temporal_span_s: float
    temporal_fraction: float
    geo_fraction: float
    dominant_ssid: str
    dominant_type: str
    reasons: tuple[str, ...]


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description=(
            "Detect MAC addresses that appear throughout a WiGLE wardrive file "
            "(likely devices you are carrying). Optionally scrub them from copies."
        )
    )
    parser.add_argument("paths", nargs="+", help="WiGLE .csv file(s) or directory.")
    parser.add_argument(
        "--recursive",
        action="store_true",
        help="Recursively scan directories for .csv / .wiglecsv files.",
    )
    parser.add_argument(
        "--allow-any-extension",
        action="store_true",
        help="Accept any file extension when scanning paths.",
    )
    parser.add_argument("--json", action="store_true", help="Emit machine-readable JSON.")
    parser.add_argument(
        "--min-rows",
        type=int,
        default=5,
        help="Minimum row count before a MAC is considered (default: 5).",
    )
    parser.add_argument(
        "--min-location-buckets",
        type=int,
        default=4,
        help="Minimum distinct GPS buckets along the route (default: 4).",
    )
    parser.add_argument(
        "--min-geo-span-m",
        type=float,
        default=400.0,
        help="Minimum geographic span in meters between sightings (default: 400).",
    )
    parser.add_argument(
        "--min-temporal-fraction",
        type=float,
        default=0.25,
        help="Fraction of file time span the MAC must cover (default: 0.25).",
    )
    parser.add_argument(
        "--min-geo-fraction",
        type=float,
        default=0.35,
        help="MAC geo span as a fraction of file geo span (default: 0.35).",
    )
    parser.add_argument(
        "--scrub",
        action="store_true",
        help="Remove detected carry MACs from output file(s).",
    )
    parser.add_argument(
        "--scrub-macs",
        metavar="MACS",
        help="Comma-separated MACs to remove (implies --scrub).",
    )
    parser.add_argument(
        "--output-dir",
        metavar="DIR",
        type=Path,
        help="Directory for scrubbed copies (default: <input>.scrubbed.csv beside source).",
    )
    parser.add_argument(
        "--in-place",
        action="store_true",
        help="Overwrite input file when scrubbing (use with --backup).",
    )
    parser.add_argument(
        "--backup",
        action="store_true",
        help="When using --in-place, keep a .bak copy beside the original.",
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="With --scrub, report rows that would be removed without writing.",
    )
    parser.add_argument(
        "-y",
        "--yes",
        action="store_true",
        help="Scrub without interactive confirmation.",
    )
    return parser.parse_args(argv)


def location_bucket(
    lat: float, lon: float, decimals: int
) -> tuple[float, float]:
    return (round(lat, decimals), round(lon, decimals))


def read_wigle_rows(path: Path) -> tuple[list[list[str]], list[str]]:
    errors: list[str] = []
    try:
        with path.open("r", encoding="utf-8", newline="") as handle:
            rows = list(csv.reader(handle))
    except OSError as exc:
        return [], [f"could not read file: {exc}"]
    except UnicodeDecodeError as exc:
        return [], [f"file is not valid UTF-8: {exc}"]
    except csv.Error as exc:
        return [], [f"CSV parse failed: {exc}"]

    if len(rows) < 2:
        return [], ["file must contain WiGLE pre-header and CSV header rows"]
    return rows, errors


def parse_observations(
    path: Path, rows: list[list[str]], options: DetectionOptions
) -> tuple[list[MacObservation], list[str]]:
    observations: list[MacObservation] = []
    errors: list[str] = []

    if rows[1] != CSV_HEADER:
        errors.append("CSV header does not match WiGLE v1.6 project header")
        return observations, errors

    for line_no, row in enumerate(rows[2:], start=3):
        if len(row) != EXPECTED_COLUMN_COUNT:
            continue
        data = dict(zip(CSV_HEADER, row))
        mac = data["MAC"].strip()
        if not MAC_RE.match(mac):
            continue
        lat = _parse_coord(data["CurrentLatitude"])
        lon = _parse_coord(data["CurrentLongitude"])
        if (lat is None) ^ (lon is None):
            lat = None
            lon = None
        observations.append(
            MacObservation(
                line=line_no,
                mac=mac.lower(),
                ssid=data["SSID"],
                row_type=data["Type"].strip(),
                timestamp=parse_timestamp(data["FirstSeen"]),
                lat=lat,
                lon=lon,
            )
        )
    return observations, errors


def _parse_coord(raw: str) -> float | None:
    raw = raw.strip()
    if raw == "":
        return None
    try:
        return float(raw)
    except ValueError:
        return None


def build_profiles(observations: Iterable[MacObservation]) -> dict[str, MacProfile]:
    profiles: dict[str, MacProfile] = {}
    for obs in observations:
        profile = profiles.setdefault(obs.mac, MacProfile(mac=obs.mac))
        profile.row_count += 1
        if obs.ssid:
            profile.ssids[obs.ssid] = profile.ssids.get(obs.ssid, 0) + 1
        if obs.row_type:
            profile.types[obs.row_type] = profile.types.get(obs.row_type, 0) + 1
        if obs.timestamp is not None:
            if profile.first_seen is None or obs.timestamp < profile.first_seen:
                profile.first_seen = obs.timestamp
            if profile.last_seen is None or obs.timestamp > profile.last_seen:
                profile.last_seen = obs.timestamp
        if obs.lat is not None and obs.lon is not None:
            profile.rows_with_coords += 1
            profile.coord_points.append((obs.lat, obs.lon))
    return profiles


def file_context(path: Path, observations: list[MacObservation]) -> FileContext:
    timestamps = [obs.timestamp for obs in observations if obs.timestamp is not None]
    time_span_s = 0.0
    if len(timestamps) >= 2:
        time_span_s = (max(timestamps) - min(timestamps)).total_seconds()

    coord_points = [
        (obs.lat, obs.lon)
        for obs in observations
        if obs.lat is not None and obs.lon is not None
    ]
    geo_span_m = 0.0
    if len(coord_points) >= 2:
        for index, (lat1, lon1) in enumerate(coord_points):
            for lat2, lon2 in coord_points[index + 1 :]:
                geo_span_m = max(geo_span_m, haversine_m(lat1, lon1, lat2, lon2))

    return FileContext(
        path=path,
        time_span_s=time_span_s,
        geo_span_m=geo_span_m,
        data_row_count=len(observations),
        rows_with_coords=len(coord_points),
    )


def finalize_profile_buckets(
    profile: MacProfile, decimals: int
) -> MacProfile:
    profile.location_buckets = {
        location_bucket(lat, lon, decimals) for lat, lon in profile.coord_points
    }
    return profile


def detect_carry_candidates(
    path: Path,
    profiles: dict[str, MacProfile],
    context: FileContext,
    options: DetectionOptions,
) -> list[CarryCandidate]:
    candidates: list[CarryCandidate] = []
    for profile in profiles.values():
        finalize_profile_buckets(profile, options.location_bucket_decimals)
        candidate = classify_profile(path, profile, context, options)
        if candidate is not None:
            candidates.append(candidate)
    candidates.sort(
        key=lambda item: (item.row_count, item.geo_span_m, item.temporal_fraction),
        reverse=True,
    )
    return candidates


def classify_profile(
    path: Path,
    profile: MacProfile,
    context: FileContext,
    options: DetectionOptions,
) -> CarryCandidate | None:
    if profile.row_count < options.min_rows:
        return None
    if profile.rows_with_coords < options.min_rows_with_coords:
        return None

    geo_span_m = profile.geo_span_m()
    bucket_count = len(profile.location_buckets)
    temporal_span_s = profile.temporal_span_s()
    temporal_fraction = (
        temporal_span_s / context.time_span_s if context.time_span_s > 0 else 0.0
    )
    geo_fraction = (
        geo_span_m / context.geo_span_m if context.geo_span_m > 0 else 0.0
    )

    geo_ok = bucket_count >= options.min_location_buckets or geo_span_m >= options.min_geo_span_m
    temporal_ok = (
        context.time_span_s <= 0
        or temporal_fraction >= options.min_temporal_fraction
    )
    route_ok = context.geo_span_m <= 0 or geo_fraction >= options.min_geo_fraction

    if not (geo_ok and temporal_ok and route_ok):
        return None

    reasons: list[str] = []
    if bucket_count >= options.min_location_buckets:
        reasons.append(f"seen at {bucket_count} distinct GPS buckets")
    if geo_span_m >= options.min_geo_span_m:
        reasons.append(f"geographic span {geo_span_m:.0f} m")
    if context.time_span_s > 0 and temporal_fraction >= options.min_temporal_fraction:
        reasons.append(f"covers {temporal_fraction * 100:.0f}% of capture time")
    if context.geo_span_m > 0 and geo_fraction >= options.min_geo_fraction:
        reasons.append(f"spans {geo_fraction * 100:.0f}% of route distance")

    return CarryCandidate(
        mac=profile.mac,
        file=str(path),
        row_count=profile.row_count,
        rows_with_coords=profile.rows_with_coords,
        location_buckets=bucket_count,
        geo_span_m=geo_span_m,
        temporal_span_s=temporal_span_s,
        temporal_fraction=temporal_fraction,
        geo_fraction=geo_fraction,
        dominant_ssid=profile.dominant_ssid(),
        dominant_type=profile.dominant_type(),
        reasons=tuple(reasons),
    )


def analyze_file(path: Path, options: DetectionOptions) -> tuple[list[CarryCandidate], list[str]]:
    rows, errors = read_wigle_rows(path)
    if errors:
        return [], errors
    observations, parse_errors = parse_observations(path, rows, options)
    errors.extend(parse_errors)
    if parse_errors:
        return [], errors
    profiles = build_profiles(observations)
    context = file_context(path, observations)
    return detect_carry_candidates(path, profiles, context, options), errors


def print_text_report(
    candidates: list[CarryCandidate], errors: list[tuple[str, str]]
) -> None:
    for file_path, message in errors:
        print(f"ERROR {file_path}: {message}", file=sys.stderr)
    if not candidates and not errors:
        print("No carry-along MAC candidates detected.")
        return
    for candidate in candidates:
        ssid = candidate.dominant_ssid or "(no SSID)"
        print(
            f"CARRY {candidate.file}: {candidate.mac} "
            f"type={candidate.dominant_type or '?'} ssid={ssid!r} "
            f"rows={candidate.row_count} buckets={candidate.location_buckets} "
            f"geo_span={candidate.geo_span_m:.0f}m "
            f"time_frac={candidate.temporal_fraction * 100:.0f}% "
            f"route_frac={candidate.geo_fraction * 100:.0f}%"
        )
        print(f"  likely because: {', '.join(candidate.reasons)}")
    print(f"Found {len(candidates)} carry candidate(s) in {len({c.file for c in candidates})} file(s).")


def scrub_path_for(path: Path, output_dir: Path | None, in_place: bool) -> Path:
    if in_place:
        return path
    if output_dir is not None:
        output_dir.mkdir(parents=True, exist_ok=True)
        return output_dir / path.name
    return path.with_name(f"{path.stem}.scrubbed{path.suffix}")


def scrub_file(
    path: Path,
    macs_to_scrub: set[str],
    output_path: Path,
    dry_run: bool,
) -> tuple[int, int]:
    rows, errors = read_wigle_rows(path)
    if errors:
        raise RuntimeError("; ".join(errors))
    if len(rows) < 2:
        raise RuntimeError("file is too short to scrub")

    kept_rows = rows[:2]
    removed = 0
    kept_data = 0
    for row in rows[2:]:
        if len(row) != EXPECTED_COLUMN_COUNT:
            kept_rows.append(row)
            continue
        mac = row[0].strip().lower()
        if mac in macs_to_scrub:
            removed += 1
            continue
        kept_rows.append(row)
        kept_data += 1

    if dry_run:
        return removed, kept_data

    output_path.parent.mkdir(parents=True, exist_ok=True)
    with output_path.open("w", encoding="utf-8", newline="") as handle:
        csv.writer(handle).writerows(kept_rows)
    return removed, kept_data


def confirm_scrub(paths: list[Path], macs: set[str]) -> bool:
    print(
        f"About to remove {len(macs)} MAC(s) from {len(paths)} file(s):",
        file=sys.stderr,
    )
    for mac in sorted(macs):
        print(f"  - {mac}", file=sys.stderr)
    try:
        answer = input("Continue? [y/N]: ").strip().lower()
    except EOFError:
        return False
    return answer in {"y", "yes"}


def scrub_targets(
    files: list[Path],
    options: DetectionOptions,
    scrub_macs: str | None,
) -> list[tuple[Path, set[str]]]:
    if scrub_macs:
        macs = {part.strip().lower() for part in scrub_macs.split(",") if part.strip()}
        invalid = sorted(mac for mac in macs if not MAC_RE.match(mac))
        if invalid:
            raise ValueError(f"invalid MAC address(es): {', '.join(invalid)}")
        return [(path, macs) for path in files]

    targets: list[tuple[Path, set[str]]] = []
    for path in files:
        candidates, errors = analyze_file(path, options)
        if errors:
            raise RuntimeError(f"{path}: {'; '.join(errors)}")
        macs = {candidate.mac for candidate in candidates}
        if macs:
            targets.append((path, macs))
    return targets


def main(argv: list[str] | None = None) -> int:
    args = parse_args(sys.argv[1:] if argv is None else argv)
    options = DetectionOptions(
        min_rows=args.min_rows,
        min_location_buckets=args.min_location_buckets,
        min_geo_span_m=args.min_geo_span_m,
        min_temporal_fraction=args.min_temporal_fraction,
        min_geo_fraction=args.min_geo_fraction,
        allow_any_extension=args.allow_any_extension,
    )

    files, collect_findings = collect_files(
        args.paths, args.recursive, args.allow_any_extension
    )
    all_candidates: list[CarryCandidate] = []
    file_errors: list[tuple[str, str]] = [
        (finding.file, finding.message) for finding in collect_findings
    ]

    for path in files:
        if not options.allow_any_extension and not _is_wigle_upload_suffix(path):
            file_errors.append((str(path), "file extension must be .csv or .wiglecsv"))
            continue
        candidates, errors = analyze_file(path, options)
        for message in errors:
            file_errors.append((str(path), message))
        all_candidates.extend(candidates)

    if args.json:
        print(
            json.dumps(
                {
                    "candidates": [asdict(candidate) for candidate in all_candidates],
                    "errors": [
                        {"file": file_path, "message": message}
                        for file_path, message in file_errors
                    ],
                },
                indent=2,
                sort_keys=True,
            )
        )
    else:
        print_text_report(all_candidates, file_errors)

    scrub_requested = args.scrub or bool(args.scrub_macs)
    if scrub_requested:
        if file_errors:
            print("Refusing to scrub because input errors were reported.", file=sys.stderr)
            return 1
        try:
            scrub_plan = scrub_targets(files, options, args.scrub_macs)
        except (ValueError, RuntimeError) as exc:
            print(exc, file=sys.stderr)
            return 1
        if not scrub_plan:
            print("No MACs selected for scrubbing.", file=sys.stderr)
            return 0

        all_macs = set()
        for _, macs in scrub_plan:
            all_macs.update(macs)
        if not args.yes and not args.dry_run:
            if not confirm_scrub([path for path, _ in scrub_plan], all_macs):
                print("Scrub cancelled.", file=sys.stderr)
                return 1

        total_removed = 0
        for path, macs_to_scrub in scrub_plan:
            output_path = scrub_path_for(path, args.output_dir, args.in_place)
            if args.in_place and args.backup and not args.dry_run:
                backup_path = path.with_suffix(path.suffix + ".bak")
                shutil.copy2(path, backup_path)
                print(f"Backup written to {backup_path}", file=sys.stderr)
            removed, kept = scrub_file(
                path, macs_to_scrub, output_path, dry_run=args.dry_run
            )
            total_removed += removed
            action = "Would remove" if args.dry_run else "Removed"
            destination = path if args.in_place else output_path
            print(
                f"{action} {removed} row(s) from {path} -> {destination} ({kept} data rows kept)",
                file=sys.stderr,
            )
        if args.dry_run:
            print(f"Dry run: {total_removed} row(s) would be removed.", file=sys.stderr)
        else:
            print(f"Scrubbed {total_removed} row(s) total.", file=sys.stderr)

    return 1 if file_errors else 0


if __name__ == "__main__":
    raise SystemExit(main())
