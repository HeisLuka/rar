import copy
import json
import pathlib
import tempfile
import unittest

from tools.finalize_pub_lab_2019_environment_manifest import finalize_manifest

ROOT = pathlib.Path(__file__).resolve().parents[1]
BASE = ROOT / "packages" / "protocol" / "pub-lab-2019" / "v1"
CHALLENGE = BASE / "synthetic-restore-challenge.json"
RAW = BASE / "synthetic-environment-capture-raw.json"
FINAL = BASE / "synthetic-environment-manifest.json"


class EnvironmentCaptureFinalizerTests(unittest.TestCase):
    def load(self, path):
        return json.loads(path.read_text(encoding="utf-8"))

    def test_raw_capture_finalizes_to_existing_canonical_manifest(self):
        actual = finalize_manifest(self.load(CHALLENGE), self.load(RAW))
        expected = self.load(FINAL)
        self.assertEqual(actual, expected)

    def test_wrong_nonce_fails_closed(self):
        raw = self.load(RAW)
        raw["restore_nonce"] = "f" * 32
        with self.assertRaises(AssertionError):
            finalize_manifest(self.load(CHALLENGE), raw)

    def test_stale_capture_fails_closed(self):
        raw = self.load(RAW)
        raw["captured_at_utc"] = "2026-09-25T14:59:59Z"
        with self.assertRaises(AssertionError):
            finalize_manifest(self.load(CHALLENGE), raw)

    def test_wrong_publisher_build_fails_closed(self):
        raw = self.load(RAW)
        raw["publisher"]["build"] = "16.0.99999.0"
        with self.assertRaises(AssertionError):
            finalize_manifest(self.load(CHALLENGE), raw)

    def test_environment_drift_fails_expected_fingerprint(self):
        raw = self.load(RAW)
        raw["default_printer"] = "Different Printer"
        with self.assertRaises(AssertionError):
            finalize_manifest(self.load(CHALLENGE), raw)


if __name__ == "__main__":
    unittest.main()
