#!/usr/bin/env python3
import copy
import unittest

from copy_ledger_v1 import (
    CopyLedgerInvalid,
    synthetic_contract_fixture,
    validate_receipt,
    with_summary,
)


class CopyLedgerTests(unittest.TestCase):
    def test_fixture_validates_and_surfaces_duplicate(self):
        receipt = synthetic_contract_fixture()
        validate_receipt(receipt)
        self.assertEqual(
            2_000_000,
            receipt["summary"]["avoidable_duplicate_bytes"],
        )
        self.assertFalse(
            receipt["evidence_authority"]["technology_decision_allowed"]
        )

    def test_avoidable_duplicate_must_preserve_semantic_identity(self):
        receipt = synthetic_contract_fixture()
        receipt["events"][1]["semantic_identity_equal"] = False
        receipt.pop("summary", None)
        with self.assertRaises(CopyLedgerInvalid):
            with_summary(receipt)

    def test_unknown_copy_class_fails_closed(self):
        receipt = synthetic_contract_fixture()
        receipt["events"][0]["copy_class"] = "magic_zero_copy"
        receipt.pop("summary", None)
        with self.assertRaises(CopyLedgerInvalid):
            with_summary(receipt)

    def test_real_authority_cannot_be_claimed_by_synthetic_fixture(self):
        receipt = synthetic_contract_fixture()
        receipt["evidence_authority"]["technology_decision_allowed"] = True
        with self.assertRaises(CopyLedgerInvalid):
            validate_receipt(receipt)

    def test_conflicting_logical_size_for_same_identity_fails(self):
        receipt = synthetic_contract_fixture()
        extra = copy.deepcopy(receipt["events"][1])
        extra["event_id"] = "story-history-second-copy"
        extra["logical_bytes"] = 1
        receipt["events"].append(extra)
        receipt.pop("summary", None)
        with self.assertRaises(CopyLedgerInvalid):
            with_summary(receipt)


if __name__ == "__main__":
    unittest.main()
