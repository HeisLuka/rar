import copy
import json
import pathlib
import unittest

from tools.validate_pub_lab_2019_restore_pair import validate_restore_pair

ROOT = pathlib.Path(__file__).resolve().parents[1]
BASE = ROOT / "packages" / "protocol" / "pub-lab-2019" / "v1"


class PubLab2019RestorePairTests(unittest.TestCase):
    def load(self):
        challenge = json.loads((BASE / "synthetic-restore-challenge.json").read_text(encoding="utf-8"))
        manifest = json.loads((BASE / "synthetic-environment-manifest.json").read_text(encoding="utf-8"))
        return challenge, manifest

    def test_valid_pair_is_challenge_bound(self):
        challenge, manifest = self.load()
        summary = validate_restore_pair(challenge, manifest)
        self.assertTrue(summary["challenge_bound"])
        self.assertTrue(summary["post_boot_capture"])
        self.assertTrue(summary["environment_match"])

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

    def test_fingerprint_chosen_after_start_cannot_replace_pinned_value(self):
        challenge, manifest = self.load()
        manifest["environment_fingerprint"] = "3" * 64
        with self.assertRaises(AssertionError):
            validate_restore_pair(challenge, manifest)

    def test_start_before_restore_request_fails_closed(self):
        challenge, manifest = self.load()
        challenge["cold_start_succeeded_at_utc"] = "2026-09-25T14:59:59Z"
        with self.assertRaises(AssertionError):
            validate_restore_pair(challenge, manifest)


if __name__ == "__main__":
    unittest.main()
