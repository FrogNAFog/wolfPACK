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
TOOLS = ROOT / "tools"

for module_name, module_path in (
    ("validate_wiglecsv", TOOLS / "validate_wiglecsv.py"),
    ("find_carry_macs", TOOLS / "find_carry_macs.py"),
):
    spec = importlib.util.spec_from_file_location(module_name, module_path)
    module = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    sys.modules[module_name] = module
    spec.loader.exec_module(module)

find_carry_macs = sys.modules["find_carry_macs"]


class FindCarryMacsTests(unittest.TestCase):
    def fixture_path(self, name: str) -> Path:
        return FIXTURES / name

    def analyze(self, name: str, **kwargs):
        options = find_carry_macs.DetectionOptions(**kwargs)
        return find_carry_macs.analyze_file(self.fixture_path(name), options)

    def test_minimal_file_has_no_candidates(self):
        candidates, errors = self.analyze("valid_minimal.csv")
        self.assertEqual(errors, [])
        self.assertEqual(candidates, [])

    def test_moving_hotspot_is_detected(self):
        candidates, errors = self.analyze("carry_route.csv")
        self.assertEqual(errors, [])
        macs = {candidate.mac for candidate in candidates}
        self.assertIn("11:22:33:44:55:66", macs)
        self.assertNotIn("aa:bb:cc:dd:ee:ff", macs)

    def test_scrub_removes_selected_mac(self):
        with tempfile.TemporaryDirectory() as tmp:
            source = Path(tmp) / "carry_route.csv"
            output = Path(tmp) / "scrubbed.csv"
            shutil.copyfile(self.fixture_path("carry_route.csv"), source)
            removed, kept = find_carry_macs.scrub_file(
                source,
                {"11:22:33:44:55:66"},
                output,
                dry_run=False,
            )
            self.assertEqual(removed, 6)
            self.assertEqual(kept, 5)
            text = output.read_text(encoding="utf-8")
            self.assertNotIn("11:22:33:44:55:66", text)
            self.assertIn("aa:bb:cc:dd:ee:ff", text)

    def test_scrub_macs_cli_writes_output(self):
        with tempfile.TemporaryDirectory() as tmp:
            source = Path(tmp) / "carry_route.csv"
            output_dir = Path(tmp) / "out"
            shutil.copyfile(self.fixture_path("carry_route.csv"), source)
            with contextlib.redirect_stdout(io.StringIO()):
                rc = find_carry_macs.main(
                    [
                        str(source),
                        "--scrub-macs",
                        "11:22:33:44:55:66",
                        "--output-dir",
                        str(output_dir),
                        "--yes",
                    ]
                )
            self.assertEqual(rc, 0)
            scrubbed = output_dir / "carry_route.csv"
            self.assertTrue(scrubbed.exists())
            self.assertNotIn("11:22:33:44:55:66", scrubbed.read_text(encoding="utf-8"))


if __name__ == "__main__":
    unittest.main()
