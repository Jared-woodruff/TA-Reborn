"""Exercise the coverage gate's real CLI, including invalid and missing evidence."""
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


SCRIPT = Path(__file__).resolve().parents[1] / "game_coverage.py"
GAME = ("apps/ta_game/src/app/lifecycle.rs", "apps/ta_game/src/keys.rs", "crates/ta_config/src/validation.rs")
DATA = ("crates/ta_formats/src/gaf.rs", "crates/ta_audio/src/decode/wav.rs")


def entry(path, count=100, covered=100):
    return {"filename": path, "summary": {"lines": {"count": count, "covered": covered}}}


class CoverageTests(unittest.TestCase):
    def run_gate(self, report, *arguments):
        with tempfile.TemporaryDirectory(prefix="ta-coverage-test-") as directory:
            path = Path(directory) / "report.json"
            if report is not None:
                path.write_text(report if isinstance(report, str) else json.dumps(report), encoding="utf-8")
            return subprocess.run([sys.executable, str(SCRIPT), str(path), *arguments], capture_output=True, text=True)

    def test_default_game_scope_accepts_windows_and_unix_paths(self):
        files = [entry("C:\\checkout\\" + GAME[0].replace("/", "\\")), entry("/checkout/" + GAME[1], covered=95), entry(GAME[2])]
        result = self.run_gate({"data": [{"files": files}]})
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("95/100 lines", result.stdout)

    def test_data_and_combined_scopes_accept_their_exact_floors(self):
        files = [entry(DATA[0], covered=95), entry(DATA[1], covered=90)]
        self.assertEqual(self.run_gate({"data": [{"files": files}]}, "--scope", "data").returncode, 0)
        result = self.run_gate({"data": [{"files": files}, {"files": [entry(path) for path in GAME]}]}, "--scope", "all")
        self.assertEqual(result.returncode, 0, result.stderr)
        files[0]["summary"]["lines"]["covered"] = 94
        self.assertEqual(self.run_gate({"data": [{"files": files}]}, "--scope", "data").returncode, 1)

    def test_missing_duplicate_and_lookalike_paths_fail(self):
        for files in [[], [entry(path) for path in GAME[1:]], [entry(path) for path in (*GAME, GAME[0])],
                      [entry("prefix" + GAME[0]), *[entry(path) for path in GAME[1:]]]]:
            with self.subTest(files=files):
                result = self.run_gate({"data": [{"files": files}]})
                self.assertEqual(result.returncode, 1)
                self.assertIn("expected one coverage entry", result.stderr)

    def test_low_and_empty_coverage_fail_even_if_reported_percent_claims_success(self):
        for count, covered in [(100, 99), (0, 0)]:
            files = [entry(path) for path in GAME]
            files[0]["summary"]["lines"].update(count=count, covered=covered, percent=100)
            result = self.run_gate({"data": [{"files": files}]})
            self.assertEqual(result.returncode, 1)
            self.assertIn("below 100%", result.stderr)

    def test_malformed_reports_and_counters_fail_with_a_readable_error(self):
        reports = [None, "{", {}, [], {"data": []}, {"data": {}}, {"data": [{"files": {}}]},
                   {"data": [{"files": [{"filename": 7}]}]}, {"data": [{"files": [{"filename": GAME[0]}]}]}]
        for count, covered in [(1, 2), (-1, 0), (1, -1), (100, "100"), (True, 1), (100.0, 100)]:
            reports.append({"data": [{"files": [entry(GAME[0], count, covered), *[entry(path) for path in GAME[1:]]]}]})
        for report in reports:
            with self.subTest(report=report):
                result = self.run_gate(report)
                self.assertEqual(result.returncode, 2)
                self.assertIn("cannot read coverage report", result.stderr)
                self.assertNotIn("Traceback", result.stderr)


if __name__ == "__main__":
    unittest.main()
