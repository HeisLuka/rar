#!/usr/bin/env python3
import unittest

from open_first_useful_page_v1 import OpenReceiptInvalid, synthetic_contract_fixture, validate_receipt


class OpenReceiptTests(unittest.TestCase):
    def test_fixture_validates_but_has_no_architecture_authority(self):
        receipt = synthetic_contract_fixture()
        validate_receipt(receipt)
        self.assertFalse(receipt["evidence_authority"]["architecture_decision_allowed"])
        self.assertGreater(
            receipt["summary"]["cold"]["runs_with_document_global_pre_first"], 0
        )

    def test_first_useful_page_must_be_honest(self):
        receipt = synthetic_contract_fixture()
        receipt["runs"][0]["first_useful_page_definition"]["visible_resources_ready"] = False
        with self.assertRaises(OpenReceiptInvalid):
            validate_receipt(receipt)

    def test_global_pre_first_list_is_derived_from_phases(self):
        receipt = synthetic_contract_fixture()
        receipt["runs"][0]["document_global_before_first_useful_page"] = []
        with self.assertRaises(OpenReceiptInvalid):
            validate_receipt(receipt)

    def test_final_equivalence_is_mandatory(self):
        receipt = synthetic_contract_fixture()
        receipt["final_equivalence"]["final_scene_equal"] = False
        with self.assertRaises(OpenReceiptInvalid):
            validate_receipt(receipt)

    def test_synthetic_cannot_authorize_lazy_architecture(self):
        receipt = synthetic_contract_fixture()
        receipt["evidence_authority"]["architecture_decision_allowed"] = True
        with self.assertRaises(OpenReceiptInvalid):
            validate_receipt(receipt)


if __name__ == "__main__":
    unittest.main()
