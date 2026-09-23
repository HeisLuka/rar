import copy
import json
import unittest
from pathlib import Path

from tools.validate_editor_fixed_page_trial_receipt import validate_receipt

FIXTURE = Path("packages/product/editor-fixed-page-trial/v1/fixtures/acceptance-receipt.synthetic.json")


class EditorFixedPageTrialReceiptTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.receipt = json.loads(FIXTURE.read_text(encoding="utf-8"))

    def test_synthetic_contract_passes(self):
        result = validate_receipt(copy.deepcopy(self.receipt))
        self.assertTrue(result["upstream_evidence_validated"])
        self.assertTrue(result["replacement_identity_bound_end_to_end"])
        self.assertTrue(result["native_wrap_authority_closed"])
        self.assertTrue(result["full_user_path_complete"])
        self.assertTrue(result["source_free_receipt"])

    def test_inconclusive_wrap_blocks_trial(self):
        r = copy.deepcopy(self.receipt)
        r["evidence_chain"]["auth_wrap"]["authority_class"] = "inconclusive"
        with self.assertRaises(AssertionError):
            validate_receipt(r)

    def test_non_native_wrap_receipt_blocks_trial(self):
        for key in ("validated", "native_observation", "closure_candidate"):
            r = copy.deepcopy(self.receipt)
            r["evidence_chain"]["auth_wrap"][key] = False
            with self.assertRaises(AssertionError, msg=key):
                validate_receipt(r)

    def test_any_unvalidated_upstream_receipt_blocks_trial(self):
        for key in ("package", "resize_node", "replace_image_ui", "replace_image_export"):
            r = copy.deepcopy(self.receipt)
            r["evidence_chain"][key]["validated"] = False
            with self.assertRaises(AssertionError, msg=key):
                validate_receipt(r)

    def test_replace_image_evidence_must_share_one_binding(self):
        r = copy.deepcopy(self.receipt)
        r["evidence_chain"]["replace_image_export"]["replacement_binding_id"] = "rb_ffffffffffffffffffffffffffffffff"
        with self.assertRaises(AssertionError):
            validate_receipt(r)

    def test_any_missing_user_step_blocks_trial(self):
        for key in self.receipt["user_path"]:
            r = copy.deepcopy(self.receipt)
            r["user_path"][key] = False
            with self.assertRaises(AssertionError, msg=key):
                validate_receipt(r)

    def test_export_must_contain_all_bounded_edits(self):
        for key in (
            "edited_story_present",
            "moved_geometry_present",
            "resized_geometry_present",
            "replacement_image_exact",
            "bounded_wrap_result_preserved",
            "approximations_explicit",
        ):
            r = copy.deepcopy(self.receipt)
            r["export_result"][key] = False
            with self.assertRaises(AssertionError, msg=key):
                validate_receipt(r)

        r = copy.deepcopy(self.receipt)
        r["export_result"]["blocking_loss_count"] = 1
        with self.assertRaises(AssertionError):
            validate_receipt(r)

    def test_private_customer_phase_redacts_source_identity(self):
        r = copy.deepcopy(self.receipt)
        r["trial_phase"] = "private_customer"
        r["source_fixture"]["public_lineage"] = None
        r["source_fixture"]["public_sha256"] = None
        result = validate_receipt(r)
        self.assertEqual(result["trial_phase"], "private_customer")

        leaked = copy.deepcopy(r)
        leaked["source_fixture"]["public_sha256"] = "a" * 64
        with self.assertRaises(AssertionError):
            validate_receipt(leaked)

    def test_native_save_or_source_mutation_cannot_be_claimed(self):
        for key, value in (
            ("source_pub_immutable", False),
            ("native_save_pub_claimed", True),
            ("unsupported_mutation_fails_closed", False),
            ("no_silent_source_image_fallback", False),
            ("no_hidden_network_upload", False),
        ):
            r = copy.deepcopy(self.receipt)
            r["safety"][key] = value
            with self.assertRaises(AssertionError, msg=key):
                validate_receipt(r)

    def test_private_values_are_rejected(self):
        for key in self.receipt["privacy"]:
            r = copy.deepcopy(self.receipt)
            r["privacy"][key] = True
            with self.assertRaises(AssertionError, msg=key):
                validate_receipt(r)

    def test_extra_customer_identity_fields_are_rejected(self):
        for key, value in (
            ("customer_name", "Example"),
            ("filename", "newsletter.pub"),
            ("local_path", "C:/Users/example/newsletter.pub"),
            ("document_text", "private text"),
        ):
            r = copy.deepcopy(self.receipt)
            r[key] = value
            with self.assertRaises(AssertionError, msg=key):
                validate_receipt(r)


if __name__ == "__main__":
    unittest.main()
