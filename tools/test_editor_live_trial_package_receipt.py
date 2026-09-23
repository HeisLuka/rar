import copy
import json
import unittest
from pathlib import Path

from tools.validate_editor_live_trial_package_receipt import validate_receipt

FIXTURE = Path("packages/product/editor-live-trial/v1/fixtures/package-receipt.synthetic.json")


class EditorLiveTrialPackageReceiptTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.receipt = json.loads(FIXTURE.read_text(encoding="utf-8"))

    def test_synthetic_receipt_passes(self):
        summary = validate_receipt(copy.deepcopy(self.receipt))
        self.assertTrue(summary["editor_controls_enabled"])
        self.assertTrue(summary["runtime_loop_complete"])
        self.assertTrue(summary["source_free_receipt"])

    def test_reader_only_build_is_rejected(self):
        receipt = copy.deepcopy(self.receipt)
        receipt["editor_boundary"]["reader_only"] = True
        with self.assertRaises(AssertionError):
            validate_receipt(receipt)

    def test_hidden_native_save_claim_is_rejected(self):
        receipt = copy.deepcopy(self.receipt)
        receipt["editor_boundary"]["native_save_pub_claimed"] = True
        with self.assertRaises(AssertionError):
            validate_receipt(receipt)

    def test_incomplete_runtime_loop_is_rejected(self):
        for key in self.receipt["runtime_smoke"]:
            receipt = copy.deepcopy(self.receipt)
            receipt["runtime_smoke"][key] = False
            with self.assertRaises(AssertionError, msg=key):
                validate_receipt(receipt)

    def test_installer_signing_or_update_claim_is_rejected(self):
        for key in ["installer_included", "code_signing_claimed", "auto_update_claimed"]:
            receipt = copy.deepcopy(self.receipt)
            receipt["package_contents"][key] = True
            with self.assertRaises(AssertionError, msg=key):
                validate_receipt(receipt)

    def test_private_document_or_customer_fields_are_rejected(self):
        for key in self.receipt["privacy"]:
            receipt = copy.deepcopy(self.receipt)
            receipt["privacy"][key] = True
            with self.assertRaises(AssertionError, msg=key):
                validate_receipt(receipt)

    def test_extra_filename_or_customer_field_is_rejected(self):
        for key, value in [
            ("filename", "newsletter.pub"),
            ("path", "C:/Users/Alice/newsletter.pub"),
            ("customer", "Example Customer"),
        ]:
            receipt = copy.deepcopy(self.receipt)
            receipt[key] = value
            with self.assertRaises(AssertionError, msg=key):
                validate_receipt(receipt)


if __name__ == "__main__":
    unittest.main()
