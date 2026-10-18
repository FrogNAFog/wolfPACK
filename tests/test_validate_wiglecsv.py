import contextlib
import importlib.util
import io
import shutil
import sys
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
FIXTURES = ROOT / "tests" / "fixtures" / "wigle"
VALIDATOR = ROOT / "tools" / "validate_wiglecsv.py"

spec = importlib.util.spec_from_file_location("validate_wiglecsv", VALIDATOR)
validate_wiglecsv = importlib.util.module_from_spec(spec)
assert spec.loader is not None
sys.modules["validate_wiglecsv"] = validate_wiglecsv
spec.loader.exec_module(validate_wiglecsv)


class WigleCsvValidatorTests(unittest.TestCase):
    def fixture_path(self, name: str) -> Path:
        path = FIXTURES / name
        if path.exists():
            return path
        stem = Path(name).stem
        for candidate in FIXTURES.glob(f"{stem}.*"):
            if validate_wiglecsv._is_wigle_upload_suffix(candidate):
                return candidate
        return path

    def validate(self, name, **kwargs):
        options = validate_wiglecsv.ValidationOptions(**kwargs)
        fixture = self.fixture_path(name)
        return validate_wiglecsv.validate_file(fixture, options)

    def codes(self, findings):
        return {finding.code for finding in findings}

    def severities(self, findings):
        return {finding.severity for finding in findings}

    def test_valid_minimal_file_has_no_findings(self):
        self.assertEqual(self.validate("valid_minimal.csv"), [])

    def test_quoted_ssid_is_parsed_as_valid_csv(self):
        self.assertEqual(self.validate("quoted_ssid.csv"), [])

    def test_bad_header_is_error(self):
        findings = self.validate("bad_header.csv")
        self.assertIn("bad-header", self.codes(findings))
        self.assertIn("ERROR", self.severities(findings))

    def test_bad_column_count_is_error(self):
        findings = self.validate("bad_column_count.csv")
        self.assertIn("wrong-column-count", self.codes(findings))
        self.assertIn("ERROR", self.severities(findings))

    def test_invalid_lat_lon_are_errors(self):
        findings = self.validate("invalid_lat_lon.csv")
        self.assertIn("bad-latitude", self.codes(findings))
        self.assertIn("bad-longitude", self.codes(findings))

    def test_improbable_sequential_speed_is_warning(self):
        findings = self.validate("impossible_jump.csv")
        self.assertIn("improbable-speed", self.codes(findings))
        self.assertIn("WARN", self.severities(findings))

    def test_same_time_coordinate_jump_is_warning(self):
        findings = self.validate("same_time_jump.csv")
        self.assertIn("same-time-coordinate-jump", self.codes(findings))
        self.assertIn("WARN", self.severities(findings))

    def test_high_altitude_is_warning(self):
        findings = self.validate("high_altitude.csv")
        self.assertIn("high-altitude", self.codes(findings))
        self.assertIn("WARN", self.severities(findings))

    def test_non_monotonic_timestamp_is_warning(self):
        findings = self.validate("non_monotonic.csv")
        self.assertIn("non-monotonic-time", self.codes(findings))
        self.assertIn("WARN", self.severities(findings))

    def test_duplicate_bssid_too_close_is_warning(self):
        findings = self.validate("duplicate_bssid_close.csv")
        self.assertIn("duplicate-bssid-window", self.codes(findings))
        self.assertIn("WARN", self.severities(findings))

    def test_duplicate_bssid_spaced_apart_is_allowed(self):
        findings = self.validate("duplicate_bssid_spaced.csv")
        self.assertNotIn("duplicate-bssid-window", self.codes(findings))
        self.assertEqual(findings, [])

    def test_cell_row_wcdma_doc_is_valid(self):
        self.assertEqual(self.validate("cell_row_wcdma_doc.csv"), [])

    def test_ble_row_minimal_android_shape_is_valid(self):
        self.assertEqual(self.validate("ble_row_minimal.csv"), [])

    def test_wifi_row_doc_rcoi_is_valid(self):
        self.assertEqual(self.validate("wifi_row_doc_rcoi.csv"), [])

    def test_bad_extension_is_error_by_default(self):
        with tempfile.TemporaryDirectory() as tmp:
            bad_path = Path(tmp) / "sample.txt"
            shutil.copyfile(self.fixture_path("valid_minimal.csv"), bad_path)
            findings = validate_wiglecsv.validate_file(
                bad_path, validate_wiglecsv.ValidationOptions()
            )
        self.assertIn("bad-extension", self.codes(findings))

    def test_allow_any_extension_accepts_valid_content(self):
        with tempfile.TemporaryDirectory() as tmp:
            bad_path = Path(tmp) / "sample.txt"
            shutil.copyfile(self.fixture_path("valid_minimal.csv"), bad_path)
            findings = validate_wiglecsv.validate_file(
                bad_path,
                validate_wiglecsv.ValidationOptions(allow_any_extension=True),
            )
        self.assertEqual(findings, [])

    def test_strict_mode_exits_nonzero_on_warning(self):
        with contextlib.redirect_stdout(io.StringIO()):
            rc = validate_wiglecsv.main(
                [str(self.fixture_path("high_altitude.csv")), "--strict"]
            )
        self.assertEqual(rc, 1)


if __name__ == "__main__":
    unittest.main()
