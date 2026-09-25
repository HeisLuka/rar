import copy
import json
import pathlib
import unittest

from tools.validate_pub_lab_2019_reset_receipt import validate_receipt

ROOT = pathlib.Path(__file__).resolve().parents[1]
FIXTURE = ROOT / "packages" / "protocol" / "pub-lab-2019" / "v1" / "synthetic-reset-receipt.json"


class PubLab2019ResetReceiptTests(unittest.TestCase):
    def load(self):
        return json.loads(FIXTURE.read_text(encoding="utf-8"))

    def test_synthetic_receipt_passes_contract(self):
        summary = validate_receipt(self.load())
        self.assertTrue(summary["restore_verified"])
        self.assertTrue(summary["source_free"])

    def test_unverified_restore_fails_closed(self):
        value = self.load()
        value["restore"]["restore_verified"] = False
        with self.assertRaises(AssertionError):
            validate_receipt(value)

    def test_local_path_is_rejected(self):
        value = self.load()
        value["vm_identity"]["config_fingerprint"] = "a" * 64
        value["leak"] = r"D:\\VMware\\PUB-LAB-2019\\PUB-LAB-2019.vmx"
        with self.assertRaises(AssertionError):
            validate_receipt(value)

    def test_secret_field_is_rejected(self):
        value = self.load()
        value["vmware"]["password"] = "nope"
        with self.assertRaises(AssertionError):
            validate_receipt(value)


if __name__ == "__main__":
    unittest.main()
