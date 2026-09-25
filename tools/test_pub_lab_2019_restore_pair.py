import json
import pathlib
import unittest

from tools.validate_pub_lab_2019_restore_pair import (
    compute_environment_fingerprint,
    validate_restore_pair,
)

ROOT = pathlib.Path(__file__).resolve().parents[1]
BASE = ROOT / "packages" / "protocol" / "pub-lab-2019" / "v1"


class PubLab2019RestorePairTests(unittest.TestCase):
    def load(self):
        challenge = json.loads((BASE / "synthetic-restore-challenge.json").read_text(encoding="utf-8"))
        manifest = json.loads((BASE / "synthetic-environment-manifest.json").read_text(encoding="utf-8"))
        return challenge, manifest

    def test_valid_pair_is_challenge_bound_and_measured(self):
        challenge, manifest = self.load()
        summary = validate_restore_pair(challenge, manifest)
        self.assertTrue(summary["challenge_bound"])
        self.assertTrue(summary["post_boot_capture"])
        self.assertTrue(summary["measured_environment"])
        self.assertTrue(summary["environment_match"])
        self.assertEqual(summary["publisher_process_count"], 0)
        self.assertEqual(summary["environment_fingerprint"], compute_environment_fingerprint(manifest))

    def test_wrong_nonce_fails_closed(self):
        challenge, manifest = self.load()
        manifest["restore_nonce"] = "f" * 32
        with self.assertRaises(AssertionError):
            validate_restore_pair(challenge, manifest)

    def test_manifest_before_cold_start_fails_closed(self):
        challenge, manifest = self.load()
        manifest["captured_at_utc"] = "2026-09-25T15:00:00Z"
        with self.assertRaises(AssertionError):
            validate_restore_pair(challenge, manifest)

    def test_self_asserted_fingerprint_without_matching_measured_state_fails(self):
        challenge, manifest = self.load()
        manifest["default_printer"] = "Different Printer"
        with self.assertRaises(AssertionError):
            validate_restore_pair(challenge, manifest)

    def test_prestart_expected_fingerprint_mismatch_fails_closed(self):
        challenge, manifest = self.load()
        challenge["expected_environment_fingerprint"] = "3" * 64
        with self.assertRaises(AssertionError):
            validate_restore_pair(challenge, manifest)

    def test_publisher_process_must_be_absent(self):
        challenge, manifest = self.load()
        manifest["publisher"]["process_count"] = 1
        with self.assertRaises(AssertionError):
            validate_restore_pair(challenge, manifest)

    def test_start_before_restore_request_fails_closed(self):
        challenge, manifest = self.load()
        challenge["cold_start_succeeded_at_utc"] = "2026-09-25T14:59:59Z"
        with self.assertRaises(AssertionError):
            validate_restore_pair(challenge, manifest)


if __name__ == "__main__":
    unittest.main()
