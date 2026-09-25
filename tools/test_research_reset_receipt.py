import importlib.util
import pathlib
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[1]
VERIFIER_PATH = ROOT / "tools" / "research-runner" / "verify_reset_receipt.py"
PACKET = "5" * 64


def load_verifier():
    spec = importlib.util.spec_from_file_location("pub_research_reset_verifier_tests", VERIFIER_PATH)
    if spec is None or spec.loader is None:
        raise RuntimeError("cannot load reset receipt verifier")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def receipt(**overrides):
    value = {
        "schema": "pub-research-reset-receipt.v1",
        "provider_id": "vmware-workstation-pub-lab-2019",
        "provider_version": "1.0",
        "baseline_id": "publisher-2019-build12527-golden-v1",
        "snapshot_id": "MODERN-2019-12527-GOLDEN-v1",
        "experiment_id": "PUB-LAB-CI-01",
        "packet_sha256": PACKET,
        "restore_started_at_utc": "2026-09-25T15:00:00.000Z",
        "restore_completed_at_utc": "2026-09-25T15:00:03.000Z",
        "pre_restore_state_sha256": "3" * 64,
        "post_restore_state_sha256": "4" * 64,
        "environment_fingerprint_sha256": "1" * 64,
        "restore_verified": True,
        "failure_reason": None,
    }
    value.update(overrides)
    return value


class ResearchResetReceiptVerifierTests(unittest.TestCase):
    def setUp(self):
        self.verifier = load_verifier()

    def validate(self, value):
        return self.verifier.validate_receipt(
            value,
            expected_baseline="publisher-2019-build12527-golden-v1",
            expected_snapshot="MODERN-2019-12527-GOLDEN-v1",
            expected_experiment="PUB-LAB-CI-01",
            expected_packet_sha256=PACKET,
        )

    def test_valid_receipt_passes(self):
        result = self.validate(receipt())
        self.assertEqual(result["provider_id"], "vmware-workstation-pub-lab-2019")

    def test_stale_packet_fails_closed(self):
        with self.assertRaises(SystemExit):
            self.validate(receipt(packet_sha256="0" * 64))

    def test_wrong_baseline_fails_closed(self):
        with self.assertRaises(SystemExit):
            self.validate(receipt(baseline_id="other-baseline"))

    def test_unverified_restore_fails_closed(self):
        with self.assertRaises(SystemExit):
            self.validate(receipt(restore_verified=False, failure_reason="restore failed"))

    def test_timestamp_reversal_fails_closed(self):
        with self.assertRaises(SystemExit):
            self.validate(
                receipt(
                    restore_started_at_utc="2026-09-25T15:00:03.000Z",
                    restore_completed_at_utc="2026-09-25T15:00:00.000Z",
                )
            )

    def test_cold_restore_environment_drift_is_detectable(self):
        first = self.validate(receipt())
        second = self.validate(receipt(environment_fingerprint_sha256="2" * 64))
        self.assertNotEqual(
            first["environment_fingerprint_sha256"],
            second["environment_fingerprint_sha256"],
        )


if __name__ == "__main__":
    unittest.main()
