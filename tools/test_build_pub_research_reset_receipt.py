import importlib.util
import json
import pathlib
import unittest

from tools.build_pub_research_reset_receipt import build_provider_receipt

ROOT = pathlib.Path(__file__).resolve().parents[1]
FIXTURE = ROOT / "packages" / "protocol" / "pub-lab-2019" / "v1" / "synthetic-vmware-restore-evidence.json"
VERIFIER_PATH = ROOT / "tools" / "research-runner" / "verify_reset_receipt.py"
PACKET = "5" * 64


def load_donor_verifier():
    spec = importlib.util.spec_from_file_location("pub_research_reset_verifier", VERIFIER_PATH)
    if spec is None or spec.loader is None:
        raise RuntimeError("cannot load donor reset verifier")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class PubResearchResetReceiptBuildTests(unittest.TestCase):
    def load(self):
        return json.loads(FIXTURE.read_text(encoding="utf-8"))

    def build(self):
        return build_provider_receipt(
            self.load(),
            baseline_id="publisher-2019-build12527-golden-v1",
            snapshot_id="MODERN-2019-12527-GOLDEN-v1",
            experiment_id="PUB-LAB-CI-01",
            packet_sha256=PACKET,
        )

    def test_maps_to_existing_provider_neutral_schema(self):
        receipt = self.build()
        self.assertEqual(receipt["schema"], "pub-research-reset-receipt.v1")
        self.assertEqual(receipt["provider_id"], "vmware-workstation-pub-lab-2019")
        self.assertTrue(receipt["restore_verified"])
        self.assertTrue(receipt["restore_started_at_utc"].endswith("Z"))
        self.assertTrue(receipt["restore_completed_at_utc"].endswith("Z"))

        verifier = load_donor_verifier()
        summary = verifier.validate_receipt(
            receipt,
            expected_baseline="publisher-2019-build12527-golden-v1",
            expected_snapshot="MODERN-2019-12527-GOLDEN-v1",
            expected_experiment="PUB-LAB-CI-01",
            expected_packet_sha256=PACKET,
        )
        self.assertEqual(summary["provider_id"], "vmware-workstation-pub-lab-2019")

    def test_offset_backend_timestamps_are_canonicalized_for_donor(self):
        evidence = self.load()
        evidence["restore"]["started_at_utc"] = "2026-09-25T17:00:00+02:00"
        evidence["restore"]["completed_at_utc"] = "2026-09-25T17:00:03+02:00"
        receipt = build_provider_receipt(
            evidence,
            baseline_id="publisher-2019-build12527-golden-v1",
            snapshot_id="MODERN-2019-12527-GOLDEN-v1",
            experiment_id="PUB-LAB-CI-01",
            packet_sha256=PACKET,
        )
        self.assertEqual(receipt["restore_started_at_utc"], "2026-09-25T15:00:00.000Z")
        self.assertEqual(receipt["restore_completed_at_utc"], "2026-09-25T15:00:03.000Z")
        verifier = load_donor_verifier()
        verifier.validate_receipt(
            receipt,
            expected_baseline="publisher-2019-build12527-golden-v1",
            expected_snapshot="MODERN-2019-12527-GOLDEN-v1",
            expected_experiment="PUB-LAB-CI-01",
            expected_packet_sha256=PACKET,
        )

    def test_wrong_baseline_cannot_be_relabelled(self):
        with self.assertRaises(AssertionError):
            build_provider_receipt(
                self.load(),
                baseline_id="some-other-baseline",
                snapshot_id="MODERN-2019-12527-GOLDEN-v1",
                experiment_id="PUB-LAB-CI-01",
                packet_sha256=PACKET,
            )

    def test_wrong_snapshot_cannot_be_relabelled(self):
        with self.assertRaises(AssertionError):
            build_provider_receipt(
                self.load(),
                baseline_id="publisher-2019-build12527-golden-v1",
                snapshot_id="other-snapshot",
                experiment_id="PUB-LAB-CI-01",
                packet_sha256=PACKET,
            )


if __name__ == "__main__":
    unittest.main()
