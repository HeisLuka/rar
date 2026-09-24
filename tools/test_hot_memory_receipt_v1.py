#!/usr/bin/env python3
import copy
import unittest

from hot_memory_receipt_v1 import HotMemoryInvalid, synthetic_contract_fixture, validate_receipt


class HotMemoryReceiptTests(unittest.TestCase):
    def test_fixture_validates_but_cannot_authorize_capacity(self):
        receipt = synthetic_contract_fixture()
        validate_receipt(receipt)
        self.assertFalse(receipt["evidence_authority"]["capacity_decision_allowed"])

    def test_eviction_order_fails_closed(self):
        receipt = synthetic_contract_fixture()
        receipt["modes"][0]["eviction"][0]["drop_layer"] = "L4"
        with self.assertRaises(HotMemoryInvalid):
            validate_receipt(receipt)

    def test_eviction_cannot_claim_increasing_rss(self):
        receipt = synthetic_contract_fixture()
        receipt["modes"][0]["eviction"][1]["rss_after_bytes"] = (
            receipt["modes"][0]["eviction"][0]["rss_after_bytes"] + 1
        )
        with self.assertRaises(HotMemoryInvalid):
            validate_receipt(receipt)

    def test_synthetic_cannot_claim_capacity_authority(self):
        receipt = synthetic_contract_fixture()
        receipt["evidence_authority"]["capacity_decision_allowed"] = True
        with self.assertRaises(HotMemoryInvalid):
            validate_receipt(receipt)

    def test_real_receipt_requires_source_free_corpus_identity(self):
        receipt = synthetic_contract_fixture()
        receipt["measurement_class"] = "real_pub_source_free"
        receipt["evidence_authority"]["real_pub_runtime"] = True
        receipt["evidence_authority"]["capacity_decision_allowed"] = True
        with self.assertRaises(HotMemoryInvalid):
            validate_receipt(receipt)


if __name__ == "__main__":
    unittest.main()
