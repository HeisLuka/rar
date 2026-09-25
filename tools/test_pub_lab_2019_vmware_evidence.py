import json
import pathlib
import unittest

from tools.validate_pub_lab_2019_vmware_evidence import validate_vmware_evidence

ROOT = pathlib.Path(__file__).resolve().parents[1]
FIXTURE = ROOT / "packages" / "protocol" / "pub-lab-2019" / "v1" / "synthetic-vmware-restore-evidence.json"


class PubLab2019VmwareEvidenceTests(unittest.TestCase):
    def load(self):
        return json.loads(FIXTURE.read_text(encoding="utf-8"))

    def test_synthetic_backend_evidence_passes(self):
        summary = validate_vmware_evidence(self.load())
        self.assertTrue(summary["restore_verified"])
        self.assertTrue(summary["source_free"])

    def test_unverified_restore_fails_closed(self):
        value = self.load()
        value["restore"]["restore_verified"] = False
        with self.assertRaises(AssertionError):
            validate_vmware_evidence(value)

    def test_restore_time_reversal_fails_closed(self):
        value = self.load()
        value["restore"]["completed_at_utc"] = "2026-09-25T14:59:59Z"
        with self.assertRaises(AssertionError):
            validate_vmware_evidence(value)

    def test_path_leak_fails_closed(self):
        value = self.load()
        value["vm_identity"]["name"] = r"D:\\VMware\\PUB-LAB-2019"
        with self.assertRaises(AssertionError):
            validate_vmware_evidence(value)


if __name__ == "__main__":
    unittest.main()
