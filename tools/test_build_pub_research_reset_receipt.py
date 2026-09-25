import json
import pathlib
import unittest

from tools.build_pub_research_reset_receipt import build_provider_receipt
from tools.research_runner_verify_reset_receipt_compat import verify_compat

ROOT = pathlib.Path(__file__).resolve().parents[1]
FIXTURE = ROOT / "packages" / "protocol" / "pub-lab-2019" / "v1" / "synthetic-vmware-restore-evidence.json"
PACKET = "5" * 64


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
        verify_compat(
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
