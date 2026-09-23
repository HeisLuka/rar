import copy
import json
import unittest
from pathlib import Path

from tools.validate_replace_image_receipt import (
    validate_export_receipt,
    validate_pair_receipts,
    validate_ui_receipt,
)

BASE = Path("packages/protocol/editor-image-replace/v1")
UI = json.loads((BASE / "fixtures/ui-producer-receipt.synthetic.json").read_text(encoding="utf-8"))
EXPORT = json.loads((BASE / "fixtures/export-producer-receipt.synthetic.json").read_text(encoding="utf-8"))


class ReplaceImageReceiptTests(unittest.TestCase):
    def test_ui_fixture_passes(self):
        result = validate_ui_receipt(copy.deepcopy(UI))
        self.assertTrue(result["crop_free_only"])
        self.assertTrue(result["content_addressed_asset"])
        self.assertTrue(result["source_free_receipt"])

    def test_crop_bearing_target_cannot_be_admitted(self):
        receipt = copy.deepcopy(UI)
        receipt["target_gate"]["explicit_crop_present"] = True
        with self.assertRaises(AssertionError):
            validate_ui_receipt(receipt)

    def test_asset_identity_cannot_be_filename_or_url(self):
        for key in ("filename_is_identity", "url_is_identity"):
            receipt = copy.deepcopy(UI)
            receipt["asset_import"][key] = True
            with self.assertRaises(AssertionError, msg=key):
                validate_ui_receipt(receipt)

    def test_ui_commit_is_exactly_one_operation(self):
        receipt = copy.deepcopy(UI)
        receipt["commit"]["operation_count_after"] += 1
        with self.assertRaises(AssertionError):
            validate_ui_receipt(receipt)

    def test_all_ui_negative_probes_are_required(self):
        for key in UI["negative_probes"]:
            receipt = copy.deepcopy(UI)
            receipt["negative_probes"][key] = False
            with self.assertRaises(AssertionError, msg=key):
                validate_ui_receipt(receipt)

    def test_replacement_binding_is_required_end_to_end(self):
        result = validate_pair_receipts(copy.deepcopy(UI), copy.deepcopy(EXPORT))
        self.assertTrue(result["replacement_identity_bound_end_to_end"])

        mismatch = copy.deepcopy(EXPORT)
        mismatch["replacement_binding"]["binding_id"] = "rb_ffffffffffffffffffffffffffffffff"
        with self.assertRaises(AssertionError):
            validate_pair_receipts(copy.deepcopy(UI), mismatch)

        derived = copy.deepcopy(UI)
        derived["replacement_binding"]["content_derived"] = True
        with self.assertRaises(AssertionError):
            validate_ui_receipt(derived)

    def test_pair_requires_same_build_and_fixture_kind(self):
        other_build = copy.deepcopy(EXPORT)
        other_build["build"]["binary_sha256"] = "b" * 64
        with self.assertRaises(AssertionError):
            validate_pair_receipts(copy.deepcopy(UI), other_build)

        other_fixture = copy.deepcopy(EXPORT)
        other_fixture["fixture_kind"] = "real_pub_sanitized"
        with self.assertRaises(AssertionError):
            validate_pair_receipts(copy.deepcopy(UI), other_fixture)

    def test_idml_cannot_claim_preserved_z_order(self):
        receipt = copy.deepcopy(EXPORT)
        receipt["idml"]["z_order"] = "preserved"
        with self.assertRaises(AssertionError):
            validate_export_receipt(receipt)

    def test_odg_cannot_hide_content_transform_loss(self):
        receipt = copy.deepcopy(EXPORT)
        receipt["odg"]["content_transform"] = "preserved"
        with self.assertRaises(AssertionError):
            validate_export_receipt(receipt)

    def test_unsupported_target_cannot_silently_fallback(self):
        for key in ("silent_drop", "silent_source_fallback"):
            receipt = copy.deepcopy(EXPORT)
            receipt["unsupported_target"][key] = True
            with self.assertRaises(AssertionError, msg=key):
                validate_export_receipt(receipt)

    def test_native_pub_writer_cannot_be_promoted(self):
        receipt = copy.deepcopy(EXPORT)
        receipt["source_boundary"]["native_pub_writer_promoted"] = True
        with self.assertRaises(AssertionError):
            validate_export_receipt(receipt)

    def test_private_fields_are_rejected(self):
        for base, validator in ((UI, validate_ui_receipt), (EXPORT, validate_export_receipt)):
            for key in base["privacy"]:
                receipt = copy.deepcopy(base)
                receipt["privacy"][key] = True
                with self.assertRaises(AssertionError, msg=key):
                    validator(receipt)

    def test_extra_document_identity_fields_are_rejected(self):
        for base, validator in ((UI, validate_ui_receipt), (EXPORT, validate_export_receipt)):
            for key, value in (
                ("filename", "newsletter.pub"),
                ("node_id", "node:1"),
                ("asset_sha256", "a" * 64),
                ("customer", "example"),
            ):
                receipt = copy.deepcopy(base)
                receipt[key] = value
                with self.assertRaises(AssertionError, msg=key):
                    validator(receipt)


if __name__ == "__main__":
    unittest.main()
